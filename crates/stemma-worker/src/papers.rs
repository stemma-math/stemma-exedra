//! LaTeX build helpers from the earlier paper executor.
use crate::isolation::Sandbox;
use crate::job::Progress;
use crate::library::{Log, run_in};
use std::path::{Path, PathBuf};
const MAX_PDF: u64 = 32 * 1024 * 1024;
pub const LATEXMK: &[&str] = &[
    "-pdf",
    "-interaction=nonstopmode",
    "-halt-on-error",
    "-file-line-error",
];

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
