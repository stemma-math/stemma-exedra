//! Running a task of the workshop, by the steps of its architecture.
//!
//! An architecture (in the repository's `workshop/architectures/`, which the
//! hub sends with the task) names its steps, which a machine runs by name:
//!
//! - `work`: the machine makes a fresh worktree of the repository at
//!   `main`, from the group's warm checkout, with the checkout's
//!   dependencies and build (so that Mathlib is not fetched again, and Lake
//!   builds only what changes); the agent works there, with the task, the
//!   conversation it came from, the architecture's instructions, Lean and
//!   Stemma's tools.
//! - `check`: the build must pass, and no definition's data may newly
//!   depend on `sorry`, as the inspection says. If not, the agent is told
//!   why and works again, as many rounds as configured.
//! - `propose`: the work is committed, with the task's provenance in
//!   trailers, and pushed to the task's branch with the job's token, which
//!   pushes nowhere else. The hub then opens a pull request, which people
//!   approve.
//!
//! A machine refuses a task with a step it does not know. Whatever the
//! end, the outcome has the agent's report and the log, and the run adds
//! up what the agent consumed. How the steps are run (here, in a worktree
//! on the machine itself) is behind [`Run`], so that another way, such as
//! a container, can take its place.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use stemma_core::api::LibraryReport;
use stemma_core::protocol::{Built, JobSpec, Outcome, TaskEnd, TaskOutcome, TaskRun, Usage};

use crate::job::{self, Context, JobDir, Progress, Prompt, within};
use crate::library::{self, Log, git, git_output, quietly};
use crate::runners::Work;

/// The steps a machine knows, by name.
pub const STEPS: &[&str] = &["work", "check", "propose"];

/// Where, in the worktree, the runner writes its report.
const REPORT: &str = ".stemma/report.md";

/// How much of a failed build's log the runner is shown, in characters.
const FEEDBACK_CHARS: usize = 12_000;

/// Runs a task to its outcome, done or failed, with its log; adds what the
/// agent consumed to `usage`.
pub async fn run(
    context: &Arc<Context>,
    job: &JobSpec,
    task: &TaskRun,
    progress: &Progress,
    usage: &mut Usage,
) -> anyhow::Result<Outcome> {
    if let Some(unknown) = task
        .architecture
        .steps
        .iter()
        .find(|step| !STEPS.contains(&step.as_str()))
    {
        anyhow::bail!(
            "This machine does not know the step “{unknown}” of the architecture {}: update \
             Stemma on it",
            task.architecture.name
        );
    }
    anyhow::ensure!(
        task.architecture.steps.iter().any(|s| s == "work"),
        "The architecture {} has no step where its agent works",
        task.architecture.name
    );
    let config = context.config.workshop.clone();
    let agent = &task.conversation.agent;
    let model = agent.model.clone().or_else(|| {
        job::cli_runner(context, &agent.runner)
            .ok()
            .and_then(|c| c.model)
    });
    let log = Log::default();
    let mut run = Run {
        context,
        job,
        task,
        progress,
        log: &log,
        model: model.clone(),
        worktree: None,
        usage: Usage::default(),
    };
    let minutes = config.timeout_minutes.unwrap_or(120);
    let ended = within(minutes, "The task", run.work()).await;
    // The worktree goes, even if the run is cancelled and dropped.
    run.worktree = None;
    usage.add(&run.usage);
    let outcome = match ended {
        Ok(outcome) => outcome,
        Err(error) => {
            log.line(&format!("{error:#}"));
            Ended::failed(format!("{error:#}"), String::new())
        }
    };
    Ok(Outcome::Task(Box::new(TaskOutcome {
        end: outcome.end,
        reason: outcome.reason,
        report: outcome.report,
        commit: outcome.commit,
        log: log.text(),
        built: outcome.built,
        runner: agent.runner.clone(),
        model,
    })))
}

