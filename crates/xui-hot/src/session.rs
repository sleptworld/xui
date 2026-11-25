//! `cargo xui dev`: build, run, watch, patch.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use indicatif::ProgressBar;
use notify_debouncer_full::notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, new_debouncer};

use crate::build::{self, Target};
use crate::keys::{self, Key};
use crate::protocol::{ADDR_ENV, BUILD_ENV, FromApp, ToApp};
use crate::shapes::{self, Snapshot};
use crate::shim;
use crate::sources::{self, Change, Sources, is_source};
use crate::symbols::{self, SymbolCache};
use crate::term::{self, DIM, ERROR, NOTE, WARN, paint};

#[derive(Debug, Clone, Default)]
pub struct DevOptions {
    pub manifest_path: Option<PathBuf>,
    pub package: Option<String>,
    pub bin: Option<String>,
    /// More arguments for `cargo rustc` (`--features`, `--profile`, ..).
    pub cargo_args: Vec<String>,
    /// For the build and the app both (the assets' bootstrap and directory).
    pub envs: Vec<(OsString, OsString)>,
    /// The app's own arguments.
    pub app_args: Vec<String>,
}

enum Event {
    Changed(Vec<PathBuf>),
    App(FromApp),
    /// Run `.0`'s app exited: only the current run's counts.
    Exited(u64, Option<i32>),
    Key(Key),
}

/// How long a patch waits for the app to say it loaded it.
const LOAD_TIMEOUT: Duration = Duration::from_secs(3);
/// Then for it to say what it mounted again: that waits for the app's main
/// thread, which macOS may run slowly while the app is in the background.
const RELOAD_TIMEOUT: Duration = Duration::from_secs(15);

/// Runs the app and patches it as its sources change. `r` restarts it
/// from a fresh build, `q` (or Ctrl-C) quits; when the app exits, the next
/// change starts it again.
pub fn dev(options: DevOptions) -> Result<()> {
    let tool = std::env::current_exe()?;
    let target = build::resolve(
        options.manifest_path.as_deref(),
        options.package.as_deref(),
        options.bin.as_deref(),
    )?;
    let dir = target.target_dir.join("xui-hot").join(&target.bin);
    std::fs::create_dir_all(dir.join("patches"))?;

    let (events, inbox) = channel();
    let _watcher = watch(&target, events.clone())?;
    let key_events = events.clone();
    let _keys = keys::listen(move |key| key_events.send(Event::Key(key)).is_ok());
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let writer: Arc<Mutex<Option<TcpStream>>> = Arc::default();
    serve(listener, writer.clone(), events.clone());

    let shown = target.show(&target.root);
    term::status_in(
        NOTE,
        "Watching",
        format!(
            "{}{}  {}",
            target.package,
            if shown.is_empty() {
                String::new()
            } else {
                format!(" ({shown})")
            },
            paint(DIM, "r restart · c clear · h help · q quit")
        ),
    );

    let mut run = 0;
    let mut statics_told = Vec::new();
    loop {
        run += 1;
        let exe = match build::fat(&target, &dir, &tool, &options.cargo_args, &options.envs) {
            Ok(exe) => exe,
            Err(error) => {
                term::error_chain(&error);
                term::status_in(WARN, "Waiting", "for a change to build again");
                match wait_for_change(&inbox) {
                    Next::Quit => return Ok(()),
                    _ => continue,
                }
            }
        };
        let cache = SymbolCache::load(&exe)?;
        let bin_record = shim::rustc_record(&dir, &target.crate_name(), "bin");
        let mut sources = Sources::load(&dir, &bin_record);
        let mut roots: Vec<&Path> = target.local_roots.iter().map(PathBuf::as_path).collect();
        roots.push(&target.workspace_root);
        sources.remember_build_files(&roots);
        let snapshot = Snapshot::take(&sources.bin);
        let statics = snapshot.mutable_statics();
        if !statics.is_empty() && statics != statics_told {
            let arm64 = cache.architecture() == object::Architecture::Aarch64;
            tell_statics(&target, &statics, arm64);
        }
        statics_told = statics;
        let build = format!("{run}-{}", std::process::id());
        term::status("Running", format!("`{}`", target.show(&exe)));
        let mut child = Command::new(&exe)
            .args(&options.app_args)
            .env(ADDR_ENV, address.to_string())
            .env(BUILD_ENV, &build)
            .envs(options.envs.iter().cloned())
            // The keys are ours.
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting {}", exe.display()))?;
        forward(child.stdout.take());
        forward(child.stderr.take());
        let child = Arc::new(Mutex::new(child));
        wait_for_exit(run, child.clone(), events.clone());
        // Whichever way this run ends, its app goes with it.
        let _guard = KillOnDrop(child.clone());

        let mut app = Running {
            run,
            target: &target,
            dir: &dir,
            cache,
            bin_record,
            sources,
            snapshot,
            deferred: None,
            build,
            aslr_reference: None,
            next_patch: 1,
            pending: None,
            writer: writer.clone(),
        };
        match app.run(&inbox)? {
            Next::Restart(None) => term::status_in(NOTE, "Restarting", &target.bin),
            Next::Restart(Some(reason)) => term::status_in(
                WARN,
                "Restarting",
                format!("{} {}", target.bin, paint(DIM, format!("— {reason}"))),
            ),
            Next::Quit => return Ok(()),
            Next::Exited(code) => {
                let how = code.map_or("by a signal".to_owned(), |code| format!("with code {code}"));
                term::status_in(WARN, "Stopped", format!("the app exited {how}"));
                term::status_in(NOTE, "Waiting", "for a change, or `r`, to start it again");
                if let Next::Quit = wait_for_change(&inbox) {
                    return Ok(());
                }
            }
        }
    }
}

