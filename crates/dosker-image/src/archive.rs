//! Deterministic OCI image-layout tar archives.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::canonical::{canonical_json, sha256_digest};
use crate::error::ImageError;
use crate::manifest::{
    ANNOTATION_REF_NAME, Descriptor, Index, MEDIA_TYPE_MANIFEST, Manifest, OCI_LAYOUT_VERSION,
    OciLayout,
};
use crate::oci::OciStore;
use crate::reference::{ImageRef, ImageReference};

/// The fixed mtime of every archive member.
pub const ARCHIVE_MTIME_SECONDS: u64 = 0;
/// The fixed mode of every archive member.
pub const ARCHIVE_MODE: u32 = 0o644;

const BLOB_PREFIX: &str = "blobs/sha256/";
const LAYOUT_MEMBER: &str = "oci-layout";
const INDEX_MEMBER: &str = "index.json";

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The result of one `save`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveReport {
    /// The saved tag.
    pub reference: String,
    /// The manifest digest of the saved image.
    pub manifest_digest: String,
    /// The number of blobs written to the archive.
    pub blobs: usize,
    /// The tar byte count.
    pub bytes: u64,
}

/// The result of one `load`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadReport {
    /// The loaded tags, in archive order.
    pub references: Vec<String>,
    /// The number of blobs written into the store.
    pub blobs_written: usize,
    /// The number of blobs already present in the store.
    pub blobs_skipped: usize,
}

