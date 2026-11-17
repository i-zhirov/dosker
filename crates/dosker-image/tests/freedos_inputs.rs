mod support;

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use dosker_image::freedos::{
    BOOT_SECTOR_TEMPLATE, BaseInputs, FREEDOS_BOOT_SECTOR_SHA256, FREEDOS_MBR_SHA256, FreedosPins,
    FreedosVersion, MBR_TEMPLATE, default_agent_executable, load_freedos_binaries,
};
use dosker_image::{ImageError, sha256_digest};
use support::{make_fat16_image, write_fat_file};

const MIB: u64 = 1024 * 1024;
const KERNEL: &[u8] = b"synthetic kernel bytes";
const COMMAND: &[u8] = b"synthetic command bytes";
const AGENT: &[u8] = b"synthetic agent bytes";
const SOURCE_ENV: &str = "DOSKER_FREEDOS_SOURCE";
const AGENT_ENV: &str = "DOSKER_AGENT_EXE";

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn source_image(dir: &Path) -> PathBuf {
    let path = dir.join("freedos.img");
    make_fat16_image(&path, MIB, 4 * MIB, 8 * MIB);
    write_fat_file(&path, MIB, "KERNEL.SYS", KERNEL);
    write_fat_file(&path, MIB, "COMMAND.COM", COMMAND);
    path
}

fn agent_file(dir: &Path) -> PathBuf {
    let path = dir.join("DOSKER.EXE");
    fs::write(&path, AGENT).unwrap();
    path
}

fn inputs(
    source: PathBuf,
    agent_executable: PathBuf,
    kernel: &[u8],
    command: &[u8],
    agent: &[u8],
) -> BaseInputs {
    BaseInputs {
        source,
        agent_executable,
        pins: FreedosPins {
            kernel_sha256: sha256_digest(kernel),
            command_sha256: sha256_digest(command),
            agent_sha256: sha256_digest(agent),
        },
    }
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

#[test]
fn embedded_templates_match_their_pins() {
    assert_eq!(sha256_digest(MBR_TEMPLATE), FREEDOS_MBR_SHA256);
    assert_eq!(
        sha256_digest(BOOT_SECTOR_TEMPLATE),
        FREEDOS_BOOT_SECTOR_SHA256
    );
    assert_eq!(MBR_TEMPLATE.len(), 512);
    assert_eq!(BOOT_SECTOR_TEMPLATE.len(), 512);

    assert_eq!(&MBR_TEMPLATE[510..512], &[0x55, 0xAA]);
    let entry = &MBR_TEMPLATE[446..462];
    assert_eq!(entry[0], 0x80, "partition is not bootable");
    assert_eq!(entry[4], 0x06, "partition type is not FAT16");
    assert_eq!(
        u32::from_le_bytes(entry[8..12].try_into().unwrap()),
        2048,
        "partition does not start at LBA 2048"
    );
    assert_eq!(
        u32::from_le_bytes(entry[12..16].try_into().unwrap()),
        63_488,
        "partition does not have the pinned sector count"
    );
}

#[test]
fn load_reads_the_kernel_and_command_from_a_source_image() {
    let dir = tempfile::tempdir().unwrap();
    let path = source_image(dir.path());
    let inputs = inputs(path, agent_file(dir.path()), KERNEL, COMMAND, AGENT);

    let binaries = load_freedos_binaries(&inputs).unwrap();

    assert_eq!(binaries.kernel, KERNEL);
    assert_eq!(binaries.command, COMMAND);
    assert_eq!(binaries.agent, AGENT);
    assert_eq!(binaries.mbr, MBR_TEMPLATE);
    assert_eq!(binaries.boot_sector, BOOT_SECTOR_TEMPLATE);
}

#[test]
fn load_prefers_the_freedos_bin_directory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("freedos.img");
    make_fat16_image(&path, MIB, 4 * MIB, 8 * MIB);
    write_fat_file(&path, MIB, "KERNEL.SYS", KERNEL);
    write_fat_file(&path, MIB, "COMMAND.COM", b"root command");
    write_fat_file(&path, MIB, "FREEDOS/BIN/COMMAND.COM", b"nested command");
    let inputs = inputs(
        path,
        agent_file(dir.path()),
        KERNEL,
        b"nested command",
        AGENT,
    );

    let binaries = load_freedos_binaries(&inputs).unwrap();

    assert_eq!(binaries.command, b"nested command");
}

#[test]
fn load_reports_a_missing_source_file() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("missing.img");
    let inputs = inputs(
        source.clone(),
        agent_file(dir.path()),
        KERNEL,
        COMMAND,
        AGENT,
    );

    match load_freedos_binaries(&inputs) {
        Err(ImageError::FreedosSourceMissing { path }) => assert_eq!(path, source),
        other => panic!("expected FreedosSourceMissing, got {other:?}"),
    }
}

#[test]
fn load_reports_a_missing_kernel() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("freedos.img");
    make_fat16_image(&path, MIB, 4 * MIB, 8 * MIB);
    write_fat_file(&path, MIB, "COMMAND.COM", COMMAND);
    let inputs = inputs(path, agent_file(dir.path()), KERNEL, COMMAND, AGENT);

    match load_freedos_binaries(&inputs) {
        Err(ImageError::FreedosFileMissing { file, .. }) => assert_eq!(file, "KERNEL.SYS"),
        other => panic!("expected FreedosFileMissing, got {other:?}"),
    }
}

