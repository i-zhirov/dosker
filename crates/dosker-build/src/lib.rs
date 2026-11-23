//! Build engine for Doskerfiles.

pub mod build;
pub mod cache;
pub mod context;
pub mod copy;
pub mod error;
pub mod lock;
pub mod normalize;
pub mod registry;
pub mod reporter;
pub mod run_host;
pub mod run_in_dos;
pub mod state;

pub use build::{BuildOptions, BuildOutcome, Builder, LICENSE_MISSING_WARNING};
pub use cache::{BuildCache, CacheLookup};
pub use context::{BuildContext, ResolvedSource, SourceEntry, SourceFile, SourceKind};
pub use copy::{CopyPlan, plan_copy};
pub use error::BuildError;
pub use lock::{
    EngineResolver, HostEngineResolver, ResolvedEngine, check_engine_pin, record_engine_pin,
    resolve_lock, write_lock,
};
pub use normalize::{
    build_config_digest, is_reserved_path, normalize_instruction, render_tokens,
    resolve_copy_destination, resolve_dos_path,
};
pub use registry::{
    ImageValidation, NamespaceOwner, Namespaces, RegistryError, RegistryMetadata, RegistryTarget,
    coverage_misses, covers_directory, covers_glob, notice_present, parse_metadata,
    parse_namespaces, registry_root, requires_notice, resolve_target, scan_tags, validate_image,
    validate_metadata, validate_namespaces, validate_registry, validate_target,
};
pub use reporter::BuildReporter;
pub use run_host::{HostRunRequest, run_host};
pub use run_in_dos::{
    EngineInDosRunner, EngineOpener, InDosRunOutcome, InDosRunRequest, InDosRunner,
    QemuEngineOpener, RUN_BOOT_TIMEOUT, RUN_EMPTY_OUTPUT_NOTE, RUN_STOP_GRACE, RUN_TRUNCATED_NOTE,
    RunDelta, build_delta,
};
pub use state::BuildState;
