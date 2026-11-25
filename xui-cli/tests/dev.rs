//! `cargo xui dev` end to end: runs it on a fresh app, edits the app's
//! sources step by step, and checks what dev says and what the app prints.
//!
//! Ignored by default: the first run builds the app with every dependency
//! linked whole (several minutes; later runs reuse
//! `target/xui-dev-e2e`). macOS only, as hot patching is. Run it with
//!
//! ```sh
//! cargo test -p xui-cli --test dev -- --ignored --nocapture
//! ```
//!
//! The app opens a window; it runs with the screen off too. Event handlers
//! are not covered: nothing here can click.
#![cfg(target_os = "macos")]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};
use std::{fs, thread};

const BUILD: Duration = Duration::from_secs(30 * 60);
const REBUILD: Duration = Duration::from_secs(5 * 60);
const PATCH: Duration = Duration::from_secs(2 * 60);

const MAIN: &str = r#"
mod model;

use std::sync::atomic::{AtomicUsize, Ordering};

use xui::prelude::*;
use xui::{MacRunnerOptions, PhysicalSize, WindowOptions};

#[component]
fn home() {
    view! {
        column(gap: 12.0) {
            greeting()
            counter()
        }
    }
}

#[component]
fn greeting() {
    let seen: u8 = mounted("greeting");
    view! { text(label()) }
}

#[component]
fn counter() {
    let seen: u8 = mounted("counter");
    let count: i32 = state!(0);
    view! { text(format!("{count}")) }
}

/// A helper a prop calls: the prop reads no signal.
fn label() -> String {
    let note = model::Note { text: "label v1" };
    println!("xui-e2e label {} hits {}", note.text, HITS.load(Ordering::Relaxed));
    note.text.to_owned()
}

fn mounted(name: &str) -> u8 {
    println!("xui-e2e mounted {name}");
    HITS.fetch_add(1, Ordering::Relaxed);
    0
}

static HITS: AtomicUsize = AtomicUsize::new(0);

fn root(app: &mut App) {
    app.mount::<Home>(HomeProps {});
}

#[xui::main]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = MacRunnerOptions {
        window: WindowOptions::default()
            .with_title("xui-e2e")
            .with_inner_size(PhysicalSize::new(320, 200)),
        ..Default::default()
    };
    xui::runner(root, Some(options)).run()?;
    Ok(())
}
"#;

const MODEL: &str = "pub struct Note {\n    pub text: &'static str,\n}\n";

#[test]
#[ignore = "builds a whole app; run with --ignored"]
fn dev_patches_what_it_can_and_restarts_for_the_rest() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = workspace();
    let status = Command::new(env!("CARGO_BIN_EXE_cargo-xui"))
        .current_dir(temp.path())
        .args([
            "xui",
            "new",
            "hello",
            "--vcs",
            "none",
            "--offline",
            "--quiet",
        ])
        .status()
        .unwrap();
    assert!(status.success(), "cargo xui new failed");
    let app = temp.path().join("hello");
    fs::copy(workspace.join("Cargo.lock"), app.join("Cargo.lock")).unwrap();
    fs::write(app.join("src/main.rs"), MAIN).unwrap();
    fs::write(app.join("src/model.rs"), MODEL).unwrap();
    let main = app.join("src/main.rs");

    let mut dev = Dev::start(&app, &workspace.join("target/xui-dev-e2e"));

    // Launch: every component mounted. The static is shared with each
    // patch, so not warned about.
    let at = dev.mark();
    dev.expect(at, BUILD, "connected", |l| l.contains("Connected"));
    dev.expect(at, PATCH, "the label", |l| {
        l.contains("xui-e2e label label v1")
    });
    dev.expect(at, PATCH, "counter mounted", |l| {
        l.contains("xui-e2e mounted counter")
    });
    // Cargo rewrote the lockfile it was given as it built: no change.
    dev.absent(at, "a restart at launch", |l| l.contains("Restarting"));
    dev.absent(at, "a statics warning", |l| l.contains("each patch starts"));

    // A helper a kept component calls: that component, and only it, again.
    let at = dev.mark();
    edit(&main, "label v1", "label v2");
    let outcome = dev.reload(at);
    assert!(
        outcome.contains("1 component mounted again (1 for code it calls)"),
        "{outcome}"
    );
    // The patch's code counts on from the app's: greeting and counter
    // mounted, then greeting again.
    dev.expect(at, PATCH, "the new label, the count shared", |l| {
        l.contains("xui-e2e label label v2 hits 3")
    });
    dev.settle();
    dev.absent(at, "counter mounted again", |l| {
        l.contains("xui-e2e mounted counter")
    });

    // A component's own code.
    let at = dev.mark();
    edit(&main, "text(label())", "text(label() + \"!\")");
    let outcome = dev.reload(at);
    assert!(outcome.contains("1 component mounted again"), "{outcome}");
    assert!(!outcome.contains("for code it calls"), "{outcome}");
    dev.expect(at, PATCH, "greeting mounted again", |l| {
        l.contains("xui-e2e mounted greeting")
    });

    // The same source written again: nothing mounted.
    let at = dev.mark();
    let same = fs::read_to_string(&main).unwrap();
    fs::write(&main, same).unwrap();
    let outcome = dev.reload(at);
    assert!(outcome.contains("0 components mounted again"), "{outcome}");

    // A source that does not compile: the app keeps running; once it
    // does, the patch goes in.
    let at = dev.mark();
    edit(&main, "note.text.to_owned()", "note.text.to_owned(");
    dev.expect(at, PATCH, "the compile error", |l| {
        l.contains("does not compile; the app keeps running the last version")
    });
    let at = dev.mark();
    edit(&main, "note.text.to_owned(", "note.text.to_owned()");
    dev.reload(at);

    // Patch after patch.
    for version in 3..=7 {
        let at = dev.mark();
        edit(
            &main,
            &format!("label v{}", version - 1),
            &format!("label v{version}"),
        );
        dev.reload(at);
        dev.expect(at, PATCH, "the label", |l| {
            l.contains(&format!("xui-e2e label label v{version}"))
        });
    }

    // A type the app holds values of: rebuilt, not patched.
    let at = dev.mark();
    edit(
        &app.join("src/model.rs"),
        "pub text: &'static str,",
        "pub text: &'static str,\n    pub done: bool,",
    );
    edit(
        &main,
        "text: \"label v7\" }",
        "text: \"label v7\", done: false }",
    );
    dev.expect(at, PATCH, "a restart for the struct", |l| {
        l.contains("Restarting") && l.contains("struct `Note` (src/model.rs) changed shape")
    });
    dev.expect(at, REBUILD, "connected again", |l| l.contains("Connected"));

    // The manifest: rebuilt.
    let at = dev.mark();
    let manifest = app.join("Cargo.toml");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(&manifest, text + "\n# touched\n").unwrap();
    dev.expect(at, PATCH, "a restart for Cargo.toml", |l| {
        l.contains("Restarting") && l.contains("Cargo.toml changed")
    });
    dev.expect(at, REBUILD, "connected again", |l| l.contains("Connected"));

    // And patching still works after a restart.
    let at = dev.mark();
    edit(&main, "label v7", "label v8");
    dev.reload(at);
    dev.expect(at, PATCH, "the label", |l| {
        l.contains("xui-e2e label label v8")
    });

    dev.quit();
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|dir| dir.join("Cargo.lock").is_file())
        .expect("xui-cli is built inside the xui workspace")
        .to_owned()
}