#[test]
fn load_reports_a_missing_command() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("freedos.img");
    make_fat16_image(&path, MIB, 4 * MIB, 8 * MIB);
    write_fat_file(&path, MIB, "KERNEL.SYS", KERNEL);
    let inputs = inputs(path, agent_file(dir.path()), KERNEL, COMMAND, AGENT);

    match load_freedos_binaries(&inputs) {
        Err(ImageError::FreedosFileMissing { file, .. }) => assert_eq!(file, "COMMAND.COM"),
        other => panic!("expected FreedosFileMissing, got {other:?}"),
    }
}

#[test]
fn load_rejects_a_tampered_kernel() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("freedos.img");
    make_fat16_image(&path, MIB, 4 * MIB, 8 * MIB);
    write_fat_file(&path, MIB, "KERNEL.SYS", b"tampered kernel");
    write_fat_file(&path, MIB, "COMMAND.COM", COMMAND);
    let inputs = inputs(
        path,
        agent_file(dir.path()),
        b"expected kernel",
        COMMAND,
        AGENT,
    );

    match load_freedos_binaries(&inputs) {
        Err(ImageError::FreedosPinMismatch {
            file,
            expected,
            actual,
        }) => {
            assert_eq!(file, "KERNEL.SYS");
            assert_eq!(expected, inputs.pins.kernel_sha256);
            assert_eq!(actual, sha256_digest(b"tampered kernel"));
            assert!(expected.starts_with("sha256:"));
            assert!(actual.starts_with("sha256:"));
        }
        other => panic!("expected FreedosPinMismatch, got {other:?}"),
    }
}

#[test]
fn load_rejects_a_tampered_agent() {
    let dir = tempfile::tempdir().unwrap();
    let path = source_image(dir.path());
    let inputs = inputs(
        path,
        agent_file(dir.path()),
        KERNEL,
        COMMAND,
        b"expected agent",
    );

    match load_freedos_binaries(&inputs) {
        Err(ImageError::FreedosPinMismatch {
            file,
            expected,
            actual,
        }) => {
            assert_eq!(file, "DOSKER.EXE");
            assert_eq!(expected, inputs.pins.agent_sha256);
            assert_eq!(actual, sha256_digest(AGENT));
            assert!(expected.starts_with("sha256:"));
            assert!(actual.starts_with("sha256:"));
        }
        other => panic!("expected FreedosPinMismatch, got {other:?}"),
    }
}

#[test]
fn load_keeps_a_corrupt_source_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("freedos.img");
    make_fat16_image(&path, MIB, 4 * MIB, 8 * MIB);
    let mut file = OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(MIB)).unwrap();
    file.write_all(&[0u8; 512]).unwrap();
    drop(file);
    let inputs = inputs(path, agent_file(dir.path()), KERNEL, COMMAND, AGENT);

    assert!(matches!(
        load_freedos_binaries(&inputs),
        Err(ImageError::Corrupt { .. })
    ));
}

#[test]
fn base_inputs_report_a_missing_source_variable() {
    let _guard = lock_env();
    let previous = env::var_os(SOURCE_ENV);
    // SAFETY: the module mutex serializes every environment mutation here.
    unsafe { env::remove_var(SOURCE_ENV) };

    let version = FreedosVersion::parse("1.3").unwrap();
    let result = BaseInputs::from_environment(&version);

    restore_var(SOURCE_ENV, previous);
    assert!(matches!(result, Err(ImageError::FreedosSourceNotSet)));
}

#[test]
fn base_inputs_read_the_source_and_agent_paths() {
    let _guard = lock_env();
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.img");
    let agent = dir.path().join("agent.exe");
    let previous_source = env::var_os(SOURCE_ENV);
    let previous_agent = env::var_os(AGENT_ENV);
    // SAFETY: the module mutex serializes every environment mutation here.
    unsafe {
        env::set_var(SOURCE_ENV, &source);
        env::set_var(AGENT_ENV, &agent);
    }

    let version = FreedosVersion::parse("1.3").unwrap();
    let result = BaseInputs::from_environment(&version);

    restore_var(SOURCE_ENV, previous_source);
    restore_var(AGENT_ENV, previous_agent);

    let inputs = result.unwrap();
    assert_eq!(inputs.source, source);
    assert_eq!(inputs.agent_executable, agent);
    assert_eq!(inputs.pins, FreedosPins::default());
}

#[test]
fn version_accepts_only_the_pinned_release() {
    assert_eq!(FreedosVersion::parse("1.3").unwrap().as_str(), "1.3");

    for version in ["1.2", "latest"] {
        match FreedosVersion::parse(version) {
            Err(ImageError::UnsupportedFreedosVersion {
                version: rejected,
                supported,
            }) => {
                assert_eq!(rejected, version);
                assert!(supported.contains("1.3"), "supported: {supported}");
            }
            other => panic!("expected UnsupportedFreedosVersion, got {other:?}"),
        }
        let message = FreedosVersion::parse(version).unwrap_err().to_string();
        assert!(message.contains("1.3"), "message: {message}");
    }
}

#[test]
fn default_agent_path_is_the_workspace_build_output() {
    let _guard = lock_env();
    let previous = env::var_os(AGENT_ENV);
    // SAFETY: the module mutex serializes every environment mutation here.
    unsafe { env::remove_var(AGENT_ENV) };

    let path = default_agent_executable();

    restore_var(AGENT_ENV, previous);
    assert!(
        path.ends_with("agent/build/DOSKER.EXE"),
        "path: {}",
        path.display()
    );
}
