use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::rc::Rc;
use std::time::Duration;

use dosker_agent_proto::{
    DEFAULT_TIMEOUT_MS, ErrorCode, ErrorPayload, Frame, FrameType, MAX_RETRIES, ProtoError,
    Session, Transport, encode_payload,
};

#[derive(Debug, Clone, Copy)]
enum TimeoutKind {
    WouldBlock,
    TimedOut,
}

enum ReadItem {
    Data(Vec<u8>),
    Timeout,
    Eof,
}

struct MockState {
    reads: VecDeque<ReadItem>,
    pending_read: Vec<u8>,
    pending_write: Vec<u8>,
    writes: Vec<Vec<u8>>,
    timeout_kind: TimeoutKind,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
}

impl Default for MockState {
    fn default() -> Self {
        Self {
            reads: VecDeque::new(),
            pending_read: Vec::new(),
            pending_write: Vec::new(),
            writes: Vec::new(),
            timeout_kind: TimeoutKind::WouldBlock,
            read_timeout: None,
            write_timeout: None,
        }
    }
}

#[derive(Clone)]
struct MockTransport {
    state: Rc<RefCell<MockState>>,
}

impl MockTransport {
    fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(MockState::default())),
        }
    }

    fn with_timeout_kind(kind: TimeoutKind) -> Self {
        let transport = Self::new();
        transport.state.borrow_mut().timeout_kind = kind;
        transport
    }

    fn queue_frame(&self, kind: FrameType, request_id: u32, sequence: u32, payload: &[u8]) {
        let bytes = Frame::new(kind, request_id, sequence, payload.to_vec())
            .expect("frame is valid")
            .encode()
            .expect("frame encodes");
        self.state
            .borrow_mut()
            .reads
            .push_back(ReadItem::Data(bytes));
    }

    fn queue_corrupt(&self, kind: FrameType, request_id: u32, sequence: u32) {
        let mut bytes = Frame::new(kind, request_id, sequence, Vec::new())
            .expect("frame is valid")
            .encode()
            .expect("frame encodes");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        self.state
            .borrow_mut()
            .reads
            .push_back(ReadItem::Data(bytes));
    }

    fn queue_timeout(&self) {
        self.state.borrow_mut().reads.push_back(ReadItem::Timeout);
    }

    fn queue_eof(&self) {
        self.state.borrow_mut().reads.push_back(ReadItem::Eof);
    }

    fn writes(&self) -> Vec<Frame> {
        self.state
            .borrow()
            .writes
            .iter()
            .map(|bytes| Frame::decode(bytes).expect("written frame decodes"))
            .collect()
    }

    fn write_count(&self) -> usize {
        self.state.borrow().writes.len()
    }

    fn last_read_timeout(&self) -> Option<Duration> {
        self.state.borrow().read_timeout
    }
}

impl Read for MockTransport {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }

        let mut state = self.state.borrow_mut();

        if state.pending_read.is_empty() {
            match state.reads.pop_front() {
                Some(ReadItem::Data(bytes)) => state.pending_read = bytes,
                Some(ReadItem::Timeout) => {
                    let kind = match state.timeout_kind {
                        TimeoutKind::WouldBlock => io::ErrorKind::WouldBlock,
                        TimeoutKind::TimedOut => io::ErrorKind::TimedOut,
                    };
                    return Err(io::Error::new(kind, "scripted timeout"));
                }
                Some(ReadItem::Eof) => return Ok(0),
                None => {
                    let kind = match state.timeout_kind {
                        TimeoutKind::WouldBlock => io::ErrorKind::WouldBlock,
                        TimeoutKind::TimedOut => io::ErrorKind::TimedOut,
                    };
                    return Err(io::Error::new(kind, "no scripted read"));
                }
            }
        }

        let count = state.pending_read.len().min(buffer.len());
        buffer[..count].copy_from_slice(&state.pending_read[..count]);
        state.pending_read.drain(..count);
        Ok(count)
    }
}

impl Write for MockTransport {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.state
            .borrow_mut()
            .pending_write
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut state = self.state.borrow_mut();
        if !state.pending_write.is_empty() {
            let bytes = std::mem::take(&mut state.pending_write);
            state.writes.push(bytes);
        }
        Ok(())
    }
}

impl Transport for MockTransport {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.state.borrow_mut().read_timeout = timeout;
        Ok(())
    }

    fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.state.borrow_mut().write_timeout = timeout;
        Ok(())
    }
}

#[test]
fn send_writes_one_frame_and_waits_for_matching_ack() {
    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);
    let request_id = session.begin_request();
    probe.queue_frame(FrameType::Ack, request_id, 0, b"");

    let sent = session.send(FrameType::Ping, b"").expect("send succeeds");

    assert_eq!(sent.kind, FrameType::Ping);
    assert_eq!(sent.request_id, request_id);
    assert_eq!(sent.sequence, 0);
    assert_eq!(probe.writes(), vec![sent]);
    assert_eq!(session.sequence(), 1);
}

