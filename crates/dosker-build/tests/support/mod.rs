#![allow(dead_code)]

//! Fixtures for the build orchestration tests.

use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dosker_build::{
    BuildError, BuildOptions, BuildOutcome, BuildReporter, Builder, EngineResolver,
    InDosRunOutcome, InDosRunRequest, InDosRunner, ResolvedEngine,
};
use dosker_core::{EngineName, LicenseBasis, LicenseEntry, LicenseStatus};
use dosker_image::{
    ANNOTATION_RAW_DISK_DIGEST, Attributes, BaseInputs, BlobRef, BootLayout, BootMetadata,
    DosFlavour, DosPath, FatType as ImageFatType, FreedosPins, Geometry, HistoryEntry, ImageConfig,
    ImageReference, Layer, LayerDelta, MEDIA_TYPE_BOOTER, MachineProfile, OciStore, Tree, TreeDir,
    TreeFile, fixed_timestamp, sha256_digest,
};
use fatfs::{
    Date, DateTime, FatType, FileSystem, FormatVolumeOptions, FsOptions, Time, TimeProvider,
    format_volume,
};

const MIB: u64 = 1024 * 1024;

/// The first byte of the agent disk's FAT volume.
pub const AGENT_DISK_PARTITION_OFFSET: u64 = 1_048_576;

/// The length of the agent disk's FAT volume.
pub const AGENT_DISK_PARTITION_LENGTH: u64 = 8 * 1024 * 1024;

/// The size of the agent disk.
pub const AGENT_DISK_SIZE: u64 = 16 * 1024 * 1024;

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
        licenses: Vec::new(),
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

/// Builds a valid redistributable `MIT` license entry.
pub fn sample_license(name: &str) -> LicenseEntry {
    LicenseEntry {
        name: name.to_owned(),
        spdx: "MIT".to_owned(),
        holder: None,
        status: LicenseStatus::Redistributable,
        basis: Some(LicenseBasis::OpenSourceLicense),
        evidence: None,
        license_url: None,
        covers: Vec::new(),
        source_url: None,
        notice: None,
    }
}

