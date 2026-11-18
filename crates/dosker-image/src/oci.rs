//! Local OCI image store, import, tags, and disk materialization.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use dosker_platform::CopyOutcome;
use dosker_platform::fs::{copy_full, copy_reflink, ensure_dir_mode, write_atomic};
use fs2::FileExt;
use sha2::{Digest, Sha256};

use crate::boot::{BootLayout, BootMetadata};
use crate::canonical::{canonical_json, hex, sha256_digest};
use crate::config::{DosFlavour, HistoryEntry, ImageConfig, MachineProfile};
use crate::error::ImageError;
use crate::fat::{read_boot_metadata, read_tree};
use crate::image::ImageMedia;
use crate::inject::{apply_agent_injection, plan_agent_injection};
use crate::layer::{Layer, effective_boot};
use crate::manifest::{
    ANNOTATION_RAW_DISK_DIGEST, ANNOTATION_REF_NAME, ANNOTATION_SOURCE_DISK_DIGEST, Descriptor,
    Index, MEDIA_TYPE_BOOTER, MEDIA_TYPE_CONFIG, MEDIA_TYPE_LAYER, MEDIA_TYPE_MANIFEST, Manifest,
    OciLayout,
};
use crate::reference::{ImageDigest, ImageRef, ImageReference};
use crate::tree::Tree;

const DIR_MODE: u32 = 0o700;
#[cfg(unix)]
const BLOB_MODE: u32 = 0o444;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A stored blob.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blob {
    pub digest: String,
    pub size: u64,
    pub created: bool,
}

/// An image as presented to the runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub reference: String,
    pub name: String,
    pub tag: String,
    pub media: ImageMedia,
    pub manifest_digest: String,
    pub config_digest: String,
    pub layer_digests: Vec<String>,
    pub raw_disk_digest: Option<String>,
    pub size: u64,
    pub source: String,
    pub disk_interface: String,
    pub agent_version: Option<String>,
    pub cmd: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Image {
    /// Returns true for an opaque booter image.
    pub fn is_booter(&self) -> bool {
        self.media.is_booter()
    }

    /// Returns true when the image expects the DOS agent.
    pub fn expects_agent(&self) -> bool {
        self.media.expects_agent()
    }
}

/// An image written by the build engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommittedImage {
    pub manifest_digest: String,
    pub config_digest: String,
    pub layer_digests: Vec<String>,
}

/// The result of removing one reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoveOutcome {
    pub untagged: bool,
    pub removed_manifest: bool,
    pub deleted_blobs: usize,
}

/// An exclusive lock on the image index.
pub struct IndexLock {
    file: File,
}

impl Drop for IndexLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// The local OCI image store under `<data-root>/images`.
#[derive(Clone, Debug)]
pub struct OciStore {
    root: PathBuf,
}

impl OciStore {
    /// Creates a store for one data root.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Returns the OCI layout directory.
    pub fn layout_dir(&self) -> PathBuf {
        self.root.join("images")
    }

    /// Returns the blob directory.
    pub fn blobs_dir(&self) -> PathBuf {
        self.layout_dir().join("blobs").join("sha256")
    }

    /// Returns the path of one digest blob.
    pub fn blob_path(&self, digest: &str) -> Result<PathBuf, ImageError> {
        let parsed = ImageDigest::parse(digest)?;
        let hex = parsed
            .as_str()
            .strip_prefix("sha256:")
            .expect("validated digest");
        Ok(self.blobs_dir().join(hex))
    }

    /// Creates the layout directories and marker.
    pub fn ensure_layout(&self) -> Result<(), ImageError> {
        ensure_dir_mode(&self.layout_dir(), DIR_MODE)?;
        ensure_dir_mode(&self.blobs_dir(), DIR_MODE)?;
        let marker = self.layout_dir().join("oci-layout");
        if !marker.exists() {
            write_atomic(&marker, &canonical_json(&OciLayout::new())?)?;
        }
        Ok(())
    }