/// Writes one tagged image to an OCI image-layout tar.
pub fn save_image(
    store: &OciStore,
    reference: &ImageReference,
    destination: &Path,
) -> Result<SaveReport, ImageError> {
    let _lock = store.lock_index()?;
    let image = store.load(&ImageRef::Tag(reference.clone()))?;
    let manifest_bytes = store.read_blob(&image.manifest_digest)?;
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).map_err(|error| ImageError::Archive {
            path: destination.to_path_buf(),
            message: error.to_string(),
        })?;

    let mut refs = BTreeMap::new();
    refs.insert(ANNOTATION_REF_NAME.to_owned(), reference.to_string());
    let mut index = Index::new();
    index.add(Descriptor {
        media_type: MEDIA_TYPE_MANIFEST.to_owned(),
        digest: image.manifest_digest.clone(),
        size: manifest_bytes.len() as u64,
        annotations: Some(refs),
    });

    let mut digests = BTreeSet::new();
    digests.insert(image.manifest_digest.clone());
    digests.insert(manifest.config.digest.clone());
    digests.extend(manifest.layers.iter().map(|layer| layer.digest.clone()));
    if let Some(raw) = manifest.raw_disk_digest() {
        digests.insert(raw.to_owned());
    }

    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let temp = parent.join(format!(
        ".dosker-save-{}-{}.tmp",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|source| ImageError::Io {
                path: temp.clone(),
                source,
            })?;
        {
            let mut builder = tar::Builder::new(file);
            write_member(
                &mut builder,
                LAYOUT_MEMBER,
                &canonical_json(&OciLayout::new())?,
            )?;
            write_member(&mut builder, INDEX_MEMBER, &canonical_json(&index)?)?;
            for digest in &digests {
                let bytes = store.read_blob(digest)?;
                let member = format!("{BLOB_PREFIX}{}", blob_hex(digest)?);
                write_member(&mut builder, &member, &bytes)?;
            }
            builder.finish().map_err(|source| ImageError::Io {
                path: temp.clone(),
                source,
            })?;
        }
        let bytes = fs::metadata(&temp)
            .map_err(|source| ImageError::Io {
                path: temp.clone(),
                source,
            })?
            .len();
        fs::rename(&temp, destination).map_err(|source| ImageError::Io {
            path: destination.to_path_buf(),
            source,
        })?;
        Ok(SaveReport {
            reference: reference.to_string(),
            manifest_digest: image.manifest_digest,
            blobs: digests.len(),
            bytes,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Loads every tag from an OCI image-layout tar and replaces existing tags.
pub fn load_image(store: &OciStore, source: &Path) -> Result<LoadReport, ImageError> {
    let first = scan_archive(source)?;
    let layout: OciLayout = parse_member(source, &first, LAYOUT_MEMBER, "layout marker")?;
    if layout.image_layout_version != OCI_LAYOUT_VERSION {
        return Err(archive_error(
            source,
            format!(
                "unsupported layout version {}; expected {OCI_LAYOUT_VERSION}",
                layout.image_layout_version
            ),
        ));
    }

    let index: Index = parse_member(source, &first, INDEX_MEMBER, "index")?;
    index
        .validate()
        .map_err(|error| archive_error(source, error.to_string()))?;

    let mut refs = BTreeSet::new();
    for descriptor in &index.manifests {
        let reference = descriptor_ref(source, descriptor)?;
        if !refs.insert(reference.to_string()) {
            return Err(archive_error(
                source,
                format!("duplicate ref annotation {reference}"),
            ));
        }
        if !first.contains_key(&blob_member(&descriptor.digest)?) {
            return Err(archive_error(
                source,
                format!("missing manifest blob {}", descriptor.digest),
            ));
        }
    }

    let mut manifests = BTreeMap::new();
    for descriptor in &index.manifests {
        let bytes = read_member(source, &first, &blob_member(&descriptor.digest)?)?;
        verify_digest(source, &descriptor.digest, &bytes)?;
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|error| ImageError::Archive {
                path: source.to_path_buf(),
                message: format!("invalid manifest {}: {error}", descriptor.digest),
            })?;
        let config_bytes = read_member(source, &first, &blob_member(&manifest.config.digest)?)?;
        verify_digest(source, &manifest.config.digest, &config_bytes)?;
        let config: crate::config::ImageConfig =
            serde_json::from_slice(&config_bytes).map_err(|error| ImageError::Archive {
                path: source.to_path_buf(),
                message: format!("invalid image config {}: {error}", manifest.config.digest),
            })?;
        manifest
            .validate(config.booter)
            .map_err(|error| archive_error(source, error.to_string()))?;
        config
            .validate()
            .map_err(|error| archive_error(source, error.to_string()))?;
        manifests.insert(descriptor.digest.clone(), (manifest, bytes.len() as u64));
    }

    let mut needed = BTreeSet::new();
    for (manifest, _) in manifests.values() {
        needed.insert(manifest.config.digest.clone());
        needed.extend(manifest.layers.iter().map(|layer| layer.digest.clone()));
        if let Some(raw) = manifest.raw_disk_digest() {
            needed.insert(raw.to_owned());
        }
    }
    needed.extend(manifests.keys().cloned());

    let mut written = 0;
    let mut skipped = 0;
    let mut found = BTreeSet::new();
    let mut archive = open_archive(source)?;
    for entry in archive
        .entries()
        .map_err(|error| archive_error(source, error.to_string()))?
    {
        let mut entry = entry.map_err(|error| archive_error(source, error.to_string()))?;
        let Some(name) = entry_name(source, &entry)? else {
            continue;
        };
        let Some(digest) = needed
            .iter()
            .find(|digest| blob_member(digest).is_ok_and(|member| member == name))
        else {
            continue;
        };
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| archive_error(source, error.to_string()))?;
        verify_digest(source, digest, &bytes)?;
        let blob = store.put_blob(&bytes)?;
        if blob.created {
            written += 1;
        } else {
            skipped += 1;
        }
        found.insert(digest.clone());
    }
    for digest in &needed {
        if !found.contains(digest) {
            return Err(archive_error(source, format!("missing blob {digest}")));
        }
    }

    let _lock = store.lock_index()?;
    let mut stored = store.load_index()?;
    let mut references = Vec::new();
    for descriptor in &index.manifests {
        let reference = descriptor_ref(source, descriptor)?;
        let (_, size) = manifests
            .get(&descriptor.digest)
            .expect("every index manifest is parsed");
        stored.remove_ref(&reference.to_string());
        let mut annotations = BTreeMap::new();
        annotations.insert(ANNOTATION_REF_NAME.to_owned(), reference.to_string());
        stored.add(Descriptor {
            media_type: MEDIA_TYPE_MANIFEST.to_owned(),
            digest: descriptor.digest.clone(),
            size: *size,
            annotations: Some(annotations),
        });
        references.push(reference.to_string());
    }
    store.write_index(&stored)?;

    Ok(LoadReport {
        references,
        blobs_written: written,
        blobs_skipped: skipped,
    })
}

fn write_member<W: Write>(
    builder: &mut tar::Builder<W>,
    name: &str,
    bytes: &[u8],
) -> Result<(), ImageError> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(ARCHIVE_MODE);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(ARCHIVE_MTIME_SECONDS);
    header.set_entry_type(tar::EntryType::Regular);
    builder
        .append_data(&mut header, name, bytes)
        .map_err(|error| ImageError::Archive {
            path: PathBuf::from(name),
            message: error.to_string(),
        })
}