/// Stores one licensed empty-tree base and returns its manifest digest.
pub fn store_licensed_tree_base(
    root: &Path,
    reference: &str,
    licenses: Vec<LicenseEntry>,
) -> String {
    let store = OciStore::new(root.to_path_buf());
    let tree = empty_tree();
    let layer =
        Layer::from_delta(&tree.to_delta(), "base", None, None).expect("the base layer builds");
    let descriptor = store.put_layer(&layer).expect("the base layer stores");
    let mut config = sample_config(sample_boot(), Some("1.2.3"));
    config.licenses = licenses;
    let reference = ImageReference::parse(reference).expect("the base reference parses");
    let committed = store
        .commit_image(Some(&reference), &config, vec![descriptor], BTreeMap::new())
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

/// Stores one final tree image with explicit licenses and returns its digest.
pub fn store_final_tree(
    root: &Path,
    reference: &str,
    tree: &Tree,
    licenses: Vec<LicenseEntry>,
) -> String {
    let store = OciStore::new(root.to_path_buf());
    let layer =
        Layer::from_delta(&tree.to_delta(), "final", None, None).expect("the final layer builds");
    let descriptor = store.put_layer(&layer).expect("the final layer stores");
    let mut config = sample_config(sample_boot(), Some("1.2.3"));
    config.licenses = licenses;
    let reference = ImageReference::parse(reference).expect("the final reference parses");
    let committed = store
        .commit_image(Some(&reference), &config, vec![descriptor], BTreeMap::new())
        .expect("the final image commits");
    committed.manifest_digest
}

/// Stores one final booter image with explicit licenses and returns its digest.
pub fn store_final_booter(
    root: &Path,
    reference: &str,
    bytes: &[u8],
    licenses: Vec<LicenseEntry>,
) -> String {
    let store = OciStore::new(root.to_path_buf());
    let blob = store.put_blob(bytes).expect("the booter blob stores");
    let descriptor = dosker_image::Descriptor {
        media_type: MEDIA_TYPE_BOOTER.to_owned(),
        digest: blob.digest,
        size: blob.size,
        annotations: None,
    };
    let mut config = sample_config(BootMetadata::opaque(bytes.len() as u64), None);
    config.licenses = licenses;
    let reference = ImageReference::parse(reference).expect("the booter reference parses");
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

/// Resolves each engine through a fixed map of versions.
pub struct VersionedResolver {
    /// The version reported for each engine name.
    pub versions: BTreeMap<String, String>,
}

impl EngineResolver for VersionedResolver {
    fn resolve(&self, engine: &str) -> Result<ResolvedEngine, BuildError> {
        let version = self
            .versions
            .get(engine)
            .cloned()
            .unwrap_or_else(|| "0.0.0".to_owned());
        Ok(ResolvedEngine {
            name: engine.to_owned(),
            version,
            source: "path".to_owned(),
        })
    }
}

/// One in-DOS `RUN` invocation as the scripted runner saw it.
#[derive(Clone, Debug)]
pub struct RecordedRun {
    /// The Doskerfile line that holds the instruction.
    pub line: usize,
    /// The command sent to the agent.
    pub command: String,
    /// The DOS working directory.
    pub workdir: String,
    /// The environment passed to the command.
    pub env: BTreeMap<String, String>,
    /// The command timeout.
    pub timeout: Duration,
    /// The boot wait timeout.
    pub boot_timeout: Duration,
    /// The projected disk path.
    pub disk_path: PathBuf,
    /// The disk interface the engine would attach.
    pub disk_interface: String,
    /// The configured serial port count.
    pub serial_ports: u8,
    /// The configured control port.
    pub control_port: u8,
    /// The `CONFIG.INI` contents before the command ran.
    pub config_ini: Option<String>,
    /// The boot metadata of the projected disk before the command ran.
    pub boot: BootMetadata,
    /// The engine selected for the step.
    pub engine: EngineName,
}

/// One scripted in-DOS command.
pub trait RunScript: Send {
    /// Runs the script against one request.
    fn run(&mut self, request: &InDosRunRequest<'_>) -> Result<InDosRunOutcome, BuildError>;
}

impl<F> RunScript for F
where
    F: for<'a> FnMut(&InDosRunRequest<'a>) -> Result<InDosRunOutcome, BuildError> + Send,
{
    fn run(&mut self, request: &InDosRunRequest<'_>) -> Result<InDosRunOutcome, BuildError> {
        self(request)
    }
}

/// The shared state of a scripted runner.
#[derive(Default)]
pub struct ScriptedState {
    /// The requests the builder made.
    pub calls: Vec<RecordedRun>,
    /// The commands still to run.
    pub scripts: VecDeque<Box<dyn RunScript>>,
}

/// Runs scripted commands and records each request.
#[derive(Clone, Default)]
pub struct ScriptedRunner {
    state: Arc<Mutex<ScriptedState>>,
}

impl ScriptedRunner {
    /// Creates a runner with no scripts.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues one script. An unscripted call succeeds with empty output.
    pub fn push<F>(&self, script: F)
    where
        F: for<'a> FnMut(&InDosRunRequest<'a>) -> Result<InDosRunOutcome, BuildError>
            + Send
            + 'static,
    {
        self.state
            .lock()
            .expect("lock the scripted runner")
            .scripts
            .push_back(Box::new(script));
    }

    /// Returns the recorded requests.
    pub fn calls(&self) -> Vec<RecordedRun> {
        self.state
            .lock()
            .expect("lock the scripted runner")
            .calls
            .clone()
    }

    /// Returns the number of recorded requests.
    pub fn call_count(&self) -> usize {
        self.state
            .lock()
            .expect("lock the scripted runner")
            .calls
            .len()
    }
}

impl InDosRunner for ScriptedRunner {
    fn run(&mut self, request: &InDosRunRequest<'_>) -> Result<InDosRunOutcome, BuildError> {
        let config_ini = dosker_image::read_file(request.disk_path, r"C:\DOSKER\CONFIG.INI")
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let boot = dosker_image::read_boot_metadata(request.disk_path)
            .expect("the projected disk has boot metadata");
        let record = RecordedRun {
            line: request.line,
            command: request.command.to_owned(),
            workdir: request.workdir.to_dos_string(),
            env: request.env.clone(),
            timeout: request.timeout,
            boot_timeout: request.boot_timeout,
            disk_path: request.disk_path.to_path_buf(),
            disk_interface: request.disk_interface.to_owned(),
            serial_ports: request.serial_ports,
            control_port: request.control_port,
            config_ini,
            boot,
            engine: request.engine,
        };

        let script = {
            let mut state = self.state.lock().expect("lock the scripted runner");
            state.calls.push(record);
            state.scripts.pop_front()
        };

        match script {
            Some(mut script) => script.run(request),
            None => Ok(InDosRunOutcome {
                exit_code: 0,
                output: String::new(),
                truncated: false,
            }),
        }
    }
}

/// A build fixture with an agent disk base.
pub struct RunnerFixture {
    /// The OCI data root.
    pub root: tempfile::TempDir,
    /// The build context.
    pub context: tempfile::TempDir,
    /// The agent disk the base was stored from.
    pub disk: PathBuf,
    /// The base's effective boot metadata.
    pub boot: BootMetadata,
    /// The base manifest digest.
    pub base_digest: String,
}

impl RunnerFixture {
    /// Returns the data root.
    pub fn root_path(&self) -> &Path {
        self.root.path()
    }

    /// Returns the build context.
    pub fn context_path(&self) -> &Path {
        self.context.path()
    }

    /// Returns the OCI store.
    pub fn store(&self) -> OciStore {
        OciStore::new(self.root_path().to_path_buf())
    }

    /// Writes the Doskerfile.
    pub fn doskerfile(&self, text: &str) {
        fs::write(self.context_path().join("Doskerfile"), text).expect("the Doskerfile is written");
    }

    /// Builds with the fake resolver and a scripted runner.
    pub fn build(
        &self,
        options: &BuildOptions,
        runner: &ScriptedRunner,
    ) -> (Result<BuildOutcome, BuildError>, TestReporter) {
        self.build_with(options, Box::new(FakeResolver), runner)
    }

    /// Builds with a supplied resolver and a scripted runner.
    pub fn build_with(
        &self,
        options: &BuildOptions,
        resolver: Box<dyn EngineResolver>,
        runner: &ScriptedRunner,
    ) -> (Result<BuildOutcome, BuildError>, TestReporter) {
        let mut reporter = TestReporter::default();
        let result = Builder::with_resolver(self.root_path().to_path_buf(), resolver)
            .with_in_dos_runner(Box::new(runner.clone()))
            .build(self.context_path(), options, &mut reporter);
        (result, reporter)
    }
}

/// Creates a fixture whose `base:latest` holds an agent disk.
pub fn runner_fixture() -> RunnerFixture {
    let root = tempfile::tempdir().expect("the data root is created");
    let context = tempfile::tempdir().expect("the context is created");
    let disk = context.path().join("agent.img");
    let boot = make_agent_disk(&disk);
    let base_digest = store_agent_base(root.path(), "base:latest", &disk);
    RunnerFixture {
        root,
        context,
        disk,
        boot,
        base_digest,
    }
}

/// Creates a FAT16 MBR disk with the agent files and returns its boot metadata.
pub fn make_agent_disk(path: &Path) -> BootMetadata {
    make_fat16_disk(
        path,
        AGENT_DISK_SIZE,
        AGENT_DISK_PARTITION_OFFSET,
        AGENT_DISK_PARTITION_LENGTH,
    );
    let tree = disk_tree();
    write_tree_at(
        path,
        AGENT_DISK_PARTITION_OFFSET,
        AGENT_DISK_PARTITION_LENGTH,
        &tree,
    );
    dosker_image::read_boot_metadata(path).expect("the agent disk boot metadata reads")
}

/// Stores a tree base built from an agent disk and returns its manifest digest.
pub fn store_agent_base(root: &Path, reference: &str, disk_path: &Path) -> String {
    store_agent_base_from_disk(root, reference, disk_path)
}

/// Stores a base from an existing agent disk and returns its manifest digest.
pub fn store_agent_base_from_disk(root: &Path, reference: &str, disk_path: &Path) -> String {
    let store = OciStore::new(root.to_path_buf());
    let tree = dosker_image::read_tree(disk_path).expect("the base tree reads");
    let boot = dosker_image::read_boot_metadata(disk_path).expect("the base boot reads");
    let disk_bytes = fs::read(disk_path).expect("the base disk reads");
    let blob = store.put_blob(&disk_bytes).expect("the raw disk stores");
    let layer =
        Layer::from_delta(&tree.to_delta(), "base", None, None).expect("the base layer builds");
    let descriptor = store.put_layer(&layer).expect("the base layer stores");
    let mut annotations = BTreeMap::new();
    annotations.insert(ANNOTATION_RAW_DISK_DIGEST.to_owned(), blob.digest);
    let reference = ImageReference::parse(reference).expect("the base reference parses");
    let committed = store
        .commit_image(
            Some(&reference),
            &sample_config(boot, Some("1.0.0")),
            vec![descriptor],
            annotations,
        )
        .expect("the base image commits");
    committed.manifest_digest
}

/// Stores an agent base whose raw disk differs only in an unused gap byte.
pub fn store_agent_base_with_gap_byte(root: &Path, reference: &str, byte: u8) -> String {
    let dir = tempfile::tempdir().expect("the gap disk directory is created");
    let disk = dir.path().join("agent.img");
    make_agent_disk(&disk);
    write_disk_byte(
        &disk,
        AGENT_DISK_PARTITION_OFFSET + AGENT_DISK_PARTITION_LENGTH + 4096,
        byte,
    );
    store_agent_base_from_disk(root, reference, &disk)
}

/// Reads one file from a disk's FAT volume.
pub fn disk_file(disk: &Path, path: &str) -> Option<Vec<u8>> {
    dosker_image::read_file(disk, path).ok()
}

/// Writes one file into a disk's FAT volume.
pub fn write_disk_file(disk: &Path, path: &str, bytes: &[u8]) {
    let dos = DosPath::parse(path).expect("the DOS path parses");
    with_volume(disk, |root| {
        let mut entry = root
            .create_file(dos.as_str())
            .expect("the disk file is created");
        entry.truncate().expect("the disk file is truncated");
        entry.write_all(bytes).expect("the disk file is written");
    });
}

/// Removes one file from a disk's FAT volume.
pub fn remove_disk_file(disk: &Path, path: &str) {
    let dos = DosPath::parse(path).expect("the DOS path parses");
    with_volume(disk, |root| {
        root.remove(dos.as_str()).expect("the disk file is removed");
    });
}

/// Reads one byte of a raw disk.
pub fn disk_byte(disk: &Path, offset: u64) -> u8 {
    let mut file = File::open(disk).expect("open the disk");
    file.seek(SeekFrom::Start(offset)).expect("seek the disk");
    let mut byte = [0u8; 1];
    file.read_exact(&mut byte).expect("read the disk byte");
    byte[0]
}

/// Inverts one byte of a raw disk.
pub fn flip_disk_byte(disk: &Path, offset: u64) {
    let byte = disk_byte(disk, offset);
    write_disk_byte(disk, offset, byte ^ 0x01);
}

/// Moves a disk's FAT volume to a new partition and returns the boot metadata.
pub fn move_disk_partition(disk: &Path, new_offset: u64, new_length: u64) -> BootMetadata {
    let tree = dosker_image::read_tree(disk).expect("the current tree reads");
    let before = dosker_image::read_boot_metadata(disk).expect("the current boot reads");

    write_mbr(disk, new_offset, new_length);
    let mut options = FormatVolumeOptions::new()
        .fat_type(FatType::Fat16)
        .total_sectors((new_length / 512) as u32)
        .sectors_per_track(before.geometry.sectors as u16)
        .heads(before.geometry.heads as u16);
    if let Some(label) = &before.volume_label {
        options = options.volume_label(label_bytes(label));
    }
    format_at(disk, new_offset, new_length, options);
    write_tree_at(disk, new_offset, new_length, &tree);

    dosker_image::read_boot_metadata(disk).expect("the moved boot metadata reads")
}

fn disk_tree() -> Tree {
    let mut delta = LayerDelta::new();
    delta.add_dir(TreeDir {
        path: DosPath::parse(r"C:\DOSKER").expect("the DOSKER path parses"),
        attributes: Attributes::default(),
        timestamp: fixed_timestamp(),
    });
    delta.add_dir(TreeDir {
        path: DosPath::parse(r"C:\OLD").expect("the OLD path parses"),
        attributes: Attributes::default(),
        timestamp: fixed_timestamp(),
    });
    for (path, bytes) in [
        (r"C:\AUTOEXEC.BAT", b"@ECHO OFF\r\n".as_slice()),
        (r"C:\HELLO.TXT", b"hello".as_slice()),
        (r"C:\OLD.TXT", b"old".as_slice()),
        (r"C:\OLD\INNER.TXT", b"inner".as_slice()),
        (r"C:\DOSKER\DOSKER.EXE", b"MZ synthetic agent".as_slice()),
    ] {
        delta.add_file(TreeFile {
            path: DosPath::parse(path).expect("the disk file path parses"),
            attributes: archive(),
            timestamp: fixed_timestamp(),
            bytes: bytes.to_vec(),
        });
    }

    let mut tree = Tree::new();
    tree.apply_delta(&delta).expect("the disk tree applies");
    tree
}

fn archive() -> Attributes {
    Attributes {
        archive: true,
        ..Attributes::default()
    }
}

fn make_fat16_disk(path: &Path, total_size: u64, offset: u64, length: u64) {
    let file = File::create(path).expect("create the disk");
    file.set_len(total_size).expect("size the disk");
    write_mbr(path, offset, length);
    let options = FormatVolumeOptions::new()
        .fat_type(FatType::Fat16)
        .total_sectors((length / 512) as u32);
    format_at(path, offset, length, options);
}

fn format_at(path: &Path, offset: u64, length: u64, options: FormatVolumeOptions) {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open the disk");
    let region = Region::new(file, offset, length);
    format_volume(region, options).expect("format FAT16");
}

fn write_mbr(path: &Path, partition_offset: u64, partition_length: u64) {
    let mut sector = [0u8; 512];
    let entry = 446;
    sector[entry] = 0x80;
    sector[entry + 4] = 0x06;
    let start_lba = (partition_offset / 512) as u32;
    let sectors = (partition_length / 512) as u32;
    sector[entry + 8..entry + 12].copy_from_slice(&start_lba.to_le_bytes());
    sector[entry + 12..entry + 16].copy_from_slice(&sectors.to_le_bytes());
    sector[510] = 0x55;
    sector[511] = 0xAA;

    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open the MBR");
    file.seek(SeekFrom::Start(0)).expect("seek the MBR");
    file.write_all(&sector).expect("write the MBR");
}

fn write_tree_at(path: &Path, offset: u64, length: u64, tree: &Tree) {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open the disk");
    let region = Region::new(file, offset, length);
    let fs = FileSystem::new(region, fixed_fs_options()).expect("mount FAT");
    {
        let root = fs.root_dir();
        for dir in tree.dirs() {
            root.create_dir(dir.path.as_str())
                .expect("create the directory");
        }
        for file in tree.files() {
            let mut entry = root
                .create_file(file.path.as_str())
                .expect("create the disk file");
            entry.truncate().expect("truncate the disk file");
            entry.write_all(&file.bytes).expect("write the disk file");
        }
    }
    fs.unmount().expect("unmount FAT");
}

fn with_volume<T>(disk: &Path, action: impl FnOnce(&fatfs::Dir<'_, Region>) -> T) -> T {
    let region = dosker_image::fat_region(disk).expect("the disk has a FAT region");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(disk)
        .expect("open the disk");
    let region = Region::new(file, region.offset, region.length);
    let fs = FileSystem::new(region, fixed_fs_options()).expect("mount FAT");
    let result = {
        let root = fs.root_dir();
        action(&root)
    };
    fs.unmount().expect("unmount FAT");
    result
}

fn write_disk_byte(disk: &Path, offset: u64, byte: u8) {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(disk)
        .expect("open the disk");
    file.seek(SeekFrom::Start(offset)).expect("seek the disk");
    file.write_all(&[byte]).expect("write the disk byte");
    file.flush().expect("flush the disk byte");
}

fn label_bytes(label: &str) -> [u8; 11] {
    let mut bytes = [b' '; 11];
    for (slot, byte) in bytes.iter_mut().zip(label.bytes().take(11)) {
        *slot = byte;
    }
    bytes
}

/// A fixed time provider so generated FAT entries are reproducible.
#[derive(Debug)]
struct FixedTime;

impl TimeProvider for FixedTime {
    fn get_current_date(&self) -> Date {
        Date {
            year: 1980,
            month: 1,
            day: 1,
        }
    }

    fn get_current_date_time(&self) -> DateTime {
        DateTime {
            date: self.get_current_date(),
            time: Time {
                hour: 0,
                min: 0,
                sec: 0,
                millis: 0,
            },
        }
    }
}

static FIXED_TIME: FixedTime = FixedTime;

fn fixed_fs_options() -> FsOptions {
    FsOptions::new().time_provider(&FIXED_TIME)
}

/// A read/write/seek view over one region of a file.
struct Region {
    file: File,
    base: u64,
    length: u64,
    position: u64,
}

impl Region {
    fn new(file: File, base: u64, length: u64) -> Self {
        Self {
            file,
            base,
            length,
            position: 0,
        }
    }

    fn remaining(&self) -> u64 {
        self.length.saturating_sub(self.position)
    }
}

impl Read for Region {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let limit = self.remaining().min(buf.len() as u64) as usize;
        if limit == 0 {
            return Ok(0);
        }
        self.file.seek(SeekFrom::Start(self.base + self.position))?;
        let read = self.file.read(&mut buf[..limit])?;
        self.position += read as u64;
        Ok(read)
    }
}

impl Write for Region {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let limit = self.remaining().min(buf.len() as u64) as usize;
        if limit == 0 && !buf.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "write past the end of the region",
            ));
        }
        self.file.seek(SeekFrom::Start(self.base + self.position))?;
        let written = self.file.write(&buf[..limit])?;
        self.position += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for Region {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = match from {
            SeekFrom::Start(offset) => offset as i128,
            SeekFrom::End(offset) => self.length as i128 + i128::from(offset),
            SeekFrom::Current(offset) => self.position as i128 + i128::from(offset),
        };
        self.position = target.clamp(0, self.length as i128) as u64;
        Ok(self.position)
    }
}
