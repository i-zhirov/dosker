//! Host client for the agent handshake and commands.

use std::thread;
use std::time::{Duration, Instant};

use crate::error::{ProtoError, Result};
use crate::frame::{
    Frame, FrameType, MAX_COMMAND_CHARS, MAX_OUTPUT_BYTES, MAX_PAYLOAD, MAX_TRANSFER_BYTES,
};
use crate::payload::{
    ErrorCode, ErrorPayload, ExecEnd, ExecRequest, GetEnd, GetRequest, Hello, HelloAck, Pong,
    PutEnd, PutRequest, ShutdownAck, ShutdownRequest, StatRequest, StatResponse, decode_payload,
    encode_payload,
};
use crate::session::{Session, Transport};

/// Read step while waiting for the agent handshake.
const HANDSHAKE_READ_STEP: Duration = Duration::from_secs(2);
/// Pause after a failed handshake ping.
const HANDSHAKE_RETRY_SLEEP: Duration = Duration::from_secs(1);
/// Session identifier length in characters.
const SESSION_ID_CHARS: usize = 6;
/// Lowercase hexadecimal digits.
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// The result of one command execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutcome {
    pub exit_code: i32,
    pub output: Vec<u8>,
    pub truncated: bool,
    pub duration_ms: u64,
    pub command_not_found: bool,
}

/// A host client over a stop-and-wait agent session.
pub struct AgentClient<T: Transport> {
    session: Session<T>,
    session_id: Option<String>,
    hello: Option<Hello>,
}

impl<T: Transport> AgentClient<T> {
    /// Create a client over the given transport.
    pub fn new(transport: T) -> Self {
        Self {
            session: Session::new(transport),
            session_id: None,
            hello: None,
        }
    }

