//! `dosker doctor`: check the host data root and the installed engines.

use std::fs;
use std::path::Path;

use dosker_engine::{EngineError, IMPLEMENTED_ENGINES};

use crate::adapter;
use crate::cli::{DoctorArgs, EXIT_ERROR, EXIT_OK};
use crate::context::Context;
use crate::error::CliError;

const DEVELOPMENT_MODE_WARNING: &str = "development mode: --bind uses QEMU vvfat (beta) and can corrupt host files. Use it for development only, never in CI.";
const SECURITY_STATEMENT: &str = "security: DOSker does not sandbox untrusted images; QEMU TCG is not a security boundary; use an outer VM for untrusted input.";

pub fn execute(_args: DoctorArgs, context: &mut Context) -> Result<u8, CliError> {
    let root = context.paths.root().to_path_buf();
    let writable = probe_root(&root);

    context
        .console
        .line(&format!("data root: {}", root.display()))?;
    context.console.line(&format!(
        "data root writable: {}",
        if writable { "yes" } else { "no" }
    ))?;

    let mut found = false;
    for provider in IMPLEMENTED_ENGINES {
        match adapter::discover(provider.name) {
            Ok(engine) => {
                found = true;
                let capability_names = engine.capabilities.names().collect::<Vec<_>>().join(",");
                context.console.line(&format!(
                    "{}: {} ({}) [{}]",
                    engine.name,
                    engine.version,
                    engine.path.display(),
                    capability_names
                ))?;
            }
            Err(error @ EngineError::NotFound(_)) => {
                context
                    .console
                    .line(&format!("{}: not found ({error})", provider.name))?;
            }
            Err(error) => {
                context
                    .console
                    .line(&format!("{}: unavailable ({error})", provider.name))?;
            }
        }
    }

    context.console.line(DEVELOPMENT_MODE_WARNING)?;
    context.console.line(SECURITY_STATEMENT)?;

    if writable && found {
        Ok(EXIT_OK)
    } else {
        Ok(EXIT_ERROR)
    }
}

fn probe_root(root: &Path) -> bool {
    let probe = root.join(".doctor.tmp");
    let result = (|| -> std::io::Result<()> {
        fs::create_dir_all(root)?;
        fs::write(&probe, b"")?;
        fs::remove_file(&probe)?;
        Ok(())
    })();

    result.is_ok()
}
