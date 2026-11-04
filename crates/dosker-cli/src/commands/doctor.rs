//! `dosker doctor`: check the host data root and the QEMU installation.

use std::fs;
use std::path::Path;

use dosker_engine::EngineError;
use dosker_engine_qemu::{QemuBinary, capabilities};

use crate::cli::{DoctorArgs, EXIT_ERROR, EXIT_OK};
use crate::context::Context;
use crate::error::CliError;

const SECURITY_STATEMENT: &str =
    "security: DOSker does not sandbox untrusted images; use an outer VM for untrusted input.";

pub fn execute(_args: DoctorArgs, context: &mut Context) -> Result<u8, CliError> {
    let root = context.paths.root().to_path_buf();
    let writable = probe_root(&root);
    let capability_names = capabilities().names().collect::<Vec<_>>().join(",");

    context
        .console
        .line(&format!("data root: {}", root.display()))?;
    context.console.line(&format!(
        "data root writable: {}",
        if writable { "yes" } else { "no" }
    ))?;

    let found = match QemuBinary::discover() {
        Ok(binary) => {
            context.console.line(&format!(
                "qemu: {} ({})",
                binary.version_string(),
                binary.path.display()
            ))?;
            true
        }
        Err(error @ EngineError::NotFound(_)) => {
            context
                .console
                .line(&format!("qemu: not found ({error})"))?;
            false
        }
        Err(error) => {
            context
                .console
                .line(&format!("qemu: unavailable ({error})"))?;
            false
        }
    };

    context
        .console
        .line(&format!("capabilities: {capability_names}"))?;
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