fn open_archive(source: &Path) -> Result<tar::Archive<File>, ImageError> {
    let file = File::open(source).map_err(|error| ImageError::Io {
        path: source.to_path_buf(),
        source: error,
    })?;
    Ok(tar::Archive::new(file))
}

fn scan_archive(source: &Path) -> Result<BTreeMap<String, u64>, ImageError> {
    let mut names = BTreeMap::new();
    let mut archive = open_archive(source)?;
    for entry in archive
        .entries()
        .map_err(|error| archive_error(source, error.to_string()))?
    {
        let entry = entry.map_err(|error| archive_error(source, error.to_string()))?;
        let Some(name) = entry_name(source, &entry)? else {
            continue;
        };
        if names.insert(name.clone(), entry.size()).is_some() {
            return Err(archive_error(source, format!("duplicate entry {name}")));
        }
    }
    Ok(names)
}

fn entry_name(source: &Path, entry: &tar::Entry<'_, File>) -> Result<Option<String>, ImageError> {
    let path = entry
        .path()
        .map_err(|error| archive_error(source, error.to_string()))?
        .into_owned();
    if path.to_string_lossy().contains('\\') {
        return Err(archive_error(
            source,
            format!("unsafe archive path {}", path.display()),
        ));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => {
                return Err(archive_error(
                    source,
                    format!("unsafe archive path {}", path.display()),
                ));
            }
        }
    }
    if parts.is_empty() {
        return Ok(None);
    }
    Ok(Some(parts.join("/")))
}

fn parse_member<T: serde::de::DeserializeOwned>(
    source: &Path,
    entries: &BTreeMap<String, u64>,
    member: &str,
    label: &str,
) -> Result<T, ImageError> {
    let bytes = read_member(source, entries, member)?;
    serde_json::from_slice(&bytes).map_err(|error| ImageError::Archive {
        path: source.to_path_buf(),
        message: format!("invalid {label} {member}: {error}"),
    })
}

fn read_member(
    source: &Path,
    entries: &BTreeMap<String, u64>,
    member: &str,
) -> Result<Vec<u8>, ImageError> {
    if !entries.contains_key(member) {
        return Err(archive_error(source, format!("missing {member}")));
    }
    let mut archive = open_archive(source)?;
    for entry in archive
        .entries()
        .map_err(|error| archive_error(source, error.to_string()))?
    {
        let mut entry = entry.map_err(|error| archive_error(source, error.to_string()))?;
        if entry_name(source, &entry)?.as_deref() != Some(member) {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| archive_error(source, error.to_string()))?;
        return Ok(bytes);
    }
    Err(archive_error(source, format!("missing {member}")))
}

fn descriptor_ref(source: &Path, descriptor: &Descriptor) -> Result<ImageReference, ImageError> {
    let value = descriptor
        .annotations
        .as_ref()
        .and_then(|annotations| annotations.get(ANNOTATION_REF_NAME))
        .ok_or_else(|| {
            archive_error(
                source,
                format!("descriptor {} has no ref annotation", descriptor.digest),
            )
        })?;
    ImageReference::parse(value).map_err(|error| archive_error(source, error.to_string()))
}

fn verify_digest(source: &Path, expected: &str, bytes: &[u8]) -> Result<(), ImageError> {
    let actual = sha256_digest(bytes);
    if actual != expected {
        return Err(archive_error(
            source,
            format!("corrupt blob {expected}: found {actual}"),
        ));
    }
    Ok(())
}

fn blob_hex(digest: &str) -> Result<&str, ImageError> {
    digest
        .strip_prefix("sha256:")
        .filter(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| ImageError::InvalidReference(digest.to_owned()))
}

fn blob_member(digest: &str) -> Result<String, ImageError> {
    Ok(format!("{BLOB_PREFIX}{}", blob_hex(digest)?))
}

fn archive_error(path: &Path, message: impl Into<String>) -> ImageError {
    ImageError::Archive {
        path: path.to_path_buf(),
        message: message.into(),
    }
}
