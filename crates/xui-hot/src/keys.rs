//! Single-key commands while `cargo xui dev` runs.
//!
//! On a terminal, stdin goes into "cbreak" mode: no line buffering, no
//! echo, but output processing and signals as usual, so the app's lines
//! still print as lines and Ctrl-C still interrupts. The terminal is put
//! back when the [`Keys`] guard drops, and on Ctrl-C. Elsewhere (stdin a
//! pipe), a command is a line.

use std::io::{BufRead, IsTerminal, Read};
use std::sync::Mutex;

/// A command typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Restart,
    Quit,
    Help,
    Clear,
}

impl Key {
    fn from_byte(byte: u8) -> Option<Self> {
        match byte.to_ascii_lowercase() {
            b'r' => Some(Self::Restart),
            b'q' => Some(Self::Quit),
            b'h' | b'?' => Some(Self::Help),
            b'c' => Some(Self::Clear),
            _ => None,
        }
    }
}

/// What a key does, for `h`.
pub const HELP: &[(&str, &str)] = &[
    ("r", "rebuild and restart the app"),
    ("c", "clear the screen"),
    ("h", "show this"),
    ("q", "quit (or Ctrl-C)"),
];

/// The terminal's settings before cbreak, to put back.
#[cfg(unix)]
static SAVED: Mutex<Option<rustix::termios::Termios>> = Mutex::new(None);

/// Puts the terminal back as it was. Idempotent.
pub fn restore() {
    #[cfg(unix)]
    if let Ok(mut saved) = SAVED.lock()
        && let Some(termios) = saved.take()
    {
        let _ = rustix::termios::tcsetattr(
            rustix::stdio::stdin(),
            rustix::termios::OptionalActions::Now,
            &termios,
        );
    }
}

/// Restores the terminal when dropped.
pub struct Keys(());

impl Drop for Keys {
    fn drop(&mut self) {
        restore();
    }
}

/// Reads commands from stdin on a thread of its own, handing each to
/// `on_key`; stops when `on_key` returns false.
pub fn listen(mut on_key: impl FnMut(Key) -> bool + Send + 'static) -> Keys {
    let terminal = std::io::stdin().is_terminal() && cbreak();
    // Ctrl-C ends the process; the terminal must not stay in cbreak.
    let _ = ctrlc::set_handler(|| {
        restore();
        std::process::exit(130);
    });
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        if terminal {
            let mut byte = [0];
            while stdin.lock().read_exact(&mut byte).is_ok() {
                if let Some(key) = Key::from_byte(byte[0])
                    && !on_key(key)
                {
                    return;
                }
            }
        } else {
            for line in stdin.lock().lines() {
                let Ok(line) = line else { return };
                let Some(&first) = line.trim().as_bytes().first() else {
                    continue;
                };
                if let Some(key) = Key::from_byte(first)
                    && !on_key(key)
                {
                    return;
                }
            }
        }
    });
    Keys(())
}

#[cfg(unix)]
fn cbreak() -> bool {
    use rustix::termios::{LocalModes, OptionalActions, SpecialCodeIndex, tcgetattr, tcsetattr};
    let stdin = rustix::stdio::stdin();
    let Ok(original) = tcgetattr(stdin) else {
        return false;
    };
    let mut termios = original.clone();
    termios.local_modes &= !(LocalModes::ICANON | LocalModes::ECHO);
    termios.special_codes[SpecialCodeIndex::VMIN] = 1;
    termios.special_codes[SpecialCodeIndex::VTIME] = 0;
    if tcsetattr(stdin, OptionalActions::Now, &termios).is_err() {
        return false;
    }
    *SAVED.lock().unwrap() = Some(original);
    true
}

#[cfg(not(unix))]
fn cbreak() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_keys_either_case() {
        assert_eq!(Key::from_byte(b'r'), Some(Key::Restart));
        assert_eq!(Key::from_byte(b'Q'), Some(Key::Quit));
        assert_eq!(Key::from_byte(b'?'), Some(Key::Help));
        assert_eq!(Key::from_byte(b'x'), None);
    }
}
