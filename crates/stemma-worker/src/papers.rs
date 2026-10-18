//! Builds of a group's working papers: their LaTeX compiled into a PDF.
//!
//! A workshop machine with LaTeX keeps a checkout of each paper in its
//! work folder (`papers/<group>/<paper>`). To build a commit it fetches
//! `main` from the hub with the job's token, checks the commit out, and
//! runs `latexmk -pdf` on its main file (pdfLaTeX, and BibTeX for the
//! references) where the machine isolates the workshop's jobs: in a
//! container of the build's own, given the checkout with its `.git`
//! hidden, or on the machine itself if its owner chose so. If it made the
//! PDF, the machine uploads it to the hub with the job's token, and then
//! says how the build went, with its log.

use std::path::{Path, PathBuf};

use stemma_core::protocol::{JobSpec, Outcome, PaperBuilt, PaperCompile};

use crate::isolation::{Container, Isolator, Mount, Sandbox};
use crate::job::{Context, Progress, within};
use crate::library::{self, Log, run_in};

/// The most a PDF may be, in bytes, to be read and uploaded.
const MAX_PDF: u64 = 200 * 1024 * 1024;

/// How `latexmk` is run: to make a PDF with pdfLaTeX, never stopping to
/// ask, at the first error, with errors as `file:line: message`.
pub const LATEXMK: &[&str] = &[
    "-pdf",
    "-interaction=nonstopmode",
    "-halt-on-error",
    "-file-line-error",
];

/// The checkout of a group's paper.
pub fn checkout_dir(context: &Context, group: &str, paper: &str) -> PathBuf {
    context.work_dir.join("papers").join(group).join(paper)
}

/// The key of the lock on a paper's checkout.
pub fn lock_key(group: &str, paper: &str) -> String {
    format!("{group}/p/{paper}")
}

/// Compiles a commit of a paper, uploads its PDF and says how it went: a
/// build that does not make the PDF is an outcome too, with its log.
pub async fn build(
    context: &Context,
    job: &JobSpec,
    compile: &PaperCompile,
    progress: &Progress,
) -> anyhow::Result<Outcome> {
    let minutes = context.config.latex.timeout_minutes.unwrap_or(15);
    let log = Log::default();
    let dir = checkout_dir(context, &job.group.slug, &compile.paper);
    let lock = context.checkout_lock(&lock_key(&job.group.slug, &compile.paper));
    let work = async {
        let _held = lock.lock().await;
        progress.say(format!(
            "Fetching {}",
            &compile.commit[..compile.commit.len().min(10)]
        ));
        let url = format!("{}{}", context.hub, compile.repository);
        if let Err(error) = library::fetch(&dir, &url, &job.token, &log).await {
            return Ok(failed(&format!("{error:#}")));
        }
        if !library::checkout(&dir, &compile.commit, &job.token, &log).await? {
            return Ok(failed("The commit is not on main"));
        }
        let lifetime = std::time::Duration::from_secs(minutes * 60 + 300);
        let sandbox = checkout_sandbox(context, job, &dir, lifetime).await?;
        if !latexmk(sandbox.as_ref(), &dir, &compile.main, &log, progress).await? {
            return Ok(failed("latexmk did not make the PDF: see the log"));
        }
        let pdf = match read_pdf(&dir, &compile.main) {
            Ok(pdf) => pdf,
            Err(error) => return Ok(failed(&format!("{error:#}"))),
        };
        progress.say("Uploading the PDF");
        log.line(&format!("Made the PDF, {} bytes.", pdf.len()));
        let hub = stemma_client::Hub::new(&context.hub, Some(job.token.clone()))?;
        hub.put_pdf(&compile.upload, pdf).await?;
        anyhow::Ok(PaperBuilt {
            succeeded: true,
            error: None,
            log: String::new(),
            pdf: true,
        })
    };
    let built = match within(minutes, "The build", work).await {
        Ok(built) => built,
        Err(error) => {
            log.line(&format!("{error:#}"));
            failed(&format!("{error:#}"))
        }
    };
    Ok(Outcome::PaperBuilt(Box::new(PaperBuilt {
        log: log.text(),
        ..built
    })))
}

/// A build that made no PDF, for `error`.
fn failed(error: &str) -> PaperBuilt {
    PaperBuilt {
        succeeded: false,
        error: Some(error.to_owned()),
        log: String::new(),
        pdf: false,
    }
}

/// Where a paper's checkout `dir` is compiled: on the machine, if its
/// owner chose so, or in a container of the job's own, given the checkout
/// with its `.git` hidden.
pub async fn checkout_sandbox(
    context: &Context,
    job: &JobSpec,
    dir: &Path,
    lifetime: std::time::Duration,
) -> anyhow::Result<Box<dyn Sandbox>> {
    match &context.isolator {
        None => anyhow::bail!("This machine does not isolate the workshop's jobs"),
        Some(Isolator::Host) => Ok(Box::new(context.host())),
        Some(Isolator::Containers(containers)) => {
            let seen = |path: &Path| crate::isolation::container_path(&context.work_dir, path);
            let mounts = vec![
                Mount::Bind {
                    source: dir.to_owned(),
                    target: seen(dir),
                    read_only: false,
                },
                Mount::Hidden {
                    target: seen(&dir.join(".git")),
                },
            ];
            let spec = containers.spec(
                job.id,
                crate::isolation::owner_of(&context.work_dir),
                mounts,
                Vec::new(),
                lifetime,
            );
            let container: Container = containers.start(&spec, &context.work_dir).await?;
            Ok(Box::new(container))
        }
    }
}

