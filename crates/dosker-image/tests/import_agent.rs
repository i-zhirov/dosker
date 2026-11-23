mod support;

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use dosker_image::inject::{
    AGENT_PATH, AUTOEXEC_OLD_PATH, AUTOEXEC_PATH, NEW_AUTOEXEC_BYTES, WRAPPED_AUTOEXEC_BYTES,
};
use dosker_image::{
    Attributes, DosFlavour, DosPath, ImageError, ImageMedia, ImageRef, ImageReference, OciStore,
    Tree, TreeFile, TreeNode, agent_license, fixed_timestamp, read_boot_metadata, read_tree,
    sha256_digest,
};
use support::{make_fat16_image, set_fat_attribute, write_fat_file};

const MIB: u64 = 1024 * 1024;
const PARTITION_LENGTH: u64 = 8 * MIB;
const TOTAL_SIZE: u64 = 16 * MIB;
const ORIGINAL_AUTOEXEC: &[u8] = b"@ECHO OFF\r\n";

fn agent() -> Vec<u8> {
    b"MZ injected agent".to_vec()
}

fn now(second: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(second, 0).unwrap()
}

fn reference(value: &str) -> ImageReference {
    ImageReference::parse(value).unwrap()
}

fn tag(value: &str) -> ImageRef {
    ImageRef::parse(value).unwrap()
}

fn dos(value: &str) -> DosPath {
    DosPath::parse(value).unwrap()
}

fn store(dir: &Path) -> OciStore {
    OciStore::new(dir.to_path_buf())
}

fn source_image(dir: &Path, name: &str, with_autoexec: bool) -> PathBuf {
    let path = dir.join(name);
    make_fat16_image(&path, MIB, PARTITION_LENGTH, TOTAL_SIZE);
    write_fat_file(&path, MIB, "APP/FOO.TXT", b"hello");
    if with_autoexec {
        write_fat_file(&path, MIB, "AUTOEXEC.BAT", ORIGINAL_AUTOEXEC);
        set_fat_attribute(&path, MIB, "AUTOEXEC.BAT", 0x21);
    }
    path
}

fn file<'a>(tree: &'a Tree, path: &str) -> &'a TreeFile {
    match tree.get(&dos(path)) {
        Some(TreeNode::File(file)) => file,
        other => panic!("expected a file at {path}, got {other:?}"),
    }
}

fn count_agent_lines(bytes: &[u8]) -> usize {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|line| line.to_ascii_uppercase().contains(r"C:\DOSKER\DOSKER.EXE"))
        .count()
}

