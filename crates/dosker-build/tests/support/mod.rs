#![allow(dead_code)]

//! Fixtures for the build orchestration tests.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use dosker_build::{BuildError, BuildReporter, EngineResolver, ResolvedEngine};
use dosker_image::{
    ANNOTATION_RAW_DISK_DIGEST, BaseInputs, BlobRef, BootLayout, BootMetadata, DosFlavour,
    FatType as ImageFatType, FreedosPins, Geometry, HistoryEntry, ImageConfig, ImageReference,
    Layer, MEDIA_TYPE_BOOTER, MachineProfile, OciStore, Tree, fixed_timestamp, sha256_digest,
};
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions, format_volume};

const MIB: u64 = 1024 * 1024;

const FREEDOS_SOURCE_KERNEL: &[u8] = b"synthetic FreeDOS kernel";
const FREEDOS_SOURCE_COMMAND: &[u8] = b"synthetic FreeDOS command interpreter";
const FAKE_AGENT: &[u8] = b"MZ synthetic dosker agent";

/// Resolves every engine through a fixed host result.
pub struct FakeResolver;

impl EngineResolver for FakeResolver {
    fn resolve(&self, engine: &str) -> Result<ResolvedEngine, BuildError> {
        Ok(ResolvedEngine {
            name: engine.to_owned(),
            version: "qemu 9.0.0".to_owned(),
            source: "path".to_owned(),
        })
    }
}

/// Collects reporter lines and command output.
#[derive(Default)]
pub struct TestReporter {
    /// The reported progress lines.
    pub lines: Vec<String>,
    /// The captured command output.
    pub output: Vec<u8>,
}

impl BuildReporter for TestReporter {
    fn line(&mut self, text: &str) -> io::Result<()> {
        self.lines.push(text.to_owned());
        Ok(())
    }

    fn output(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.output.extend_from_slice(bytes);
        Ok(())
    }
}

impl TestReporter {
    /// Returns the captured command output as text.
    pub fn output_text(&self) -> String {
        String::from_utf8_lossy(&self.output).into_owned()
    }
}

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

/// Writes a 4 MiB superfloppy FAT16 image holding the FreeDOS inputs.
pub fn write_freedos_source(path: &Path) -> (Vec<u8>, Vec<u8>) {
    let size = 4 * MIB;
    let file = File::create(path).expect("create the FreeDOS source");
    file.set_len(size).expect("size the FreeDOS source");

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open the FreeDOS source");
    let options = FormatVolumeOptions::new()
        .fat_type(FatType::Fat16)
        .total_sectors((size / 512) as u32);
    format_volume(file, options).expect("format the FreeDOS source");

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("reopen the FreeDOS source");
    let fs = FileSystem::new(file, FsOptions::new()).expect("mount the FreeDOS source");
    {
        let root = fs.root_dir();
        let mut kernel = root.create_file("KERNEL.SYS").expect("create KERNEL.SYS");
        kernel
            .write_all(FREEDOS_SOURCE_KERNEL)
            .expect("write KERNEL.SYS");
        let mut command = root.create_file("COMMAND.COM").expect("create COMMAND.COM");
        command
            .write_all(FREEDOS_SOURCE_COMMAND)
            .expect("write COMMAND.COM");
    }
    fs.unmount().expect("unmount the FreeDOS source");

    (
        FREEDOS_SOURCE_KERNEL.to_vec(),
        FREEDOS_SOURCE_COMMAND.to_vec(),
    )
}

/// Writes a small fake agent executable.
pub fn write_agent(path: &Path) -> Vec<u8> {
    fs::write(path, FAKE_AGENT).expect("write the agent executable");
    FAKE_AGENT.to_vec()
}

/// Builds inputs whose pins match the written source and agent files.
pub fn synthetic_inputs(source: &Path, agent: &Path) -> BaseInputs {
    let kernel =
        dosker_image::read_file(source, "\\KERNEL.SYS").expect("read the source KERNEL.SYS");
    let command =
        dosker_image::read_file(source, "\\COMMAND.COM").expect("read the source COMMAND.COM");
    let agent_bytes = fs::read(agent).expect("read the agent executable");
    BaseInputs {
        source: source.to_path_buf(),
        agent_executable: agent.to_path_buf(),
        pins: FreedosPins {
            kernel_sha256: sha256_digest(&kernel),
            command_sha256: sha256_digest(&command),
            agent_sha256: sha256_digest(&agent_bytes),
        },
    }
}

/// Returns an empty canonical tree.
pub fn empty_tree() -> Tree {
    Tree::new()
}
