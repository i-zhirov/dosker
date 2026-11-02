#![cfg(unix)]

mod support;

use std::ffi::OsString;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvGuard {
    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);

        // The environment tests hold ENV_LOCK while the guard is alive.
        unsafe {
            std::env::remove_var(key);
        }

        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe {
                std::env::set_var(self.key, value);
            },
            None => unsafe {
                std::env::remove_var(self.key);
            },
        }
    }
}

fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn integration_is_disabled_without_the_env() {
    let _lock = env_lock();
    let _guard = EnvGuard::remove(support::INTEGRATION_ENV);

    assert!(!support::integration_enabled());
}

#[test]
fn agent_image_requires_the_env() {
    let _lock = env_lock();
    let _guard = EnvGuard::remove(support::AGENT_IMAGE_ENV);

    let error = support::agent_image().expect_err("the agent image should be required");

    assert!(
        error.contains(support::AGENT_IMAGE_ENV),
        "error does not name {}: {error}",
        support::AGENT_IMAGE_ENV
    );
}

#[test]
fn qemu_binary_defaults_to_qemu_system_i386() {
    let _lock = env_lock();
    let _guard = EnvGuard::remove(support::QEMU_ENV);

    assert_eq!(support::qemu_binary(), support::DEFAULT_QEMU);
}

#[test]
fn boot_timeout_defaults_to_60000_ms() {
    let _lock = env_lock();
    let _guard = EnvGuard::remove(support::BOOT_TIMEOUT_ENV);

    assert_eq!(
        support::boot_timeout(),
        Duration::from_millis(support::DEFAULT_BOOT_TIMEOUT_MS)
    );
}

#[test]
fn qemu_command_uses_com1_none_and_com2_unix_socket() {
    let image = Path::new("/tmp/dosker-image.img");
    let socket = Path::new("/tmp/dosker-agent-1-2.sock");
    let arguments = support::qemu_arguments(image, socket);

    let serial_values: Vec<&str> = arguments
        .iter()
        .enumerate()
        .filter(|(_, argument)| argument.as_str() == "-serial")
        .map(|(index, _)| arguments[index + 1].as_str())
        .collect();

    assert_eq!(
        serial_values,
        vec![
            "none".to_owned(),
            format!("unix:{},server=on,wait=off", socket.display()),
        ]
    );
}

#[test]
fn qemu_command_uses_ide_raw_drive_and_headless_display() {
    let image = Path::new("/tmp/dosker-image.img");
    let socket = Path::new("/tmp/dosker-agent-1-2.sock");
    let arguments = support::qemu_arguments(image, socket);
    let joined = arguments.join(" ");

    assert!(
        joined.contains("format=raw,if=ide"),
        "drive is not a raw IDE drive: {joined}"
    );
    assert!(
        joined.contains("-display none"),
        "display is not headless: {joined}"
    );

    for option in ["cyls", "heads", "secs"] {
        assert!(
            !arguments.iter().any(|argument| argument.contains(option)),
            "drive contains {option}: {joined}"
        );
    }
}

#[test]
fn socket_path_is_short() {
    let path = support::socket_path();
    let length = path.as_os_str().len();

    assert!(
        length <= 100,
        "socket path is {length} bytes: {}",
        path.display()
    );
}
