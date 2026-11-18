use chrono::TimeDelta;
use dosker_build::run_in_dos::{RunDelta, build_delta};
use dosker_image::{
    Attributes, BlobRef, BootLayout, BootMetadata, DosPath, FatType, Geometry, LayerDelta, Tree,
    TreeDir, TreeFile, fixed_timestamp,
};

fn dos(path: &str) -> DosPath {
    DosPath::parse(path).expect("the DOS path parses")
}

fn archive() -> Attributes {
    Attributes {
        archive: true,
        ..Attributes::default()
    }
}

fn file(path: &str, bytes: &[u8]) -> TreeFile {
    TreeFile {
        path: dos(path),
        attributes: archive(),
        timestamp: fixed_timestamp(),
        bytes: bytes.to_vec(),
    }
}

fn dir(path: &str) -> TreeDir {
    TreeDir {
        path: dos(path),
        attributes: Attributes::default(),
        timestamp: fixed_timestamp(),
    }
}

fn tree(files: Vec<TreeFile>, dirs: Vec<TreeDir>) -> Tree {
    let mut delta = LayerDelta::new();
    for entry in dirs {
        delta.add_dir(entry);
    }
    for entry in files {
        delta.add_file(entry);
    }
    let mut tree = Tree::new();
    tree.apply_delta(&delta).expect("the tree applies");
    tree
}

fn boot(sector_byte: u8) -> BootMetadata {
    BootMetadata {
        layout: BootLayout::MbrPartitioned,
        disk_size: 16 * 1024 * 1024,
        geometry: Geometry {
            cylinders: 16,
            heads: 16,
            sectors: 32,
            sector_size: 512,
        },
        partition_start: Some(2048),
        partition_sectors: Some(16_384),
        mbr: Some(BlobRef::from_bytes(&[0x11; 512])),
        boot_sector: Some(BlobRef::from_bytes(&[sector_byte; 512])),
        fat_type: Some(FatType::Fat16),
        volume_label: Some("DOSKER".to_owned()),
        media_descriptor: Some(0xF8),
    }
}

fn assert_no_changes(run: &RunDelta) {
    assert!(run.delta.whiteouts.is_empty(), "{:?}", run.delta.whiteouts);
    assert!(run.delta.files.is_empty(), "{:?}", run.delta.files);
    assert!(run.delta.dirs.is_empty(), "{:?}", run.delta.dirs);
}

#[test]
fn delta_adds_a_new_file() {
    let staged = Tree::new();
    let read_back = tree(vec![file(r"C:\NEW.TXT", b"hello")], Vec::new());

    let run = build_delta(&staged, &read_back, &boot(0xAA), &boot(0xAA));

    assert!(run.delta.whiteouts.is_empty());
    assert!(run.delta.dirs.is_empty());
    assert_eq!(run.delta.files.len(), 1);
    let added = &run.delta.files[0];
    assert_eq!(added.path, dos(r"C:\NEW.TXT"));
    assert_eq!(added.bytes, b"hello");
    assert_eq!(added.attributes, archive());
    assert_eq!(added.timestamp, fixed_timestamp());
    assert_eq!(run.boot, None);
}

#[test]
fn delta_replaces_a_changed_file() {
    let staged = tree(vec![file(r"C:\OLD.TXT", b"old")], Vec::new());
    let read_back = tree(vec![file(r"C:\OLD.TXT", b"new")], Vec::new());

    let run = build_delta(&staged, &read_back, &boot(0xAA), &boot(0xAA));

    assert!(run.delta.whiteouts.is_empty());
    assert!(run.delta.dirs.is_empty());
    assert_eq!(run.delta.files.len(), 1);
    let replaced = &run.delta.files[0];
    assert_eq!(replaced.path, dos(r"C:\OLD.TXT"));
    assert_eq!(replaced.bytes, b"new");
    assert_eq!(replaced.attributes, archive());
    assert_eq!(replaced.timestamp, fixed_timestamp());
}

#[test]
fn delta_whiteouts_a_deleted_file_once() {
    let staged = tree(vec![file(r"C:\DELETE.TXT", b"delete")], Vec::new());
    let read_back = Tree::new();

    let run = build_delta(&staged, &read_back, &boot(0xAA), &boot(0xAA));

    assert_eq!(run.delta.whiteouts, vec![dos(r"C:\DELETE.TXT")]);
    assert!(run.delta.files.is_empty());
    assert!(run.delta.dirs.is_empty());
}

