use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Processor, Rgb};
use anyhow::{Context, Result};
use lanes_core::session::LaunchSpec;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

#[derive(Clone)]
pub struct Listener(Sender<Event>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.0.send(event);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSize {
    pub columns: usize,
    pub rows: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

type Writer = Arc<Mutex<Box<dyn Write + Send>>>;

/// One interactive claude process in a ConPTY, parsed into an alacritty grid.
pub struct PtySession {
    pub term: Arc<FairMutex<Term<Listener>>>,
    parser: Arc<Mutex<Processor>>,
    writer: Writer,
    master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    dirty: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    /// Unix millis of the last byte claude wrote; its spinner redraws continuously while it works.
    last_output: Arc<AtomicU64>,
    size: Mutex<GridSize>,
}

impl PtySession {
    pub fn spawn(spec: &LaunchSpec, size: GridSize) -> Result<Arc<Self>> {
        let pair = native_pty_system().openpty(pty_size(size)).context("opening a pseudo console")?;
        let child = pair.slave.spawn_command(build_command(spec)?).context("starting claude")?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let reader = pair.master.try_clone_reader()?;
        let writer: Writer = Arc::new(Mutex::new(pair.master.take_writer()?));

        let (events_tx, events_rx) = channel();
        let term = Term::new(Config::default(), &size, Listener(events_tx));
        let session = Arc::new(Self {
            term: Arc::new(FairMutex::new(term)),
            parser: Arc::new(Mutex::new(Processor::new())),
            writer,
            master: Arc::new(Mutex::new(Some(pair.master))),
            killer: Mutex::new(killer),
            dirty: Arc::new(AtomicBool::new(true)),
            exited: Arc::new(AtomicBool::new(false)),
            last_output: Arc::new(AtomicU64::new(unix_millis())),
            size: Mutex::new(size),
        });
        session.start_reader(reader);
        session.start_event_pump(events_rx);
        session.start_exit_watcher(child);
        Ok(session)
    }

    pub fn write(&self, bytes: &[u8]) {
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer.write_all(bytes);
            let _ = writer.flush();
        }
    }

    pub fn resize(&self, size: GridSize) {
        let mut current = self.size.lock().unwrap();
        if *current == size || size.columns < 2 || size.rows < 2 {
            return;
        }
        *current = size;
        if let Some(master) = self.master.lock().unwrap().as_ref() {
            let _ = master.resize(pty_size(size));
        }
        self.term.lock().resize(size);
        self.mark_dirty();
    }

    pub fn size(&self) -> GridSize {
        *self.size.lock().unwrap()
    }

    /// Returns true when the grid changed since the last call; also flushes synchronized-update buffers that timed out.
    pub fn take_dirty(&self) -> bool {
        self.flush_expired_sync();
        self.dirty.swap(false, Ordering::AcqRel)
    }

    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
    }

    pub fn has_exited(&self) -> bool {
        self.exited.load(Ordering::Acquire)
    }

    pub fn quiet_for(&self) -> Duration {
        Duration::from_millis(unix_millis().saturating_sub(self.last_output.load(Ordering::Acquire)))
    }

    pub fn kill(&self) {
        let _ = self.killer.lock().unwrap().kill();
    }

    fn start_reader(self: &Arc<Self>, mut reader: Box<dyn Read + Send>) {
        let (term, parser, dirty, last_output) = (self.term.clone(), self.parser.clone(), self.dirty.clone(), self.last_output.clone());
        std::thread::spawn(move || {
            let mut buffer = vec![0u8; 64 * 1024];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                let mut term = term.lock();
                parser.lock().unwrap().advance(&mut *term, &buffer[..read]);
                dirty.store(true, Ordering::Release);
                last_output.store(unix_millis(), Ordering::Release);
            }
        });
    }

    fn start_event_pump(self: &Arc<Self>, events: Receiver<Event>) {
        let (writer, size) = (self.writer.clone(), Arc::downgrade(self));
        std::thread::spawn(move || {
            for event in events {
                let reply = match event {
                    Event::PtyWrite(text) => Some(text),
                    Event::ColorRequest(_, format) => Some(format(Rgb { r: 0xd4, g: 0xd4, b: 0xd4 })),
                    Event::TextAreaSizeRequest(format) => size.upgrade().map(|s| format(window_size(s.size()))),
                    _ => None,
                };
                if let Some(reply) = reply {
                    let _ = writer.lock().map(|mut w| w.write_all(reply.as_bytes()));
                }
            }
        });
    }

    fn start_exit_watcher(self: &Arc<Self>, mut child: Box<dyn portable_pty::Child + Send + Sync>) {
        let (master, exited, dirty) = (self.master.clone(), self.exited.clone(), self.dirty.clone());
        std::thread::spawn(move || {
            let _ = child.wait();
            exited.store(true, Ordering::Release);
            dirty.store(true, Ordering::Release);
            // ConPTY keeps the output pipe open until the pseudo console is closed, so drop the master to end the reader.
            master.lock().unwrap().take();
        });
    }

    fn flush_expired_sync(&self) {
        let mut parser = self.parser.lock().unwrap();
        let expired = parser.sync_timeout().sync_timeout().is_some_and(|deadline| deadline <= Instant::now());
        if expired {
            parser.stop_sync(&mut *self.term.lock());
            self.mark_dirty();
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        self.kill();
    }
}

fn build_command(spec: &LaunchSpec) -> Result<CommandBuilder> {
    let mut command = CommandBuilder::new(resolve_program(&spec.program));
    command.args(&spec.args);
    command.cwd(&spec.cwd);
    for key in lanes_core::launch::inherited_session_vars() {
        command.env_remove(key);
    }
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    Ok(command)
}

/// Resolves a bare program name to its .exe on PATH, since portable-pty would otherwise prefer an extensionless npm shim.
pub fn resolve_program(program: &str) -> PathBuf {
    let path = Path::new(program);
    if path.is_absolute() || path.extension().is_some() {
        return path.to_path_buf();
    }
    let exe = format!("{program}.exe");
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(&exe))
        .find(|candidate| candidate.is_file())
        .or_else(|| dirs_local_bin(&exe))
        .unwrap_or_else(|| path.to_path_buf())
}

fn dirs_local_bin(exe: &str) -> Option<PathBuf> {
    let candidate = std::env::var_os("USERPROFILE").map(PathBuf::from)?.join(".local").join("bin").join(exe);
    candidate.is_file().then_some(candidate)
}

fn pty_size(size: GridSize) -> PtySize {
    PtySize { rows: size.rows as u16, cols: size.columns as u16, pixel_width: 0, pixel_height: 0 }
}

fn window_size(size: GridSize) -> WindowSize {
    WindowSize { num_lines: size.rows as u16, num_cols: size.columns as u16, cell_width: 8, cell_height: 16 }
}

fn unix_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or_default()
}
