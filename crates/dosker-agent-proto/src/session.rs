//! Stop-and-wait session over a byte transport.

use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use crate::codec::{Incoming, read_frame, write_frame};
use crate::error::{ProtoError, Result};
use crate::frame::{DEFAULT_TIMEOUT_MS, Frame, FrameType, MAX_RETRIES};
use crate::payload::{ErrorPayload, decode_payload};

/// Consecutive corrupt frames that close a session.
const MAX_CORRUPT_FRAMES: u32 = MAX_RETRIES + 1;

/// A byte stream that supports read and write timeouts.
pub trait Transport: Read + Write {
    /// Set the read timeout for the transport.
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()>;

    /// Set the write timeout for the transport.
    fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()>;
}

#[cfg(unix)]
impl Transport for std::os::unix::net::UnixStream {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        std::os::unix::net::UnixStream::set_read_timeout(self, timeout)
    }

    fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        std::os::unix::net::UnixStream::set_write_timeout(self, timeout)
    }
}

#[cfg(windows)]
impl Transport for std::net::TcpStream {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        std::net::TcpStream::set_read_timeout(self, timeout)
    }

    fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        std::net::TcpStream::set_write_timeout(self, timeout)
    }
}

/// A stop-and-wait protocol session.
pub struct Session<T: Transport> {
    transport: T,
    request_id: u32,
    sequence: u32,
    timeout: Duration,
    max_retries: u32,
    closed: bool,
    consecutive_corrupt: u32,
}

impl<T: Transport> Session<T> {
    /// Create a session with the default timeout and retry count.
    pub fn new(transport: T) -> Self {
        Self::with_config(
            transport,
            Duration::from_millis(DEFAULT_TIMEOUT_MS),
            MAX_RETRIES,
        )
    }

    /// Create a session with an explicit timeout and retry count.
    pub fn with_config(transport: T, timeout: Duration, max_retries: u32) -> Self {
        Self {
            transport,
            request_id: 0,
            sequence: 0,
            timeout,
            max_retries,
            closed: false,
            consecutive_corrupt: 0,
        }
    }

    /// Start a new request and reset the sequence.
    pub fn begin_request(&mut self) -> u32 {
        self.request_id = self.request_id.wrapping_add(1);
        self.sequence = 0;
        self.request_id
    }

    /// Return the current request id.
    pub fn request_id(&self) -> u32 {
        self.request_id
    }

    /// Return the next sequence number.
    pub fn sequence(&self) -> u32 {
        self.sequence
    }

    /// Return true when the session is closed.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Close the session.
    pub fn close(&mut self) {
        self.closed = true;
    }

    /// Consume the session and return its transport.
    pub fn into_inner(self) -> T {
        self.transport
    }

    /// Send one frame and retry until it is acknowledged or retries fail.
    ///
    /// A failed retry closes the session.
    pub fn send(&mut self, kind: FrameType, payload: &[u8]) -> Result<Frame> {
        let result = self.try_send(kind, payload);
        if matches!(&result, Err(ProtoError::RetriesExhausted { .. })) {
            self.closed = true;
        }
        result
    }

    /// Send one frame before a deadline and retry until it is acknowledged.
    ///
    /// A failed retry closes the session.
    pub(crate) fn send_with_deadline(
        &mut self,
        kind: FrameType,
        payload: &[u8],
        deadline: Instant,
    ) -> Result<Frame> {
        let result = self.try_send_with_deadline(kind, payload, deadline);
        if matches!(&result, Err(ProtoError::RetriesExhausted { .. })) {
            self.closed = true;
        }
        result
    }

    /// Send one frame and retry until it is acknowledged or retries fail.
    ///
    /// A failed retry does not close the session.
    pub fn try_send(&mut self, kind: FrameType, payload: &[u8]) -> Result<Frame> {
        self.try_send_until(kind, payload, None)
    }

    /// Send one frame before a deadline.
    ///
    /// A failed retry does not close the session.
    pub(crate) fn try_send_with_deadline(
        &mut self,
        kind: FrameType,
        payload: &[u8],
        deadline: Instant,
    ) -> Result<Frame> {
        self.try_send_until(kind, payload, Some(deadline))
    }