    /// Takes the exclusive image index lock.
    pub fn lock_index(&self) -> Result<IndexLock, ImageError> {
        self.ensure_layout()?;
        let path = self.layout_dir().join(".lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|source| ImageError::Lock {
                path: path.clone(),
                source,
            })?;
        file.lock_exclusive().map_err(|source| ImageError::Lock {
            path: path.clone(),
            source,
        })?;
        Ok(IndexLock { file })
    }

    /// Stores a small blob.
    pub fn put_blob(&self, bytes: &[u8]) -> Result<Blob, ImageError> {
        self.ensure_layout()?;
        let digest = sha256_digest(bytes);
        let path = self.blob_path(&digest)?;
        if path.exists() {
            verify_blob(&path, &digest, bytes.len() as u64)?;
            return Ok(Blob {
                digest,
                size: bytes.len() as u64,
                created: false,
            });
        }
        write_atomic(&path, bytes)?;
        set_blob_mode(&path)?;
        Ok(Blob {
            digest,
            size: bytes.len() as u64,
            created: true,
        })
    }

    /// Stores a file blob while hashing it.
    pub fn put_blob_from_file(&self, source: &Path) -> Result<Blob, ImageError> {
        self.ensure_layout()?;
        let temp = self.blobs_dir().join(format!(
            ".tmp.{}.{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let mut input = File::open(source).map_err(|source_error| ImageError::Io {
            path: source.to_path_buf(),
            source: source_error,
        })?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|source_error| ImageError::Io {
                path: temp.clone(),
                source: source_error,
            })?;
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = input
                .read(&mut buffer)
                .map_err(|source_error| ImageError::Io {
                    path: source.to_path_buf(),
                    source: source_error,
                })?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            output
                .write_all(&buffer[..read])
                .map_err(|source_error| ImageError::Io {
                    path: temp.clone(),
                    source: source_error,
                })?;
            size += read as u64;
        }
        output.sync_all().map_err(|source_error| ImageError::Io {
            path: temp.clone(),
            source: source_error,
        })?;
        drop(output);
        let digest = format!("sha256:{}", hex(&hasher.finalize()));
        let destination = self.blob_path(&digest)?;
        if destination.exists() {
            verify_blob(&destination, &digest, size)?;
            let _ = fs::remove_file(&temp);
            return Ok(Blob {
                digest,
                size,
                created: false,
            });
        }
        set_blob_mode(&temp)?;
        fs::rename(&temp, &destination).map_err(|source_error| ImageError::Io {
            path: destination,
            source: source_error,
        })?;
        Ok(Blob {
            digest,
            size,
            created: true,
        })
    }

    /// Writes a canonical layer tar and returns its descriptor.
    pub fn put_layer(&self, layer: &Layer) -> Result<Descriptor, ImageError> {
        let tar = layer.write_tar()?;
        let blob = self.put_blob(&tar)?;
        Ok(descriptor(MEDIA_TYPE_LAYER, &blob.digest, blob.size))
    }

    /// Returns a tree-layer descriptor for a stored blob.
    pub fn layer_descriptor(&self, digest: &str) -> Result<Descriptor, ImageError> {
        let path = self.blob_path(digest)?;
        let metadata = fs::metadata(&path).map_err(|source| ImageError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(descriptor(MEDIA_TYPE_LAYER, digest, metadata.len()))
    }

    /// Reads a canonical layer tar by digest.
    pub fn load_layer(&self, digest: &str) -> Result<Layer, ImageError> {
        let bytes = self.read_blob(digest)?;
        Layer::read_tar(&bytes)
    }

    /// Loads a manifest by tag or digest.
    pub fn load_manifest(&self, reference: &ImageRef) -> Result<Manifest, ImageError> {
        let (_, manifest, _) = self.resolve(reference)?;
        Ok(manifest)
    }

    /// Loads an image config by tag or digest.
    pub fn load_config(&self, reference: &ImageRef) -> Result<ImageConfig, ImageError> {
        let (_, _, config) = self.resolve(reference)?;
        Ok(config)
    }

    /// Loads the tree by applying every layer in order.
    pub fn load_tree(&self, reference: &ImageRef) -> Result<Tree, ImageError> {
        let (_, manifest, config) = self.resolve(reference)?;
        if config.booter {
            return Err(ImageError::NoTreeImage(reference.to_string()));
        }
        let mut tree = Tree::new();
        let mut previous: Option<String> = None;
        for descriptor in &manifest.layers {
            let layer = self.load_layer(&descriptor.digest)?;
            layer.validate_parent(previous.as_deref())?;
            layer.apply_to(&mut tree)?;
            previous = Some(descriptor.digest.clone());
        }
        Ok(tree)
    }

    /// Loads the effective boot metadata after applying every layer override.
    pub fn load_effective_boot(&self, reference: &ImageRef) -> Result<BootMetadata, ImageError> {
        let (_, manifest, config) = self.resolve(reference)?;
        if config.booter {
            return Ok(config.boot);
        }
        let mut layers = Vec::with_capacity(manifest.layers.len());
        for descriptor in &manifest.layers {
            layers.push(self.load_layer(&descriptor.digest)?);
        }
        Ok(effective_boot(&config, &layers))
    }

    /// Writes a config and manifest, then adds or replaces a tag.
    pub fn commit_image(
        &self,
        reference: Option<&ImageReference>,
        config: &ImageConfig,
        layers: Vec<Descriptor>,
        annotations: BTreeMap<String, String>,
    ) -> Result<CommittedImage, ImageError> {
        config.validate()?;
        let config_bytes = canonical_json(config)?;
        let config_digest = sha256_digest(&config_bytes);
        let manifest = Manifest::new(
            descriptor(MEDIA_TYPE_CONFIG, &config_digest, config_bytes.len() as u64),
            layers.clone(),
            if annotations.is_empty() {
                None
            } else {
                Some(annotations)
            },
        );
        manifest.validate(config.booter)?;
        let manifest_bytes = canonical_json(&manifest)?;
        let config_blob = self.put_blob(&config_bytes)?;
        let manifest_blob = self.put_blob(&manifest_bytes)?;
        if let Some(reference) = reference {
            let _lock = self.lock_index()?;
            let mut index = self.load_index()?;
            index.remove_ref(&reference.to_string());
            let mut refs = BTreeMap::new();
            refs.insert(ANNOTATION_REF_NAME.to_owned(), reference.to_string());
            index.add(Descriptor {
                media_type: MEDIA_TYPE_MANIFEST.to_owned(),
                digest: manifest_blob.digest.clone(),
                size: manifest_blob.size,
                annotations: Some(refs),
            });
            self.write_index(&index)?;
        }
        Ok(CommittedImage {
            manifest_digest: manifest_blob.digest,
            config_digest: config_blob.digest,
            layer_digests: layers.iter().map(|layer| layer.digest.clone()).collect(),
        })
    }

    /// Reads and verifies a blob.
    pub fn read_blob(&self, digest: &str) -> Result<Vec<u8>, ImageError> {
        let path = self.blob_path(digest)?;
        let bytes = fs::read(&path).map_err(|source| ImageError::Io {
            path: path.clone(),
            source,
        })?;
        let actual = sha256_digest(&bytes);
        if actual != digest {
            return Err(ImageError::DigestMismatch {
                expected: digest.to_owned(),
                actual,
            });
        }
        Ok(bytes)
    }

    /// Copies a blob to a destination.
    pub fn copy_blob_to(
        &self,
        digest: &str,
        destination: &Path,
    ) -> Result<CopyOutcome, ImageError> {
        copy_reflink(&self.blob_path(digest)?, destination).map_err(ImageError::from)
    }

    /// Copies the raw disk of an image to a destination.
    pub fn copy_disk_to(
        &self,
        reference: &ImageRef,
        destination: &Path,
    ) -> Result<CopyOutcome, ImageError> {
        let image = self.load(reference)?;
        let Some(digest) = image.raw_disk_digest else {
            return Err(ImageError::MissingRawDisk(reference.to_string()));
        };
        let path = self.blob_path(&digest)?;
        if !path.exists() {
            return Err(ImageError::MissingRawDisk(reference.to_string()));
        }
        self.copy_blob_to(&digest, destination)
    }

    /// Imports a raw disk or a booter blob.
    pub fn import(
        &self,
        source: &Path,
        reference: &ImageReference,
        media: ImageMedia,
        agent: Option<&[u8]>,
        now: DateTime<Utc>,
    ) -> Result<Image, ImageError> {
        let _lock = self.lock_index()?;
        let mut index = self.load_index()?;
        if index.find_ref(&reference.to_string()).is_some() {
            return Err(ImageError::AlreadyExists(reference.clone()));
        }

        let mut created = Vec::new();
        let result = (|| {
            let size = fs::metadata(source).map_or(0, |metadata| metadata.len());
            let mut config = import_config(source, media, size, now);
            let mut annotations = BTreeMap::new();
            let layers = match media {
                ImageMedia::Booter => {
                    let raw = self.put_blob_from_file(source)?;
                    if raw.created {
                        created.push(raw.digest.clone());
                    }
                    config.boot = BootMetadata::opaque(raw.size);
                    config.booter = true;
                    config.dos_flavour = DosFlavour::None;
                    vec![descriptor(MEDIA_TYPE_BOOTER, &raw.digest, raw.size)]
                }
                ImageMedia::Disk => {
                    let agent = agent.ok_or(ImageError::AgentMissing)?;
                    let source_boot = read_boot_metadata(source)?;
                    let original_tree = read_tree(source)?;
                    let injection = plan_agent_injection(&original_tree, agent)?;

                    let raw = if injection.applied {
                        self.ensure_layout()?;
                        let temp = self.blobs_dir().join(format!(
                            ".import.{}.{}",
                            std::process::id(),
                            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
                        ));
                        let stored = copy_full(source, &temp)
                            .map_err(ImageError::from)
                            .and_then(|()| apply_agent_injection(&temp, &injection))
                            .and_then(|()| self.put_blob_from_file(&temp));
                        let _ = fs::remove_file(&temp);
                        stored?
                    } else {
                        self.put_blob_from_file(source)?
                    };
                    if raw.created {
                        created.push(raw.digest.clone());
                    }

                    let blob_path = self.blob_path(&raw.digest)?;
                    let boot = read_boot_metadata(&blob_path)?;
                    if boot != source_boot {
                        return Err(ImageError::Corrupt {
                            path: blob_path,
                            message: "agent injection changed the boot metadata".to_owned(),
                        });
                    }

                    let first = Layer::from_delta(&original_tree.to_delta(), "import", None, None)?;
                    let first_blob = self.put_blob(&first.write_tar()?)?;
                    if first_blob.created {
                        created.push(first_blob.digest.clone());
                    }
                    let first_descriptor =
                        descriptor(MEDIA_TYPE_LAYER, &first_blob.digest, first_blob.size);
                    let mut layers = vec![first_descriptor.clone()];

                    if injection.applied {
                        let second = Layer::from_delta(
                            &injection.delta,
                            "agent-inject",
                            Some(first_descriptor.digest.clone()),
                            None,
                        )?;
                        let second_blob = self.put_blob(&second.write_tar()?)?;
                        if second_blob.created {
                            created.push(second_blob.digest.clone());
                        }
                        layers.push(descriptor(
                            MEDIA_TYPE_LAYER,
                            &second_blob.digest,
                            second_blob.size,
                        ));
                    }

                    config.boot = boot.clone();
                    config.booter = false;
                    config.dos_flavour = DosFlavour::Other;
                    config.agent_version = Some(dosker_agent_proto::AGENT_VERSION.to_owned());
                    config.machine.disk_interface = match boot.layout {
                        BootLayout::MbrPartitioned => "ide".to_owned(),
                        BootLayout::Superfloppy => "floppy".to_owned(),
                        BootLayout::Opaque => unreachable!("disk import is not opaque"),
                    };
                    annotations.insert(ANNOTATION_RAW_DISK_DIGEST.to_owned(), raw.digest.clone());
                    annotations.insert(
                        ANNOTATION_SOURCE_DISK_DIGEST.to_owned(),
                        sha256_file(source)?,
                    );
                    layers
                }
            };
            config.validate()?;
            let config_blob = self.put_blob(&canonical_json(&config)?)?;
            if config_blob.created {
                created.push(config_blob.digest.clone());
            }
            let manifest = Manifest::new(
                descriptor(MEDIA_TYPE_CONFIG, &config_blob.digest, config_blob.size),
                layers,
                if annotations.is_empty() {
                    None
                } else {
                    Some(annotations)
                },
            );
            manifest.validate(media == ImageMedia::Booter)?;
            let manifest_blob = self.put_blob(&canonical_json(&manifest)?)?;
            if manifest_blob.created {
                created.push(manifest_blob.digest.clone());
            }
            let mut refs = BTreeMap::new();
            refs.insert(ANNOTATION_REF_NAME.to_owned(), reference.to_string());
            index.add(Descriptor {
                media_type: MEDIA_TYPE_MANIFEST.to_owned(),
                digest: manifest_blob.digest,
                size: manifest_blob.size,
                annotations: Some(refs),
            });
            self.write_index(&index)?;
            self.load(&ImageRef::Tag(reference.clone()))
        })();
        if result.is_err() {
            for digest in created {
                if let Ok(path) = self.blob_path(&digest) {
                    let _ = fs::remove_file(path);
                }
            }
        }
        result
    }

    /// Loads an image by tag or digest.
    pub fn load(&self, reference: &ImageRef) -> Result<Image, ImageError> {
        let (descriptor, manifest, config) = self.resolve(reference)?;
        let index = self.load_index()?;
        let (name, tag) = match reference {
            ImageRef::Tag(tag) => (
                tag.name().as_str().to_owned(),
                tag.tag().as_str().to_owned(),
            ),
            ImageRef::Digest(digest) => index
                .find_digest(digest.as_str())
                .into_iter()
                .find_map(|descriptor| {
                    descriptor
                        .annotations
                        .as_ref()
                        .and_then(|annotations| annotations.get(ANNOTATION_REF_NAME))
                        .and_then(|value| ImageReference::parse(value).ok())
                        .map(|reference| {
                            (reference.name().to_string(), reference.tag().to_string())
                        })
                })
                .unwrap_or_default(),
        };
        let raw_disk_digest = if config.booter {
            manifest.layers.first().map(|layer| layer.digest.clone())
        } else {
            manifest.raw_disk_digest().map(str::to_owned)
        };
        let size = raw_disk_digest
            .as_deref()
            .and_then(|digest| self.blob_path(digest).ok())
            .and_then(|path| fs::metadata(path).ok())
            .map_or(0, |metadata| metadata.len());
        let last_history = config.history.last();
        let source = last_history
            .and_then(|entry| entry.comment.clone())
            .unwrap_or_default();
        let created_at = last_history
            .map(|entry| entry.created_at)
            .unwrap_or_else(Utc::now);
        Ok(Image {
            reference: reference.to_string(),
            name,
            tag,
            media: if config.booter {
                ImageMedia::Booter
            } else {
                ImageMedia::Disk
            },
            manifest_digest: descriptor.digest,
            config_digest: manifest.config.digest,
            layer_digests: manifest
                .layers
                .iter()
                .map(|layer| layer.digest.clone())
                .collect(),
            raw_disk_digest,
            size,
            source,
            disk_interface: if config.booter {
                "ide".to_owned()
            } else {
                config.machine.disk_interface
            },
            agent_version: config.agent_version,
            cmd: config.cmd,
            created_at,
        })
    }

    /// Lists every tag, sorted by name and tag.
    pub fn list(&self) -> Result<Vec<Image>, ImageError> {
        let index = self.load_index()?;
        let mut images = Vec::new();
        for descriptor in &index.manifests {
            let Some(reference) = descriptor
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.get(ANNOTATION_REF_NAME))
                .and_then(|value| ImageReference::parse(value).ok())
            else {
                continue;
            };
            if let Ok(image) = self.load(&ImageRef::Tag(reference)) {
                images.push(image);
            }
        }
        images.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.tag.cmp(&right.tag))
        });
        Ok(images)
    }

    /// Adds or replaces a tag.
    pub fn tag(&self, source: &ImageRef, target: &ImageReference) -> Result<(), ImageError> {
        let _lock = self.lock_index()?;
        let image = self.load(source)?;
        let mut index = self.load_index()?;
        index.remove_ref(&target.to_string());
        let mut annotations = BTreeMap::new();
        annotations.insert(ANNOTATION_REF_NAME.to_owned(), target.to_string());
        let manifest_path = self.blob_path(&image.manifest_digest)?;
        let size = fs::metadata(&manifest_path)
            .map_err(|source| ImageError::Io {
                path: manifest_path,
                source,
            })?
            .len();
        index.add(Descriptor {
            media_type: MEDIA_TYPE_MANIFEST.to_owned(),
            digest: image.manifest_digest,
            size,
            annotations: Some(annotations),
        });
        self.write_index(&index)
    }

    /// Removes a tag or an untagged digest.
    pub fn remove(&self, reference: &ImageRef) -> Result<RemoveOutcome, ImageError> {
        let _lock = self.lock_index()?;
        let mut index = self.load_index()?;
        match reference {
            ImageRef::Tag(tag) => {
                let target = tag.to_string();
                if index.remove_ref(&target).is_none() {
                    return Err(ImageError::NotFound(tag.clone()));
                }
                self.write_index(&index)?;
                Ok(RemoveOutcome {
                    untagged: true,
                    removed_manifest: false,
                    deleted_blobs: 0,
                })
            }
            ImageRef::Digest(digest) => {
                if !index.find_digest(digest.as_str()).is_empty() {
                    return Err(ImageError::ImageInUse(digest.to_string()));
                }
                let image = self.load(reference)?;
                let manifest_path = self.blob_path(&image.manifest_digest)?;
                let manifest = self.read_blob(&image.manifest_digest)?;
                let manifest: Manifest =
                    serde_json::from_slice(&manifest).map_err(|error| ImageError::Corrupt {
                        path: manifest_path.clone(),
                        message: error.to_string(),
                    })?;
                let mut owned = BTreeSet::new();
                owned.insert(image.manifest_digest.clone());
                owned.insert(manifest.config.digest.clone());
                owned.extend(manifest.layers.iter().map(|layer| layer.digest.clone()));
                if let Some(raw) = manifest.raw_disk_digest() {
                    owned.insert(raw.to_owned());
                }
                fs::remove_file(&manifest_path).map_err(|source| ImageError::Io {
                    path: manifest_path.clone(),
                    source,
                })?;
                let mut referenced = BTreeSet::new();
                for entry in fs::read_dir(self.blobs_dir()).map_err(|source| ImageError::Io {
                    path: self.blobs_dir(),
                    source,
                })? {
                    let entry = entry.map_err(|source| ImageError::Io {
                        path: self.blobs_dir(),
                        source,
                    })?;
                    let path = entry.path();
                    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                        continue;
                    };
                    if name.starts_with(".tmp.") || name.len() != 64 {
                        continue;
                    }
                    let digest = format!("sha256:{name}");
                    let Ok(bytes) = self.read_blob(&digest) else {
                        continue;
                    };
                    let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes) else {
                        continue;
                    };
                    referenced.insert(manifest.config.digest.clone());
                    referenced.extend(manifest.layers.iter().map(|layer| layer.digest.clone()));
                    if let Some(raw) = manifest.raw_disk_digest() {
                        referenced.insert(raw.to_owned());
                    }
                }
                let mut deleted = 0;
                for digest in owned {
                    if referenced.contains(&digest) {
                        continue;
                    }
                    if let Ok(path) = self.blob_path(&digest)
                        && fs::remove_file(path).is_ok()
                    {
                        deleted += 1;
                    }
                }
                self.write_index(&index)?;
                Ok(RemoveOutcome {
                    untagged: false,
                    removed_manifest: true,
                    deleted_blobs: deleted,
                })
            }
        }
    }

    fn resolve(
        &self,
        reference: &ImageRef,
    ) -> Result<(Descriptor, Manifest, ImageConfig), ImageError> {
        let index = self.load_index()?;
        let descriptor = match reference {
            ImageRef::Tag(tag) => index
                .find_ref(&tag.to_string())
                .cloned()
                .ok_or_else(|| ImageError::NotFound(tag.clone()))?,
            ImageRef::Digest(digest) => index
                .find_digest(digest.as_str())
                .into_iter()
                .next()
                .cloned()
                .unwrap_or_else(|| Descriptor {
                    media_type: MEDIA_TYPE_MANIFEST.to_owned(),
                    digest: digest.as_str().to_owned(),
                    size: 0,
                    annotations: None,
                }),
        };
        let manifest_bytes = self.read_blob(&descriptor.digest)?;
        let manifest: Manifest =
            serde_json::from_slice(&manifest_bytes).map_err(|error| ImageError::Corrupt {
                path: self.blob_path(&descriptor.digest).unwrap_or_default(),
                message: error.to_string(),
            })?;
        let config_bytes = self.read_blob(&manifest.config.digest)?;
        let config: ImageConfig =
            serde_json::from_slice(&config_bytes).map_err(|error| ImageError::Corrupt {
                path: self.blob_path(&manifest.config.digest).unwrap_or_default(),
                message: error.to_string(),
            })?;
        manifest.validate(config.booter)?;
        config.validate()?;
        Ok((descriptor, manifest, config))
    }

    fn load_index(&self) -> Result<Index, ImageError> {
        let path = self.layout_dir().join("index.json");
        match fs::read(&path) {
            Ok(bytes) => {
                let index: Index =
                    serde_json::from_slice(&bytes).map_err(|error| ImageError::Corrupt {
                        path,
                        message: error.to_string(),
                    })?;
                index.validate()?;
                Ok(index)
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(Index::new()),
            Err(source) => Err(ImageError::Io { path, source }),
        }
    }

    fn write_index(&self, index: &Index) -> Result<(), ImageError> {
        self.ensure_layout()?;
        write_atomic(
            &self.layout_dir().join("index.json"),
            &canonical_json(index)?,
        )?;
        Ok(())
    }
}

