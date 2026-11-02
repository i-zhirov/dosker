//! A scripted agent transport for client tests.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use dosker_agent_proto::{
    AGENT_VERSION, Attributes, ErrorPayload, ExecEnd, ExecRequest, Frame, FrameType, GetEnd, Hello,
    Pong, PutEnd, PutRequest, ShutdownAck, StatResponse, Transport, decode_payload, encode_payload,
};

pub fn hello() -> Hello {
    Hello {
        protocol_version: 1,
        agent_version: AGENT_VERSION.to_owned(),
        dos: "FreeDOS 1.3".to_owned(),
        memory_kb: 640,
        control_port: 2,
        max_payload: 4096,
        features: vec![
            "exec".to_owned(),
            "put".to_owned(),
            "get".to_owned(),
            "stat".to_owned(),
        ],
    }
}

pub fn exec_request() -> ExecRequest {
    ExecRequest {
        command: "TEST.EXE".to_owned(),
        env: std::collections::BTreeMap::from([("PATH".to_owned(), "C:\\APP;C:\\DOS".to_owned())]),
        workdir: "C:\\APP".to_owned(),
        max_output_bytes: 1_048_576,
    }
}

pub fn exec_end(exit_code: i32, output_bytes: u64, duration_ms: u64) -> ExecEnd {
    ExecEnd {
        exit_code,
        output_bytes,
        truncated: false,
        duration_ms,
        command_not_found: false,
    }
}

pub fn stat_response() -> StatResponse {
    StatResponse {
        exists: true,
        size: 12_345,
        directory: false,
        attributes: Attributes {
            read_only: false,
            hidden: false,
            system: false,
            archive: true,
        },
        timestamp: Some("1996-03-14T12:00:00Z".to_owned()),
    }
}

#[derive(Clone)]
enum PingResponse {
    Pong(Pong),
    Hello(Hello),
    Silent,
}

#[derive(Clone)]
enum ExecResponse {
    Frames { chunks: Vec<Vec<u8>>, end: ExecEnd },
    Error(ErrorPayload),
    Silent,
}

enum ReadItem {
    Data(Vec<u8>),
}

struct MockState {
    reads: VecDeque<ReadItem>,
    pending_read: Vec<u8>,
    pending_write: Vec<u8>,
    host_frames: Vec<Frame>,
    response_request_id: Option<u32>,
    response_sequence: u32,
    read_delay: Option<Duration>,
    read_timeout: Option<Duration>,
    silent_ack: bool,
    hello_instead_of_ack: Option<Hello>,
    ping: PingResponse,
    exec: ExecResponse,
    get_chunks: Vec<Vec<u8>>,
    get_end: GetEnd,
    stat: StatResponse,
    put_expected: u64,
    put_received: u64,
    put_ok: bool,
    put_end_bytes: Option<u64>,
    shutdown_ack: ShutdownAck,
}

impl Default for MockState {
    fn default() -> Self {
        Self {
            reads: VecDeque::new(),
            pending_read: Vec::new(),
            pending_write: Vec::new(),
            host_frames: Vec::new(),
            response_request_id: None,
            response_sequence: 0,
            read_delay: None,
            read_timeout: None,
            silent_ack: false,
            hello_instead_of_ack: None,
            ping: PingResponse::Pong(Pong {
                agent_version: AGENT_VERSION.to_owned(),
                busy: false,
            }),
            exec: ExecResponse::Frames {
                chunks: Vec::new(),
                end: exec_end(0, 0, 0),
            },
            get_chunks: Vec::new(),
            get_end: GetEnd { ok: true, size: 0 },
            stat: stat_response(),
            put_expected: 0,
            put_received: 0,
            put_ok: true,
            put_end_bytes: None,
            shutdown_ack: ShutdownAck { ready: true },
        }
    }
}

impl MockState {
    fn next_sequence(&mut self, request_id: u32) -> u32 {
        if self.response_request_id != Some(request_id) {
            self.response_request_id = Some(request_id);
            self.response_sequence = 0;
        }

        let sequence = self.response_sequence;
        self.response_sequence = self.response_sequence.wrapping_add(1);
        sequence
    }