enum Next {
    /// Build again and start the app again; why, when it was not asked.
    Restart(Option<String>),
    Quit,
    Exited(Option<i32>),
}

/// Waits, with no app running, for a source change or a key.
fn wait_for_change(inbox: &Receiver<Event>) -> Next {
    loop {
        match inbox.recv() {
            Ok(Event::Changed(paths)) if paths.iter().any(|p| is_source(p)) => {
                return Next::Restart(None);
            }
            Ok(Event::Key(Key::Restart)) => return Next::Restart(None),
            Ok(Event::Key(Key::Quit)) | Err(_) => return Next::Quit,
            Ok(Event::Key(key)) => common_key(key),
            Ok(_) => {}
        }
    }
}

/// Warns of the binary crate's statics that hold state and a patch does
/// not share with the app: each patch starts them over. On arm64, only
/// thread-locals ([`crate::alias`] shares the rest).
fn tell_statics(target: &Target, statics: &[(String, PathBuf, bool)], arm64: bool) {
    const SHOWN: usize = 4;
    let fresh: Vec<&(String, PathBuf, bool)> = statics
        .iter()
        .filter(|(_, _, local)| *local || !arm64)
        .collect();
    if fresh.is_empty() {
        return;
    }
    let mut names: Vec<String> = fresh
        .iter()
        .take(SHOWN)
        .map(|(name, file, _)| format!("`{name}` ({})", target.show(file)))
        .collect();
    if fresh.len() > SHOWN {
        names.push(format!("{} more", fresh.len() - SHOWN));
    }
    let what = match (fresh.len(), arm64) {
        (1, true) => "a thread-local",
        (_, true) => "the thread-locals",
        (1, false) => "a static",
        (_, false) => "the statics",
    };
    term::warn(format!(
        "each patch starts {what} of `{}` over: {}",
        target.bin,
        names.join(", ")
    ));
    term::help("keep state that must outlive a patch in a component, or in another crate");
}

/// The keys that do the same whatever is running.
fn common_key(key: Key) {
    match key {
        Key::Help => {
            for (key, what) in keys::HELP {
                term::println(format!("{} {what}", paint(NOTE, format!("{key:>12}"))));
            }
        }
        Key::Clear => {
            // Erase the screen and the scrollback, cursor home.
            eprint!("\x1b[2J\x1b[3J\x1b[H");
        }
        Key::Restart | Key::Quit => {}
    }
}

