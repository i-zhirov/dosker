use std::env;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use dosker_platform::PlatformError;
use dosker_platform::data_root::data_root;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct EnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl EnvGuard {
    fn new(names: &[&'static str]) -> Self {
        let saved = names
            .iter()
            .map(|name| (*name, env::var_os(name)))
            .collect();
        Self { saved }
    }

    fn set(&self, name: &'static str, value: impl AsRef<OsStr>) {
        unsafe { env::set_var(name, value) };
    }

    fn remove(&self, name: &'static str) {
        unsafe { env::remove_var(name) };
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (name, value) in &self.saved {
            unsafe {
                match value {
                    Some(value) => env::set_var(name, value),
                    None => env::remove_var(name),
                }
            }
        }
    }
}

fn managed_variables() -> [&'static str; 4] {
    ["DOSKER_ROOT", "HOME", "XDG_DATA_HOME", "LOCALAPPDATA"]
}

#[test]
fn dosker_root_overrides_every_host() {
    let _lock = lock_env();
    let env = EnvGuard::new(&managed_variables());
    let root = tempfile::tempdir().unwrap();

    env.set("DOSKER_ROOT", root.path());

    assert_eq!(data_root().unwrap(), root.path());
}

#[test]
fn dosker_root_relative_is_made_absolute() {
    let _lock = lock_env();
    let env = EnvGuard::new(&managed_variables());
    let relative = PathBuf::from("dosker-relative-root-test");

    env.set("DOSKER_ROOT", &relative);

    let expected = env::current_dir().unwrap().join(&relative);
    assert_eq!(data_root().unwrap(), expected);
}

#[test]
fn dosker_root_empty_falls_back() {
    let _lock = lock_env();
    let env = EnvGuard::new(&managed_variables());
    let home = tempfile::tempdir().unwrap();

    env.set("DOSKER_ROOT", "");
    env.set("HOME", home.path());

    #[cfg(target_os = "linux")]
    {
        env.remove("XDG_DATA_HOME");
        assert_eq!(
            data_root().unwrap(),
            home.path().join(".local").join("share").join("dosker")
        );
    }

    #[cfg(target_os = "macos")]
    assert_eq!(
        data_root().unwrap(),
        home.path()
            .join("Library")
            .join("Application Support")
            .join("dosker")
    );

    #[cfg(windows)]
    {
        let local = tempfile::tempdir().unwrap();
        env.set("LOCALAPPDATA", local.path());
        assert_eq!(data_root().unwrap(), local.path().join("dosker"));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_uses_xdg_data_home() {
    let _lock = lock_env();
    let env = EnvGuard::new(&managed_variables());
    let xdg = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();

    env.remove("DOSKER_ROOT");
    env.set("XDG_DATA_HOME", xdg.path());
    env.set("HOME", home.path());

    assert_eq!(data_root().unwrap(), xdg.path().join("dosker"));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_falls_back_to_home_local_share() {
    let _lock = lock_env();
    let env = EnvGuard::new(&managed_variables());
    let home = tempfile::tempdir().unwrap();

    env.remove("DOSKER_ROOT");
    env.set("HOME", home.path());

    for xdg in [None, Some("")] {
        match xdg {
            None => env.remove("XDG_DATA_HOME"),
            Some(value) => env.set("XDG_DATA_HOME", value),
        }

        assert_eq!(
            data_root().unwrap(),
            home.path().join(".local").join("share").join("dosker")
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_uses_application_support() {
    let _lock = lock_env();
    let env = EnvGuard::new(&managed_variables());
    let home = tempfile::tempdir().unwrap();

    env.remove("DOSKER_ROOT");
    env.set("HOME", home.path());

    assert_eq!(
        data_root().unwrap(),
        home.path()
            .join("Library")
            .join("Application Support")
            .join("dosker")
    );
}

#[cfg(windows)]
#[test]
fn windows_uses_localappdata() {
    let _lock = lock_env();
    let env = EnvGuard::new(&managed_variables());
    let local = tempfile::tempdir().unwrap();

    env.remove("DOSKER_ROOT");
    env.set("LOCALAPPDATA", local.path());

    assert_eq!(data_root().unwrap(), local.path().join("dosker"));
}

#[cfg(unix)]
#[test]
fn missing_home_is_an_error() {
    let _lock = lock_env();
    let env = EnvGuard::new(&managed_variables());

    env.remove("DOSKER_ROOT");
    env.remove("HOME");

    #[cfg(target_os = "linux")]
    env.remove("XDG_DATA_HOME");

    match data_root() {
        Err(PlatformError::MissingEnvironment(name)) => assert_eq!(name, "HOME"),
        other => panic!("expected a missing environment error, got {other:?}"),
    }
}