    fn push_frame(&mut self, kind: FrameType, request_id: u32, sequence: u32, payload: Vec<u8>) {
        let frame = Frame::new(kind, request_id, sequence, payload).expect("mock frame is valid");
        let bytes = frame.encode().expect("mock frame encodes");
        self.reads.push_back(ReadItem::Data(bytes));
    }

    fn push_next(&mut self, kind: FrameType, request_id: u32, payload: Vec<u8>) {
        let sequence = self.next_sequence(request_id);
        self.push_frame(kind, request_id, sequence, payload);
    }

    fn ack(&mut self, frame: &Frame) {
        self.push_frame(FrameType::Ack, frame.request_id, frame.sequence, Vec::new());
    }

    fn put_end(&self) -> PutEnd {
        PutEnd {
            ok: self.put_ok,
            bytes: self.put_end_bytes.unwrap_or(self.put_received),
        }
    }
}

/// A transport that answers host frames with scripted agent frames.
#[derive(Clone)]
pub struct MockAgent {
    state: Rc<RefCell<MockState>>,
}

impl MockAgent {
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(MockState::default())),
        }
    }

    pub fn queue_hello(&self, hello: Hello) {
        let mut state = self.state.borrow_mut();
        let payload = encode_payload(&hello).expect("hello encodes");
        state.push_next(FrameType::Hello, 1, payload);
    }

    pub fn queue_pong(&self, pong: Pong) {
        let mut state = self.state.borrow_mut();
        let payload = encode_payload(&pong).expect("pong encodes");
        state.push_next(FrameType::Pong, 1, payload);
    }

    pub fn set_ping_response_hello(&self, hello: Hello) {
        self.state.borrow_mut().ping = PingResponse::Hello(hello);
    }

    pub fn set_ping_silent(&self) {
        self.state.borrow_mut().ping = PingResponse::Silent;
    }

    pub fn set_hello_instead_of_ack(&self, hello: Hello) {
        self.state.borrow_mut().hello_instead_of_ack = Some(hello);
    }

    pub fn set_read_delay(&self, delay: Duration) {
        self.state.borrow_mut().read_delay = Some(delay);
    }

    pub fn set_silent_ack(&self, silent: bool) {
        self.state.borrow_mut().silent_ack = silent;
    }

    pub fn set_exec_output(&self, chunks: Vec<Vec<u8>>, end: ExecEnd) {
        self.state.borrow_mut().exec = ExecResponse::Frames { chunks, end };
    }

    pub fn set_exec_error(&self, error: ErrorPayload) {
        self.state.borrow_mut().exec = ExecResponse::Error(error);
    }

    pub fn set_exec_silent(&self) {
        self.state.borrow_mut().exec = ExecResponse::Silent;
    }

    pub fn set_get_output(&self, chunks: Vec<Vec<u8>>, end: GetEnd) {
        let mut state = self.state.borrow_mut();
        state.get_chunks = chunks;
        state.get_end = end;
    }

    pub fn set_put_end_ok(&self, ok: bool) {
        self.state.borrow_mut().put_ok = ok;
    }

    pub fn set_put_end_bytes(&self, bytes: u64) {
        self.state.borrow_mut().put_end_bytes = Some(bytes);
    }

    pub fn set_stat_response(&self, response: StatResponse) {
        self.state.borrow_mut().stat = response;
    }

    pub fn set_shutdown_ack(&self, ack: ShutdownAck) {
        self.state.borrow_mut().shutdown_ack = ack;
    }

    pub fn host_frames(&self) -> Vec<Frame> {
        self.state.borrow().host_frames.clone()
    }

    pub fn host_frames_of(&self, kind: FrameType) -> Vec<Frame> {
        self.state
            .borrow()
            .host_frames
            .iter()
            .filter(|frame| frame.kind == kind)
            .cloned()
            .collect()
    }
}

impl Read for MockAgent {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }

        let delay = {
            let state = self.state.borrow();

            if !state.pending_read.is_empty() {
                None
            } else if state.reads.is_empty() {
                state.read_delay.map(|delay| {
                    state
                        .read_timeout
                        .map_or(delay, |timeout| timeout.min(delay))
                })
            } else {
                state.read_delay
            }
        };

        if let Some(delay) = delay {
            thread::sleep(delay);
        }

        let mut state = self.state.borrow_mut();

        if state.pending_read.is_empty() {
            match state.reads.pop_front() {
                Some(ReadItem::Data(bytes)) => state.pending_read = bytes,
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "no scripted response",
                    ));
                }
            }
        }

        let count = state.pending_read.len().min(buffer.len());
        buffer[..count].copy_from_slice(&state.pending_read[..count]);
        state.pending_read.drain(..count);
        Ok(count)
    }
}