    fn try_send_until(
        &mut self,
        kind: FrameType,
        payload: &[u8],
        deadline: Option<Instant>,
    ) -> Result<Frame> {
        if self.closed {
            return Err(ProtoError::Closed);
        }

        let attempts = self.max_retries.saturating_add(1);

        'attempts: for _ in 0..attempts {
            let mut timeout = self.attempt_timeout(deadline)?;
            let frame = Frame::new(kind, self.request_id, self.sequence, payload.to_vec())?;

            // Bound the write by the remaining budget so a blocked transport
            // cannot hold the request past its deadline.
            self.transport.set_write_timeout(Some(timeout))?;
            match write_frame(&mut self.transport, &frame) {
                Ok(()) => {}
                Err(ProtoError::Io(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    if deadline.is_some() {
                        return Err(ProtoError::Timeout);
                    }
                    continue 'attempts;
                }
                Err(error) => return Err(error),
            }

            timeout = timeout.min(self.attempt_timeout(deadline)?);
            self.transport.set_read_timeout(Some(timeout))?;

            match read_frame(&mut self.transport) {
                Ok(Incoming::Frame(response)) => match response.kind {
                    FrameType::Ack => {
                        if response.request_id == self.request_id
                            && response.sequence == self.sequence
                        {
                            self.sequence = self.sequence.wrapping_add(1);
                            return Ok(frame);
                        }

                        return Err(ProtoError::UnexpectedFrame {
                            expected: "ACK with matching request id and sequence",
                            actual: FrameType::Ack,
                        });
                    }
                    FrameType::Nak => {
                        if response.request_id == self.request_id
                            && response.sequence == self.sequence
                        {
                            continue 'attempts;
                        }

                        return Err(ProtoError::UnexpectedFrame {
                            expected: "NAK with matching request id and sequence",
                            actual: FrameType::Nak,
                        });
                    }
                    FrameType::Error => {
                        if response.request_id != self.request_id {
                            return Err(ProtoError::UnexpectedFrame {
                                expected: "ACK",
                                actual: FrameType::Error,
                            });
                        }

                        let error = decode_payload::<ErrorPayload>(&response.payload)?;
                        self.send_ack(response.request_id, response.sequence)?;
                        return Err(ProtoError::Agent(error));
                    }
                    FrameType::Hello => return Err(ProtoError::GuestRebooted),
                    other => {
                        return Err(ProtoError::UnexpectedFrame {
                            expected: "ACK",
                            actual: other,
                        });
                    }
                },
                Ok(Incoming::Corrupt { .. }) => continue 'attempts,
                Ok(Incoming::Eof) => {
                    self.closed = true;
                    return Err(ProtoError::Closed);
                }
                Err(error) if is_timeout_error(&error) => continue 'attempts,
                Err(error) => return Err(error),
            }
        }

        if let Some(deadline) = deadline
            && Instant::now() >= deadline
        {
            return Err(ProtoError::Timeout);
        }

        Err(ProtoError::RetriesExhausted {
            request_id: self.request_id,
            sequence: self.sequence,
            attempts,
        })
    }

    fn attempt_timeout(&self, deadline: Option<Instant>) -> Result<Duration> {
        match deadline {
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(Instant::now());

                if remaining.is_zero() {
                    return Err(ProtoError::Timeout);
                }

                Ok(self.timeout.min(remaining))
            }
            None => Ok(self.timeout),
        }
    }

    /// Receive one frame with the session timeout.
    pub fn recv(&mut self) -> Result<Frame> {
        self.recv_with_timeout(self.timeout)
    }

    /// Receive one frame with an explicit timeout.
    ///
    /// Valid data frames are acknowledged. Corrupt frames get a NAK. Stray
    /// ACK and NAK frames are ignored.
    pub fn recv_with_timeout(&mut self, timeout: Duration) -> Result<Frame> {
        if self.closed {
            return Err(ProtoError::Closed);
        }

        self.transport.set_read_timeout(Some(timeout))?;

        loop {
            match read_frame(&mut self.transport) {
                Ok(Incoming::Frame(frame)) => {
                    if matches!(frame.kind, FrameType::Ack | FrameType::Nak) {
                        continue;
                    }

                    self.send_ack(frame.request_id, frame.sequence)?;
                    self.consecutive_corrupt = 0;
                    return Ok(frame);
                }
                Ok(Incoming::Corrupt {
                    request_id,
                    sequence,
                }) => {
                    self.send_nak(request_id, sequence)?;
                    self.consecutive_corrupt = self.consecutive_corrupt.saturating_add(1);
                    if self.consecutive_corrupt >= MAX_CORRUPT_FRAMES {
                        self.closed = true;
                        return Err(ProtoError::CrcFailed {
                            attempts: self.consecutive_corrupt,
                        });
                    }
                }
                Ok(Incoming::Eof) => {
                    self.closed = true;
                    return Err(ProtoError::Closed);
                }
                Err(error) if is_timeout_error(&error) => return Err(ProtoError::Timeout),
                Err(error) => return Err(error),
            }
        }
    }

    fn send_ack(&mut self, request_id: u32, sequence: u32) -> Result<()> {
        self.send_control(FrameType::Ack, request_id, sequence)
    }

    fn send_nak(&mut self, request_id: u32, sequence: u32) -> Result<()> {
        self.send_control(FrameType::Nak, request_id, sequence)
    }

    fn send_control(&mut self, kind: FrameType, request_id: u32, sequence: u32) -> Result<()> {
        let frame = Frame::new(kind, request_id, sequence, Vec::new())?;
        write_frame(&mut self.transport, &frame)
    }
}

fn is_timeout_error(error: &ProtoError) -> bool {
    match error {
        ProtoError::Timeout => true,
        ProtoError::Io(error) => matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ),
        _ => false,
    }
}