/// Kills the app when its run ends.
struct KillOnDrop(Arc<Mutex<Child>>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let mut child = self.0.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Why no patch went out.
enum PatchError {
    /// The binary crate does not compile: the app keeps running.
    Compile(anyhow::Error),
    /// It compiled, but the patch did not link or could not be sent: only
    /// a fresh build gets the change in.
    Patch(anyhow::Error),
}

/// A patch sent, waiting for the app's report.
struct Pending {
    id: u64,
    files: String,
    started: Instant,
    compile: Duration,
    link: Duration,
    sent: Instant,
    /// When the app said it loaded the patch.
    applied: Option<Instant>,
    spinner: ProgressBar,
}

impl Pending {
    fn deadline(&self) -> Instant {
        match self.applied {
            Some(applied) => applied + RELOAD_TIMEOUT,
            None => self.sent + LOAD_TIMEOUT,
        }
    }
}

/// One run of the app, from its launch.
struct Running<'a> {
    run: u64,
    target: &'a Target,
    dir: &'a Path,
    cache: SymbolCache,
    /// Where the binary crate's rustc call is recorded.
    bin_record: PathBuf,
    /// Which crate each source is in.
    sources: Sources,
    /// The binary crate's sources as the app runs them.
    snapshot: Snapshot,
    /// Files changed before the app connected, to patch once it does.
    deferred: Option<String>,
    build: String,
    /// Where the app's `main` is: known once it said hello.
    aslr_reference: Option<u64>,
    next_patch: u64,
    pending: Option<Pending>,
    writer: Arc<Mutex<Option<TcpStream>>>,
}

