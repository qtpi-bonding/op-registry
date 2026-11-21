//! The streams one CLI run reads and writes.

use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

/// A writer many holders share: the runner, the notice sink and an app's
/// hooks write whole lines to it under one lock.
#[derive(Clone)]
pub struct SharedWriter(Arc<Mutex<Box<dyn Write + Send>>>);

impl SharedWriter {
    /// Shares `writer`.
    pub fn new(writer: Box<dyn Write + Send>) -> Self {
        Self(Arc::new(Mutex::new(writer)))
    }

    fn with<T>(&self, f: impl FnOnce(&mut (dyn Write + Send)) -> T) -> T {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut **guard)
    }
}

impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.with(|w| w.write(buf))
    }
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.with(|w| w.write_all(buf))
    }
    fn flush(&mut self) -> io::Result<()> {
        self.with(|w| w.flush())
    }
}

/// Reads one secret line from a terminal with echo off.
#[cfg(feature = "secrets")]
pub trait Prompter: Send + Sync {
    /// Reads one line from the terminal with echo off, without its line ending.
    fn read_secret(&self, label: &str) -> io::Result<zeroize::Zeroizing<String>>;
}

/// The prompter of a real terminal: the prompt goes to the terminal, not to
/// the streams of the run, and the answer is not echoed.
#[cfg(feature = "prompt")]
struct TtyPrompter;

#[cfg(feature = "prompt")]
impl Prompter for TtyPrompter {
    fn read_secret(&self, label: &str) -> io::Result<zeroize::Zeroizing<String>> {
        rpassword::prompt_password(format!("{label}: ")).map(zeroize::Zeroizing::new)
    }
}

/// The streams of a run and whether the output ones are terminals.
#[non_exhaustive]
pub struct Io {
    /// Where the result is printed.
    pub stdout: Box<dyn Write + Send>,
    /// Where notices, the app's log text and the error line go.
    pub stderr: SharedWriter,
    /// What `-` and `@-` read; `None` when there is no stdin.
    pub stdin: Option<Box<dyn Read + Send>>,
    /// Whether stdout is a terminal.
    pub stdout_tty: bool,
    /// Whether stderr is a terminal.
    pub stderr_tty: bool,
    /// Whether stdin is a terminal.
    pub stdin_tty: bool,
    /// Reads a secret from the terminal; `None` when there is none to ask.
    #[cfg(feature = "secrets")]
    pub prompter: Option<Arc<dyn Prompter>>,
}

/// What a memory run wrote.
#[derive(Clone)]
pub struct Captured {
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

impl Captured {
    /// Everything written to stdout, as text.
    pub fn stdout(&self) -> String {
        text(&self.stdout)
    }

    /// Everything written to stderr, as text.
    pub fn stderr(&self) -> String {
        text(&self.stderr)
    }
}

fn text(buffer: &Mutex<Vec<u8>>) -> String {
    let guard = buffer.lock().unwrap_or_else(|e| e.into_inner());
    String::from_utf8_lossy(&guard).into_owned()
}

/// A writer into a shared buffer.
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Io {
    /// The process streams. A stream is a terminal when the process says so.
    pub fn std() -> Io {
        use std::io::IsTerminal;
        let stdin_tty = io::stdin().is_terminal();
        let stderr_tty = io::stderr().is_terminal();
        Io {
            stdout: Box::new(io::stdout()),
            stderr: SharedWriter::new(Box::new(io::stderr())),
            stdin: Some(Box::new(io::stdin())),
            stdout_tty: io::stdout().is_terminal(),
            stderr_tty,
            stdin_tty,
            #[cfg(feature = "secrets")]
            prompter: operator_prompter(stdin_tty && stderr_tty),
        }
    }

    /// In-memory streams for tests: `stdin` is read from, the output is
    /// captured, and the two flags say what the streams pretend to be.
    pub fn memory(stdin: &[u8], stdout_tty: bool, stderr_tty: bool) -> (Io, Captured) {
        let captured = Captured {
            stdout: Arc::default(),
            stderr: Arc::default(),
        };
        let io = Io {
            stdout: Box::new(Buffer(captured.stdout.clone())),
            stderr: SharedWriter::new(Box::new(Buffer(captured.stderr.clone()))),
            stdin: Some(Box::new(io::Cursor::new(stdin.to_vec()))),
            stdout_tty,
            stderr_tty,
            stdin_tty: false,
            #[cfg(feature = "secrets")]
            prompter: None,
        };
        (io, captured)
    }

    /// Says whether stdin is a terminal.
    pub fn with_stdin_tty(mut self, yes: bool) -> Io {
        self.stdin_tty = yes;
        self
    }

    /// Sets the prompter secrets are read with. Without the `prompt` feature
    /// only prompted fields use it; `CallOptions::prompt_secret` returns
    /// `None` regardless.
    #[cfg(feature = "secrets")]
    pub fn with_prompter(mut self, prompter: Arc<dyn Prompter>) -> Io {
        self.prompter = Some(prompter);
        self
    }
}

#[cfg(feature = "prompt")]
fn operator_prompter(on_terminal: bool) -> Option<Arc<dyn Prompter>> {
    on_terminal.then(|| Arc::new(TtyPrompter) as Arc<dyn Prompter>)
}

#[cfg(all(feature = "secrets", not(feature = "prompt")))]
fn operator_prompter(_on_terminal: bool) -> Option<Arc<dyn Prompter>> {
    None
}
