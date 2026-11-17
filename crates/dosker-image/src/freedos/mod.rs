//! The pinned FreeDOS base and the inputs it is built from.

pub mod build;
pub mod inputs;
pub mod store;

pub use build::{
    BASE_AUTOEXEC_BYTES, BYTES_PER_CLUSTER, BYTES_PER_SECTOR, BaseImage, CONFIG_SYS_BYTES,
    DISK_SIZE, HEADS, MEDIA_DESCRIPTOR, PARTITION_SECTORS, PARTITION_START, ROOT_DIR_ENTRIES,
    SECTORS_PER_TRACK, VOLUME_ID, VOLUME_LABEL, build_base_image,
};
pub use dosker_agent_proto::AGENT_VERSION;
pub use inputs::{
    BOOT_SECTOR_TEMPLATE, BaseInputs, FREEDOS_AGENT_SHA256, FREEDOS_BOOT_SECTOR_SHA256,
    FREEDOS_COMMAND_SHA256, FREEDOS_KERNEL_SHA256, FREEDOS_MBR_SHA256, FREEDOS_VERSION,
    FreedosBinaries, FreedosPins, FreedosVersion, MBR_TEMPLATE, default_agent_executable,
    load_agent_executable, load_freedos_binaries,
};
pub use store::ensure_freedos_base;