/// How a run ended, before its log is added.
struct Ended {
    end: TaskEnd,
    reason: Option<String>,
    report: String,
    commit: Option<String>,
    built: Option<Built>,
}

impl Ended {
    fn failed(reason: String, report: String) -> Ended {
        Ended {
            end: TaskEnd::Failed,
            reason: Some(reason),
            report,
            commit: None,
            built: None,
        }
    }
}

/// What checking the work found.
enum Checked {
    Passed(Built),
    /// It does not pass: why, in a line, and what to tell the runner.
    Refused {
        reason: String,
        feedback: String,
    },
}

/// A task's run on this machine: its steps, in a worktree of the warm
/// checkout. The seam where another way of running them (in a container)
/// fits.
struct Run<'a> {
    context: &'a Arc<Context>,
    job: &'a JobSpec,
    task: &'a TaskRun,
    progress: &'a Progress,
    log: &'a Log,
    model: Option<String>,
    /// The worktree, once made.
    worktree: Option<Worktree>,
    /// What the agent consumed, round after round.
    usage: Usage,
}

/// A worktree of the warm checkout, removed when dropped.
struct Worktree {
    path: PathBuf,
    warm: PathBuf,
}

impl Drop for Worktree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
        let _ = std::process::Command::new("git")
            .current_dir(&self.warm)
            .args(["worktree", "prune"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

impl Run<'_> {
    fn warm(&self) -> PathBuf {
        library::checkout_dir(self.context, &self.job.group.slug)
    }

    fn token(&self) -> &str {
        &self.job.token
    }

    fn has(&self, step: &str) -> bool {
        self.task.architecture.steps.iter().any(|s| s == step)
    }

    async fn work(&mut self) -> anyhow::Result<Ended> {
        let base = self.prepare().await?;
        let worktree = self.worktree.as_ref().expect("prepared").path.clone();
        let config = self.context.config.workshop.clone();
        let rounds = if self.has("check") {
            config.rounds.unwrap_or(2).max(1)
        } else {
            1
        };
        let dir = JobDir::create(&self.context.work_dir, self.job)?;

        let mut feedback: Option<String> = None;
        let mut said = String::new();
        let mut built = None;
        for round in 1..=rounds {
            self.log.line(&format!(
                "── Round {round}: {} works on the task, with {}.",
                self.task.conversation.agent.display_name, self.task.conversation.agent.runner
            ));
            self.progress.say(if round == 1 {
                "Working".to_owned()
            } else {
                format!("Working again (round {round})")
            });
            let prompt = self.prompt(feedback.as_deref());
            let work = Work {
                dir: &worktree,
                edits: true,
                max_turns: Some(config.max_turns.unwrap_or(80)),
            };
            match job::run_cli(
                self.context,
                self.job,
                &self.task.conversation.agent,
                &dir,
                &work,
                &prompt,
                self.progress,
            )
            .await
            {
                Ok(answer) => {
                    self.usage.add(&answer.usage);
                    said = answer.text;
                }
                Err(error) => {
                    self.log.line(&format!("The agent failed: {error:#}"));
                    return Ok(Ended::failed(format!("{error:#}"), self.report(&said)));
                }
            }
            if !self.changed(&worktree, &base).await? {
                self.log.line("Nothing in the repository changed.");
                return Ok(Ended {
                    end: TaskEnd::Done,
                    reason: None,
                    report: self.report(&said),
                    commit: None,
                    built: None,
                });
            }
            if !self.has("check") {
                break;
            }
            match self.check(&worktree).await? {
                Checked::Passed(passed) => {
                    built = Some(passed);
                    break;
                }
                Checked::Refused {
                    reason,
                    feedback: told,
                } => {
                    self.log.line(&format!("Not accepted: {reason}"));
                    if round == rounds {
                        return Ok(Ended::failed(reason, self.report(&said)));
                    }
                    feedback = Some(told);
                }
            }
        }
        let report = self.report(&said);
        if !self.has("propose") {
            self.log
                .line("The architecture proposes nothing: the work is left out.");
            return Ok(Ended {
                end: TaskEnd::Done,
                reason: None,
                report,
                commit: None,
                built,
            });
        }
        self.commit(&worktree, &base).await?;
        self.propose(&worktree, report, built).await
    }

    /// Brings the warm checkout up to date and makes the worktree, at
    /// `main`, with the checkout's dependencies and build. Returns the
    /// commit it is at.
    async fn prepare(&mut self) -> anyhow::Result<String> {
        self.progress.say("Preparing the library");
        let warm = self.warm();
        let lock = self.context.checkout_lock(&self.job.group.slug);
        let _held = lock.lock().await;
        let url = format!("{}{}", self.context.hub, self.task.repository);
        library::fetch(&warm, &url, self.token(), self.log).await?;
        let base = git_output(&warm, self.token(), &["rev-parse", "origin/main"]).await?;
        self.log.line(&format!("main is at {base}."));
        // The dependencies, as `main` needs them.
        if library::checkout(&warm, &base, self.token(), self.log).await? {
            let cache = self.context.config.lean.cache.unwrap_or(true);
            if cache {
                library::get_cache(&warm, self.log, self.progress).await?;
            }
        }
        // What is made in the worktree is not the library's.
        let exclude = warm.join(".git/info/exclude");
        let excluded = std::fs::read_to_string(&exclude).unwrap_or_default();
        if !excluded.lines().any(|l| l == ".stemma/") {
            std::fs::create_dir_all(warm.join(".git/info"))?;
            std::fs::write(&exclude, format!("{excluded}\n.lake/\n.stemma/\n"))?;
        }

        let worktree = self.context.work_dir.join("tasks").join(format!(
            "{}-w{}-{:08x}",
            self.job.group.slug,
            self.task.task,
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(worktree.parent().expect("in a folder"))?;
        let mut prune = git(&warm, self.token());
        prune.args(["worktree", "prune"]);
        quietly(prune, self.log).await?;
        let mut add = git(&warm, self.token());
        add.args(["worktree", "add", "-q", "--detach"])
            .arg(&worktree)
            .arg(&base);
        anyhow::ensure!(
            quietly(add, self.log).await?,
            "Could not make a worktree of the library"
        );
        self.worktree = Some(Worktree {
            path: worktree.clone(),
            warm: warm.clone(),
        });
        let lake = worktree.join(".lake");
        std::fs::create_dir_all(&lake)?;
        if warm.join(".lake/packages").is_dir() {
            symlink_dir(&warm.join(".lake/packages"), &lake.join("packages"))?;
        }
        for kept in ["build", "stemma"] {
            let from = warm.join(".lake").join(kept);
            if from.is_dir() {
                copy_dir(&from, &lake.join(kept))?;
            }
        }
        std::fs::create_dir_all(worktree.join(".stemma"))?;
        Ok(base)
    }

    /// The runner's prompt: the task and its conversation, and in later
    /// rounds, why the work did not pass.
    fn prompt(&self, feedback: Option<&str>) -> Prompt {
        let task = self.task;
        let conversation = &task.conversation;
        let system = task
            .architecture
            .instructions
            .replace("{agent}", &conversation.agent.display_name)
            .replace("{requester}", &conversation.requester.display_name)
            .replace("{group}", &self.job.group.name)
            .replace("{task}", &task.task.to_string())
            .replace("{report}", REPORT);
        let mut user = job::conversation_text(&self.job.group, conversation);
        user.push_str("---\n\n");
        if !task.data_sorry.is_empty() {
            user.push_str(&format!(
                "These definitions' data already depends on `sorry`: {}. Leave them, unless \
                 the task is about them.\n\n",
                task.data_sorry.join(", ")
            ));
        }
        user.push_str(&format!(
            "Task w{}, asked in message t{}#{} by {} (@{}): {}\n\n{}\n",
            task.task,
            conversation.thread.number,
            conversation.trigger,
            conversation.requester.display_name,
            conversation.requester.username,
            task.title,
            task.instructions.trim()
        ));
        if let Some(feedback) = feedback {
            user.push_str(&format!(
                "\n---\n\nYou worked on this task already, and your work is in this folder, but \
                 it does not pass yet:\n\n{feedback}\n\nFix it, keeping the statements faithful \
                 to what was asked, and write your report again.\n"
            ));
        }
        Prompt { system, user }
    }

    /// The runner's report: what it wrote in the report's file, or else
    /// what it ended with.
    fn report(&self, said: &str) -> String {
        let written = self
            .worktree
            .as_ref()
            .and_then(|w| std::fs::read_to_string(w.path.join(REPORT)).ok())
            .filter(|r| !r.trim().is_empty());
        written.unwrap_or_else(|| said.to_owned()).trim().to_owned()
    }

    /// Whether the worktree differs from `base`, committed or not.
    async fn changed(&self, worktree: &Path, base: &str) -> anyhow::Result<bool> {
        let head = git_output(worktree, self.token(), &["rev-parse", "HEAD"]).await?;
        let status = git_output(worktree, self.token(), &["status", "--porcelain"]).await?;
        Ok(head != base || !status.is_empty())
    }

    /// Builds and inspects the worktree, and checks that no definition's
    /// data newly depends on `sorry`.
    async fn check(&self, worktree: &Path) -> anyhow::Result<Checked> {
        let started = self.log.len();
        if !library::lake_build(worktree, self.log, self.progress).await? {
            let build_log = self.log.since(started);
            return Ok(Checked::Refused {
                reason: "The build does not pass".into(),
                feedback: format!(
                    "`lake build` fails. The end of what it wrote:\n\n```\n{}\n```",
                    tail_chars(&errors_of(&build_log), FEEDBACK_CHARS)
                ),
            });
        }
        let report =
            match library::inspect(&self.context.work_dir, worktree, self.log, self.progress)
                .await?
            {
                Ok(report) => report,
                Err(error) => {
                    return Ok(Checked::Refused {
                        reason: error.clone(),
                        feedback: format!("The library could not be inspected: {error}"),
                    });
                }
            };
        let new_data_sorry = new_data_sorry(&report, &self.task.data_sorry);
        if !new_data_sorry.is_empty() {
            let names = new_data_sorry.join(", ");
            return Ok(Checked::Refused {
                reason: format!("The data of {names} depends on sorry"),
                feedback: format!(
                    "The data of these definitions depends on `sorry`, which a definition must \
                     not: {names}. Complete them; `sorry` goes only in proofs."
                ),
            });
        }
        let sorry = report.declarations.iter().filter(|d| d.sorry).count();
        self.log.line(&format!(
            "Accepted: the build passes, and no definition's data newly depends on sorry ({} \
             declarations, {sorry} depending on sorry).",
            report.declarations.len()
        ));
        Ok(Checked::Passed(Built {
            succeeded: true,
            error: None,
            log: String::new(),
            report: Some(report),
        }))
    }

    /// Commits the work over `base`, as one commit, with the task's
    /// provenance.
    async fn commit(&self, worktree: &Path, base: &str) -> anyhow::Result<()> {
        self.progress.say("Committing");
        let task = self.task;
        let conversation = &task.conversation;
        // The runner's own commits, if it made any, become one.
        let mut reset = git(worktree, self.token());
        reset.args(["reset", "-q", "--soft", base]);
        anyhow::ensure!(quietly(reset, self.log).await?, "git reset failed");
        let mut add = git(worktree, self.token());
        add.args(["add", "-A"]);
        anyhow::ensure!(quietly(add, self.log).await?, "git add failed");
        let mut message = format!(
            "{}\n\nTask: w{}\nOrigin: t{}#{}\nRunner: {}\n",
            task.title,
            task.task,
            conversation.thread.number,
            conversation.trigger,
            conversation.agent.runner
        );
        if let Some(model) = &self.model {
            message.push_str(&format!("Model: {model}\n"));
        }
        let author = format!(
            "{}, for {}",
            conversation.agent.display_name, conversation.requester.display_name
        );
        let email = format!("{}@stemma", conversation.requester.username);
        let mut commit = git(worktree, self.token());
        commit
            .args([
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "--no-verify",
                "-m",
            ])
            .arg(&message)
            .env("GIT_AUTHOR_NAME", &author)
            .env("GIT_AUTHOR_EMAIL", &email)
            .env("GIT_COMMITTER_NAME", &author)
            .env("GIT_COMMITTER_EMAIL", &email);
        anyhow::ensure!(quietly(commit, self.log).await?, "git commit failed");
        Ok(())
    }

    /// Pushes the commit to the task's branch, for the hub to propose it.
    async fn propose(
        &self,
        worktree: &Path,
        report: String,
        built: Option<Built>,
    ) -> anyhow::Result<Ended> {
        self.progress.say("Proposing the change");
        let branch = &self.task.branch;
        let mut push = git(worktree, self.token());
        push.args([
            "push",
            "-q",
            "--force",
            "origin",
            &format!("HEAD:refs/heads/{branch}"),
        ]);
        anyhow::ensure!(
            quietly(push, self.log).await?,
            "Could not push the work to the branch {branch}"
        );
        let commit = git_output(worktree, self.token(), &["rev-parse", "HEAD"]).await?;
        self.log.line(&format!("Pushed {commit} to {branch}."));
        Ok(Ended {
            end: TaskEnd::Done,
            reason: None,
            report,
            commit: Some(commit),
            built,
        })
    }
}

/// The definitions whose data depends on `sorry` and did not before.
fn new_data_sorry(report: &LibraryReport, before: &[String]) -> Vec<String> {
    report
        .declarations
        .iter()
        .filter(|d| d.data_sorry && !before.contains(&d.name))
        .map(|d| d.name.clone())
        .collect()
}

/// The lines of a build's log that say what is wrong, or else all of it.
fn errors_of(log: &str) -> String {
    let errors: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("error") || line.starts_with("✖"))
        .collect();
    if errors.is_empty() {
        log.to_owned()
    } else {
        errors.join("\n")
    }
}

/// The last `max` characters of `text`.
fn tail_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_owned();
    }
    let rest: String = text.chars().skip(count - max).collect();
    format!("[…]\n{rest}")
}

