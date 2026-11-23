//! Image references, FAT image access, and the local image store.

pub mod archive;
pub mod boot;
pub mod cache;
pub mod canonical;
pub mod config;
pub mod error;
pub mod fat;
pub mod freedos;
pub mod host;
pub mod image;
pub mod inject;
pub mod layer;
pub mod licenses;
pub mod manifest;
pub mod oci;
pub mod path;
pub mod project;
pub mod reference;
pub mod tree;
pub mod volume;

pub use archive::{
    ARCHIVE_MODE, ARCHIVE_MTIME_SECONDS, LoadReport, SaveReport, load_image, save_image,
};
pub use boot::{BlobRef, BootLayout, BootMetadata, FatType, Geometry};
pub use cache::{CacheKeyInput, CacheRecord, CacheStore, cache_key};
pub use canonical::{canonical_digest, canonical_json, sha256_digest};
pub use config::{
    DosFlavour, HistoryEntry, ImageConfig, MachineProfile, PortDeclaration, PortProtocol,
    VolumeDeclaration, config_digest,
};
pub use error::ImageError;
pub use fat::{
    CONFIG_DIR, CONFIG_FILE, CONFIG_PATH, DEFAULT_BAUD, DEFAULT_CONTROL_PORT, FatLayout, FatRegion,
    fat_region, read_boot_metadata, read_file, read_tree, write_config,
};
pub use freedos::{
    BASE_AUTOEXEC_BYTES, BYTES_PER_CLUSTER, BYTES_PER_SECTOR, BaseImage, BaseInputs,
    CONFIG_SYS_BYTES, DISK_SIZE, FREEDOS_VERSION, FreedosBinaries, FreedosPins, FreedosVersion,
    HEADS, MEDIA_DESCRIPTOR, PARTITION_SECTORS, PARTITION_START, ROOT_DIR_ENTRIES,
    SECTORS_PER_TRACK, VOLUME_ID, VOLUME_LABEL, build_base_image, default_agent_executable,
    ensure_freedos_base, load_agent_executable, load_freedos_binaries,
};
pub use host::{read_host_tree, validate_host_tree, write_host_tree};
pub use image::ImageMedia;
pub use inject::{
    AGENT_PATH, AUTOEXEC_OLD_PATH, AgentInjection, apply_agent_injection, plan_agent_injection,
    starts_agent,
};
pub use layer::{
    FIXED_TIMESTAMP_SECONDS, LAYER_META_FILE, LAYER_SCHEMA_VERSION, Layer, LayerEntry, LayerMeta,
    effective_boot, fixed_timestamp,
};
pub use licenses::{agent_license, freedos_base_licenses, freedos_license};
pub use manifest::{
    ANNOTATION_RAW_DISK_DIGEST, ANNOTATION_REF_NAME, ANNOTATION_SOURCE_DISK_DIGEST, Descriptor,
    Index, MEDIA_TYPE_BOOTER, MEDIA_TYPE_CONFIG, MEDIA_TYPE_INDEX, MEDIA_TYPE_LAYER,
    MEDIA_TYPE_MANIFEST, Manifest, OCI_LAYOUT_VERSION, OciLayout,
};
pub use oci::{Blob, CommittedImage, Image, IndexLock, OciStore, RemoveOutcome};
pub use path::{DosName, DosPath, normalize_tree_member};
pub use project::{apply_boot_metadata, project_tree};
pub use reference::{
    DEFAULT_REGISTRY, ImageDigest, ImageName, ImageRef, ImageReference, ImageTag, Namespace,
    REGISTRY_ENV, ReferenceSelector, RegistryHost, RegistryReference, Repository,
    default_registry_url,
};
pub use tree::{Attributes, LayerDelta, Tree, TreeDir, TreeFile, TreeKind, TreeNode};
pub use volume::{
    DEFAULT_VOLUME_SIZE, FLOPPY_1440_SIZE, FLOPPY_2880_SIZE, MAX_PARTITIONED_VOLUME_SIZE,
    MIN_PARTITIONED_VOLUME_SIZE, VolumeImage, VolumeLayout, create_volume,
};
