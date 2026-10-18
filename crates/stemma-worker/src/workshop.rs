//! Running a task of the workshop.
//!
//! The worker makes a fresh worktree of the library at `main`, from the
//! group's warm checkout, with the checkout's dependencies and build (so
//! that Mathlib is not fetched again, and Lake builds only what changes).
//! The task's runner works there, with the task, the conversation it came
//! from, and Lean. Then the worker checks the work:
//! the build must pass, and no definition's data may newly depend on
//! `sorry`, as the inspection says. If it does not pass, the runner is told
//! why and works again, as many rounds as configured. Work that passes is
//! committed, with the task's provenance in trailers, rebased on `main` if
//! `main` moved, checked again, and pushed with the job's token; if `main`
//! moved again meanwhile, once more, and then the task is blocked. Whatever
//! the end, the outcome has the runner's report and the log.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use stemma_core::api::LibraryReport;
use stemma_core::protocol::{Built, JobSpec, Outcome, TaskEnd, TaskOutcome, TaskRun};

use crate::job::{self, Context, JobDir, Progress, Prompt, within};
use crate::library::{self, Log, git, git_output, quietly};
use crate::runners::Work;

/// Stemma's instructions to the workshop.
pub const TASK_INSTRUCTIONS: &str = include_str!("../prompts/task.md");

/// Where, in the worktree, the runner writes its report.
const REPORT: &str = ".stemma/report.md";

/// How much of a failed build's log the runner is shown, in characters.
const FEEDBACK_CHARS: usize = 12_000;

/// Runs a task to its outcome: done, failed or blocked, with its log.
pub async fn run(
    context: &Arc<Context>,
    job: &JobSpec,
    task: &TaskRun,
    progress: &Progress,
) -> anyhow::Result<Outcome> {
    let config = context.config.workshop.clone().unwrap_or_default();
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
    };
    let minutes = config.timeout_minutes.unwrap_or(120);
    let ended = within(minutes, "The task", run.work()).await;
    // The worktree goes, even if the run is cancelled and dropped.
    run.worktree = None;
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
        commits: outcome.commits,
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
    commits: Vec<String>,
    built: Option<Built>,
}

impl Ended {
    fn failed(reason: String, report: String) -> Ended {
        Ended {
            end: TaskEnd::Failed,
            reason: Some(reason),
            report,
            commits: Vec::new(),
            built: None,
        }
    }

