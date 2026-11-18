//! Build engine for Doskerfiles.

pub mod build;
pub mod cache;
pub mod context;
pub mod copy;
pub mod error;
pub mod lock;
pub mod normalize;
pub mod reporter;
pub mod run_host;
pub mod run_in_dos;
pub mod state;

pub use build::{BuildOptions, BuildOutcome, Builder};
pub use cache::{BuildCache, CacheLookup};
pub use context::{BuildContext, ResolvedSource, SourceEntry, SourceFile, SourceKind};
pub use copy::{CopyPlan, plan_copy};
pub use error::BuildError;
pub use lock::{EngineResolver, HostEngineResolver, ResolvedEngine, resolve_lock, write_lock};
pub use normalize::{
    build_config_digest, is_reserved_path, normalize_instruction, render_tokens,
    resolve_copy_destination, resolve_dos_path,
};
pub use reporter::BuildReporter;
pub use run_host::{HostRunRequest, run_host};
pub use run_in_dos::{
    InDosRunOutcome, InDosRunRequest, InDosRunner, RUN_BOOT_TIMEOUT, RUN_EMPTY_OUTPUT_NOTE,
    RUN_STOP_GRACE, RUN_TRUNCATED_NOTE, RunDelta, build_delta,
};
pub use state::BuildState;
