//! What `cargo xui` prints, in Cargo's manner: a right-aligned verb, then
//! the message.
//!
//! Every line goes through here, so a spinner in progress stays below what
//! is printed instead of being torn by it. Colour follows Cargo's switches:
//! `--color`, then `CARGO_TERM_COLOR`, then `NO_COLOR` and whether stderr
//! is a terminal.

use std::fmt::Display;
use std::io::IsTerminal;
use std::sync::OnceLock;
use std::time::Duration;

use anstyle::{AnsiColor, Style};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

// Cargo's palette.
pub const GOOD: Style = AnsiColor::BrightGreen.on_default().bold();
pub const NOTE: Style = AnsiColor::BrightCyan.on_default().bold();
pub const WARN: Style = AnsiColor::BrightYellow.on_default().bold();
pub const ERROR: Style = AnsiColor::BrightRed.on_default().bold();
pub const DIM: Style = Style::new().dimmed();
pub const BOLD: Style = Style::new().bold();

struct Term {
    color: bool,
    bars: MultiProgress,
}

static TERM: OnceLock<Term> = OnceLock::new();

/// Sets the colour choice (`always`, `never`, `auto`); the first call wins,
/// and printing before any call means `auto`. What is written through
/// `anstream` (to stdout, say) follows the same choice.
pub fn init(choice: Option<&str>) {
    if TERM.set(new(choice)).is_ok() {
        let global = match choice
            .map(str::to_owned)
            .or_else(|| std::env::var("CARGO_TERM_COLOR").ok())
        {
            Some(ref c) if c == "always" => anstream::ColorChoice::Always,
            Some(ref c) if c == "never" => anstream::ColorChoice::Never,
            _ => anstream::ColorChoice::Auto,
        };
        global.write_global();
    }
}

fn new(choice: Option<&str>) -> Term {
    let choice = choice
        .map(str::to_owned)
        .or_else(|| std::env::var("CARGO_TERM_COLOR").ok());
    let tty = std::io::stderr().is_terminal();
    let color = match choice.as_deref() {
        Some("always") => true,
        Some("never") => false,
        _ => tty && std::env::var_os("NO_COLOR").is_none(),
    };
    let bars = MultiProgress::with_draw_target(if tty {
        ProgressDrawTarget::stderr()
    } else {
        ProgressDrawTarget::hidden()
    });
    Term { color, bars }
}

fn term() -> &'static Term {
    TERM.get_or_init(|| new(None))
}

pub fn color() -> bool {
    term().color
}

/// `text` in `style`, when colouring.
pub fn paint(style: Style, text: impl Display) -> String {
    if color() {
        format!("{style}{text}{style:#}")
    } else {
        text.to_string()
    }
}

/// Prints a line (or several) above any spinner.
pub fn println(text: impl Display) {
    let term = term();
    let text = text.to_string();
    let text = if term.color {
        text
    } else {
        anstream::adapter::strip_str(&text).to_string()
    };
    if term.bars.is_hidden() {
        eprintln!("{text}");
    } else {
        term.bars.println(text.trim_end_matches('\n')).ok();
    }
}

/// `     Compiling foo`: Cargo's status line.
pub fn status(verb: &str, message: impl Display) {
    status_in(GOOD, verb, message);
}

pub fn status_in(style: Style, verb: &str, message: impl Display) {
    println(format!("{} {message}", paint(style, format!("{verb:>12}"))));
}

pub fn warn(message: impl Display) {
    println(format!("{} {message}", paint(WARN, "warning:")));
}

pub fn note(message: impl Display) {
    println(format!("{} {message}", paint(NOTE, "note:")));
}

pub fn help(message: impl Display) {
    println(format!("{} {message}", paint(NOTE, "help:")));
}

pub fn error(message: impl Display) {
    println(format!("{} {message}", paint(ERROR, "error:")));
}

/// An error and what caused it, one cause per line, as Cargo shows them.
pub fn error_chain(failure: &anyhow::Error) {
    error(failure);
    for cause in failure.chain().skip(1) {
        println(format!("\n{}\n  {cause}", paint(BOLD, "Caused by:")));
    }
}

/// A spinner after a status verb, until finished: `    Building ⠋ foo`.
pub fn spinner(verb: &str, message: impl Into<String>) -> ProgressBar {
    let bar = term().bars.add(ProgressBar::new_spinner());
    bar.set_style(
        ProgressStyle::with_template("{prefix} {spinner} {wide_msg}")
            .expect("a valid template")
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "),
    );
    bar.set_prefix(paint(NOTE, format!("{verb:>12}")));
    bar.set_message(message.into());
    bar.enable_steady_tick(Duration::from_millis(80));
    bar
}

/// A count with its noun: `1 crate`, `3 crates`.
pub fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// A duration the way a person reads one: `850ms`, `1.24s`, `1m 05s`.
pub fn duration(elapsed: Duration) -> String {
    let millis = elapsed.as_millis();
    if millis < 1000 {
        format!("{millis}ms")
    } else if millis < 60_000 {
        format!("{:.2}s", elapsed.as_secs_f64())
    } else {
        format!("{}m {:02}s", millis / 60_000, (millis / 1000) % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_durations_as_a_person_would() {
        assert_eq!(duration(Duration::from_millis(850)), "850ms");
        assert_eq!(duration(Duration::from_millis(1240)), "1.24s");
        assert_eq!(duration(Duration::from_secs(65)), "1m 05s");
    }

    #[test]
    fn counts_with_the_right_noun() {
        assert_eq!(count(1, "crate"), "1 crate");
        assert_eq!(count(0, "crate"), "0 crates");
    }
}