    fn blocked(reason: String, report: String) -> Ended {
        Ended {
            end: TaskEnd::Blocked,
            ..Ended::failed(reason, report)
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

struct Run<'a> {
    context: &'a Arc<Context>,
    job: &'a JobSpec,
    task: &'a TaskRun,
    progress: &'a Progress,
    log: &'a Log,
    model: Option<String>,
    /// The worktree, once made.
    worktree: Option<Worktree>,
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

    async fn work(&mut self) -> anyhow::Result<Ended> {
        let base = self.prepare().await?;
        let worktree = self.worktree.as_ref().expect("prepared").path.clone();
        let config = self.context.config.workshop.clone().unwrap_or_default();
        let rounds = config.rounds.unwrap_or(2).max(1);
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
                Ok(text) => said = text,
                Err(error) => {
                    self.log.line(&format!("The runner failed: {error:#}"));
                    return Ok(Ended::failed(format!("{error:#}"), self.report(&said)));
                }
            }
            if !self.changed(&worktree, &base).await? {
                self.log.line("Nothing in the library changed.");
                return Ok(Ended {
                    end: TaskEnd::Done,
                    reason: None,
                    report: self.report(&said),
                    commits: Vec::new(),
                    built: None,
                });
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
        let built = built.expect("passed");
        self.commit(&worktree, &base).await?;
        self.push(&worktree, report, built).await
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
            let cache = self
                .context
                .config
                .lean
                .as_ref()
                .and_then(|l| l.cache)
                .unwrap_or(true);
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
        let system = TASK_INSTRUCTIONS
            .replace("{agent}", &conversation.agent.display_name)
            .replace("{requester}", &conversation.requester.display_name)
            .replace("{group}", &self.job.group.name)
            .replace("{task}", &task.task.to_string())
            .replace("{report}", REPORT);
        let mut system = system;
        if let Some(more) = conversation.agent.instructions.as_deref() {
            system.push_str("\nFrom the group, for you:\n\n");
            system.push_str(more.trim());
            system.push('\n');
        }
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

    /// Pushes the commit to `main`, rebasing it first if `main` moved; if
    /// `main` moves again before it is pushed, once more, and then the task
    /// is blocked.
    async fn push(&self, worktree: &Path, report: String, built: Built) -> anyhow::Result<Ended> {
        let mut built = built;
        for attempt in 1..=2 {
            self.progress.say("Putting it in the library");
            let mut fetch = git(worktree, self.token());
            fetch.args([
                "fetch",
                "-q",
                "--force",
                "origin",
                "+refs/heads/main:refs/remotes/origin/main",
            ]);
            anyhow::ensure!(
                quietly(fetch, self.log).await?,
                "Could not fetch the library from the hub"
            );
            let main = git_output(worktree, self.token(), &["rev-parse", "origin/main"]).await?;
            let parent = git_output(worktree, self.token(), &["rev-parse", "HEAD~1"]).await?;
            if main != parent {
                self.log
                    .line(&format!("main moved to {main} meanwhile: rebasing on it."));
                // The rebased commit keeps the committer its task gave it, so
                // the worker's machine needs no git identity of its own.
                let identity = ["log", "-1", "--format=%cn%n%ce"];
                let committer = git_output(worktree, self.token(), &identity).await?;
                let (name, email) = committer
                    .split_once('\n')
                    .unwrap_or((committer.as_str(), ""));
                let mut rebase = git(worktree, self.token());
                rebase
                    .args(["-c", "commit.gpgsign=false", "rebase", "-q", "origin/main"])
                    .env("GIT_COMMITTER_NAME", name)
                    .env("GIT_COMMITTER_EMAIL", email);
                if !quietly(rebase, self.log).await? {
                    let conflicts = git_output(
                        worktree,
                        self.token(),
                        &["diff", "--name-only", "--diff-filter=U"],
                    )
                    .await
                    .unwrap_or_default();
                    let mut abort = git(worktree, self.token());
                    abort.args(["rebase", "--abort"]);
                    let _ = quietly(abort, self.log).await;
                    let files = conflicts.split_whitespace().collect::<Vec<_>>().join(", ");
                    return Ok(Ended::blocked(
                        if files.is_empty() {
                            "Main changed where the workshop worked, and the work does not \
                             apply on it"
                                .into()
                        } else {
                            format!("Main changed where the workshop worked: {files}")
                        },
                        report,
                    ));
                }
                match self.check(worktree).await? {
                    Checked::Passed(passed) => built = passed,
                    Checked::Refused { reason, .. } => {
                        return Ok(Ended::blocked(
                            format!("On main as it is now, the work does not pass: {reason}"),
                            report,
                        ));
                    }
                }
            }
            let mut push = git(worktree, self.token());
            push.args(["push", "-q", "origin", "HEAD:refs/heads/main"]);
            if quietly(push, self.log).await? {
                let commit = git_output(worktree, self.token(), &["rev-parse", "HEAD"]).await?;
                self.log.line(&format!("Pushed {commit} to main."));
                return Ok(Ended {
                    end: TaskEnd::Done,
                    reason: None,
                    report,
                    commits: vec![commit],
                    built: Some(built),
                });
            }
            self.log
                .line(&format!("The push was refused (attempt {attempt})."));
        }
        Ok(Ended::blocked(
            "Main kept moving while the workshop put its work in".into(),
            report,
        ))
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