impl Write for MockAgent {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.state
            .borrow_mut()
            .pending_write
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut state = self.state.borrow_mut();

        if state.pending_write.is_empty() {
            return Ok(());
        }

        let bytes = std::mem::take(&mut state.pending_write);
        let frame = Frame::decode(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        state.host_frames.push(frame.clone());

        if matches!(frame.kind, FrameType::Ack | FrameType::Nak) {
            return Ok(());
        }

        if state.silent_ack {
            return Ok(());
        }

        if let Some(hello) = state.hello_instead_of_ack.take() {
            let payload = encode_payload(&hello).expect("hello encodes");
            state.push_next(FrameType::Hello, frame.request_id, payload);
            return Ok(());
        }

        if frame.kind == FrameType::Ping && matches!(&state.ping, PingResponse::Silent) {
            return Ok(());
        }

        state.ack(&frame);

        match frame.kind {
            FrameType::Ping => {
                let response = state.ping.clone();
                match response {
                    PingResponse::Pong(pong) => {
                        let payload = encode_payload(&pong).expect("pong encodes");
                        state.push_next(FrameType::Pong, frame.request_id, payload);
                    }
                    PingResponse::Hello(hello) => {
                        let payload = encode_payload(&hello).expect("hello encodes");
                        state.push_next(FrameType::Hello, frame.request_id, payload);
                    }
                    PingResponse::Silent => {}
                }
            }
            FrameType::ExecReq => {
                let response = state.exec.clone();
                match response {
                    ExecResponse::Frames { chunks, end } => {
                        for chunk in chunks {
                            state.push_next(FrameType::ExecData, frame.request_id, chunk);
                        }
                        let payload = encode_payload(&end).expect("exec end encodes");
                        state.push_next(FrameType::ExecEnd, frame.request_id, payload);
                    }
                    ExecResponse::Error(error) => {
                        let payload = encode_payload(&error).expect("error encodes");
                        state.push_next(FrameType::Error, frame.request_id, payload);
                    }
                    ExecResponse::Silent => {}
                }
            }
            FrameType::PutReq => {
                let request: PutRequest =
                    decode_payload(&frame.payload).expect("put request decodes");
                state.put_expected = request.size;
                state.put_received = 0;

                if request.size == 0 {
                    let end = state.put_end();
                    let payload = encode_payload(&end).expect("put end encodes");
                    state.push_next(FrameType::PutEnd, frame.request_id, payload);
                }
            }
            FrameType::PutData => {
                state.put_received += frame.payload.len() as u64;

                if state.put_received >= state.put_expected {
                    let end = state.put_end();
                    let payload = encode_payload(&end).expect("put end encodes");
                    state.push_next(FrameType::PutEnd, frame.request_id, payload);
                }
            }
            FrameType::GetReq => {
                let chunks = state.get_chunks.clone();
                let end = state.get_end.clone();

                for chunk in chunks {
                    state.push_next(FrameType::GetData, frame.request_id, chunk);
                }

                let payload = encode_payload(&end).expect("get end encodes");
                state.push_next(FrameType::GetEnd, frame.request_id, payload);
            }
            FrameType::StatReq => {
                let response = state.stat.clone();
                let payload = encode_payload(&response).expect("stat response encodes");
                state.push_next(FrameType::StatResp, frame.request_id, payload);
            }
            FrameType::Shutdown => {
                let ack = state.shutdown_ack.clone();
                let payload = encode_payload(&ack).expect("shutdown ack encodes");
                state.push_next(FrameType::ShutdownAck, frame.request_id, payload);
            }
            _ => {}
        }

        Ok(())
    }
}

impl Transport for MockAgent {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.state.borrow_mut().read_timeout = timeout;
        Ok(())
    }

    fn set_write_timeout(&mut self, _timeout: Option<Duration>) -> io::Result<()> {
        Ok(())
    }
}
