//! OCI manifest, index, and descriptor models.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::ImageError;

/// The DOSker tree-layer media type.
pub const MEDIA_TYPE_LAYER: &str = "application/vnd.dosker.layer.v1.tar";
/// The DOSker image-config media type.
pub const MEDIA_TYPE_CONFIG: &str = "application/vnd.dosker.image.config.v1+json";
/// The DOSker booter media type.
pub const MEDIA_TYPE_BOOTER: &str = "application/vnd.dosker.booter.v1.raw";
/// The OCI manifest media type.
pub const MEDIA_TYPE_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
/// The OCI index media type.
pub const MEDIA_TYPE_INDEX: &str = "application/vnd.oci.image.index.v1+json";
/// The OCI tag annotation.
pub const ANNOTATION_REF_NAME: &str = "org.opencontainers.image.ref.name";
/// The DOSker raw-disk annotation.
pub const ANNOTATION_RAW_DISK_DIGEST: &str = "org.dosker.rawdisk.digest";
/// The OCI layout version.
pub const OCI_LAYOUT_VERSION: &str = "1.0.0";

/// One OCI descriptor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub media_type: String,
    pub digest: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,
}

/// An OCI manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub media_type: String,
    pub config: Descriptor,
    pub layers: Vec<Descriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,
}

impl Manifest {
    /// Builds a manifest with the standard schema and media type.
    pub fn new(
        config: Descriptor,
        layers: Vec<Descriptor>,
        annotations: Option<BTreeMap<String, String>>,
    ) -> Self {
        Self {
            schema_version: 2,
            media_type: MEDIA_TYPE_MANIFEST.to_owned(),
            config,
            layers,
            annotations,
        }
    }

    /// Returns the raw-disk digest annotation.
    pub fn raw_disk_digest(&self) -> Option<&str> {
        self.annotations
            .as_ref()
            .and_then(|map| map.get(ANNOTATION_RAW_DISK_DIGEST))
            .map(String::as_str)
    }

    /// Validates the manifest against the image kind.
    pub fn validate(&self, booter: bool) -> Result<(), ImageError> {
        if self.schema_version != 2 || self.media_type != MEDIA_TYPE_MANIFEST {
            return Err(invalid("unsupported manifest schema or media type"));
        }
        validate_descriptor(&self.config, MEDIA_TYPE_CONFIG)?;
        if booter {
            if self.layers.len() != 1 || self.layers[0].media_type != MEDIA_TYPE_BOOTER {
                return Err(invalid("a booter has exactly one booter layer"));
            }
            if self.raw_disk_digest().is_some() {
                return Err(invalid("a booter has no raw-disk annotation"));
            }
        } else {
            if self.layers.is_empty()
                || self
                    .layers
                    .iter()
                    .any(|layer| layer.media_type != MEDIA_TYPE_LAYER)
            {
                return Err(invalid("a tree image needs tree layers"));
            }
            if let Some(digest) = self.raw_disk_digest()
                && !valid_digest(digest)
            {
                return Err(invalid("the raw-disk annotation is not a digest"));
            }
        }
        Ok(())
    }
}

/// An OCI image index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Index {
    pub schema_version: u32,
    pub media_type: String,
    pub manifests: Vec<Descriptor>,
}

impl Index {
    /// Creates an empty index.
    pub fn new() -> Self {
        Self {
            schema_version: 2,
            media_type: MEDIA_TYPE_INDEX.to_owned(),
            manifests: Vec::new(),
        }
    }

    /// Adds one descriptor.
    pub fn add(&mut self, descriptor: Descriptor) {
        self.manifests.push(descriptor);
    }

    /// Finds the descriptor for a tag.
    pub fn find_ref(&self, name: &str) -> Option<&Descriptor> {
        self.manifests.iter().find(|descriptor| {
            descriptor
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.get(ANNOTATION_REF_NAME))
                .is_some_and(|value| value == name)
        })
    }

    /// Finds every descriptor for a digest.
    pub fn find_digest(&self, digest: &str) -> Vec<&Descriptor> {
        self.manifests
            .iter()
            .filter(|descriptor| descriptor.digest == digest)
            .collect()
    }

    /// Removes one tag and returns its descriptor.
    pub fn remove_ref(&mut self, name: &str) -> Option<Descriptor> {
        let position = self.manifests.iter().position(|descriptor| {
            descriptor
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.get(ANNOTATION_REF_NAME))
                .is_some_and(|value| value == name)
        })?;
        Some(self.manifests.remove(position))
    }

    /// Removes every descriptor for a digest.
    pub fn remove_digest(&mut self, digest: &str) -> Vec<Descriptor> {
        let mut removed = Vec::new();
        let mut kept = Vec::new();
        for descriptor in self.manifests.drain(..) {
            if descriptor.digest == digest {
                removed.push(descriptor);
            } else {
                kept.push(descriptor);
            }
        }
        self.manifests = kept;
        removed
    }

    /// Validates the index and its descriptors.
    pub fn validate(&self) -> Result<(), ImageError> {
        if self.schema_version != 2 || self.media_type != MEDIA_TYPE_INDEX {
            return Err(invalid("unsupported index schema or media type"));
        }
        for descriptor in &self.manifests {
            validate_descriptor(descriptor, MEDIA_TYPE_MANIFEST)?;
        }
        Ok(())
    }
}

impl Default for Index {
    fn default() -> Self {
        Self::new()
    }
}

/// The OCI layout marker file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OciLayout {
    #[serde(rename = "imageLayoutVersion")]
    pub image_layout_version: String,
}

impl OciLayout {
    /// Creates the version-1 layout marker.
    pub fn new() -> Self {
        Self {
            image_layout_version: OCI_LAYOUT_VERSION.to_owned(),
        }
    }
}

impl Default for OciLayout {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_descriptor(descriptor: &Descriptor, media_type: &str) -> Result<(), ImageError> {
    if descriptor.media_type != media_type {
        return Err(ImageError::UnsupportedMediaType(
            descriptor.media_type.clone(),
        ));
    }
    if !valid_digest(&descriptor.digest) || descriptor.size == 0 {
        return Err(invalid("descriptor digest or size is invalid"));
    }
    Ok(())
}

pub(crate) fn valid_digest(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn invalid(reason: &str) -> ImageError {
    ImageError::InvalidManifest {
        reason: reason.to_owned(),
    }
}
