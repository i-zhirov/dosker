#![cfg(unix)]

use std::cell::RefCell;
use std::io::{self, Read, Write};
use std::rc::Rc;
use std::time::Duration;

use dosker_agent_proto::Transport;
use dosker_engine_qemu::serial_log::SerialLog;

type TimeoutLog = Rc<RefCell<Vec<(&'static str, Option<Duration>)>>>;

#[derive(Clone, Default)]
struct ScriptedTransport {
    input: Rc<RefCell<Vec<u8>>>,
    writes: Rc<RefCell<Vec<u8>>>,
    timeouts: TimeoutLog,
}

impl ScriptedTransport {
    fn with_input(input: &[u8]) -> Self {
        Self {
            input: Rc::new(RefCell::new(input.to_vec())),
            ..Self::default()
        }
    }

    fn writes(&self) -> Vec<u8> {
        self.writes.borrow().clone()
    }

    fn timeouts(&self) -> Vec<(&'static str, Option<Duration>)> {
        self.timeouts.borrow().clone()
    }
}

impl Read for ScriptedTransport {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut input = self.input.borrow_mut();
        let count = input.len().min(buffer.len());
        buffer[..count].copy_from_slice(&input[..count]);
        input.drain(..count);

        Ok(count)
    }
}

impl Write for ScriptedTransport {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.writes.borrow_mut().extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Transport for ScriptedTransport {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.timeouts.borrow_mut().push(("read", timeout));
        Ok(())
    }

    fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.timeouts.borrow_mut().push(("write", timeout));
        Ok(())
    }
}

#[test]
fn serial_log_records_reads_and_writes() {
    let transport = ScriptedTransport::with_input(b"response");
    let log = tempfile::NamedTempFile::new().expect("create log file");
    let mut serial = SerialLog::new(transport.clone(), log.reopen().expect("reopen log file"));

    serial.write_all(b"request").expect("write request");
    let mut buffer = [0u8; 8];
    serial.read_exact(&mut buffer).expect("read response");

    assert_eq!(&buffer, b"response");
    assert_eq!(
        std::fs::read(log.path()).expect("read log"),
        b"requestresponse"
    );
    assert_eq!(transport.writes(), b"request");
}

#[test]
fn serial_log_write_failure_is_best_effort() {
    let transport = ScriptedTransport::default();
    let log = tempfile::NamedTempFile::new().expect("create log file");
    let read_only = std::fs::File::open(log.path()).expect("open the log read-only");
    let mut serial = SerialLog::new(transport.clone(), read_only);

    serial
        .write_all(b"protocol bytes")
        .expect("protocol traffic continues when the log cannot be written");

    assert_eq!(transport.writes(), b"protocol bytes");
}

#[test]
fn serial_log_forwards_timeouts() {
    let transport = ScriptedTransport::default();
    let log = tempfile::NamedTempFile::new().expect("create log file");
    let mut serial = SerialLog::new(transport.clone(), log.reopen().expect("reopen log file"));
    let read_timeout = Some(Duration::from_millis(250));
    let write_timeout = Some(Duration::from_millis(750));

    serial
        .set_read_timeout(read_timeout)
        .expect("set read timeout");
    serial
        .set_write_timeout(write_timeout)
        .expect("set write timeout");

    assert_eq!(
        transport.timeouts(),
        vec![("read", read_timeout), ("write", write_timeout)]
    );
}