#[test]
fn import_injects_the_agent_into_a_dos_image() {
    let dir = tempfile::tempdir().unwrap();
    let source = source_image(dir.path(), "source.img", true);
    let store = store(dir.path());
    let image = store
        .import(
            &source,
            &reference("dos:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
        .unwrap();

    let tree = store.load_tree(&tag("dos:latest")).unwrap();
    assert!(
        matches!(tree.get(&dos("DOSKER")), Some(TreeNode::Dir(_))),
        "the agent directory exists"
    );

    let injected = file(&tree, AGENT_PATH);
    assert_eq!(injected.bytes, agent());
    assert_eq!(injected.attributes, Attributes::default());

    let old = file(&tree, AUTOEXEC_OLD_PATH);
    assert_eq!(old.bytes, ORIGINAL_AUTOEXEC);
    assert_eq!(old.attributes, Attributes::default());
    assert_eq!(old.timestamp, fixed_timestamp());

    let autoexec = file(&tree, AUTOEXEC_PATH);
    assert_eq!(autoexec.bytes, WRAPPED_AUTOEXEC_BYTES);
    assert_eq!(
        autoexec.attributes,
        Attributes {
            read_only: true,
            archive: true,
            ..Attributes::default()
        }
    );
    assert_eq!(autoexec.timestamp, fixed_timestamp());

    let config = store.load_config(&tag("dos:latest")).unwrap();
    assert_eq!(config.dos_flavour, DosFlavour::Other);
    assert_eq!(config.agent_version.as_deref(), Some("1.0.0"));
    assert_eq!(config.licenses, vec![agent_license()]);
    assert_eq!(image.layer_digests.len(), 2, "source and injection layers");

    let manifest = store.load_manifest(&tag("dos:latest")).unwrap();
    assert_ne!(
        manifest.raw_disk_digest(),
        manifest.source_disk_digest(),
        "the injected disk differs from the source"
    );
}

#[test]
fn import_tree_matches_the_injected_raw_disk() {
    let dir = tempfile::tempdir().unwrap();
    let source = source_image(dir.path(), "source.img", true);
    let store = store(dir.path());
    let image = store
        .import(
            &source,
            &reference("dos:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
        .unwrap();

    let blob_path = store
        .blob_path(image.raw_disk_digest.as_deref().unwrap())
        .unwrap();
    let raw_tree = read_tree(&blob_path).unwrap();
    let tree = store.load_tree(&tag("dos:latest")).unwrap();

    assert_eq!(tree, raw_tree, "the layer tree equals the injected disk");
    let autoexec = file(&raw_tree, AUTOEXEC_PATH);
    assert!(autoexec.attributes.read_only);
    assert!(autoexec.attributes.archive);
    assert_eq!(autoexec.timestamp, fixed_timestamp());
}

#[test]
fn import_preserves_the_mbr_partition_and_boot_sector() {
    let dir = tempfile::tempdir().unwrap();
    let source = source_image(dir.path(), "source.img", true);
    let source_bytes = fs::read(&source).unwrap();
    let store = store(dir.path());
    let image = store
        .import(
            &source,
            &reference("dos:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
        .unwrap();

    let injected = store
        .read_blob(image.raw_disk_digest.as_deref().unwrap())
        .unwrap();
    let partition_offset = MIB as usize;
    assert_eq!(
        injected.len(),
        source_bytes.len(),
        "the disk length is kept"
    );
    assert_eq!(
        &injected[..partition_offset],
        &source_bytes[..partition_offset],
        "the MBR and the bytes before the partition are unchanged"
    );
    assert_eq!(
        &injected[partition_offset..partition_offset + 512],
        &source_bytes[partition_offset..partition_offset + 512],
        "the boot sector is unchanged"
    );

    let blob_path = store
        .blob_path(image.raw_disk_digest.as_deref().unwrap())
        .unwrap();
    assert_eq!(
        read_boot_metadata(&blob_path).unwrap(),
        read_boot_metadata(&source).unwrap()
    );
}

#[test]
fn import_accepts_a_read_only_source() {
    let dir = tempfile::tempdir().unwrap();
    let source = source_image(dir.path(), "source.img", true);
    let mut permissions = fs::metadata(&source)
        .expect("the source metadata reads")
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&source, permissions).expect("the source becomes read-only");

    let store = store(dir.path());
    let image = store
        .import(
            &source,
            &reference("dos:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
        .expect("a read-only source imports");

    let tree = store.load_tree(&tag("dos:latest")).unwrap();
    assert_eq!(file(&tree, AGENT_PATH).bytes, agent());
    assert_eq!(image.layer_digests.len(), 2, "source and injection layers");
}

#[test]
fn import_does_not_inject_twice() {
    let dir = tempfile::tempdir().unwrap();
    let source = source_image(dir.path(), "source.img", true);
    let store = store(dir.path());
    let first = store
        .import(
            &source,
            &reference("dos:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
        .unwrap();
    let first_tree = store.load_tree(&tag("dos:latest")).unwrap();
    let first_raw = first.raw_disk_digest.clone().unwrap();

    let injected_path = dir.path().join("injected.img");
    store
        .copy_disk_to(&tag("dos:latest"), &injected_path)
        .unwrap();
    let second = store
        .import(
            &injected_path,
            &reference("again:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(2),
        )
        .unwrap();

    let second_tree = store.load_tree(&tag("again:latest")).unwrap();
    assert_eq!(
        first_tree.digest().unwrap(),
        second_tree.digest().unwrap(),
        "the second import does not change the tree"
    );
    assert_eq!(
        second.raw_disk_digest.as_deref(),
        Some(first_raw.as_str()),
        "the second import does not change the raw disk"
    );

    let autoexec = file(&second_tree, AUTOEXEC_PATH);
    assert_eq!(count_agent_lines(&autoexec.bytes), 1);
    let old = file(&second_tree, AUTOEXEC_OLD_PATH);
    assert_eq!(old.bytes, ORIGINAL_AUTOEXEC);
}

#[test]
fn import_writes_a_new_autoexec_when_none_exists() {
    let dir = tempfile::tempdir().unwrap();
    let source = source_image(dir.path(), "source.img", false);
    let store = store(dir.path());
    store
        .import(
            &source,
            &reference("dos:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
        .unwrap();

    let tree = store.load_tree(&tag("dos:latest")).unwrap();
    assert_eq!(file(&tree, AUTOEXEC_PATH).bytes, NEW_AUTOEXEC_BYTES);
    assert!(tree.get(&dos(AUTOEXEC_OLD_PATH)).is_none());
}

#[test]
fn import_booter_stays_opaque() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("boot.bin");
    fs::write(&source, b"opaque booter").unwrap();
    let store = store(dir.path());
    let image = store
        .import(
            &source,
            &reference("boot:latest"),
            ImageMedia::Booter,
            None,
            now(1),
        )
        .unwrap();

    let config = store.load_config(&tag("boot:latest")).unwrap();
    assert!(config.booter);
    assert_eq!(config.dos_flavour, DosFlavour::None);
    assert_eq!(config.agent_version, None);
    assert_eq!(image.layer_digests.len(), 1);

    let manifest = store.load_manifest(&tag("boot:latest")).unwrap();
    assert_eq!(manifest.layers.len(), 1);
    assert_eq!(
        manifest.layers[0].digest,
        sha256_digest(&fs::read(&source).unwrap()),
        "the source digest is the booter layer digest"
    );
    assert_eq!(manifest.raw_disk_digest(), None);
    assert_eq!(manifest.source_disk_digest(), None);
    assert!(matches!(
        store.load_tree(&tag("boot:latest")),
        Err(ImageError::NoTreeImage(_))
    ));
}

#[test]
fn import_booter_has_no_agent_entry() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("boot.bin");
    fs::write(&source, b"opaque booter").unwrap();
    let store = store(dir.path());
    store
        .import(
            &source,
            &reference("boot:latest"),
            ImageMedia::Booter,
            None,
            now(1),
        )
        .unwrap();

    let config = store.load_config(&tag("boot:latest")).unwrap();
    assert!(config.licenses.is_empty());
}

#[test]
fn import_rejects_an_unreadable_disk() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("blank.img");
    fs::write(&source, vec![0u8; 4096]).unwrap();

    assert!(matches!(
        store(dir.path()).import(
            &source,
            &reference("bad:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        ),
        Err(ImageError::NotBootable(_))
    ));
}
