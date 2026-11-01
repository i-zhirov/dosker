//! Container state, control protocol, stale recovery, and the supervisor.

pub mod paths;
pub mod protocol;
pub mod record;
pub mod stale;
pub mod store;
pub mod supervisor;

pub use paths::RuntimePaths;
pub use protocol::{
    ContainerSummary, DEFAULT_EXEC_TIMEOUT_MS, DEFAULT_STOP_GRACE_MS, ErrorCode, FrameError,
    FrameRead, MAX_MESSAGE_BYTES, ProtocolError, Reply, Request, RequestParse, Response,
    parse_request, read_frame, write_message,
};
pub use record::{CONTROL_SOCKET, ContainerRecord};
pub use stale::{HostProbe, SystemProbe, recover_stale};
pub use store::{StateStore, StoreError};
pub use supervisor::{
    ACCEPT_POLL, DEFAULT_READY_TIMEOUT, SERVER_READ_TIMEOUT, Supervisor, SupervisorConfig,
    SupervisorError,
};