impl Running<'_> {
    fn run(&mut self, inbox: &Receiver<Event>) -> Result<Next> {
        loop {
            let event = match &self.pending {
                Some(pending) => {
                    let left = pending.deadline().saturating_duration_since(Instant::now());
                    match inbox.recv_timeout(left) {
                        Ok(event) => event,
                        Err(RecvTimeoutError::Timeout) => {
                            self.finish(None);
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => return Ok(Next::Quit),
                    }
                }
                None => match inbox.recv() {
                    Ok(event) => event,
                    Err(_) => return Ok(Next::Quit),
                },
            };
            if let Some(next) = self.handle(event) {
                if let Some(pending) = self.pending.take() {
                    pending.spinner.finish_and_clear();
                }
                return Ok(next);
            }
        }
    }

    fn handle(&mut self, event: Event) -> Option<Next> {
        match event {
            Event::Changed(paths) => return self.changed(paths),
            Event::App(FromApp::Hello {
                pid,
                aslr_reference,
                build,
            }) => {
                if build == self.build {
                    self.aslr_reference = Some(aslr_reference);
                    term::status_in(NOTE, "Connected", paint(DIM, format!("pid {pid}")));
                    if let Some(files) = self.deferred.take() {
                        return self.start_patch(files);
                    }
                }
            }
            Event::App(FromApp::Applied {
                id,
                error: Some(error),
            }) => {
                if self.pending.as_ref().is_some_and(|p| p.id == id) {
                    let pending = self.pending.take().expect("checked");
                    pending.spinner.finish_and_clear();
                }
                term::error(format!("the app could not load patch {id}: {error}"));
                return Some(Next::Restart(Some(format!("patch {id} did not load"))));
            }
            Event::App(FromApp::Applied { id, .. }) => {
                if let Some(pending) = self.pending.as_mut().filter(|p| p.id == id) {
                    pending.applied = Some(Instant::now());
                    pending
                        .spinner
                        .set_message(format!("{} — waiting for the app to reload", pending.files));
                }
            }
            Event::App(report @ FromApp::Reloaded { .. }) => {
                if self.pending.is_some() {
                    self.finish(Some(report));
                } else {
                    // After its patch gave up waiting.
                    term::status("Reloaded", outcome(&report, None));
                }
            }
            Event::Exited(run, code) if run == self.run => return Some(Next::Exited(code)),
            Event::Exited(..) => {}
            Event::Key(Key::Restart) => return Some(Next::Restart(None)),
            Event::Key(Key::Quit) => return Some(Next::Quit),
            Event::Key(key) => common_key(key),
        }
        None
    }

    fn changed(&mut self, paths: Vec<PathBuf>) -> Option<Next> {
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter(|path| !path.starts_with(&self.target.target_dir))
            .collect();
        let target = self.target;
        let files =
            match sources::classify(&paths, &self.sources, &target.root, &target.bin, |path| {
                target.show(path)
            }) {
                Change::Ignore => return None,
                Change::Restart(reason) => return Some(Next::Restart(Some(reason))),
                Change::Patch(files) => files,
            };
        let files = files
            .iter()
            .map(|path| target.show(path))
            .collect::<Vec<_>>()
            .join(", ");
        if self.aslr_reference.is_none() {
            term::status_in(
                NOTE,
                "Waiting",
                format!("{files} changed; patching once the app connects"),
            );
            self.deferred = Some(files);
            return None;
        }
        self.start_patch(files)
    }

    /// Patches the app with the binary crate as its sources are now, or
    /// restarts it if they changed what a patch cannot carry.
    fn start_patch(&mut self, files: String) -> Option<Next> {
        let aslr_reference = self.aslr_reference?;
        let shapes::Check {
            reshaped,
            remount,
            candidate,
        } = self.snapshot.check();
        if !reshaped.is_empty() {
            let target = self.target;
            return Some(Next::Restart(Some(shapes::describe(&reshaped, |path| {
                target.show(path)
            }))));
        }
        if let Some(pending) = self.pending.take() {
            pending.spinner.finish_and_clear();
        }
        let spinner = term::spinner("Patching", format!("{files} — compiling"));
        match self.patch(aslr_reference, remount, &spinner) {
            Ok((id, compile, link, started)) => {
                spinner.set_message(format!("{files} — loading"));
                self.snapshot.commit(candidate);
                // A module declared since is the binary crate's now.
                let built = std::mem::take(&mut self.sources.built);
                self.sources = Sources::load(self.dir, &self.bin_record);
                self.sources.built = built;
                self.snapshot.track(&self.sources.bin);
                self.pending = Some(Pending {
                    id,
                    files,
                    started,
                    compile,
                    link,
                    sent: Instant::now(),
                    applied: None,
                    spinner,
                });
            }
            Err(PatchError::Compile(error)) => {
                spinner.finish_and_clear();
                term::error_chain(&error);
            }
            Err(PatchError::Patch(error)) => {
                spinner.finish_and_clear();
                term::error_chain(&error);
                return Some(Next::Restart(Some("the patch could not be made".into())));
            }
        }
        None
    }

    /// Builds a patch and sends it, asking for `remount` to be mounted
    /// again: its id, how long compiling and linking took, and when it
    /// started.
    fn patch(
        &mut self,
        aslr_reference: u64,
        remount: Vec<String>,
        spinner: &ProgressBar,
    ) -> Result<(u64, Duration, Duration, Instant), PatchError> {
        let started = Instant::now();
        let thin = build::thin(self.target, self.dir).map_err(PatchError::Compile)?;
        let compile = started.elapsed();
        self.link_and_send(thin, aslr_reference, remount, spinner)
            .map(|(id, link)| (id, compile, link, started))
            .map_err(PatchError::Patch)
    }

    /// Links the patch a thin build compiled, and sends it: its id, and
    /// how long linking took.
    fn link_and_send(
        &mut self,
        thin: build::Thin,
        aslr_reference: u64,
        remount: Vec<String>,
        spinner: &ProgressBar,
    ) -> Result<(u64, Duration)> {
        let started = Instant::now();
        spinner.set_message(spinner.message().replace("compiling", "linking"));
        let (linked_objects, shared) = self.share_statics(&thin.objects)?;
        let (stub, mirror) = symbols::stub(&self.cache, &linked_objects, aslr_reference, &shared)?;
        let stub_path = self.dir.join("stub.o");
        std::fs::write(&stub_path, stub)?;
        let id = self.next_patch;
        self.next_patch += 1;
        let library = self
            .dir
            .join("patches")
            .join(format!("libpatch-{id}.dylib"));
        let mut objects = linked_objects;
        objects.push(stub_path);
        let linked = crate::link::thin(&objects, &thin.link, &library);
        for object in &thin.objects {
            let _ = std::fs::remove_file(object);
        }
        linked?;
        let table = symbols::jump_table(&self.cache, &library)?;
        let link = started.elapsed();
        self.send(&ToApp::Patch {
            id,
            table,
            remount,
            mirror,
        })?;
        Ok((id, link))
    }

    /// Takes the binary crate's statics the app has out of `objects`, so
    /// that the patch shares them ([`crate::alias`]): the objects to link,
    /// and the statics' names.
    ///
    /// A rewritten object is a copy in the dev directory: rustc's are hard
    /// links into its incremental cache, which the next build reuses.
    fn share_statics(&self, objects: &[PathBuf]) -> Result<(Vec<PathBuf>, Vec<String>)> {
        if self.cache.architecture() != object::Architecture::Aarch64 {
            return Ok((objects.to_vec(), Vec::new()));
        }
        let dir = self.dir.join("shared");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let mut linked = Vec::with_capacity(objects.len());
        let mut shared = Vec::new();
        for path in objects {
            let bytes = std::fs::read(path)?;
            let rewritten = crate::alias::rewrite(&bytes, |name| self.cache.is_shared_static(name))
                .with_context(|| format!("sharing the statics of {}", path.display()))?;
            match rewritten {
                Some(rewritten) => {
                    let copy = dir.join(path.file_name().context("an object without a name")?);
                    std::fs::write(&copy, rewritten.bytes)?;
                    linked.push(copy);
                    shared.extend(rewritten.statics);
                }
                None => linked.push(path.clone()),
            }
        }
        Ok((linked, shared))
    }

    /// Prints how the pending patch went: with the app's report, or
    /// without, when it gave none in time.
    fn finish(&mut self, report: Option<FromApp>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        pending.spinner.finish_and_clear();
        let total = term::duration(pending.started.elapsed());
        let mut timings = vec![
            format!("compile {}", term::duration(pending.compile)),
            format!("link {}", term::duration(pending.link)),
        ];
        // Time the app took to get to it, when that was noticeable.
        if let Some(applied) = pending.applied
            && report.is_some()
            && applied.elapsed() > Duration::from_millis(250)
        {
            timings.push(format!("app waited {}", term::duration(applied.elapsed())));
        }
        let outcome = match &report {
            Some(report) => outcome(report, Some(&timings)),
            None if pending.applied.is_some() => format!(
                "{} {}",
                paint(WARN, "loaded; the app has not reloaded yet"),
                paint(DIM, format!("({})", timings.join(" · ")))
            ),
            None => format!(
                "{} {}",
                paint(WARN, "the app did not answer"),
                paint(DIM, format!("({})", timings.join(" · ")))
            ),
        };
        term::status(
            "Patched",
            format!("{} in {total} · {outcome}", pending.files),
        );
    }

    fn send(&self, message: &ToApp) -> Result<()> {
        let mut writer = self.writer.lock().unwrap();
        let stream = writer.as_mut().context("the app is not connected")?;
        let mut line = serde_json::to_vec(message)?;
        line.push(b'\n');
        stream
            .write_all(&line)
            .context("sending the patch to the app")?;
        Ok(())
    }
}

