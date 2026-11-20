//! DOSBox-X configuration file generation.

use std::fmt::Write as _;
use std::path::PathBuf;

use dosker_engine::{Capability, EngineError, EngineSpec, EngineVolumeKind};

use crate::adapter::capabilities;
use crate::endpoints::DosboxXEndpoints;

/// The DOSBox-X configuration file name.
pub const CONFIG_FILE_NAME: &str = "dosbox-x.conf";

/// Renders the DOSBox-X configuration for one spec.
///
/// The output is deterministic: the fixed sections come first in declaration
/// order, then the raw `dosbox-x` engine-config lines, then `[autoexec]`.
pub fn render_config(
    spec: &EngineSpec,
    endpoints: &DosboxXEndpoints,
) -> Result<String, EngineError> {
    reject_unsupported_volumes(spec)?;

    let mut config = String::new();

    push_header(&mut config, "[dosbox]");
    config.push_str("machine = svga_s3\n");
    config.push_str("memsize = 16\n");
    if capabilities().contains(Capability::DebugNative) {
        let port = debug_port(endpoints)?;
        writeln!(config, "mcp_server = {port}").expect("write to a string");
    }

    push_header(&mut config, "[cpu]");
    config.push_str("cputype = 486\n");

    push_header(&mut config, "[sblaster]");
    config.push_str("sbtype = sb16\n");

    push_header(&mut config, "[ide, primary]");
    config.push_str("enable = true\n");

    render_network(spec, &mut config)?;

    push_header(&mut config, "[serial]");
    render_serial(spec, endpoints, &mut config)?;

    let raw_lines = spec.engine_config.get("dosbox-x");
    let has_autoexec_header =
        raw_lines.is_some_and(|lines| lines.iter().any(|line| line.trim() == "[autoexec]"));
    if let Some(lines) = raw_lines {
        if !lines.is_empty() {
            config.push('\n');
        }
        for line in lines {
            config.push_str(line);
            config.push('\n');
        }
    }

    if !has_autoexec_header {
        config.push('\n');
        config.push_str("[autoexec]\n");
    }

    writeln!(config, "IMGMOUNT C \"{}\" -t hdd", spec.disk_path.display())
        .expect("write to a string");
    config.push_str("BOOT C:\n");

    Ok(config)
}

/// Writes the DOSBox-X configuration for one spec into its container
/// directory and returns the path.
pub fn write_config(
    spec: &EngineSpec,
    endpoints: &DosboxXEndpoints,
) -> Result<PathBuf, EngineError> {
    std::fs::create_dir_all(&spec.log_dir).map_err(EngineError::Io)?;
    let path = spec.log_dir.join(CONFIG_FILE_NAME);
    let text = render_config(spec, endpoints)?;
    std::fs::write(&path, text).map_err(EngineError::Io)?;

    Ok(path)
}

fn push_header(config: &mut String, header: &str) {
    if !config.is_empty() {
        config.push('\n');
    }
    config.push_str(header);
    config.push('\n');
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

fn render_network(spec: &EngineSpec, config: &mut String) -> Result<(), EngineError> {
    match spec.net.as_str() {
        "none" => Ok(()),
        "ne2000" => {
            if !capabilities().contains(Capability::Nat) {
                return Err(EngineError::Unsupported("network address translation"));
            }

            push_header(config, "[ne2000]");
            config.push_str("ne2000 = true\n");
            config.push_str("nicbase = 300\n");
            config.push_str("nicirq = 9\n");
            config.push_str("backend = slirp\n");

            push_header(config, "[ethernet, slirp]");

            Ok(())
        }
        other => Err(EngineError::Setup(format!(
            "unknown network device {other}"
        ))),
    }
}

fn render_serial(
    spec: &EngineSpec,
    endpoints: &DosboxXEndpoints,
    config: &mut String,
) -> Result<(), EngineError> {
    let agent_port = if spec.booter {
        None
    } else {
        Some(agent_port(endpoints)?)
    };

    if spec.control_port == 1
        && let Some(port) = agent_port
    {
        writeln!(
            config,
            "serial1 = nullmodem port:{port} sock:0 transparent:1"
        )
        .expect("write to a string");
        return Ok(());
    }

    config.push_str("serial1 = dummy\n");
    if let Some(port) = agent_port {
        writeln!(
            config,
            "serial{} = nullmodem port:{port} sock:0 transparent:1",
            spec.control_port
        )
        .expect("write to a string");
    }

    Ok(())
}

fn agent_port(endpoints: &DosboxXEndpoints) -> Result<u16, EngineError> {
    let kind = endpoints
        .agent_serial
        .as_ref()
        .ok_or_else(|| EngineError::Setup("the spec has no agent serial endpoint".to_owned()))?;

    match kind {
        dosker_engine::EngineEndpointKind::Tcp { port, .. } => Ok(*port),
        dosker_engine::EngineEndpointKind::UnixSocket(path) => Err(EngineError::Setup(format!(
            "the agent serial endpoint is not TCP: {}",
            path.display()
        ))),
    }
}

fn debug_port(endpoints: &DosboxXEndpoints) -> Result<u16, EngineError> {
    let kind = endpoints
        .debug
        .as_ref()
        .ok_or_else(|| EngineError::Setup("the spec has no debug endpoint".to_owned()))?;

    match kind {
        dosker_engine::EngineEndpointKind::Tcp { port, .. } => Ok(*port),
        dosker_engine::EngineEndpointKind::UnixSocket(path) => Err(EngineError::Setup(format!(
            "the debug endpoint is not TCP: {}",
            path.display()
        ))),
    }
}
