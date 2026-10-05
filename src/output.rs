//! How output looks: whether a stream gets colour and icons, the result
//! lines of multi-repository commands, and the headers forwarding prints
//! between repositories.
//!
//! Colour is decided once per command and per stream — stdout and stderr
//! can differ, as when one is piped — and kept for the thread running the
//! command. Words never change with it: a result line says `ok`, `skip` or
//! `FAIL` with colour or without, so scripts, tests and screen readers read
//! the same text.

use std::cell::Cell;

/// `--color`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

/// What one command's output goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Streams {
    /// stdout is a terminal.
    pub stdout_tty: bool,
    /// stderr is a terminal.
    pub stderr_tty: bool,
}

/// The decision for one command: colour on each stream, and what each is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub stdout: bool,
    pub stderr: bool,
    pub streams: Streams,
}

thread_local! {
    static CURRENT: Cell<Style> = const { Cell::new(Style {
        stdout: false,
        stderr: false,
        streams: Streams { stdout_tty: false, stderr_tty: false },
    }) };
}

/// Whether a stream gets colour: `--color` first, then `NO_COLOR` (set at
/// all) or `TERM=dumb` turn it off, and otherwise a terminal gets it.
pub fn decide(choice: ColorChoice, is_tty: bool, var: &dyn Fn(&str) -> Option<String>) -> bool {
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            if var("NO_COLOR").is_some() || var("TERM").as_deref() == Some("dumb") {
                return false;
            }
            is_tty
        }
    }
}

/// Decide for `streams` and keep it for this thread.
pub fn init(choice: ColorChoice, streams: Streams) -> Style {
    let var = |name: &str| std::env::var_os(name).map(|v| v.to_string_lossy().into_owned());
    let style = Style {
        stdout: decide(choice, streams.stdout_tty, &var),
        stderr: decide(choice, streams.stderr_tty, &var),
        streams,
    };
    set(style);
    style
}

/// Keep `style` for this thread: a worker thread takes its command's.
pub fn set(style: Style) {
    CURRENT.with(|c| c.set(style));
}

pub fn current() -> Style {
    CURRENT.with(Cell::get)
}

/// Whether stdout gets colour.
pub fn stdout() -> bool {
    current().stdout
}

/// Whether stderr gets colour.
pub fn stderr() -> bool {
    current().stderr
}

pub const GREEN: &str = "32";
pub const YELLOW: &str = "33";
pub const CYAN: &str = "36";
pub const DIM: &str = "2";
pub const BOLD: &str = "1";
pub const BOLD_RED: &str = "1;31";

/// `text` in the SGR `code` when `on`, as it is otherwise.
pub fn paint(on: bool, code: &str, text: &str) -> String {
    if on && !text.is_empty() {
        format!("\x1b[{}m{}\x1b[0m", code, text)
    } else {
        text.to_string()
    }
}

/// What a result line reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Skip,
    Fail,
}

/// One result line, without its newline. Without colour, exactly
/// `  ok    msg`; with it, an icon in front and the word coloured.
pub fn result_line(outcome: Outcome, msg: &str, color: bool) -> String {
    let (word, icon, code) = match outcome {
        Outcome::Ok => ("ok  ", "✔", GREEN),
        Outcome::Skip => ("skip", "○", DIM),
        Outcome::Fail => ("FAIL", "✘", BOLD_RED),
    };
    if !color {
        return format!("  {}  {}", word, msg);
    }
    format!(
        "  {} {}  {}",
        paint(true, code, icon),
        paint(true, code, word),
        msg
    )
}

/// A `hint:` line, yellow with colour.
pub fn hint(text: &str, color: bool) -> String {
    format!("{} {}", paint(color, YELLOW, "hint:"), text)
}

/// The rule that ends a header off a terminal.
const PLAIN_RULE: usize = 16;