    /// Complete the handshake, waiting up to the given timeout.
    pub fn handshake(&mut self, timeout: Duration) -> Result<Hello> {
        let deadline = Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ProtoError::HandshakeTimeout);
            }

            match self
                .session
                .recv_with_timeout(remaining.min(HANDSHAKE_READ_STEP))
            {
                Ok(frame) => return self.accept_handshake_frame(frame),
                Err(ProtoError::Timeout) => {
                    if Instant::now() >= deadline {
                        return Err(ProtoError::HandshakeTimeout);
                    }

                    if let Some(hello) = self.ping_for_hello(deadline)? {
                        return Ok(hello);
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Return the session identifier assigned during the handshake.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Return the handshake announcement.
    pub fn hello(&self) -> Option<&Hello> {
        self.hello.as_ref()
    }

    /// Borrow the session for raw-frame access.
    pub fn session_mut(&mut self) -> &mut Session<T> {
        &mut self.session
    }

    /// Execute a command and collect its output.
    pub fn exec(&mut self, request: &ExecRequest, timeout: Duration) -> Result<ExecOutcome> {
        let deadline = Instant::now() + timeout;
        request.validate()?;
        let max_output = (request.max_output_bytes as usize).min(MAX_OUTPUT_BYTES);

        self.session.begin_request();
        let payload = encode_payload(request)?;
        self.session
            .send_with_deadline(FrameType::ExecReq, &payload, deadline)?;

        let mut output = Vec::new();

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ProtoError::Timeout);
            }

            let frame = Self::classify(self.session.recv_with_timeout(remaining)?)?;

            match frame.kind {
                FrameType::ExecData => {
                    output.extend_from_slice(&frame.payload);

                    if output.len() > max_output {
                        return Err(ProtoError::OutputTooLarge {
                            length: output.len(),
                            max: max_output,
                        });
                    }
                }
                FrameType::ExecEnd => {
                    let end: ExecEnd = decode_payload(&frame.payload)?;
                    let expected = output.len() as u64;

                    if end.output_bytes != expected {
                        return Err(ProtoError::ByteCountMismatch {
                            expected,
                            actual: end.output_bytes,
                        });
                    }

                    return Ok(ExecOutcome {
                        exit_code: end.exit_code,
                        output,
                        truncated: end.truncated,
                        duration_ms: end.duration_ms,
                        command_not_found: end.command_not_found,
                    });
                }
                other => {
                    return Err(ProtoError::UnexpectedFrame {
                        expected: "EXEC_DATA or EXEC_END",
                        actual: other,
                    });
                }
            }
        }
    }

    /// Upload a file in payload-sized chunks.
    pub fn put(&mut self, path: &str, data: &[u8], overwrite: bool) -> Result<PutEnd> {
        let request = PutRequest {
            path: path.to_owned(),
            size: data.len() as u64,
            overwrite,
        };
        request.validate()?;

        self.session.begin_request();
        let payload = encode_payload(&request)?;
        self.session.send(FrameType::PutReq, &payload)?;

        for chunk in data.chunks(MAX_PAYLOAD) {
            self.session.send(FrameType::PutData, chunk)?;
        }

        let frame = self.receive()?;
        if frame.kind != FrameType::PutEnd {
            return Err(ProtoError::UnexpectedFrame {
                expected: "PUT_END",
                actual: frame.kind,
            });
        }

        let end: PutEnd = decode_payload(&frame.payload)?;
        if !end.ok {
            return Err(internal_error(
                self.session.request_id(),
                "file transfer failed",
            ));
        }

        let expected = data.len() as u64;
        if end.bytes != expected {
            return Err(ProtoError::ByteCountMismatch {
                expected,
                actual: end.bytes,
            });
        }

        Ok(end)
    }

    /// Download a file and return its bytes.
    pub fn get(&mut self, path: &str) -> Result<Vec<u8>> {
        validate_path(path)?;

        self.session.begin_request();
        let payload = encode_payload(&GetRequest {
            path: path.to_owned(),
        })?;
        self.session.send(FrameType::GetReq, &payload)?;

        let mut data = Vec::new();

        loop {
            let frame = self.receive()?;

            match frame.kind {
                FrameType::GetData => {
                    data.extend_from_slice(&frame.payload);
                    let size = data.len() as u64;

                    if size > MAX_TRANSFER_BYTES {
                        return Err(ProtoError::TransferTooLarge {
                            size,
                            max: MAX_TRANSFER_BYTES,
                        });
                    }
                }
                FrameType::GetEnd => {
                    let end: GetEnd = decode_payload(&frame.payload)?;
                    if !end.ok {
                        return Err(internal_error(
                            self.session.request_id(),
                            "file transfer failed",
                        ));
                    }

                    let expected = data.len() as u64;
                    if end.size != expected {
                        return Err(ProtoError::ByteCountMismatch {
                            expected,
                            actual: end.size,
                        });
                    }

                    return Ok(data);
                }
                other => {
                    return Err(ProtoError::UnexpectedFrame {
                        expected: "GET_DATA or GET_END",
                        actual: other,
                    });
                }
            }
        }
    }

    /// Query a path and return its attributes.
    pub fn stat(&mut self, path: &str) -> Result<StatResponse> {
        validate_path(path)?;

        self.session.begin_request();
        let payload = encode_payload(&StatRequest {
            path: path.to_owned(),
        })?;
        self.session.send(FrameType::StatReq, &payload)?;

        let frame = self.receive()?;
        if frame.kind != FrameType::StatResp {
            return Err(ProtoError::UnexpectedFrame {
                expected: "STAT_RESP",
                actual: frame.kind,
            });
        }

        decode_payload(&frame.payload)
    }

    /// Check liveness and return the agent status.
    pub fn ping(&mut self) -> Result<Pong> {
        self.session.begin_request();
        self.session.send(FrameType::Ping, b"")?;

        let frame = self.receive()?;
        if frame.kind != FrameType::Pong {
            return Err(ProtoError::UnexpectedFrame {
                expected: "PONG",
                actual: frame.kind,
            });
        }

        decode_payload(&frame.payload)
    }

    /// Ask the agent to flush its files and stop.
    pub fn shutdown(&mut self, grace_ms: u64) -> Result<ShutdownAck> {
        self.session.begin_request();
        let payload = encode_payload(&ShutdownRequest { grace_ms })?;
        self.session.send(FrameType::Shutdown, &payload)?;

        let frame = self.receive()?;
        if frame.kind != FrameType::ShutdownAck {
            return Err(ProtoError::UnexpectedFrame {
                expected: "SHUTDOWN_ACK",
                actual: frame.kind,
            });
        }

        decode_payload(&frame.payload)
    }

    /// Ask the agent to flush its files and stop within a caller deadline.
    pub fn shutdown_with_timeout(
        &mut self,
        grace_ms: u64,
        timeout: Duration,
    ) -> Result<ShutdownAck> {
        let deadline = Instant::now() + timeout;
        self.session.begin_request();
        let payload = encode_payload(&ShutdownRequest { grace_ms })?;
        self.session
            .send_with_deadline(FrameType::Shutdown, &payload, deadline)?;

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ProtoError::Timeout);
        }

        let frame = Self::classify(self.session.recv_with_timeout(remaining)?)?;
        if frame.kind != FrameType::ShutdownAck {
            return Err(ProtoError::UnexpectedFrame {
                expected: "SHUTDOWN_ACK",
                actual: frame.kind,
            });
        }

        decode_payload(&frame.payload)
    }

    /// Consume the client and return its transport.
    pub fn into_inner(self) -> T {
        self.session.into_inner()
    }

    fn ping_for_hello(&mut self, deadline: Instant) -> Result<Option<Hello>> {
        self.session.begin_request();

        match self
            .session
            .try_send_with_deadline(FrameType::Ping, b"", deadline)
        {
            Ok(_) => {}
            Err(ProtoError::RetriesExhausted { .. }) => {
                sleep_before_retry(deadline)?;
                return Ok(None);
            }
            Err(ProtoError::GuestRebooted) if self.session_id.is_none() => {
                // A HELLO can arrive while the PING acknowledgement is in
                // flight. Before the handshake completes this is the agent
                // announcing itself, not a guest reboot.
                return Ok(None);
            }
            Err(ProtoError::Timeout) => return Err(ProtoError::HandshakeTimeout),
            Err(error) => return Err(error),
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ProtoError::HandshakeTimeout);
        }

        match self
            .session
            .recv_with_timeout(remaining.min(HANDSHAKE_READ_STEP))
        {
            Ok(frame) => Ok(Some(self.accept_handshake_frame(frame)?)),
            Err(ProtoError::Timeout) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn accept_handshake_frame(&mut self, frame: Frame) -> Result<Hello> {
        match frame.kind {
            FrameType::Hello => {
                if self.session_id.is_some() {
                    return Err(ProtoError::GuestRebooted);
                }

                let hello: Hello = decode_payload(&frame.payload)?;
                hello.validate()?;

                let request_id = self.session.begin_request();
                let session_id = generate_session_id(request_id);
                let payload = encode_payload(&HelloAck {
                    session_id: session_id.clone(),
                })?;
                self.session.send(FrameType::HelloAck, &payload)?;

                self.session_id = Some(session_id);
                self.hello = Some(hello.clone());

                Ok(hello)
            }
            FrameType::Pong => Err(ProtoError::AlreadyHandshaken),
            FrameType::Error => {
                let error: ErrorPayload = decode_payload(&frame.payload)?;
                Err(ProtoError::Agent(error))
            }
            other => Err(ProtoError::UnexpectedFrame {
                expected: "HELLO",
                actual: other,
            }),
        }
    }

    fn receive(&mut self) -> Result<Frame> {
        Self::classify(self.session.recv()?)
    }

    fn classify(frame: Frame) -> Result<Frame> {
        match frame.kind {
            FrameType::Error => {
                let error: ErrorPayload = decode_payload(&frame.payload)?;
                Err(ProtoError::Agent(error))
            }
            FrameType::Hello => Err(ProtoError::GuestRebooted),
            _ => Ok(frame),
        }
    }
}

