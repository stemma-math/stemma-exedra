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
//! - `compile`: for a paper, `latexmk` must make its PDF; if not, the agent
//!   is told why and works again, as many rounds as configured. What
//!   compiling made is then cleaned away.
//! - `propose`: the work is committed, with the task's provenance in
//!   trailers, and pushed to the task's branch with the job's token, which
//!   pushes nowhere else. The hub then opens a pull request, which people
//!   approve.
//!
//! A task on a paper works the same way on the paper's repository, from
//! its checkout (`papers/<group>/<paper>`), with no Lean in it.
//!
//! A machine refuses a task with a step it does not know. Whatever the
//! end, the outcome has the agent's report and the log, and the run adds
//! up what the agent consumed.
//!
//! The steps' commands, the agent's and Lake's, run in a [`Sandbox`]: a
//! container of the task's own, given only the worktree, the library's
//! shared packages read-only, the job's folder, and how the agent signs
//! in; or the machine itself, if its owner chose so (see
//! [`crate::isolation`]). Git runs on the machine, with the job's token,
//! which pushes only to the task's branch; before it does, the worktree's
//! link to the checkout's repository is put back as it was, whatever the
//! agent made of it, and no hook runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use stemma_core::api::LibraryReport;
use stemma_core::protocol::{Built, JobSpec, Outcome, TaskEnd, TaskOutcome, TaskRun, Usage};

use crate::isolation::{self, Isolator, Mount, Sandbox};
use crate::job::{self, Context, JobDir, Progress, Prompt, within};
use crate::library::{self, Log, git, git_output, quietly};
use crate::papers;
use crate::runners::Work;

/// The steps a machine knows, by name.
pub const STEPS: &[&str] = &["work", "check", "compile", "propose"];

/// Where, in the worktree, the runner writes its report.
const REPORT: &str = ".stemma/report.md";

/// How much of a failed build's log the runner is shown, in characters.
const FEEDBACK_CHARS: usize = 12_000;

/// The most of a report read, in bytes.
const MAX_REPORT: u64 = 200_000;

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
    let minutes = config.timeout_minutes.unwrap_or(120);
    let mut run = Run {
        context,
        job,
        task,
        progress,
        log: &log,
        model: model.clone(),
        lifetime: std::time::Duration::from_secs(minutes * 60 + 300),
        sandbox: None,
        worktree: None,
        usage: Usage::default(),
    };
    let ended = within(minutes, "The task", run.work()).await;
    // The container and the worktree go, even if the run is cancelled and
    // dropped, or took too long.
    run.sandbox = None;
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
    /// It passes; for the library, with its build and inspection.
    Passed(Option<Built>),
    /// It does not pass: why, in a line, and what to tell the runner.
    Refused { reason: String, feedback: String },
}

/// A task's run on this machine: its steps, in a worktree of the warm
/// checkout, their commands in its sandbox.
struct Run<'a> {
    context: &'a Arc<Context>,
    job: &'a JobSpec,
    task: &'a TaskRun,
    progress: &'a Progress,
    log: &'a Log,
    model: Option<String>,
    /// How long its container lives at most.
    lifetime: std::time::Duration,
    /// Where its commands run, once the worktree is made.
    sandbox: Option<Box<dyn Sandbox>>,
    /// The worktree, once made.
    worktree: Option<Worktree>,
    /// What the agent consumed, round after round.
    usage: Usage,
}

/// A worktree of the warm checkout, removed when dropped.
struct Worktree {
    path: PathBuf,
    warm: PathBuf,
    /// Its `.git` file, which links it to the checkout's repository.
    link: Vec<u8>,
}