/// A header before one repository's output: `── 2/5 imports/core ──…`, with
/// `skip (REASON)` for a repository that is not run. `count` is `(n, total)`;
/// `None` for the placement's header. `width` is the terminal's, which the
/// rule fills; off a terminal the rule is 16 dashes.
pub fn header(
    count: Option<(usize, usize)>,
    name: &str,
    skip: Option<&str>,
    width: Option<usize>,
    color: bool,
) -> String {
    let mut text = String::new();
    let mut shown = 0usize;
    let mut push = |text: &mut String, part: String, len: usize| {
        text.push_str(&part);
        shown += len;
    };
    push(&mut text, paint(color, DIM, "──"), 2);
    push(&mut text, " ".to_string(), 1);
    if let Some((n, total)) = count {
        let digits = total.to_string().len();
        let n = if total >= 10 {
            format!("{:>width$}", n, width = digits)
        } else {
            n.to_string()
        };
        let counted = format!("{}/{}", n, total);
        let len = counted.chars().count();
        push(&mut text, paint(color, DIM, &counted), len);
        push(&mut text, " ".to_string(), 1);
    }
    push(&mut text, paint(color, BOLD, name), name.chars().count());
    push(&mut text, " ".to_string(), 1);
    if let Some(reason) = skip {
        push(&mut text, paint(color, DIM, "──"), 2);
        let said = format!(" skip ({}) ", reason);
        let len = said.chars().count();
        push(&mut text, paint(color, DIM, &said), len);
    }
    let rule = match width {
        Some(width) => width.saturating_sub(shown).max(2),
        None => PLAIN_RULE,
    };
    text.push_str(&paint(color, DIM, &"─".repeat(rule)));
    text
}

/// The width of the terminal on stdout, when it is one.
pub fn terminal_width() -> Option<usize> {
    let (_, cols) = console::Term::stdout().size_checked()?;
    Some(cols as usize)
}

/// The header width to use for this command: the terminal's when stdout is
/// one.
pub fn header_width() -> Option<usize> {
    if current().streams.stdout_tty {
        terminal_width()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn colour_follows_the_flag_then_the_environment_then_the_terminal() {
        assert!(decide(ColorChoice::Always, false, &none));
        assert!(!decide(ColorChoice::Never, true, &none));
        assert!(decide(ColorChoice::Auto, true, &none));
        assert!(!decide(ColorChoice::Auto, false, &none));
        let no_color = |n: &str| (n == "NO_COLOR").then(String::new);
        assert!(!decide(ColorChoice::Auto, true, &no_color));
        assert!(decide(ColorChoice::Always, true, &no_color));
        let dumb = |n: &str| (n == "TERM").then(|| "dumb".to_string());
        assert!(!decide(ColorChoice::Auto, true, &dumb));
    }

    #[test]
    fn plain_result_lines_are_the_words_alone() {
        assert_eq!(result_line(Outcome::Ok, "a", false), "  ok    a");
        assert_eq!(result_line(Outcome::Skip, "a", false), "  skip  a");
        assert_eq!(result_line(Outcome::Fail, "a", false), "  FAIL  a");
        let coloured = result_line(Outcome::Fail, "a", true);
        assert!(coloured.contains('✘') && coloured.contains("FAIL"));
    }

    #[test]
    fn a_plain_header_ends_in_sixteen_dashes() {
        assert_eq!(
            header(Some((1, 3)), "imports/core", None, None, false),
            format!("── 1/3 imports/core {}", "─".repeat(16))
        );
        assert_eq!(
            header(Some((2, 12)), ".", Some("nothing to commit"), None, false),
            format!("──  2/12 . ── skip (nothing to commit) {}", "─".repeat(16))
        );
        assert_eq!(
            header(None, "placing", None, None, false),
            format!("── placing {}", "─".repeat(16))
        );
    }

    #[test]
    fn a_terminal_header_fills_the_width() {
        let line = header(Some((1, 3)), "core", None, Some(40), false);
        assert_eq!(line.chars().count(), 40);
    }
}
