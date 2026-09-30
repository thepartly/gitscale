use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::io::Write;

/// Returns true if stdout is a terminal (interactive session).
pub fn is_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

/// Result status of a single repo operation.
pub enum RepoStatus {
    Ok(String),
    Skip(String),
    Fail(String),
}

/// Split a failure into its error and the `hint:` lines that follow it.
///
/// When every repo fails for the same reason — no ssh agent, a CI token the
/// server refuses — the hint is the same for each, so it is printed once
/// after the list rather than under every repo.
fn split_hint(msg: &str) -> (&str, Option<&str>) {
    match msg.split_once("\nhint: ") {
        Some((error, hint)) => (error, Some(hint)),
        None => (msg, None),
    }
}

fn note_hint(hints: &mut Vec<String>, hint: Option<&str>) {
    if let Some(hint) = hint {
        if !hints.iter().any(|h| h == hint) {
            hints.push(hint.to_string());
        }
    }
}

/// Run operations on repos in parallel with a live-updating display on TTY,
/// or sequentially to a writer for non-TTY / tests.
///
/// `interactive`: when true, use indicatif progress bars directly on stderr.
/// Returns the number of failures.
pub fn run_parallel<F>(
    heading: &str,
    names: &[String],
    interactive: bool,
    op: F,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> anyhow::Result<usize>
where
    F: Fn(&str) -> RepoStatus + Send + Sync,
{
    if names.is_empty() {
        return Ok(0);
    }

    if interactive {
        run_interactive(heading, names, &op)
    } else {
        writeln!(out, "{}", heading)?;
        run_sequential(names, &op, out, err)
    }
}

fn run_interactive<F>(
    heading: &str,
    names: &[String],
    op: &F,
) -> anyhow::Result<usize>
where
    F: Fn(&str) -> RepoStatus + Send + Sync,
{
    let mp = MultiProgress::new();

    // Heading line
    let header = mp.add(ProgressBar::new_spinner());
    header.set_style(ProgressStyle::with_template("{msg}").unwrap());
    header.set_message(heading.to_string());
    header.finish();

    // Create a progress bar per repo
    let bars: Vec<ProgressBar> = names
        .iter()
        .map(|name| {
            let pb = mp.add(ProgressBar::new_spinner());
            pb.set_style(
                ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                    .unwrap()
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
            );
            pb.set_message(format!("{:<24} waiting", name));
            pb.enable_steady_tick(std::time::Duration::from_millis(80));
            pb
        })
        .collect();

    let failed = std::sync::atomic::AtomicUsize::new(0);
    let hints = std::sync::Mutex::new(Vec::new());

    std::thread::scope(|s| {
        let handles: Vec<_> = names
            .iter()
            .zip(bars.iter())
            .map(|(name, pb)| {
                let failed = &failed;
                let hints = &hints;
                s.spawn(move || {
                    pb.set_message(format!("{:<24} running...", name));
                    let result = op(name);
                    let done_style = ProgressStyle::with_template("  {msg}").unwrap();
                    pb.set_style(done_style);
                    match &result {
                        RepoStatus::Ok(msg) => pb.finish_with_message(format!("ok    {}", msg)),
                        RepoStatus::Skip(msg) => {
                            pb.finish_with_message(format!("skip  {}", msg))
                        }
                        RepoStatus::Fail(msg) => {
                            failed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let (error, hint) = split_hint(msg);
                            note_hint(&mut hints.lock().unwrap(), hint);
                            pb.finish_with_message(format!("FAIL  {}", error));
                        }
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    });

    for hint in hints.into_inner().unwrap() {
        eprintln!("hint: {}", hint);
    }
    Ok(failed.load(std::sync::atomic::Ordering::Relaxed))
}

fn run_sequential<F>(
    names: &[String],
    op: &F,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> anyhow::Result<usize>
where
    F: Fn(&str) -> RepoStatus + Send + Sync,
{
    let mut failed = 0;
    let mut hints = Vec::new();
    for name in names {
        let result = op(name);
        match &result {
            RepoStatus::Ok(msg) => writeln!(out, "  ok    {}", msg)?,
            RepoStatus::Skip(msg) => writeln!(out, "  skip  {}", msg)?,
            RepoStatus::Fail(msg) => {
                let (error, hint) = split_hint(msg);
                note_hint(&mut hints, hint);
                writeln!(err, "  FAIL  {}", error)?;
                failed += 1;
            }
        }
    }
    for hint in hints {
        writeln!(err, "hint: {}", hint)?;
    }
    Ok(failed)
}


#[cfg(test)]
mod tests {
    use super::{run_parallel, RepoStatus};

    #[test]
    fn prints_a_shared_hint_once_after_the_failures() {
        let names = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let failed = run_parallel(
            "Pulling...",
            &names,
            false,
            |name| match name {
                "c" => RepoStatus::Fail("c: fatal: other".to_string()),
                _ => RepoStatus::Fail(format!(
                    "{}: fatal: denied\nhint: load the key\nhint: then retry",
                    name
                )),
            },
            &mut out,
            &mut err,
        )
        .unwrap();
        assert_eq!(failed, 3);
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "  FAIL  a: fatal: denied\n  FAIL  b: fatal: denied\n  FAIL  c: fatal: other\n\
             hint: load the key\nhint: then retry\n"
        );
    }
}