/// What a reload report says: how many components were mounted again and
/// kept, then `timings` and the reload's own time, dim.
fn outcome(report: &FromApp, timings: Option<&[String]>) -> String {
    let FromApp::Reloaded {
        changed,
        called,
        renamed,
        kept,
        micros,
        full,
        ..
    } = report
    else {
        return String::new();
    };
    let mut details: Vec<String> = timings.map(<[String]>::to_vec).unwrap_or_default();
    details.push(format!("reload {}", micros_text(*micros)));
    let details = paint(DIM, format!("({})", details.join(" · ")));
    if *full {
        return format!("{} {details}", paint(WARN, "everything mounted again"));
    }
    let mut again = format!("{} mounted again", term::count(*changed, "component"));
    if *called > 0 {
        again.push_str(&format!(" ({called} for code it calls)"));
    }
    let mut parts = vec![again, format!("{kept} kept")];
    if *renamed > 0 {
        parts.push(paint(
            WARN,
            format!("{renamed} reset (renamed by the compiler)"),
        ));
    }
    format!("{} {details}", parts.join(" · "))
}

fn micros_text(micros: u64) -> String {
    if micros < 1000 {
        format!("{micros}µs")
    } else {
        format!("{:.1}ms", micros as f64 / 1000.0)
    }
}