/// Copies a folder, as it is.
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_dir(&path, &target)?;
        } else if kind.is_file() {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn symlink_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(from, to)
}

#[cfg(not(unix))]
fn symlink_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    copy_dir(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stemma_core::api::Declaration;

    #[test]
    fn finds_new_data_sorry() {
        let declaration = |name: &str, data_sorry: bool| Declaration {
            name: name.into(),
            module: "M".into(),
            kind: "definition".into(),
            type_: "Nat".into(),
            doc: None,
            range: None,
            sorry: data_sorry,
            data_sorry,
            axioms: Vec::new(),
            uses: Vec::new(),
        };
        let report = LibraryReport {
            lean: "4".into(),
            modules: vec!["M".into()],
            declarations: vec![
                declaration("old", true),
                declaration("new", true),
                declaration("fine", false),
            ],
        };
        assert_eq!(new_data_sorry(&report, &["old".into()]), ["new"]);
    }

    #[test]
    fn shows_the_errors_of_a_build() {
        let log = "$ lake build\n✔ [1/3] Built A\nerror: B.lean:3:0: unknown identifier\n✖ [2/3] Building B";
        assert_eq!(
            errors_of(log),
            "error: B.lean:3:0: unknown identifier\n✖ [2/3] Building B"
        );
        assert_eq!(tail_chars("abcdef", 3), "[…]\ndef");
    }
}