#[test]
fn delta_whiteouts_a_deleted_directory_once() {
    let staged = tree(
        vec![
            file(r"C:\DATA\ONE.TXT", b"one"),
            file(r"C:\DATA\SUB\TWO.TXT", b"two"),
        ],
        vec![dir(r"C:\DATA"), dir(r"C:\DATA\SUB")],
    );
    let read_back = Tree::new();

    let run = build_delta(&staged, &read_back, &boot(0xAA), &boot(0xAA));

    assert_eq!(run.delta.whiteouts, vec![dos(r"C:\DATA")]);
    assert!(run.delta.files.is_empty());
    assert!(run.delta.dirs.is_empty());
}

#[test]
fn delta_handles_a_file_replaced_by_a_directory() {
    let staged = tree(vec![file(r"C:\ITEM", b"was a file")], Vec::new());
    let read_back = tree(Vec::new(), vec![dir(r"C:\ITEM")]);

    let run = build_delta(&staged, &read_back, &boot(0xAA), &boot(0xAA));

    assert_eq!(run.delta.whiteouts, vec![dos(r"C:\ITEM")]);
    assert!(run.delta.files.is_empty());
    assert_eq!(run.delta.dirs.len(), 1);
    let added = &run.delta.dirs[0];
    assert_eq!(added.path, dos(r"C:\ITEM"));
    assert_eq!(added.attributes, Attributes::default());
    assert_eq!(added.timestamp, fixed_timestamp());
}

#[test]
fn delta_ignores_the_agent_directory() {
    let staged = tree(
        vec![
            file(r"C:\KEEP.TXT", b"keep"),
            file(r"C:\DOSKER\DOSKER.EXE", b"agent"),
        ],
        vec![dir(r"C:\DOSKER")],
    );
    let read_back = tree(
        vec![
            file(r"C:\KEEP.TXT", b"keep"),
            file(r"C:\DOSKER\CONFIG.INI", b"CONTROL_PORT=2\r\nBAUD=9600\r\n"),
            file(r"C:\DOSKER\OUT.TMP", b"output"),
        ],
        vec![dir(r"C:\DOSKER")],
    );

    let run = build_delta(&staged, &read_back, &boot(0xAA), &boot(0xAA));

    assert_no_changes(&run);
}

#[test]
fn delta_ignores_timestamp_and_attribute_only_changes() {
    let staged_file = file(r"C:\SAME.TXT", b"same");
    let staged = tree(vec![staged_file.clone()], vec![dir(r"C:\SAME")]);

    let mut changed_file = staged_file;
    changed_file.attributes = Attributes::default();
    changed_file.timestamp = fixed_timestamp() + TimeDelta::hours(1);
    let changed_dir = TreeDir {
        path: dos(r"C:\SAME"),
        attributes: Attributes {
            hidden: true,
            ..Attributes::default()
        },
        timestamp: fixed_timestamp() + TimeDelta::hours(2),
    };
    let read_back = tree(vec![changed_file], vec![changed_dir]);

    let run = build_delta(&staged, &read_back, &boot(0xAA), &boot(0xAA));

    assert_no_changes(&run);
}

#[test]
fn delta_records_a_boot_override() {
    let before = boot(0xAA);
    let after = boot(0xAB);

    let run = build_delta(&Tree::new(), &Tree::new(), &before, &after);

    assert_eq!(run.boot, Some(after));
}

#[test]
fn delta_records_a_partition_override() {
    let before = boot(0xAA);
    let mut after = before.clone();
    after.partition_start =
        Some(before.partition_start.expect("the fixture is partitioned") + 2048);

    let run = build_delta(&Tree::new(), &Tree::new(), &before, &after);

    assert_eq!(run.boot, Some(after));
}

#[test]
fn delta_keeps_boot_null_when_the_artifacts_match() {
    let before = boot(0xAA);
    let after = before.clone();

    let run = build_delta(&Tree::new(), &Tree::new(), &before, &after);

    assert_eq!(run.boot, None);
    assert_no_changes(&run);
}