/// Runs `latexmk` on `main` in `dir`; whether it made the PDF.
pub async fn latexmk(
    sandbox: &dyn Sandbox,
    dir: &Path,
    main: &str,
    log: &Log,
    progress: &Progress,
) -> anyhow::Result<bool> {
    progress.say("Compiling");
    let started = std::time::Instant::now();
    let mut args: Vec<&str> = LATEXMK.to_vec();
    args.push(main);
    let said = progress.clone();
    let status = run_in(sandbox, "latexmk", &args, dir, log, move |line| {
        if let Some(step) = compile_progress(line) {
            said.say(step);
        }
    })
    .await?;
    if status.success() {
        log.line(&format!(
            "Compiled in {:.1}s.",
            started.elapsed().as_secs_f64()
        ));
    }
    Ok(status.success() && pdf_of(dir, main).is_file())
}

/// What `latexmk` says it runs, for people watching: `Running pdflatex`.
fn compile_progress(line: &str) -> Option<String> {
    let rest = line.strip_prefix("Latexmk: applying rule '")?;
    let rule = rest.split('\'').next()?;
    let program = rule.split_whitespace().next().unwrap_or(rule);
    Some(format!("Running {program}"))
}

/// The PDF `main` makes, beside it.
fn pdf_of(dir: &Path, main: &str) -> PathBuf {
    dir.join(Path::new(main).with_extension("pdf"))
}

/// The PDF compiled in `dir`, if it is a file there (not a link to one
/// elsewhere) and not too large.
pub fn read_pdf(dir: &Path, main: &str) -> anyhow::Result<Vec<u8>> {
    let path = pdf_of(dir, main);
    let metadata = std::fs::symlink_metadata(&path)?;
    anyhow::ensure!(metadata.is_file(), "{} is not a file", path.display());
    anyhow::ensure!(
        metadata.len() <= MAX_PDF,
        "The PDF is larger than {} MB",
        MAX_PDF / 1024 / 1024
    );
    Ok(std::fs::read(&path)?)
}

/// The lines of `latexmk`'s log that say what is wrong, or else its end.
pub fn errors_of(log: &str) -> String {
    let lines: Vec<&str> = log.lines().collect();
    let mut errors: Vec<&str> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with('!') || line.contains(":error:") || is_file_line_error(line) {
            // An error, and the lines that show where.
            errors.extend(lines.iter().skip(i).take(3));
        }
    }
    if errors.is_empty() {
        let start = lines.len().saturating_sub(40);
        lines[start..].join("\n")
    } else {
        errors.dedup();
        errors.join("\n")
    }
}

/// Whether a line is an error as `-file-line-error` writes it:
/// `./main.tex:12: Undefined control sequence.`
fn is_file_line_error(line: &str) -> bool {
    let mut parts = line.splitn(3, ':');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(file), Some(number), Some(_)) => {
            file.ends_with(".tex")
                && !number.is_empty()
                && number.bytes().all(|b| b.is_ascii_digit())
        }
        _ => false,
    }
}

/// Whether LaTeX is installed: `latexmk` on the path.
pub fn has_latex() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    crate::detect::find_program("latexmk", Some(&path)).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_latexmks_progress() {
        assert_eq!(
            compile_progress("Latexmk: applying rule 'pdflatex'...").as_deref(),
            Some("Running pdflatex")
        );
        assert_eq!(
            compile_progress("Latexmk: applying rule 'bibtex main'...").as_deref(),
            Some("Running bibtex")
        );
        assert_eq!(compile_progress("This is pdfTeX"), None);
    }

    #[test]
    fn shows_the_errors_of_a_compilation() {
        let log = "This is pdfTeX\n./main.tex:12: Undefined control sequence.\nl.12 \\foo\n\n\
                   Latexmk: Errors";
        assert_eq!(
            errors_of(log),
            "./main.tex:12: Undefined control sequence.\nl.12 \\foo\n"
        );
        assert_eq!(errors_of("all fine"), "all fine");
    }

    #[cfg(unix)]
    #[test]
    fn reads_no_pdf_through_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "%PDF-the machine's").unwrap();
        assert!(read_pdf(dir.path(), "main.tex").is_err());
        std::os::unix::fs::symlink(outside.path().join("secret"), dir.path().join("main.pdf"))
            .unwrap();
        assert!(read_pdf(dir.path(), "main.tex").is_err());
        std::fs::remove_file(dir.path().join("main.pdf")).unwrap();
        std::fs::write(dir.path().join("main.pdf"), "%PDF-1.5").unwrap();
        assert_eq!(read_pdf(dir.path(), "main.tex").unwrap(), b"%PDF-1.5");
    }
}