fn import_config(source: &Path, media: ImageMedia, size: u64, now: DateTime<Utc>) -> ImageConfig {
    ImageConfig {
        schema_version: 1,
        dos_flavour: if media == ImageMedia::Booter {
            DosFlavour::None
        } else {
            DosFlavour::Other
        },
        booter: media == ImageMedia::Booter,
        boot: BootMetadata::opaque(size),
        machine: MachineProfile::default(),
        env: BTreeMap::new(),
        workdir: if media == ImageMedia::Booter {
            String::new()
        } else {
            r"C:\".to_owned()
        },
        cmd: None,
        exposed_ports: Vec::new(),
        volumes: Vec::new(),
        engine_pin: None,
        engine_config: BTreeMap::new(),
        requires: Vec::new(),
        agent_version: None,
        created_by: "import".to_owned(),
        history: vec![HistoryEntry {
            created_by: "import".to_owned(),
            created_at: now,
            empty_layer: false,
            comment: Some(source.display().to_string()),
        }],
    }
}

fn sha256_file(path: &Path) -> Result<String, ImageError> {
    let mut file = File::open(path).map_err(|source| ImageError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| ImageError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("sha256:{}", hex(&hasher.finalize())))
}

fn descriptor(media_type: &str, digest: &str, size: u64) -> Descriptor {
    Descriptor {
        media_type: media_type.to_owned(),
        digest: digest.to_owned(),
        size,
        annotations: None,
    }
}

fn verify_blob(path: &Path, digest: &str, size: u64) -> Result<(), ImageError> {
    let metadata = fs::metadata(path).map_err(|source| ImageError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() != size {
        return Err(ImageError::DigestMismatch {
            expected: digest.to_owned(),
            actual: format!("{}:{}", digest, metadata.len()),
        });
    }
    let bytes = fs::read(path).map_err(|source| ImageError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let actual = sha256_digest(&bytes);
    if actual != digest {
        return Err(ImageError::DigestMismatch {
            expected: digest.to_owned(),
            actual,
        });
    }
    Ok(())
}

#[cfg(unix)]
fn set_blob_mode(path: &Path) -> Result<(), ImageError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(BLOB_MODE)).map_err(|source| {
        ImageError::Io {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(windows)]
fn set_blob_mode(path: &Path) -> Result<(), ImageError> {
    let _ = path;
    Ok(())
}
