//! Bochs configuration file generation.

use std::fmt::Write as _;
use std::path::PathBuf;

use dosker_engine::{DisplayMode, EngineEndpointKind, EngineError, EngineSpec, EngineVolumeKind};

use crate::endpoints::BochsEndpoints;
use crate::geometry::DiskGeometry;

/// The Bochs configuration file name.
pub const CONFIG_FILE_NAME: &str = "bochsrc";

/// The headless display library Bochs runs with.
pub const HEADLESS_DISPLAY: &str = "nogui";

/// The CPU model Bochs emulates.
pub const CPU_MODEL: &str = "i486dx4";

/// The amount of guest memory in mebibytes.
pub const MEMORY_MB: u32 = 16;

/// The Bochs log file name.
const LOG_FILE_NAME: &str = "bochs.log";

/// Renders the Bochs configuration for one spec.
///
/// The output is deterministic: the fixed keys come first in declaration
/// order, then the raw `bochs` engine-config lines.
pub fn render_config(spec: &EngineSpec, endpoints: &BochsEndpoints) -> Result<String, EngineError> {
    reject_unsupported_volumes(spec)?;
    reject_unsupported_network(spec)?;
    reject_unsupported_display(spec)?;
    reject_unsupported_control_port(spec)?;

    let geometry = DiskGeometry::from_size(disk_size(spec)?)?;

    let mut config = String::new();
    config.push_str("romimage: file=$BXSHARE/BIOS-bochs-latest\n");
    config.push_str("vgaromimage: file=$BXSHARE/VGABIOS-lgpl-latest.bin\n");
    writeln!(config, "cpu: model={CPU_MODEL}").expect("write to a string");
    writeln!(config, "megs: {MEMORY_MB}").expect("write to a string");
    writeln!(
        config,
        "ata0-master: type=disk, path=\"{}\", mode=flat, cylinders={}, heads={}, spt={}",
        spec.disk_path.display(),
        geometry.cylinders,
        geometry.heads,
        geometry.sectors
    )
    .expect("write to a string");
    config.push_str("boot: disk\n");
    writeln!(config, "display_library: {HEADLESS_DISPLAY}").expect("write to a string");
    render_serial(spec, endpoints, &mut config)?;
    writeln!(
        config,
        "log: \"{}\"",
        spec.log_dir.join(LOG_FILE_NAME).display()
    )
    .expect("write to a string");
    config.push_str("panic: action=fatal\n");
    config.push_str("error: action=report\n");
    config.push_str("info: action=report\n");
    config.push_str("debug: action=ignore\n");

    if let Some(lines) = spec.engine_config.get("bochs") {
        for line in lines {
            config.push_str(line);
            config.push('\n');
        }
    }

    Ok(config)
}

/// Writes the Bochs configuration for one spec into its container directory
/// and returns the path.
pub fn write_config(spec: &EngineSpec, endpoints: &BochsEndpoints) -> Result<PathBuf, EngineError> {
    std::fs::create_dir_all(&spec.log_dir).map_err(EngineError::Io)?;
    let path = spec.log_dir.join(CONFIG_FILE_NAME);
    let text = render_config(spec, endpoints)?;
    std::fs::write(&path, text).map_err(EngineError::Io)?;

    Ok(path)
}

fn reject_unsupported_volumes(spec: &EngineSpec) -> Result<(), EngineError> {
    if spec
        .volumes
        .iter()
        .any(|volume| volume.kind == EngineVolumeKind::HostDir)
    {
        return Err(EngineError::Unsupported("host directory"));
    }

    if spec
        .volumes
        .iter()
        .any(|volume| volume.kind != EngineVolumeKind::HostDir)
    {
        return Err(EngineError::Unsupported("volume image"));
    }

    Ok(())
}

fn reject_unsupported_network(spec: &EngineSpec) -> Result<(), EngineError> {
    match spec.net.as_str() {
        "none" => Ok(()),
        "ne2000" => Err(EngineError::Unsupported("network address translation")),
        other => Err(EngineError::Setup(format!(
            "unknown network device {other}"
        ))),
    }
}

fn reject_unsupported_display(spec: &EngineSpec) -> Result<(), EngineError> {
    match spec.display {
        DisplayMode::None => Ok(()),
        DisplayMode::Window => Err(EngineError::Unsupported("window display")),
        DisplayMode::Vnc => Err(EngineError::Unsupported("vnc")),
    }
}

fn reject_unsupported_control_port(spec: &EngineSpec) -> Result<(), EngineError> {
    if !(1..=2).contains(&spec.control_port) {
        return Err(EngineError::Setup(format!(
            "the control port {} is outside the two Bochs serial ports",
            spec.control_port
        )));
    }

    Ok(())
}

fn disk_size(spec: &EngineSpec) -> Result<u64, EngineError> {
    std::fs::metadata(&spec.disk_path)
        .map(|metadata| metadata.len())
        .map_err(EngineError::Io)
}

fn render_serial(
    spec: &EngineSpec,
    endpoints: &BochsEndpoints,
    config: &mut String,
) -> Result<(), EngineError> {
    let agent_port = if spec.booter {
        None
    } else {
        Some(agent_port(endpoints)?)
    };

    for number in 1..=2u8 {
        match agent_port {
            Some(port) if number == spec.control_port => {
                writeln!(
                    config,
                    "com{number}: enabled=1, mode=socket-server, dev=localhost:{port}"
                )
                .expect("write to a string");
            }
            _ => {
                writeln!(config, "com{number}: enabled=1, mode=null").expect("write to a string");
            }
        }
    }

    Ok(())
}

fn agent_port(endpoints: &BochsEndpoints) -> Result<u16, EngineError> {
    let kind = endpoints
        .agent_serial
        .as_ref()
        .ok_or_else(|| EngineError::Setup("the spec has no agent serial endpoint".to_owned()))?;

    match kind {
        EngineEndpointKind::Tcp { port, .. } => Ok(*port),
        EngineEndpointKind::UnixSocket(path) => Err(EngineError::Setup(format!(
            "the agent serial endpoint is not TCP: {}",
            path.display()
        ))),
    }
}
