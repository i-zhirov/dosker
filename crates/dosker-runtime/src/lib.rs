//! Container state, control protocol, stale recovery, and the supervisor.

pub mod paths;
pub mod ports;
pub mod protocol;
pub mod record;
pub mod stale;
pub mod store;
pub mod supervisor;
pub mod volume;

pub use paths::RuntimePaths;
pub use ports::{PortConflict, find_conflict};
pub use protocol::{
    ContainerSummary, CpDirection, DEFAULT_EXEC_TIMEOUT_MS, DEFAULT_STOP_GRACE_MS, ErrorCode,
    FrameError, FrameRead, MAX_CP_BYTES, MAX_MESSAGE_BYTES, ProtocolError, Reply, Request,
    RequestParse, Response, parse_request, read_frame, write_message,
};
pub use record::{CONTROL_SOCKET, ContainerRecord, PortMapping, VolumeKind, VolumeMount};
pub use stale::{HostProbe, SystemProbe, recover_stale};
pub use store::{StateStore, StoreError};
pub use supervisor::{
    ACCEPT_POLL, ATTACH_POLL, DEFAULT_READY_TIMEOUT, SERVER_READ_TIMEOUT, Supervisor,
    SupervisorConfig, SupervisorError,
};
pub use volume::{Volume, VolumeError, VolumeHolder, VolumeLock, VolumeMeta, VolumeStore};
