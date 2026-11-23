mod support;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use dosker_image::freedos::{
    AGENT_VERSION, BOOT_SECTOR_TEMPLATE, BaseImage, BaseInputs, FreedosBinaries, FreedosPins,
    FreedosVersion, MBR_TEMPLATE, build_base_image, ensure_freedos_base,
};
use dosker_image::{
    ANNOTATION_RAW_DISK_DIGEST, BlobRef, DosFlavour, ImageError, ImageRef, ImageReference, Layer,
    OciStore, fixed_timestamp, freedos_base_licenses, sha256_digest,
};
use support::{make_fat16_image, write_fat_file};

const MIB: u64 = 1024 * 1024;
const KERNEL: &[u8] = b"synthetic kernel bytes";
const COMMAND: &[u8] = b"synthetic command bytes";
const AGENT: &[u8] = b"synthetic agent bytes";
const SOURCE_ENV: &str = "DOSKER_FREEDOS_SOURCE";
const PARTITION_OFFSET: usize = 2048 * 512;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn restore_var(name: &str, previous: Option<std::ffi::OsString>) {
    // SAFETY: the module mutex serializes every environment mutation in this
    // test binary.
    unsafe {
        match previous {
            Some(value) => env::set_var(name, value),
            None => env::remove_var(name),
        }
    }
}

fn version() -> FreedosVersion {
    FreedosVersion::parse("1.3").unwrap()
}

fn reference(value: &str) -> ImageReference {
    ImageReference::parse(value).unwrap()
}

fn tag() -> ImageRef {
    ImageRef::Tag(reference("freedos:1.3"))
}

fn binaries() -> FreedosBinaries {
    FreedosBinaries {
        kernel: KERNEL.to_vec(),
        command: COMMAND.to_vec(),
        agent: AGENT.to_vec(),
        mbr: MBR_TEMPLATE.to_vec(),
        boot_sector: BOOT_SECTOR_TEMPLATE.to_vec(),
    }
}

fn built() -> BaseImage {
    build_base_image(&binaries(), &version()).unwrap()
}

fn source_image(dir: &Path) -> PathBuf {
    let path = dir.join("freedos.img");
    make_fat16_image(&path, MIB, 4 * MIB, 8 * MIB);
    write_fat_file(&path, MIB, "KERNEL.SYS", KERNEL);
    write_fat_file(&path, MIB, "COMMAND.COM", COMMAND);
    path
}

fn synthetic_inputs(dir: &Path) -> BaseInputs {
    let source = source_image(dir);
    let agent_executable = dir.join("DOSKER.EXE");
    fs::write(&agent_executable, AGENT).unwrap();
    BaseInputs {
        source,
        agent_executable,
        pins: FreedosPins {
            kernel_sha256: sha256_digest(KERNEL),
            command_sha256: sha256_digest(COMMAND),
            agent_sha256: sha256_digest(AGENT),
        },
    }
}

fn commit_base(
    store: &OciStore,
    reference: &ImageReference,
    image: &BaseImage,
    config: dosker_image::ImageConfig,
) {
    let layer = Layer::from_delta(&image.tree.to_delta(), "freedos-base", None, None).unwrap();
    let descriptor = store.put_layer(&layer).unwrap();
    let raw = store.put_blob(&image.raw).unwrap();
    let mut annotations = BTreeMap::new();
    annotations.insert(ANNOTATION_RAW_DISK_DIGEST.to_owned(), raw.digest);
    store
        .commit_image(Some(reference), &config, vec![descriptor], annotations)
        .unwrap();
}

#[test]
fn store_commits_a_freedos_config() {
    let dir = tempfile::tempdir().unwrap();
    let image = built();
    let store = OciStore::new(dir.path().to_path_buf());
    let reference = reference("freedos:1.3");

    image.store(&store, &reference).unwrap();

    let config = store.load_config(&tag()).unwrap();
    assert_eq!(config.dos_flavour, DosFlavour::Freedos);
    assert!(!config.booter);
    assert_eq!(config.agent_version.as_deref(), Some("1.0.0"));
    assert_eq!(config.boot, image.boot);
    assert_eq!(config.machine.disk_interface, "ide");
    assert_eq!(config.workdir, r"C:\");
    assert_eq!(config.created_by, "freedos-base");
    assert_eq!(config.history.len(), 1);
    assert_eq!(config.history[0].created_by, "freedos-base");
    assert_eq!(config.history[0].created_at, fixed_timestamp());
    assert!(!config.history[0].empty_layer);
    assert_eq!(config.licenses, freedos_base_licenses());
    for entry in &config.licenses {
        entry.validate().unwrap();
    }
}

#[test]
fn freedos_base_entries_are_valid_and_redistributable() {
    let entries = freedos_base_licenses();
    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, vec!["FreeDOS", "DOSKER"]);
    for entry in &entries {
        entry.validate().unwrap();
    }
    assert_eq!(
        dosker_core::license::license_summary(&entries),
        Some(dosker_core::license::LicenseSummary::Redistributable)
    );
}

#[test]
fn store_round_trips_the_tree_and_raw_disk() {
    let dir = tempfile::tempdir().unwrap();
    let image = built();
    let store = OciStore::new(dir.path().to_path_buf());
    let reference = reference("freedos:1.3");

    image.store(&store, &reference).unwrap();

    assert_eq!(store.load_tree(&tag()).unwrap(), image.tree);

    let destination = dir.path().join("copy.img");
    store.copy_disk_to(&tag(), &destination).unwrap();
    assert_eq!(fs::read(&destination).unwrap(), image.raw);

    let manifest = store.load_manifest(&tag()).unwrap();
    assert_eq!(
        manifest.raw_disk_digest(),
        Some(sha256_digest(&image.raw).as_str())
    );

    let layer = store.load_layer(&manifest.layers[0].digest).unwrap();
    assert_eq!(layer.meta.parent, None);
}

