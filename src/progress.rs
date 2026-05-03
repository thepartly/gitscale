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

    std::thread::scope(|s| {
        let handles: Vec<_> = names
            .iter()
            .zip(bars.iter())
            .map(|(name, pb)| {
                let failed = &failed;
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
                            pb.finish_with_message(format!("FAIL  {}", msg));
                        }
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    });

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
    for name in names {
        let result = op(name);
        match &result {
            RepoStatus::Ok(msg) => writeln!(out, "  ok    {}", msg)?,
            RepoStatus::Skip(msg) => writeln!(out, "  skip  {}", msg)?,
            RepoStatus::Fail(msg) => {
                writeln!(err, "  FAIL  {}", msg)?;
                failed += 1;
            }
        }
    }
    Ok(failed)
}