/// Replaces the one `from` in `file` with `to`.
fn edit(file: &Path, from: &str, to: &str) {
    let text = fs::read_to_string(file).unwrap();
    assert_eq!(
        text.matches(from).count(),
        1,
        "`{from}` once in {}",
        file.display()
    );
    fs::write(file, text.replacen(from, to, 1)).unwrap();
}

/// A running `cargo xui dev` and every line it printed (the app's too).
struct Dev {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    log: Vec<String>,
}

impl Dev {
    fn start(app: &Path, target_dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cargo-xui"))
            .current_dir(app)
            .args(["xui", "dev", "--color", "never"])
            .env("CARGO_TARGET_DIR", target_dir)
            .env("CARGO_NET_OFFLINE", "true")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let (send, lines) = channel();
        for stream in [
            Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
            Box::new(child.stderr.take().unwrap()),
        ] {
            let send = send.clone();
            thread::spawn(move || {
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    if std::env::var_os("XUI_E2E_ECHO").is_some() {
                        eprintln!("{line}");
                    }
                    if send.send(line).is_err() {
                        break;
                    }
                }
            });
        }
        Self {
            stdin: child.stdin.take(),
            child,
            lines,
            log: Vec::new(),
        }
    }

    /// Where the next step's lines start.
    fn mark(&mut self) -> usize {
        self.drain();
        self.log.len()
    }

    fn drain(&mut self) {
        while let Ok(line) = self.lines.try_recv() {
            self.log.push(line);
        }
    }

    /// The first line since `at` that `is` takes, waiting up to `timeout`.
    /// From `at`, not from the last line looked at: the app's lines and
    /// dev's come by separate pipes, in either order.
    fn expect(
        &mut self,
        at: usize,
        timeout: Duration,
        what: &str,
        is: impl Fn(&str) -> bool,
    ) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            self.drain();
            if let Some(line) = self.log[at..].iter().find(|line| is(line)) {
                return line.clone();
            }
            if Instant::now() >= deadline {
                self.fail(&format!("no {what} in {timeout:?}"));
            }
            match self.lines.recv_timeout(Duration::from_millis(200)) {
                Ok(line) => self.log.push(line),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.fail(&format!("dev exited before {what}"))
                }
            }
        }
    }

    /// What the app said it mounted again for the patch since `at`.
    fn reload(&mut self, at: usize) -> String {
        self.expect(at, PATCH, "reload", |line| {
            (line.contains("Patched") || line.contains("Reloaded"))
                && line.contains("mounted again")
        })
    }

    /// Lets the app's output catch up.
    fn settle(&mut self) {
        thread::sleep(Duration::from_millis(500));
        self.drain();
    }

    fn absent(&mut self, at: usize, what: &str, is: impl Fn(&str) -> bool) {
        self.drain();
        if self.log[at..].iter().any(|line| is(line)) {
            self.fail(&format!("unexpected: {what}"));
        }
    }

    fn quit(mut self) {
        self.send("q");
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if let Ok(Some(status)) = self.child.try_wait() {
                assert!(status.success(), "dev exited with {status}");
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        self.fail("dev did not quit on `q`");
    }

    fn send(&mut self, line: &str) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = writeln!(stdin, "{line}");
            let _ = stdin.flush();
        }
    }

    fn fail(&mut self, message: &str) -> ! {
        self.drain();
        let tail = self.log.len().saturating_sub(60);
        panic!(
            "{message}\n--- the last lines of `cargo xui dev` ---\n{}",
            self.log[tail..].join("\n")
        );
    }
}

impl Drop for Dev {
    /// `q` first: dev stops the app it started. Killed, it would not.
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            self.send("q");
            for _ in 0..100 {
                if !matches!(self.child.try_wait(), Ok(None)) {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