fn sleep_before_retry(deadline: Instant) -> Result<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());

    if remaining.is_zero() {
        return Err(ProtoError::HandshakeTimeout);
    }

    thread::sleep(remaining.min(HANDSHAKE_RETRY_SLEEP));
    Ok(())
}

fn validate_path(path: &str) -> Result<()> {
    let length = path.chars().count();

    if length > MAX_COMMAND_CHARS {
        return Err(ProtoError::CommandTooLong {
            length,
            max: MAX_COMMAND_CHARS,
        });
    }

    Ok(())
}

fn internal_error(request_id: u32, message: &str) -> ProtoError {
    ProtoError::Agent(ErrorPayload {
        code: ErrorCode::Internal,
        message: message.to_owned(),
        request_id,
    })
}

fn generate_session_id(request_id: u32) -> String {
    let mut bytes = [0u8; SESSION_ID_CHARS];

    if getrandom::fill(&mut bytes).is_ok() {
        return bytes
            .iter()
            .map(|byte| HEX_DIGITS[(byte & 0x0f) as usize] as char)
            .collect();
    }

    let low = request_id & 0x00ff_ffff;
    let mut session_id = String::with_capacity(SESSION_ID_CHARS);

    for shift in (0..24).step_by(4).rev() {
        session_id.push(HEX_DIGITS[((low >> shift) & 0x0f) as usize] as char);
    }

    session_id
}
