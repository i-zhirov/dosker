//! Lock file resolution and the host engine resolver.

use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use dosker_core::{AgentPin, BasePin, EnginePin, LOCK_FILE_NAME, LockFile};
use dosker_engine_qemu::{QEMU_ENV, QemuBinary, engine_source};
use dosker_platform::fs::write_atomic;

use crate::error::BuildError;

/// A resolved engine binary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedEngine {
    /// The engine name.
    pub name: String,
    /// The reported version.
    pub version: String,
    /// The source that supplied the binary.
    pub source: String,
}

/// Resolves an engine name to a usable binary.
pub trait EngineResolver {
    /// Resolves one engine.
    fn resolve(&self, engine: &str) -> Result<ResolvedEngine, BuildError>;
}

/// Resolves engines from the host.
pub struct HostEngineResolver {
    data_root: PathBuf,
}

impl HostEngineResolver {
    /// Creates a resolver over a data root.
    pub fn new(data_root: PathBuf) -> Self {
        Self { data_root }
    }
}

impl EngineResolver for HostEngineResolver {
    fn resolve(&self, engine: &str) -> Result<ResolvedEngine, BuildError> {
        match engine {
            "qemu" => {
                let binary = QemuBinary::discover().map_err(|error| BuildError::Engine {
                    engine: engine.to_owned(),
                    reason: error.to_string(),
                })?;
                let from_env = env::var_os(QEMU_ENV).is_some_and(|value| !value.is_empty());
                let source = engine_source(&binary.path, &self.data_root, from_env);

                Ok(ResolvedEngine {
                    name: engine.to_owned(),
                    version: binary.version_string(),
                    source: source.as_str().to_owned(),
                })
            }
            other => Err(BuildError::Engine {
                engine: other.to_owned(),
                reason: "unsupported engine".to_owned(),
            }),
        }
    }
}

/// Resolves the lock pins for one build.
///
/// A missing lock file starts from an empty lock. An existing pin is compared
/// when its identity matches; a mismatch fails unless `update` is set. The
/// returned lock records the current base, the selected engine, and the agent
/// version when the base reports one.
pub fn resolve_lock(
    context_dir: &Path,
    base: BasePin,
    agent_version: Option<&str>,
    engine: &ResolvedEngine,
    update: bool,
) -> Result<LockFile, BuildError> {
    let existing = read_lock(context_dir)?;
    check_base(&existing, &base, update)?;
    check_engine_pin(&existing, engine, update)?;
    if let Some(version) = agent_version {
        check_agent(&existing, version, update)?;
    }

    let mut lock = LockFile::new();
    lock.base.push(base);
    lock.engines = existing.engines.clone();
    record_engine_pin(&mut lock, engine);
    lock.agent = agent_version.map(|version| AgentPin {
        version: version.to_owned(),
    });

    Ok(lock)
}

/// Writes the lock file atomically with a trailing newline.
pub fn write_lock(context_dir: &Path, lock: &LockFile) -> Result<(), BuildError> {
    let mut text = lock.to_toml()?;
    if !text.ends_with('\n') {
        text.push('\n');
    }

    write_atomic(&context_dir.join(LOCK_FILE_NAME), text.as_bytes())?;
    Ok(())
}

fn read_lock(context_dir: &Path) -> Result<LockFile, BuildError> {
    let path = context_dir.join(LOCK_FILE_NAME);
    match fs::read_to_string(&path) {
        Ok(text) => Ok(LockFile::from_toml(&text)?),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(LockFile::new()),
        Err(source) => Err(BuildError::Io { path, source }),
    }
}

fn check_base(existing: &LockFile, base: &BasePin, update: bool) -> Result<(), BuildError> {
    let Some(pinned) = existing
        .base
        .iter()
        .find(|pin| pin.name == base.name && pin.tag == base.tag)
    else {
        return Ok(());
    };

    if update || pinned.digest == base.digest || pinned.tag.is_empty() {
        return Ok(());
    }

    Err(BuildError::LockMismatch {
        message: format!(
            "base image {}:{} digest mismatch: expected {}, found {}",
            base.name, base.tag, pinned.digest, base.digest
        ),
    })
}

/// Checks one resolved engine against the lock, unless `update` refreshes it.
pub fn check_engine_pin(
    existing: &LockFile,
    engine: &ResolvedEngine,
    update: bool,
) -> Result<(), BuildError> {
    if update {
        return Ok(());
    }

    let Some(pinned) = existing.engines.get(&engine.name) else {
        return Ok(());
    };

    if pinned.version == engine.version {
        return Ok(());
    }

    Err(BuildError::LockMismatch {
        message: format!(
            "engine {} version mismatch: expected {}, found {}",
            engine.name, pinned.version, engine.version
        ),
    })
}

/// Records one resolved engine in the lock.
pub fn record_engine_pin(lock: &mut LockFile, engine: &ResolvedEngine) {
    lock.engines.insert(
        engine.name.clone(),
        EnginePin {
            version: engine.version.clone(),
            source: engine.source.clone(),
        },
    );
}

fn check_agent(existing: &LockFile, version: &str, update: bool) -> Result<(), BuildError> {
    if update {
        return Ok(());
    }

    let Some(pinned) = &existing.agent else {
        return Ok(());
    };

    if pinned.version == version {
        return Ok(());
    }

    Err(BuildError::LockMismatch {
        message: format!(
            "agent version mismatch: expected {}, found {}",
            pinned.version, version
        ),
    })
}