type Watcher = notify_debouncer_full::Debouncer<
    notify_debouncer_full::notify::RecommendedWatcher,
    notify_debouncer_full::RecommendedCache,
>;

/// Watches the local packages the binary is built from, and the
/// workspace's manifest and lockfile; a burst of events (one save is
/// several) as one change.
fn watch(target: &Target, events: Sender<Event>) -> Result<Watcher> {
    let mut debouncer = new_debouncer(
        Duration::from_millis(80),
        None,
        move |result: DebounceEventResult| {
            let Ok(batch) = result else { return };
            let paths: Vec<PathBuf> = batch
                .into_iter()
                .filter(|event| matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)))
                .flat_map(|event| event.event.paths)
                .collect();
            if !paths.is_empty() {
                let _ = events.send(Event::Changed(paths));
            }
        },
    )
    .context("starting the file watcher")?;
    let mut watched: Vec<&Path> = Vec::new();
    for root in &target.local_roots {
        if watched.iter().any(|dir| root.starts_with(dir)) {
            continue;
        }
        debouncer
            .watch(root, RecursiveMode::Recursive)
            .with_context(|| format!("watching {}", root.display()))?;
        watched.push(root);
    }
    if !watched
        .iter()
        .any(|dir| target.workspace_root.starts_with(dir))
    {
        debouncer
            .watch(&target.workspace_root, RecursiveMode::NonRecursive)
            .with_context(|| format!("watching {}", target.workspace_root.display()))?;
    }
    Ok(debouncer)
}

/// Prints the app's output, each line marked as the app's.
fn forward(stream: Option<impl std::io::Read + Send + 'static>) {
    let Some(stream) = stream else { return };
    std::thread::spawn(move || {
        let bar = paint(DIM, "│");
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            term::println(format!("{bar} {line}"));
        }
    });
}

/// Accepts the app's connection (a restarted app makes a new one) and
/// reads its messages.
fn serve(listener: TcpListener, writer: Arc<Mutex<Option<TcpStream>>>, events: Sender<Event>) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let Ok(write_half) = stream.try_clone() else {
                continue;
            };
            *writer.lock().unwrap() = Some(write_half);
            let events = events.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    match serde_json::from_str::<FromApp>(&line) {
                        Ok(message) => {
                            if events.send(Event::App(message)).is_err() {
                                break;
                            }
                        }
                        Err(error) => term::println(format!(
                            "{} a message from the app: {error}",
                            paint(ERROR, "bad")
                        )),
                    }
                }
            });
        }
    });
}

fn wait_for_exit(run: u64, child: Arc<Mutex<Child>>, events: Sender<Event>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let status = child.lock().unwrap().try_wait();
            match status {
                Ok(Some(status)) => {
                    let _ = events.send(Event::Exited(run, status.code()));
                    return;
                }
                Ok(None) => {}
                Err(_) => return,
            }
        }
    });
}
