//! End-to-end checks for `dosker image import`.

mod common;

use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

use common::{CliRoot, make_disk_image, run_captured, stderr_text, stdout_text};
use dosker_image::{ImageReference, ImageStore};

const MIB: u64 = 1024 * 1024;
const SECTOR_SIZE: u64 = 512;
const PARTITION_TABLE_OFFSET: usize = 446;

fn disk_source(root: &CliRoot, name: &str) -> std::path::PathBuf {
    let path = root.path().join(name);
    make_disk_image(&path);
    path
}

fn run_import(root: &CliRoot, source: &Path, reference: &str) -> Output {
    root.run(&[
        "image",
        "import",
        source.to_str().expect("source path is utf-8"),
        reference,
    ])
}

/// Rewrites an MBR-partitioned image as a superfloppy: the FAT boot sector
/// moves to offset zero and the partition table is cleared.
fn make_superfloppy(image: &Path) {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(image)
        .expect("open image");
    let mut sector = [0u8; SECTOR_SIZE as usize];
    file.seek(SeekFrom::Start(MIB)).expect("seek boot sector");
    file.read_exact(&mut sector).expect("read boot sector");
    for byte in &mut sector[PARTITION_TABLE_OFFSET..510] {
        *byte = 0;
    }
    sector[510] = 0x55;
    sector[511] = 0xAA;
    file.seek(SeekFrom::Start(0)).expect("seek image start");
    file.write_all(&sector)
        .expect("write superfloppy boot sector");
    file.flush().expect("flush image");
}

#[test]
fn image_import_prints_the_reference_and_digest() {
    let root = CliRoot::new();
    let source = disk_source(&root, "source.img");
    let size = fs::metadata(&source).expect("source metadata").len();
    let size_text = size.to_string();

    let output = run_import(&root, &source, "freedos-test:latest");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );

    let text = stdout_text(&output);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1, "output: {text}");
    let fields: Vec<&str> = lines[0].split_whitespace().collect();
    assert_eq!(fields.first(), Some(&"imported"));
    assert_eq!(fields.get(1), Some(&"freedos-test:latest"));
    let digest = fields.get(2).expect("digest field");
    assert!(digest.starts_with("sha256:"), "digest: {digest}");
    assert_eq!(digest.len(), "sha256:".len() + 64);
    assert_eq!(fields.get(3), Some(&size_text.as_str()));
    assert_eq!(fields.get(4), Some(&"bytes"));
}

#[test]
fn image_import_booter_accepts_an_opaque_blob() {
    let root = CliRoot::new();
    let source = root.path().join("boot.img");
    fs::write(&source, b"not a filesystem").expect("write booter blob");

    let output = root.run(&[
        "image",
        "import",
        "--booter",
        source.to_str().expect("source path is utf-8"),
        "booter-test:latest",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(stdout_text(&output).contains("booter-test:latest"));
}

#[test]
fn image_import_rejects_a_missing_file() {
    let root = CliRoot::new();
    let source = root.path().join("absent.img");

    let output = run_import(&root, &source, "freedos-test:latest");
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn image_import_rejects_a_non_bootable_disk() {
    let root = CliRoot::new();
    let source = root.path().join("random.img");
    fs::write(&source, vec![0x5a; (MIB) as usize]).expect("write random file");

    let output = run_import(&root, &source, "freedos-test:latest");
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn image_import_accepts_the_mbr_cli_fixture() {
    let root = CliRoot::new();
    let source = disk_source(&root, "source.img");

    let output = run_import(&root, &source, "freedos-test:latest");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn image_import_rejects_a_superfloppy_disk() {
    let root = CliRoot::new();
    let source = disk_source(&root, "superfloppy.img");
    make_superfloppy(&source);

    let output = run_import(&root, &source, "freedos-test:latest");
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}",
        stdout_text(&output)
    );
    assert!(
        stderr_text(&output).contains("unsupported image layout"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn image_import_rejects_a_duplicate_reference() {
    let root = CliRoot::new();
    let source = disk_source(&root, "source.img");

    let first = run_import(&root, &source, "freedos-test:latest");
    assert_eq!(
        first.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&first)
    );

    let second = run_import(&root, &source, "freedos-test:latest");
    assert_eq!(second.status.code(), Some(1));
    assert!(
        stderr_text(&second).contains("already exists"),
        "stderr: {}",
        stderr_text(&second)
    );
}

#[test]
fn image_import_without_a_tag_uses_latest() {
    let root = CliRoot::new();
    let source = disk_source(&root, "source.img");

    let output = run_import(&root, &source, "freedos-test");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(stdout_text(&output).contains("freedos-test:latest"));

    let store = ImageStore::new(root.path().to_path_buf());
    let reference = ImageReference::parse("freedos-test:latest").expect("parse reference");
    let record = store.load(&reference).expect("load the imported image");
    assert_eq!(record.name, "freedos-test");
    assert_eq!(record.tag, "latest");
}

#[test]
fn image_import_rejects_an_invalid_reference_with_exit_two() {
    let root = CliRoot::new();
    let source = disk_source(&root, "source.img");

    for reference in ["FreeDOS:latest", "freedos:"] {
        let mut command: Command = root.command();
        command.args([
            "image",
            "import",
            source.to_str().expect("source path is utf-8"),
            reference,
        ]);
        let output = run_captured(&mut command, Duration::from_secs(15));
        assert_eq!(
            output.status.code(),
            Some(2),
            "reference {reference}; stderr: {}",
            stderr_text(&output)
        );
    }
}
