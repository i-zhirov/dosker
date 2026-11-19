use std::fs::{self, File};
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use dosker_core::{ContainerId, ContainerName};
use dosker_engine::{EngineVolume, EngineVolumeKind};
use dosker_image::{
    Attributes, DosPath, ImageError, LayerDelta, Tree, TreeFile, TreeNode, fixed_timestamp,
    project_tree, read_tree,
};
use dosker_platform::EndpointRecord;
use dosker_runtime::{
    BIND_IMAGE_SIZE, BindError, ContainerRecord, RuntimePaths, VolumeKind, VolumeMount,
    discard_binds, prepare_binds, sync_binds, vvfat_disabled,
};

const ID: &str = "0123456789ab";
const HOST_BYTES: &[u8] = b"hello";

fn at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    fs::create_dir_all(&root).unwrap();
    let host = dir.path().join("host");
    fs::create_dir_all(&host).unwrap();
    fs::write(host.join("HOST.TXT"), HOST_BYTES).unwrap();
    (dir, root, host)
}

fn bind_mount(drive: &str, host: &Path, read_only: bool) -> VolumeMount {
    VolumeMount {
        drive: drive.to_owned(),
        kind: VolumeKind::Bind,
        name: None,
        host_path: Some(host.to_path_buf()),
        read_only,
    }
}

fn record_with(root: &Path, mounts: Vec<VolumeMount>) -> ContainerRecord {
    let id = ContainerId::parse(ID).unwrap();
    let name = ContainerName::parse("dos-app").unwrap();
    let control = EndpointRecord::UnixSocket {
        path: root.join("control.sock"),
    };
    fs::create_dir_all(root.join("containers").join(ID)).unwrap();
    ContainerRecord::new(
        id,
        name,
        "freedos:latest".to_owned(),
        "qemu".to_owned(),
        control,
        at(),
    )
    .with_volumes(mounts)
}

fn record(root: &Path, host: &Path, read_only: bool) -> ContainerRecord {
    record_with(root, vec![bind_mount("D:", host, read_only)])
}

fn guest_tree(name: &str, bytes: &[u8]) -> Tree {
    let mut delta = LayerDelta::new();
    delta.add_file(TreeFile {
        path: DosPath::parse(name).unwrap(),
        attributes: Attributes::default(),
        timestamp: fixed_timestamp(),
        bytes: bytes.to_vec(),
    });
    let mut tree = Tree::new();
    tree.apply_delta(&delta).unwrap();
    tree
}

fn tree_file(tree: &Tree, name: &str) -> Vec<u8> {
    let path = DosPath::parse(name).unwrap();
    match tree.get(&path) {
        Some(TreeNode::File(file)) => file.bytes.clone(),
        other => panic!("expected the file {name}, got {other:?}"),
    }
}

#[test]
fn vvfat_is_the_default_and_only_the_exact_value_one_disables_it() {
    assert!(!vvfat_disabled(None));
    assert!(!vvfat_disabled(Some("0")));
    assert!(!vvfat_disabled(Some("true")));
    assert!(vvfat_disabled(Some("1")));
}

#[test]
fn prepare_binds_uses_the_host_directory_for_vvfat() {
    let (_dir, root, host) = fixture();
    let record = record(&root, &host, false);
    let paths = RuntimePaths::new(root.clone());

    let volumes = prepare_binds(&record, &paths, true).unwrap();

    assert_eq!(
        volumes,
        vec![EngineVolume {
            drive: "D:".to_owned(),
            path: host.clone(),
            read_only: false,
            kind: EngineVolumeKind::HostDir,
        }]
    );
    assert!(!paths.bind_image(&record.id, "D:").exists());
}

#[test]
fn prepare_binds_projects_the_host_directory_into_a_fallback_image() {
    let (_dir, root, host) = fixture();
    let record = record(&root, &host, false);
    let paths = RuntimePaths::new(root.clone());
    let image = root.join("containers").join(ID).join("bind-d.img");

    let volumes = prepare_binds(&record, &paths, false).unwrap();

    assert_eq!(
        volumes,
        vec![EngineVolume {
            drive: "D:".to_owned(),
            path: image.clone(),
            read_only: false,
            kind: EngineVolumeKind::Image,
        }]
    );
    assert!(image.is_file());
    let tree = read_tree(&image).unwrap();
    assert_eq!(tree_file(&tree, "HOST.TXT"), HOST_BYTES);
}

#[test]
fn prepare_binds_uses_a_read_only_fallback_for_a_read_only_bind() {
    let (_dir, root, host) = fixture();
    let record = record(&root, &host, true);
    let paths = RuntimePaths::new(root.clone());
    let image = paths.bind_image(&record.id, "D:");

    let volumes = prepare_binds(&record, &paths, true).unwrap();

    assert_eq!(
        volumes,
        vec![EngineVolume {
            drive: "D:".to_owned(),
            path: image.clone(),
            read_only: true,
            kind: EngineVolumeKind::Image,
        }]
    );
    assert!(image.is_file());
}

#[test]
fn prepare_binds_rejects_a_host_directory_larger_than_the_image() {
    let (_dir, root, host) = fixture();
    let file = File::create(host.join("BIG.TXT")).unwrap();
    file.set_len(BIND_IMAGE_SIZE + 1).unwrap();
    let record = record(&root, &host, false);
    let paths = RuntimePaths::new(root.clone());

    let error = prepare_binds(&record, &paths, false).unwrap_err();

    match error {
        BindError::Image(ImageError::InvalidHostPath { path, .. }) => {
            assert_eq!(path, host);
        }
        other => panic!("expected InvalidHostPath, got {other}"),
    }
}

