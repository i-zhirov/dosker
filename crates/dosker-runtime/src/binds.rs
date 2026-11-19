//! Host-directory bind mounts: `vvfat` selection and the fallback image.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use dosker_engine::{EngineVolume, EngineVolumeKind};
use dosker_image::{
    DEFAULT_VOLUME_SIZE, ImageError, create_volume, project_tree, read_host_tree, read_tree,
    validate_host_tree, write_host_tree,
};

use crate::paths::RuntimePaths;
use crate::record::{ContainerRecord, VolumeKind};

/// The size of the fallback image of a bind mount.
pub const BIND_IMAGE_SIZE: u64 = DEFAULT_VOLUME_SIZE;
/// The variable that selects the fallback image instead of `vvfat`.
pub const VVFAT_DISABLED_ENV: &str = "DOSKER_NO_VVFAT";

const BIND_IMAGE_NAME: &str = "bind";

/// A failure in a bind mount operation.
#[derive(Debug, thiserror::Error)]
pub enum BindError {
    #[error(transparent)]
    Image(#[from] ImageError),
    #[error("bind mount {drive} has no host path")]
    MissingHostPath { drive: String },
}

/// Returns true when `value` disables `vvfat`.
pub fn vvfat_disabled(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Returns true when bind mounts use `vvfat`.
pub fn use_vvfat() -> bool {
    !vvfat_disabled(env::var(VVFAT_DISABLED_ENV).ok().as_deref())
}

/// Builds the engine volumes of the bind mounts of a container.
pub fn prepare_binds(
    record: &ContainerRecord,
    paths: &RuntimePaths,
    use_vvfat: bool,
) -> Result<Vec<EngineVolume>, BindError> {
    let mut volumes = Vec::new();
    let mut created = Vec::new();

    for mount in record
        .volumes
        .iter()
        .filter(|mount| mount.kind == VolumeKind::Bind)
    {
        let Some(host) = mount.host_path.as_deref() else {
            remove_images(&created);
            return Err(BindError::MissingHostPath {
                drive: mount.drive.clone(),
            });
        };
        let image = paths.bind_image(&record.id, &mount.drive);
        match fs::remove_file(&image) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                remove_images(&created);
                return Err(ImageError::Io {
                    path: image,
                    source,
                }
                .into());
            }
        }

        if use_vvfat && !mount.read_only {
            if let Err(error) = validate_host_tree(host) {
                remove_images(&created);
                return Err(error.into());
            }
            volumes.push(EngineVolume {
                drive: mount.drive.clone(),
                path: host.to_path_buf(),
                read_only: false,
                kind: EngineVolumeKind::HostDir,
            });
            continue;
        }

        match project_fallback(host, &image) {
            Ok(()) => {
                created.push(image.clone());
                volumes.push(EngineVolume {
                    drive: mount.drive.clone(),
                    path: image,
                    read_only: mount.read_only,
                    kind: EngineVolumeKind::Image,
                });
            }
            Err(error) => {
                let _ = fs::remove_file(&image);
                remove_images(&created);
                return Err(error);
            }
        }
    }

    Ok(volumes)
}

/// Copies the fallback images of the read-write bind mounts back to the host.
pub fn sync_binds(record: &ContainerRecord, paths: &RuntimePaths) -> Result<(), BindError> {
    for mount in record
        .volumes
        .iter()
        .filter(|mount| mount.kind == VolumeKind::Bind)
    {
        if mount.read_only {
            continue;
        }
        let Some(host) = mount.host_path.as_deref() else {
            continue;
        };
        let image = paths.bind_image(&record.id, &mount.drive);
        if !image.is_file() {
            continue;
        }
        let tree = read_tree(&image)?;
        write_host_tree(host, &tree)?;
    }
    Ok(())
}

/// Removes the fallback images of the bind mounts of a container.
pub fn discard_binds(record: &ContainerRecord, paths: &RuntimePaths) {
    for mount in record
        .volumes
        .iter()
        .filter(|mount| mount.kind == VolumeKind::Bind)
    {
        let _ = fs::remove_file(paths.bind_image(&record.id, &mount.drive));
    }
}

fn project_fallback(host: &Path, image: &Path) -> Result<(), BindError> {
    let tree = read_host_tree(host)?;
    let bytes: u64 = tree.files().map(|file| file.bytes.len() as u64).sum();
    if bytes > BIND_IMAGE_SIZE {
        return Err(ImageError::InvalidHostPath {
            path: host.to_path_buf(),
            reason: format!(
                "the files use {bytes} bytes, more than the {BIND_IMAGE_SIZE}-byte bind image"
            ),
        }
        .into());
    }

    create_volume(image, BIND_IMAGE_SIZE, BIND_IMAGE_NAME)?;
    project_tree(image, &tree)?;
    Ok(())
}

fn remove_images(images: &[PathBuf]) {
    for image in images {
        let _ = fs::remove_file(image);
    }
}
