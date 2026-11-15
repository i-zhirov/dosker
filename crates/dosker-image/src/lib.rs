//! Image references, FAT image access, and the local image store.

pub mod boot;
pub mod cache;
pub mod canonical;
pub mod config;
pub mod error;
pub mod fat;
pub mod image;
pub mod layer;
pub mod manifest;
pub mod oci;
pub mod path;
pub mod reference;
pub mod tree;

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
pub use image::ImageMedia;
pub use layer::{
    FIXED_TIMESTAMP_SECONDS, LAYER_META_FILE, LAYER_SCHEMA_VERSION, Layer, LayerEntry, LayerMeta,
    effective_boot, fixed_timestamp,
};
pub use manifest::{
    ANNOTATION_RAW_DISK_DIGEST, ANNOTATION_REF_NAME, Descriptor, Index, MEDIA_TYPE_BOOTER,
    MEDIA_TYPE_CONFIG, MEDIA_TYPE_INDEX, MEDIA_TYPE_LAYER, MEDIA_TYPE_MANIFEST, Manifest,
    OCI_LAYOUT_VERSION, OciLayout,
};
pub use oci::{Blob, Image, IndexLock, OciStore, RemoveOutcome};
pub use path::{DosName, DosPath, normalize_tree_member};
pub use reference::{ImageDigest, ImageName, ImageRef, ImageReference, ImageTag};
pub use tree::{Attributes, LayerDelta, Tree, TreeDir, TreeFile, TreeKind, TreeNode};
