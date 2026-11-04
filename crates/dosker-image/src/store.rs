//! The local directory store for imported images.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use dosker_platform::CopyOutcome;
use dosker_platform::fs::{copy_reflink, ensure_dir_mode, write_atomic};
use sha2::{Digest, Sha256};

use crate::error::ImageError;
use crate::fat::{FatLayout, fat_region};
use crate::image::{IMAGE_SCHEMA_VERSION, ImageMedia, ImageRecord};
use crate::reference::ImageReference;

/// The name of the image record inside an image directory.
pub const IMAGE_RECORD_FILE: &str = "image.json";
/// The name of the stored blob inside an image directory.
pub const IMAGE_DISK_FILE: &str = "disk.img";

const DIR_MODE: u32 = 0o700;
const DISK_MODE: u32 = 0o444;
const COPY_BUFFER_BYTES: usize = 64 * 1024;

/// Stores image records and blobs under a data root.
#[derive(Clone, Debug)]
pub struct ImageStore {
    root: PathBuf,
}

impl ImageStore {
    /// Builds a store for a data root.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Returns the directory that holds every image.
    pub fn images_dir(&self) -> PathBuf {
        self.root.join("images")
    }

    /// Returns the directory of one image reference.
    pub fn image_dir(&self, reference: &ImageReference) -> PathBuf {
        self.images_dir()
            .join(reference.name().as_str())
            .join(reference.tag().as_str())
    }

    /// Returns the stored blob path of one image reference.
    pub fn disk_path(&self, reference: &ImageReference) -> PathBuf {
        self.image_dir(reference).join(IMAGE_DISK_FILE)
    }

    /// Imports a source file as an image and writes its record.
    pub fn import(
        &self,
        source: &Path,
        reference: &ImageReference,
        media: ImageMedia,
        now: DateTime<Utc>,
    ) -> Result<ImageRecord, ImageError> {
        let metadata = fs::metadata(source).map_err(|source_error| ImageError::Io {
            path: source.to_path_buf(),
            source: source_error,
        })?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(ImageError::NotBootable(source.to_path_buf()));
        }

        if media == ImageMedia::Disk {
            let region = fat_region(source)?;
            if region.layout == FatLayout::Superfloppy {
                return Err(ImageError::UnsupportedLayout(source.to_path_buf()));
            }
        }

        let image_dir = self.image_dir(reference);
        let record_path = image_dir.join(IMAGE_RECORD_FILE);
        if record_path.exists() {
            return Err(ImageError::AlreadyExists(reference.clone()));
        }

        self.ensure_layout(reference)?;

        let disk_path = image_dir.join(IMAGE_DISK_FILE);
        let temp_path = image_dir.join(format!("{IMAGE_DISK_FILE}.tmp.{}", std::process::id()));

        let result = (|| -> Result<ImageRecord, ImageError> {
            let (size, digest) = copy_hashed(source, &temp_path)?;
            fs::rename(&temp_path, &disk_path).map_err(|source_error| ImageError::Io {
                path: disk_path.clone(),
                source: source_error,
            })?;
            restrict_disk(&disk_path)?;

            let record = ImageRecord {
                schema_version: IMAGE_SCHEMA_VERSION,
                name: reference.name().to_string(),
                tag: reference.tag().to_string(),
                media,
                digest,
                size,
                source: source.display().to_string(),
                agent_version: None,
                cmd: None,
                imported_at: now,
            };
            write_record(&record_path, &record)?;
            Ok(record)
        })();

