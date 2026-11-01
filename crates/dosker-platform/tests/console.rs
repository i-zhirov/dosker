use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use dosker_platform::console::Console;

#[derive(Clone, Default)]
struct RecordingWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
    flushes: Arc<AtomicUsize>,
}

impl RecordingWriter {
    fn new() -> Self {
        Self::default()
    }

    fn contents(&self) -> Vec<u8> {
        self.bytes.lock().unwrap().clone()
    }

    fn flush_count(&self) -> usize {
        self.flushes.load(Ordering::SeqCst)
    }
}

impl Write for RecordingWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.bytes.lock().unwrap().extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct BrokenPipeWriter;

impl Write for BrokenPipeWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe"))
    }
}

#[test]
fn line_appends_newline() {
    let stdout = RecordingWriter::new();
    let stderr = RecordingWriter::new();
    let mut console = Console::new(Box::new(stdout.clone()), Box::new(stderr.clone()));

    console.line("a").unwrap();

    assert_eq!(stdout.contents(), b"a\n");
    assert_eq!(stderr.contents(), b"");
    assert_eq!(stdout.flush_count(), 1);
}

#[test]
fn error_line_uses_stderr() {
    let stdout = RecordingWriter::new();
    let stderr = RecordingWriter::new();
    let mut console = Console::new(Box::new(stdout.clone()), Box::new(stderr.clone()));

    console.error_line("e").unwrap();

    assert_eq!(stdout.contents(), b"");
    assert_eq!(stderr.contents(), b"e\n");
    assert_eq!(stderr.flush_count(), 1);
}

#[test]
fn raw_writes_without_newline() {
    let stdout = RecordingWriter::new();
    let stderr = RecordingWriter::new();
    let mut console = Console::new(Box::new(stdout.clone()), Box::new(stderr.clone()));

    console.raw("x").unwrap();

    assert_eq!(stdout.contents(), b"x");
    assert_eq!(stderr.contents(), b"");
    assert_eq!(stdout.flush_count(), 1);
}

#[test]
fn broken_pipe_is_ignored() {
    let mut console = Console::new(Box::new(BrokenPipeWriter), Box::new(BrokenPipeWriter));

    assert!(console.line("a").is_ok());
    assert!(console.error_line("e").is_ok());
    assert!(console.raw("x").is_ok());
}