#[test]
fn begin_request_increments_request_id_and_resets_sequence() {
    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);

    let first = session.begin_request();
    assert_eq!(first, 1);
    probe.queue_frame(FrameType::Ack, first, 0, b"");
    session.send(FrameType::Ping, b"").expect("send succeeds");
    assert_eq!(session.sequence(), 1);

    let second = session.begin_request();
    assert_eq!(second, 2);
    assert_eq!(session.request_id(), 2);
    assert_eq!(session.sequence(), 0);
}

#[test]
fn sequence_increments_after_each_sent_frame() {
    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);
    let request_id = session.begin_request();
    probe.queue_frame(FrameType::Ack, request_id, 0, b"");
    probe.queue_frame(FrameType::Ack, request_id, 1, b"");

    session
        .send(FrameType::Ping, b"one")
        .expect("send succeeds");
    assert_eq!(session.sequence(), 1);

    session
        .send(FrameType::Ping, b"two")
        .expect("send succeeds");
    assert_eq!(session.sequence(), 2);

    let writes = probe.writes();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].sequence, 0);
    assert_eq!(writes[1].sequence, 1);
}

#[test]
fn send_retries_on_timeout_three_times_then_closes() {
    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);
    let request_id = session.begin_request();
    for _ in 0..=MAX_RETRIES {
        probe.queue_timeout();
    }

    let result = session.send(FrameType::Ping, b"");

    match result {
        Err(ProtoError::RetriesExhausted {
            request_id: failed_id,
            sequence,
            attempts,
        }) => {
            assert_eq!(failed_id, request_id);
            assert_eq!(sequence, 0);
            assert_eq!(attempts, 4);
        }
        other => panic!("expected retries exhausted, got {other:?}"),
    }

    assert!(session.is_closed());
    assert_eq!(probe.write_count(), 4);
    let writes = probe.writes();
    assert!(writes.iter().all(|frame| frame == &writes[0]));
}

#[test]
fn send_retries_on_nak_three_times_then_closes() {
    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);
    let request_id = session.begin_request();
    for _ in 0..=MAX_RETRIES {
        probe.queue_frame(FrameType::Nak, request_id, 0, b"");
    }

    let result = session.send(FrameType::Ping, b"");

    assert!(matches!(result, Err(ProtoError::RetriesExhausted { .. })));
    assert!(session.is_closed());
    assert_eq!(probe.write_count(), 4);
}

#[test]
fn try_send_retries_without_closing() {
    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);
    session.begin_request();
    for _ in 0..=MAX_RETRIES {
        probe.queue_timeout();
    }

    let result = session.try_send(FrameType::Ping, b"");

    assert!(matches!(result, Err(ProtoError::RetriesExhausted { .. })));
    assert!(!session.is_closed());
    assert_eq!(probe.write_count(), 4);
}

#[test]
fn ack_with_wrong_sequence_is_an_error() {
    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);
    let request_id = session.begin_request();
    probe.queue_frame(FrameType::Ack, request_id, 9, b"");

    match session.send(FrameType::Ping, b"") {
        Err(ProtoError::UnexpectedFrame { actual, .. }) => {
            assert_eq!(actual, FrameType::Ack);
        }
        other => panic!("expected an unexpected frame, got {other:?}"),
    }

    assert_eq!(session.sequence(), 0);
    assert_eq!(probe.write_count(), 1);
}

#[test]
fn ack_with_wrong_request_id_is_an_error() {
    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);
    session.begin_request();
    probe.queue_frame(FrameType::Ack, 999, 0, b"");

    match session.send(FrameType::Ping, b"") {
        Err(ProtoError::UnexpectedFrame { actual, .. }) => {
            assert_eq!(actual, FrameType::Ack);
        }
        other => panic!("expected an unexpected frame, got {other:?}"),
    }

    assert_eq!(session.sequence(), 0);
    assert_eq!(probe.write_count(), 1);
}

#[test]
fn error_frame_is_surfaced() {
    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);
    let request_id = session.begin_request();
    let payload = ErrorPayload {
        code: ErrorCode::Busy,
        message: "a command is running".to_string(),
        request_id,
    };
    probe.queue_frame(
        FrameType::Error,
        request_id,
        0,
        &encode_payload(&payload).expect("payload encodes"),
    );

    match session.send(FrameType::Ping, b"") {
        Err(ProtoError::Agent(error)) => {
            assert_eq!(error.code, ErrorCode::Busy);
            assert_eq!(error.request_id, request_id);
        }
        other => panic!("expected an agent error, got {other:?}"),
    }

    let writes = probe.writes();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[1].kind, FrameType::Ack);
    assert_eq!(writes[1].request_id, request_id);
    assert_eq!(writes[1].sequence, 0);
    assert_eq!(session.sequence(), 0);
}