#[test]
fn store_is_repeatable() {
    let dir = tempfile::tempdir().unwrap();
    let image = built();
    let store = OciStore::new(dir.path().to_path_buf());

    let first = image.store(&store, &reference("freedos:1.3")).unwrap();
    let second = image.store(&store, &reference("freedos:other")).unwrap();

    assert_eq!(first.manifest_digest, second.manifest_digest);
    assert_eq!(first.config_digest, second.config_digest);
    assert_eq!(first.layer_digests, second.layer_digests);
    assert_eq!(
        store.load_manifest(&tag()).unwrap().raw_disk_digest(),
        store
            .load_manifest(&ImageRef::Tag(reference("freedos:other")))
            .unwrap()
            .raw_disk_digest()
    );
}

#[test]
fn ensure_builds_and_tags_a_missing_base() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = synthetic_inputs(dir.path());
    let store = OciStore::new(dir.path().to_path_buf());

    let image = ensure_freedos_base(&store, Some(&inputs), &version()).unwrap();

    assert_eq!(image.name, "freedos");
    assert_eq!(image.tag, "1.3");
    assert_eq!(image.agent_version.as_deref(), Some(AGENT_VERSION));
    assert!(store.load_manifest(&tag()).is_ok());
}

#[test]
fn ensure_reuses_a_valid_base() {
    let _guard = lock_env();
    let previous = env::var_os(SOURCE_ENV);
    // SAFETY: the module mutex serializes every environment mutation here.
    unsafe { env::remove_var(SOURCE_ENV) };

    let dir = tempfile::tempdir().unwrap();
    let inputs = synthetic_inputs(dir.path());
    let store = OciStore::new(dir.path().to_path_buf());
    let first = ensure_freedos_base(&store, Some(&inputs), &version()).unwrap();
    let manifest_digest = first.manifest_digest.clone();

    let result = ensure_freedos_base(&store, None, &version());

    restore_var(SOURCE_ENV, previous);
    let second = result.unwrap();
    assert_eq!(second.manifest_digest, manifest_digest);
    assert_eq!(second.agent_version.as_deref(), Some(AGENT_VERSION));
}

#[test]
fn ensure_reports_missing_inputs_when_no_base() {
    let _guard = lock_env();
    let previous = env::var_os(SOURCE_ENV);
    // SAFETY: the module mutex serializes every environment mutation here.
    unsafe { env::remove_var(SOURCE_ENV) };

    let dir = tempfile::tempdir().unwrap();
    let store = OciStore::new(dir.path().to_path_buf());

    let result = ensure_freedos_base(&store, None, &version());

    restore_var(SOURCE_ENV, previous);
    assert!(matches!(result, Err(ImageError::FreedosSourceNotSet)));
}

#[test]
fn ensure_rebuilds_a_stale_base() {
    let dir = tempfile::tempdir().unwrap();
    let image = built();
    let store = OciStore::new(dir.path().to_path_buf());
    let mut config = image.base_config();
    config.agent_version = Some("0.0.0".to_owned());
    commit_base(&store, &reference("freedos:1.3"), &image, config);

    let inputs = synthetic_inputs(dir.path());
    ensure_freedos_base(&store, Some(&inputs), &version()).unwrap();

    let stored = store.load_config(&tag()).unwrap();
    assert_eq!(stored.agent_version.as_deref(), Some(AGENT_VERSION));
}

#[test]
fn ensure_stale_base_requires_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let image = built();
    let store = OciStore::new(dir.path().to_path_buf());
    let mut config = image.base_config();
    config.agent_version = Some("0.0.0".to_owned());
    commit_base(&store, &reference("freedos:1.3"), &image, config);

    let _guard = lock_env();
    let previous = env::var_os(SOURCE_ENV);
    // SAFETY: the module mutex serializes every environment mutation here.
    unsafe { env::remove_var(SOURCE_ENV) };

    let result = ensure_freedos_base(&store, None, &version());

    restore_var(SOURCE_ENV, previous);
    assert!(matches!(result, Err(ImageError::FreedosSourceNotSet)));
}

#[test]
fn ensure_rebuilds_a_base_with_a_changed_config() {
    let dir = tempfile::tempdir().unwrap();
    let image = built();
    let store = OciStore::new(dir.path().to_path_buf());
    let mut config = image.base_config();
    config.workdir = r"C:\APP".to_owned();
    commit_base(&store, &reference("freedos:1.3"), &image, config);

    let inputs = synthetic_inputs(dir.path());
    ensure_freedos_base(&store, Some(&inputs), &version()).unwrap();

    assert_eq!(store.load_config(&tag()).unwrap().workdir, r"C:\");
}

#[test]
fn ensure_rebuilds_a_base_with_a_changed_boot_sector() {
    let dir = tempfile::tempdir().unwrap();
    let image = built();
    let store = OciStore::new(dir.path().to_path_buf());
    let mut config = image.base_config();
    let mut sector = image.raw[PARTITION_OFFSET..PARTITION_OFFSET + 512].to_vec();
    sector[0x3E] ^= 0xFF;
    config.boot.boot_sector = Some(BlobRef::from_bytes(&sector));
    commit_base(&store, &reference("freedos:1.3"), &image, config);

    let inputs = synthetic_inputs(dir.path());
    ensure_freedos_base(&store, Some(&inputs), &version()).unwrap();

    assert_eq!(store.load_config(&tag()).unwrap().boot, image.boot);
}
