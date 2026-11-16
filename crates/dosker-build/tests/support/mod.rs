#![allow(dead_code)]

//! Fixtures for the build orchestration tests.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use dosker_image::{
    ANNOTATION_RAW_DISK_DIGEST, BlobRef, BootLayout, BootMetadata, DosFlavour,
    FatType as ImageFatType, Geometry, HistoryEntry, ImageConfig, ImageReference, Layer,
    MEDIA_TYPE_BOOTER, MachineProfile, OciStore, Tree, fixed_timestamp,
};
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions, format_volume};

const MIB: u64 = 1024 * 1024;

/// Builds a valid MBR-partitioned boot metadata value.
pub fn sample_boot() -> BootMetadata {
    BootMetadata {
        layout: BootLayout::MbrPartitioned,
        disk_size: 16 * MIB,
        geometry: Geometry {
            cylinders: 16,
            heads: 4,
            sectors: 32,
            sector_size: 512,
        },
        partition_start: Some(1),
        partition_sectors: Some(32_768),
        mbr: Some(BlobRef::from_bytes(&[0u8; 512])),
        boot_sector: Some(BlobRef::from_bytes(&[0u8; 512])),
        fat_type: Some(ImageFatType::Fat16),
        volume_label: Some("DOSKER".to_owned()),
        media_descriptor: Some(0xf8),
    }
}

/// Builds a valid tree or booter config with the given agent version.
pub fn sample_config(boot: BootMetadata, agent_version: Option<&str>) -> ImageConfig {
    let booter = boot.layout == BootLayout::Opaque;
    let mut machine = MachineProfile::default();
    if !booter {
        machine.disk_interface = match boot.layout {
            BootLayout::MbrPartitioned => "ide".to_owned(),
            BootLayout::Superfloppy => "floppy".to_owned(),
            BootLayout::Opaque => unreachable!("opaque is a booter"),
        };
    }
    ImageConfig {
        schema_version: 1,
        dos_flavour: if booter {
            DosFlavour::None
        } else {
            DosFlavour::Other
        },
        booter,
        boot,
        machine,
        env: BTreeMap::new(),
        workdir: if booter {
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
        agent_version: agent_version.map(str::to_owned),
        created_by: "base".to_owned(),
        history: vec![HistoryEntry {
            created_by: "base".to_owned(),
            created_at: fixed_timestamp(),
            empty_layer: false,
            comment: Some("base".to_owned()),
        }],
    }
}

/// Stores one tree base image and returns its manifest digest.
pub fn store_tree_base(
    root: &Path,
    reference: &str,
    tree: &Tree,
    raw_disk: Option<&[u8]>,
    agent_version: Option<&str>,
) -> String {
    let store = OciStore::new(root.to_path_buf());
    let layer =
        Layer::from_delta(&tree.to_delta(), "base", None, None).expect("the base layer builds");
    let descriptor = store.put_layer(&layer).expect("the base layer stores");
    let mut annotations = BTreeMap::new();
    if let Some(bytes) = raw_disk {
        let blob = store.put_blob(bytes).expect("the raw disk stores");
        annotations.insert(ANNOTATION_RAW_DISK_DIGEST.to_owned(), blob.digest);
    }
    let reference = ImageReference::parse(reference).expect("the base reference parses");
    let committed = store
        .commit_image(
            Some(&reference),
            &sample_config(sample_boot(), agent_version),
            vec![descriptor],
            annotations,
        )
        .expect("the base image commits");
    committed.manifest_digest
}

/// Stores one booter base image and returns its manifest digest.
pub fn store_booter_base(root: &Path, reference: &str, bytes: &[u8]) -> String {
    let store = OciStore::new(root.to_path_buf());
    let blob = store.put_blob(bytes).expect("the booter blob stores");
    let descriptor = dosker_image::Descriptor {
        media_type: MEDIA_TYPE_BOOTER.to_owned(),
        digest: blob.digest,
        size: blob.size,
        annotations: None,
    };
    let reference = ImageReference::parse(reference).expect("the booter reference parses");
    let config = sample_config(BootMetadata::opaque(bytes.len() as u64), None);
    let committed = store
        .commit_image(Some(&reference), &config, vec![descriptor], BTreeMap::new())
        .expect("the booter image commits");
    committed.manifest_digest
}

/// Writes a 4 MiB superfloppy FAT16 image with an `AUTOEXEC.BAT`.
pub fn write_fat_image(path: &Path) {
    let size = 4 * MIB;
    let file = File::create(path).expect("create the disk image");
    file.set_len(size).expect("size the disk image");

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open the disk image");
    let options = FormatVolumeOptions::new()
        .fat_type(FatType::Fat16)
        .total_sectors((size / 512) as u32);
    format_volume(file, options).expect("format the disk image");

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("reopen the disk image");
    let fs = FileSystem::new(file, FsOptions::new()).expect("mount the disk image");
    {
        let root = fs.root_dir();
        let mut entry = root
            .create_file("AUTOEXEC.BAT")
            .expect("create AUTOEXEC.BAT");
        entry
            .write_all(b"@ECHO OFF\r\n")
            .expect("write AUTOEXEC.BAT");
    }
    fs.unmount().expect("unmount the disk image");
}

/// Returns an empty canonical tree.
pub fn empty_tree() -> Tree {
    Tree::new()
}