#[test]
fn prepare_binds_rejects_an_invalid_host_name_and_names_the_path() {
    let (_dir, root, host) = fixture();
    fs::write(host.join("TOOLONGNAME.TXT"), b"x").unwrap();
    let record = record(&root, &host, false);
    let paths = RuntimePaths::new(root.clone());

    let error = prepare_binds(&record, &paths, false).unwrap_err();
    let message = error.to_string();

    assert!(message.contains(host.to_str().unwrap()), "{message}");
    assert!(message.contains("TOOLONGNAME.TXT"), "{message}");
}

#[test]
fn prepare_binds_rejects_a_bind_without_a_host_path() {
    let (_dir, root, host) = fixture();
    let mut record = record(&root, &host, false);
    record.volumes[0].host_path = None;
    let paths = RuntimePaths::new(root.clone());

    let error = prepare_binds(&record, &paths, false).unwrap_err();

    match error {
        BindError::MissingHostPath { drive } => assert_eq!(drive, "D:"),
        other => panic!("expected MissingHostPath, got {other}"),
    }
}

#[test]
fn prepare_binds_removes_a_stale_fallback_image() {
    let (_dir, root, host) = fixture();
    let record = record(&root, &host, false);
    let paths = RuntimePaths::new(root.clone());
    let image = paths.bind_image(&record.id, "D:");
    fs::write(&image, b"stale").unwrap();

    prepare_binds(&record, &paths, false).unwrap();

    assert!(image.is_file());
    assert_eq!(fs::metadata(&image).unwrap().len(), BIND_IMAGE_SIZE);
    let tree = read_tree(&image).unwrap();
    assert_eq!(tree_file(&tree, "HOST.TXT"), HOST_BYTES);
}

#[test]
fn prepare_binds_fails_when_the_stale_image_cannot_be_removed() {
    let (_dir, root, host) = fixture();
    let record = record(&root, &host, false);
    let paths = RuntimePaths::new(root.clone());
    let image = paths.bind_image(&record.id, "D:");
    fs::create_dir_all(&image).unwrap();
    fs::write(image.join("STALE.TXT"), b"stale").unwrap();

    let error = prepare_binds(&record, &paths, true).unwrap_err();

    match error {
        BindError::Image(ImageError::Io { path, .. }) => assert_eq!(path, image),
        other => panic!("expected an image i/o error, got {other}"),
    }
}

#[test]
fn sync_binds_copies_a_guest_change_back_to_the_host() {
    let (_dir, root, host) = fixture();
    let record = record(&root, &host, false);
    let paths = RuntimePaths::new(root.clone());
    prepare_binds(&record, &paths, false).unwrap();
    let image = paths.bind_image(&record.id, "D:");
    project_tree(&image, &guest_tree("FROMDOS.TXT", b"from dos")).unwrap();

    sync_binds(&record, &paths).unwrap();

    assert_eq!(fs::read(host.join("FROMDOS.TXT")).unwrap(), b"from dos");
    assert!(!host.join("HOST.TXT").exists());
}

#[test]
fn sync_binds_skips_a_read_only_mount() {
    let (_dir, root, host) = fixture();
    let record = record(&root, &host, true);
    let paths = RuntimePaths::new(root.clone());
    prepare_binds(&record, &paths, false).unwrap();
    let image = paths.bind_image(&record.id, "D:");
    project_tree(&image, &guest_tree("FROMDOS.TXT", b"from dos")).unwrap();

    sync_binds(&record, &paths).unwrap();

    assert_eq!(fs::read(host.join("HOST.TXT")).unwrap(), HOST_BYTES);
    assert!(!host.join("FROMDOS.TXT").exists());
}

#[test]
fn sync_binds_does_nothing_for_a_host_directory_bind() {
    let (_dir, root, host) = fixture();
    let record = record(&root, &host, false);
    let paths = RuntimePaths::new(root.clone());
    prepare_binds(&record, &paths, true).unwrap();

    sync_binds(&record, &paths).unwrap();

    assert_eq!(fs::read(host.join("HOST.TXT")).unwrap(), HOST_BYTES);
    assert_eq!(fs::read_dir(&host).unwrap().count(), 1);
}

#[test]
fn discard_binds_removes_the_fallback_image() {
    let (dir, root, host) = fixture();
    let read_only_host = dir.path().join("vvfat");
    fs::create_dir_all(&read_only_host).unwrap();
    fs::write(read_only_host.join("HOST.TXT"), b"vvfat").unwrap();
    let record = record_with(
        &root,
        vec![
            bind_mount("D:", &host, false),
            bind_mount("E:", &read_only_host, true),
        ],
    );
    let paths = RuntimePaths::new(root.clone());
    let volumes = prepare_binds(&record, &paths, true).unwrap();
    assert_eq!(volumes[0].kind, EngineVolumeKind::HostDir);
    assert_eq!(volumes[1].kind, EngineVolumeKind::Image);
    let image = paths.bind_image(&record.id, "E:");
    assert!(image.is_file());
    assert!(!paths.bind_image(&record.id, "D:").exists());

    discard_binds(&record, &paths);

    assert!(!image.exists());
    assert_eq!(fs::read(host.join("HOST.TXT")).unwrap(), HOST_BYTES);
    assert_eq!(fs::read(read_only_host.join("HOST.TXT")).unwrap(), b"vvfat");
}