        if result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }

        result
    }

    /// Loads the record of one image reference.
    pub fn load(&self, reference: &ImageReference) -> Result<ImageRecord, ImageError> {
        let path = self.image_dir(reference).join(IMAGE_RECORD_FILE);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Err(ImageError::NotFound(reference.clone()));
            }
            Err(source) => {
                return Err(ImageError::Io {
                    path: path.clone(),
                    source,
                });
            }
        };

        let record: ImageRecord =
            serde_json::from_slice(&bytes).map_err(|error| ImageError::Corrupt {
                path: path.clone(),
                message: error.to_string(),
            })?;

        if record.schema_version != IMAGE_SCHEMA_VERSION {
            return Err(ImageError::Corrupt {
                path,
                message: format!("unsupported schema version {}", record.schema_version),
            });
        }

        Ok(record)
    }

    /// Loads every image record, sorted by name and then tag.
    pub fn list(&self) -> Result<Vec<ImageRecord>, ImageError> {
        let directory = self.images_dir();
        let names = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(ImageError::Io {
                    path: directory,
                    source,
                });
            }
        };

        let mut records = Vec::new();
        for name_entry in names {
            let name_entry = name_entry.map_err(|source| ImageError::Io {
                path: directory.clone(),
                source,
            })?;
            let name_path = name_entry.path();
            if !name_entry
                .file_type()
                .map_err(|source| ImageError::Io {
                    path: name_path.clone(),
                    source,
                })?
                .is_dir()
            {
                continue;
            }

            let Some(name) = name_path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };

            let tags = match fs::read_dir(&name_path) {
                Ok(entries) => entries,
                Err(source) => {
                    return Err(ImageError::Io {
                        path: name_path.clone(),
                        source,
                    });
                }
            };

            for tag_entry in tags {
                let tag_entry = tag_entry.map_err(|source| ImageError::Io {
                    path: name_path.clone(),
                    source,
                })?;
                let tag_path = tag_entry.path();
                if !tag_entry
                    .file_type()
                    .map_err(|source| ImageError::Io {
                        path: tag_path.clone(),
                        source,
                    })?
                    .is_dir()
                {
                    continue;
                }
                if !tag_path.join(IMAGE_RECORD_FILE).is_file() {
                    continue;
                }
                let Some(tag) = tag_path.file_name().and_then(|value| value.to_str()) else {
                    continue;
                };
                let Ok(reference) = ImageReference::parse(&format!("{name}:{tag}")) else {
                    continue;
                };

                match self.load(&reference) {
                    Ok(record) => records.push(record),
                    Err(ImageError::NotFound(_)) => {}
                    Err(error) => return Err(error),
                }
            }
        }

        records.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.tag.cmp(&right.tag))
        });

        Ok(records)
    }

    /// Copies the stored blob to a destination without modifying the image.
    pub fn copy_disk_to(
        &self,
        reference: &ImageReference,
        destination: &Path,
    ) -> Result<CopyOutcome, ImageError> {
        copy_reflink(&self.disk_path(reference), destination).map_err(ImageError::from)
    }

    fn ensure_layout(&self, reference: &ImageReference) -> Result<(), ImageError> {
        ensure_dir_mode(&self.images_dir(), DIR_MODE)?;
        ensure_dir_mode(&self.images_dir().join(reference.name().as_str()), DIR_MODE)?;
        ensure_dir_mode(&self.image_dir(reference), DIR_MODE)?;
        Ok(())
    }
}

fn copy_hashed(source: &Path, destination: &Path) -> Result<(u64, String), ImageError> {
    let mut input = File::open(source).map_err(|source_error| ImageError::Io {
        path: source.to_path_buf(),
        source: source_error,
    })?;
    let mut output = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(destination)
        .map_err(|source_error| ImageError::Io {
            path: destination.to_path_buf(),
            source: source_error,
        })?;

    let mut hasher = Sha256::new();
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    let mut size = 0u64;
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
                path: destination.to_path_buf(),
                source: source_error,
            })?;
        size += read as u64;
    }

    output.sync_all().map_err(|source_error| ImageError::Io {
        path: destination.to_path_buf(),
        source: source_error,
    })?;

    Ok((size, format!("sha256:{}", hex(&hasher.finalize()))))
}

fn write_record(path: &Path, record: &ImageRecord) -> Result<(), ImageError> {
    let mut bytes = serde_json::to_vec_pretty(record).map_err(|error| ImageError::Corrupt {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)?;
    Ok(())
}

#[cfg(unix)]
fn restrict_disk(path: &Path) -> Result<(), ImageError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(DISK_MODE)).map_err(|source| {
        ImageError::Io {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(not(unix))]
fn restrict_disk(_path: &Path) -> Result<(), ImageError> {
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}