impl Worktree {
    /// Puts its link to the checkout's repository back as it was, for git
    /// on the machine to use that, whatever the agent made of it.
    fn restore_link(&self) -> anyhow::Result<()> {
        let link = self.path.join(".git");
        if let Ok(metadata) = std::fs::symlink_metadata(&link) {
            if metadata.is_dir() {
                std::fs::remove_dir_all(&link)?;
            } else {
                std::fs::remove_file(&link)?;
            }
        }
        std::fs::write(&link, &self.link)?;
        Ok(())
    }
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
        match &self.task.paper {
            Some(paper) => papers::checkout_dir(self.context, &self.job.group.slug, paper),
            None => library::checkout_dir(self.context, &self.job.group.slug),
        }
    }

    /// The key of the lock on the warm checkout.
    fn lock_key(&self) -> String {
        match &self.task.paper {
            Some(paper) => papers::lock_key(&self.job.group.slug, paper),
            None => self.job.group.slug.clone(),
        }
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
        let rounds = if self.has("check") || self.has("compile") {
            config.rounds.unwrap_or(2).max(1)
        } else {
            1
        };
        let dir = JobDir::create(&self.context.work_dir, self.job)?;
        self.sandbox = Some(self.sandbox_for(&worktree, &dir).await?);
        let contained_program = match &self.context.isolator {
            Some(Isolator::Containers(containers)) => {
                containers.program(&self.task.conversation.agent.runner)
            }
            _ => None,
        };

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
                sandbox: self.sandbox.as_deref().expect("made"),
                contained_program: contained_program.clone(),
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
            let checked = if self.has("compile") && self.task.paper.is_some() {
                self.compile(&worktree).await?
            } else if self.has("check") && self.task.paper.is_none() {
                self.check(&worktree).await?
            } else {
                break;
            };
            match checked {
                Checked::Passed(passed) => {
                    built = passed;
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
        let paper = self.task.paper.is_some();
        self.progress.say(if paper {
            "Preparing the paper"
        } else {
            "Preparing the library"
        });
        let warm = self.warm();
        let lock = self.context.checkout_lock(&self.lock_key());
        let _held = lock.lock().await;
        let url = format!("{}{}", self.context.hub, self.task.repository);
        library::fetch(&warm, &url, self.token(), self.log).await?;
        let base = git_output(&warm, self.token(), &["rev-parse", "origin/main"]).await?;
        self.log.line(&format!("main is at {base}."));
        // The dependencies, as `main` needs them.
        if library::checkout(&warm, &base, self.token(), self.log).await? && !paper {
            let cache = self.context.config.lean.cache.unwrap_or(true);
            if cache {
                // As a build of `main` does, in a container of its own.
                let sandbox =
                    library::checkout_sandbox(self.context, self.job, &warm, self.lifetime).await?;
                library::get_cache(sandbox.as_ref(), &warm, self.log, self.progress).await?;
            }
        }
        // What is made in the worktree is not the repository's.
        let exclude = warm.join(".git/info/exclude");
        let excluded = std::fs::read_to_string(&exclude).unwrap_or_default();
        if !excluded.lines().any(|l| l == ".stemma/") {
            std::fs::create_dir_all(warm.join(".git/info"))?;
            std::fs::write(&exclude, format!("{excluded}\n.lake/\n.stemma/\n"))?;
        }

        let worktree = self
            .context
            .work_dir
            .join(if paper { "paper-tasks" } else { "tasks" });
        let worktree = worktree.join(format!(
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
            link: std::fs::read(worktree.join(".git"))?,
        });
        if paper {
            std::fs::create_dir_all(worktree.join(".stemma"))?;
            return Ok(base);
        }
        let lake = worktree.join(".lake");
        std::fs::create_dir_all(&lake)?;
        if warm.join(".lake/packages").is_dir() {
            // Relative, from `tasks/<task>/.lake` to
            // `libraries/<group>/.lake/packages`: the same on the machine
            // and in a container, which sees the work folder as `/stemma`.
            let packages = Path::new("../../../libraries")
                .join(&self.job.group.slug)
                .join(".lake/packages");
            symlink_dir(&packages, &lake.join("packages"))?;
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

    /// Where the task's commands run: on the machine, if its owner chose
    /// so, or in a container of the task's own, given the worktree, the
    /// checkout's packages read-only, the job's folder `dir`, the compiled
    /// inspections read-only, and how the agent signs in.
    async fn sandbox_for(&self, worktree: &Path, dir: &JobDir) -> anyhow::Result<Box<dyn Sandbox>> {
        let containers = match &self.context.isolator {
            None => anyhow::bail!("This machine does not isolate the workshop's jobs"),
            Some(Isolator::Host) => return Ok(Box::new(self.context.host())),
            Some(Isolator::Containers(containers)) => containers,
        };
        self.progress.say("Starting the task's container");
        let work_dir = &self.context.work_dir;
        let seen = |path: &Path| isolation::container_path(work_dir, path);
        let mut mounts = vec![
            Mount::Bind {
                source: worktree.to_owned(),
                target: seen(worktree),
                read_only: false,
            },
            Mount::Bind {
                source: dir.path.clone(),
                target: seen(&dir.path),
                read_only: false,
            },
        ];
        let packages = self.warm().join(".lake/packages");
        let library = self.task.paper.is_none();
        if library && packages.is_dir() {
            mounts.push(Mount::Bind {
                source: packages.clone(),
                target: seen(&packages),
                read_only: true,
            });
        }
        let inspections = library::inspection_root(self.context, true);
        if library && inspections.is_dir() {
            mounts.push(Mount::Bind {
                source: inspections.clone(),
                target: seen(&inspections),
                read_only: true,
            });
        }
        let runner = &self.task.conversation.agent.runner;
        let (pass_env, files) = containers.sign_in(runner, dirs::home_dir().as_deref())?;
        mounts.extend(files);
        let spec = containers.spec(
            self.job.id,
            isolation::owner_of(work_dir),
            mounts,
            pass_env,
            self.lifetime,
        );
        let container = containers.start(&spec, work_dir).await?;
        self.log.line(&format!(
            "The task runs in the container {}, with {}.",
            container.name(),
            containers.image
        ));
        Ok(Box::new(container))
    }

    fn sandbox(&self) -> &dyn Sandbox {
        self.sandbox.as_deref().expect("made")
    }

    /// The worktree's link to the checkout's repository, as it was.
    fn restore_link(&self) -> anyhow::Result<()> {
        match &self.worktree {
            Some(worktree) => worktree.restore_link(),
            None => Ok(()),
        }
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
            .replace(
                "{paper}",
                &task
                    .paper
                    .as_deref()
                    .map(|p| format!("p/{p}"))
                    .unwrap_or_default(),
            )
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
        let on = match &task.paper {
            Some(paper) => format!(" on the paper p/{paper}"),
            None => String::new(),
        };
        user.push_str(&format!(
            "Task w{}{on}, asked in message t{}#{} by {} (@{}): {}\n\n{}\n",
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
            .and_then(|w| read_report(&w.path))
            .filter(|r| !r.trim().is_empty());
        written.unwrap_or_else(|| said.to_owned()).trim().to_owned()
    }

    /// Whether the worktree differs from `base`, committed or not.
    async fn changed(&self, worktree: &Path, base: &str) -> anyhow::Result<bool> {
        self.restore_link()?;
        let head = git_output(worktree, self.token(), &["rev-parse", "HEAD"]).await?;
        let status = git_output(worktree, self.token(), &["status", "--porcelain"]).await?;
        Ok(head != base || !status.is_empty())
    }

    /// Builds and inspects the worktree, and checks that no definition's
    /// data newly depends on `sorry`.
    async fn check(&self, worktree: &Path) -> anyhow::Result<Checked> {
        let started = self.log.len();
        if !library::lake_build(self.sandbox(), worktree, self.log, self.progress).await? {
            let build_log = self.log.since(started);
            return Ok(Checked::Refused {
                reason: "The build does not pass".into(),
                feedback: format!(
                    "`lake build` fails. The end of what it wrote:\n\n```\n{}\n```",
                    tail_chars(&errors_of(&build_log), FEEDBACK_CHARS)
                ),
            });
        }
        // A task's container compiles no inspection for others to run: it
        // uses the one its builds compiled, or interprets it.
        let contained = self.sandbox().isolation().contained();
        let inspections = library::inspection_root(self.context, contained);
        let report = match library::inspect(
            self.sandbox(),
            &inspections,
            !contained,
            worktree,
            self.log,
            self.progress,
        )
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
        Ok(Checked::Passed(Some(Built {
            succeeded: true,
            error: None,
            log: String::new(),
            report: Some(report),
        })))
    }

    /// Compiles the paper in the worktree: `latexmk` must make its PDF.
    /// Then what compiling made is cleaned away, not to be committed.
    async fn compile(&self, worktree: &Path) -> anyhow::Result<Checked> {
        let started = self.log.len();
        let main = stemma_core::api::PAPER_MAIN;
        let made = papers::latexmk(self.sandbox(), worktree, main, self.log, self.progress).await?;
        let compiled = self.log.since(started);
        // What latexmk made, the bibliography included.
        let clean = ["-e", "$bibtex_use = 2", "-C", main];
        library::run_in(
            self.sandbox(),
            "latexmk",
            &clean,
            worktree,
            self.log,
            |_| {},
        )
        .await?;
        if !made {
            return Ok(Checked::Refused {
                reason: "The paper does not compile".into(),
                feedback: format!(
                    "`latexmk -pdf {main}` does not make the PDF. What it says is wrong:\n\n\
                     ```\n{}\n```",
                    tail_chars(&papers::errors_of(&compiled), FEEDBACK_CHARS)
                ),
            });
        }
        self.log.line("Accepted: the paper compiles.");
        Ok(Checked::Passed(None))
    }

    /// Commits the work over `base`, as one commit, with the task's
    /// provenance.
    async fn commit(&self, worktree: &Path, base: &str) -> anyhow::Result<()> {
        self.progress.say("Committing");
        self.restore_link()?;
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
        self.restore_link()?;
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

/// The report the agent wrote in `worktree`, if it is a file there (not a
/// link to one elsewhere), read up to a limit.
fn read_report(worktree: &Path) -> Option<String> {
    use std::io::Read as _;
    let mut path = worktree.to_owned();
    for part in Path::new(REPORT).components() {
        path.push(part);
        if std::fs::symlink_metadata(&path)
            .ok()?
            .file_type()
            .is_symlink()
        {
            return None;
        }
    }
    if !std::fs::symlink_metadata(&path).ok()?.is_file() {
        return None;
    }
    let mut text = String::new();
    std::fs::File::open(&path)
        .ok()?
        .take(MAX_REPORT)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
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

    #[cfg(unix)]
    #[test]
    fn reads_no_report_through_a_link() {
        let worktree = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "the machine's").unwrap();
        assert_eq!(read_report(worktree.path()), None);
        std::fs::create_dir(worktree.path().join(".stemma")).unwrap();
        std::fs::write(worktree.path().join(REPORT), "Done.").unwrap();
        assert_eq!(read_report(worktree.path()).as_deref(), Some("Done."));
        std::fs::remove_file(worktree.path().join(REPORT)).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), worktree.path().join(REPORT))
            .unwrap();
        assert_eq!(read_report(worktree.path()), None);
        std::fs::remove_dir_all(worktree.path().join(".stemma")).unwrap();
        std::os::unix::fs::symlink(outside.path(), worktree.path().join(".stemma")).unwrap();
        std::fs::rename(
            outside.path().join("secret"),
            outside.path().join("report.md"),
        )
        .unwrap();
        assert_eq!(read_report(worktree.path()), None);
    }

    #[test]
    fn puts_the_worktrees_link_back() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = Worktree {
            path: dir.path().join("w"),
            warm: dir.path().join("warm"),
            link: b"gitdir: /w/libraries/g/.git/worktrees/w\n".to_vec(),
        };
        std::fs::create_dir_all(worktree.path.join(".git/hooks")).unwrap();
        worktree.restore_link().unwrap();
        assert_eq!(
            std::fs::read(worktree.path.join(".git")).unwrap(),
            worktree.link
        );
        std::fs::write(worktree.path.join(".git"), "gitdir: elsewhere").unwrap();
        worktree.restore_link().unwrap();
        assert_eq!(
            std::fs::read(worktree.path.join(".git")).unwrap(),
            worktree.link
        );
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