#[test]
fn recv_acks_a_valid_frame() {
    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);
    let expected =
        Frame::new(FrameType::ExecData, 7, 3, b"hello".to_vec()).expect("frame is valid");
    probe.queue_frame(
        expected.kind,
        expected.request_id,
        expected.sequence,
        &expected.payload,
    );

    let received = session.recv().expect("recv succeeds");

    assert_eq!(received, expected);
    let writes = probe.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].kind, FrameType::Ack);
    assert_eq!(writes[0].request_id, 7);
    assert_eq!(writes[0].sequence, 3);
    assert_eq!(
        probe.last_read_timeout(),
        Some(Duration::from_millis(DEFAULT_TIMEOUT_MS))
    );
}

#[test]
fn recv_naks_a_corrupt_frame_and_returns_the_next_frame() {
    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);
    probe.queue_corrupt(FrameType::ExecData, 4, 1);
    probe.queue_frame(FrameType::ExecData, 4, 2, b"next");

    let received = session.recv().expect("recv succeeds");

    assert_eq!(received.kind, FrameType::ExecData);
    assert_eq!(received.sequence, 2);
    let writes = probe.writes();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].kind, FrameType::Nak);
    assert_eq!(writes[0].request_id, 4);
    assert_eq!(writes[0].sequence, 1);
    assert_eq!(writes[1].kind, FrameType::Ack);
    assert_eq!(writes[1].request_id, 4);
    assert_eq!(writes[1].sequence, 2);
}

#[test]
fn recv_closes_after_four_consecutive_corrupt_frames() {
    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);
    for sequence in 0..4 {
        probe.queue_corrupt(FrameType::ExecData, 8, sequence);
    }

    match session.recv() {
        Err(ProtoError::CrcFailed { attempts }) => assert_eq!(attempts, 4),
        other => panic!("expected a crc failure, got {other:?}"),
    }

    assert!(session.is_closed());
    let writes = probe.writes();
    assert_eq!(writes.len(), 4);
    assert!(writes.iter().all(|frame| frame.kind == FrameType::Nak));
}

#[test]
fn recv_skips_stray_ack_frames() {
    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);
    probe.queue_frame(FrameType::Ack, 1, 0, b"");
    probe.queue_frame(FrameType::Nak, 1, 1, b"");
    probe.queue_frame(FrameType::Ping, 2, 0, b"hi");

    let received = session.recv().expect("recv succeeds");

    assert_eq!(received.kind, FrameType::Ping);
    let writes = probe.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].kind, FrameType::Ack);
    assert_eq!(writes[0].request_id, 2);
    assert_eq!(writes[0].sequence, 0);
}

#[test]
fn recv_returns_timeout_on_silence() {
    let transport = MockTransport::with_timeout_kind(TimeoutKind::TimedOut);
    let probe = transport.clone();
    let mut session = Session::new(transport);
    probe.queue_timeout();

    match session.recv() {
        Err(ProtoError::Timeout) => {}
        other => panic!("expected a timeout, got {other:?}"),
    }

    assert_eq!(
        probe.last_read_timeout(),
        Some(Duration::from_millis(DEFAULT_TIMEOUT_MS))
    );
    assert!(!session.is_closed());
}

#[test]
fn recv_returns_closed_on_eof() {
    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);
    probe.queue_eof();

    assert!(matches!(session.recv(), Err(ProtoError::Closed)));
}

#[test]
fn send_after_close_returns_closed() {
    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);
    session.close();

    assert!(session.is_closed());
    assert!(matches!(
        session.send(FrameType::Ping, b""),
        Err(ProtoError::Closed)
    ));
    assert_eq!(probe.write_count(), 0);
}

#[test]
fn frame_timeout_is_500_ms_and_retries_are_three() {
    assert_eq!(DEFAULT_TIMEOUT_MS, 500);
    assert_eq!(MAX_RETRIES, 3);

    let transport = MockTransport::new();
    let probe = transport.clone();
    let mut session = Session::new(transport);
    probe.queue_timeout();
    let _ = session.recv();
    assert_eq!(
        probe.last_read_timeout(),
        Some(Duration::from_millis(DEFAULT_TIMEOUT_MS))
    );

    session.begin_request();
    for _ in 0..=MAX_RETRIES {
        probe.queue_timeout();
    }

    match session.try_send(FrameType::Ping, b"") {
        Err(ProtoError::RetriesExhausted { attempts, .. }) => {
            assert_eq!(attempts, MAX_RETRIES + 1);
        }
        other => panic!("expected retries exhausted, got {other:?}"),
    }
}
