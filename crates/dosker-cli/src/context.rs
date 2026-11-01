//! The runtime context a CLI command operates on.

use std::path::PathBuf;

use dosker_platform::console::Console;
use dosker_platform::data_root::data_root;
use dosker_runtime::{RuntimePaths, StateStore};

use crate::error::CliError;

/// The data root, state store, and console shared by a command.
pub struct Context {
    pub paths: RuntimePaths,
    pub store: StateStore,
    pub console: Console,
}

impl Context {
    /// Loads the context from the host data root.
    pub fn load() -> Result<Self, CliError> {
        let root = data_root()?;
        Ok(Self::with_console(root, Console::std()))
    }

    /// Builds a context over an explicit root and console.
    pub fn with_console(root: PathBuf, console: Console) -> Self {
        Self {
            paths: RuntimePaths::new(root.clone()),
            store: StateStore::new(root),
            console,
        }
    }
}
