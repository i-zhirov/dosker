//! Normalized instruction rendering and build config digests.

use std::collections::BTreeMap;

use dosker_core::{Instruction, InstructionKind, LicenseEntry};
use dosker_image::{
    DosPath, ImageConfig, MachineProfile, PortDeclaration, VolumeDeclaration, canonical_digest,
};
use serde::Serialize;

use crate::error::BuildError;

/// Capabilities sort in the adapter declaration order so that a normalized
/// `REQUIRES` list stays stable across engines.
const CAPABILITY_ORDER: [&str; 11] = [
    "boot",
    "serial",
    "stop",
    "headless",
    "host_dir_live",
    "debug_native",
    "gdbstub",
    "screenshot",
    "vnc",
    "nat",
    "audio",
];

/// Renders tokens with one space between them.
///
/// A token is quoted when it is empty or contains whitespace, a double quote,
/// or a backslash. Inside quotes, a double quote and a backslash are escaped
/// with a backslash.
pub fn render_tokens(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|token| quote_token(token))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Resolves a raw DOS path against the current workdir.
///
/// An absolute path replaces the workdir; a relative path joins it. Only the
/// `C:` drive is accepted, and a `..` component is rejected.
pub fn resolve_dos_path(workdir: &DosPath, raw: &str) -> Result<DosPath, BuildError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(path_error("a DOS path cannot be empty"));
    }

    let (rest, has_drive) = split_drive(raw)?;
    let absolute = has_drive || rest.starts_with(['\\', '/']);
    let body = rest.trim_start_matches(['\\', '/']);

    let mut components = Vec::new();
    for component in body.split(['\\', '/']) {
        if component.is_empty() {
            continue;
        }
        if component == ".." {
            return Err(path_error(format!(
                "path {raw} may not contain a '..' component"
            )));
        }
        components.push(component);
    }

    let joined = if absolute {
        components.join("/")
    } else if components.is_empty() {
        return Err(path_error(format!("path {raw} is empty")));
    } else if workdir.is_root() {
        components.join("/")
    } else {
        format!("{}/{}", workdir.as_str(), components.join("/"))
    };

    Ok(DosPath::parse_relative(&joined)?)
}

/// Resolves a `COPY` destination and returns whether it denotes a directory.
///
/// A trailing separator or a root form marks the destination as a directory.
pub fn resolve_copy_destination(
    workdir: &DosPath,
    raw: &str,
) -> Result<(DosPath, bool), BuildError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(path_error("a COPY destination cannot be empty"));
    }

    let is_directory = raw.ends_with(['\\', '/']) || is_root_form(raw);
    Ok((resolve_dos_path(workdir, raw)?, is_directory))
}

/// Returns true for `DOSKER` and every path below it.
pub fn is_reserved_path(path: &DosPath) -> bool {
    path.components()
        .next()
        .is_some_and(|component| component.eq_ignore_ascii_case("DOSKER"))
}

/// Renders a non-`COPY` instruction into its normalized form.
///
/// `COPY` is rendered by the copy planner because it depends on the context
/// and the staged tree.
pub fn normalize_instruction(
    instruction: &Instruction,
    workdir: &DosPath,
) -> Result<String, BuildError> {
    let normalized = match &instruction.kind {
        InstructionKind::From(source) => format!("FROM {}", render_tokens(&[source.text()])),
        InstructionKind::Copy { .. } => {
            return Err(BuildError::Instruction {
                line: instruction.line,
                instruction: "COPY".to_owned(),
                reason: "COPY is rendered by the copy planner".to_owned(),
            });
        }
        InstructionKind::Run { host, raw, .. } => {
            if *host {
                format!("RUN --host {raw}")
            } else {
                format!("RUN {raw}")
            }
        }
        InstructionKind::Env { key, value } => {
            format!("ENV {}", render_tokens(&[format!("{key}={value}")]))
        }
        InstructionKind::Workdir { path } => {
            let resolved = resolve_dos_path(workdir, path)?;
            format!("WORKDIR {}", render_tokens(&[resolved.to_dos_string()]))
        }
        InstructionKind::Cmd { raw, .. } => format!("CMD {raw}"),
        InstructionKind::Machine { settings } => {
            let mut tokens = vec!["MACHINE".to_owned()];
            tokens.extend(settings.iter().map(|(key, value)| format!("{key}={value}")));
            render_tokens(&tokens)
        }
        InstructionKind::Engine { name } => format!("ENGINE {}", name.as_str()),
        InstructionKind::EngineConfig { engine, raw } => {
            format!("ENGINE-CONFIG {} {raw}", engine.as_str())
        }
        InstructionKind::Requires { capabilities } => {
            format!("REQUIRES {}", sorted_capabilities(capabilities).join(","))
        }
        InstructionKind::Expose { port, protocol } => format!("EXPOSE {port}/{protocol}"),
        InstructionKind::Volume { drive } => format!("VOLUME {drive}"),
        InstructionKind::License(entry) => format!("LICENSE {}", render_license(entry)),
    };

    Ok(normalized.trim_end().to_owned())
}

fn render_license(entry: &LicenseEntry) -> String {
    let mut tokens = vec![
        format!("NAME={}", entry.name),
        format!("SPDX={}", entry.spdx),
        format!("STATUS={}", entry.status.as_str()),
    ];
    if let Some(holder) = &entry.holder {
        tokens.push(format!("HOLDER={holder}"));
    }
    if let Some(basis) = entry.basis {
        tokens.push(format!("BASIS={}", basis.as_str()));
    }
    if let Some(evidence) = &entry.evidence {
        tokens.push(format!("EVIDENCE={evidence}"));
    }
    if let Some(license_url) = &entry.license_url {
        tokens.push(format!("LICENSE_URL={license_url}"));
    }
    if !entry.covers.is_empty() {
        tokens.push(format!("COVERS={}", entry.covers.join(",")));
    }
    if let Some(source_url) = &entry.source_url {
        tokens.push(format!("SOURCE_URL={source_url}"));
    }
    if let Some(notice) = &entry.notice {
        tokens.push(format!("NOTICE={notice}"));
    }
    render_tokens(&tokens)
}

/// Returns the canonical digest of the effective build config fields.
///
/// Engine config lines, boot metadata, history, and the agent version are not
/// part of the build configuration.
pub fn build_config_digest(config: &ImageConfig) -> Result<String, BuildError> {
    #[derive(Serialize)]
    struct BuildConfig<'a> {
        env: &'a BTreeMap<String, String>,
        workdir: &'a str,
        machine: &'a MachineProfile,
        engine_pin: &'a Option<String>,
        requires: &'a [String],
        exposed_ports: &'a [PortDeclaration],
        volumes: &'a [VolumeDeclaration],
        cmd: &'a Option<String>,
    }

    let digest = canonical_digest(&BuildConfig {
        env: &config.env,
        workdir: &config.workdir,
        machine: &config.machine,
        engine_pin: &config.engine_pin,
        requires: &config.requires,
        exposed_ports: &config.exposed_ports,
        volumes: &config.volumes,
        cmd: &config.cmd,
    })?;
    Ok(digest)
}

fn quote_token(token: &str) -> String {
    let needs_quotes = token.is_empty()
        || token
            .chars()
            .any(|character| character.is_whitespace() || character == '"' || character == '\\');
    if !needs_quotes {
        return token.to_owned();
    }

    let mut quoted = String::with_capacity(token.len() + 2);
    quoted.push('"');
    for character in token.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

fn split_drive(raw: &str) -> Result<(&str, bool), BuildError> {
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        if !bytes[0].eq_ignore_ascii_case(&b'C') {
            return Err(path_error(format!("path {raw} must use the C: drive")));
        }
        return Ok((&raw[2..], true));
    }
    Ok((raw, false))
}

fn is_root_form(raw: &str) -> bool {
    raw == "/"
        || raw == "\\"
        || raw.eq_ignore_ascii_case("C:")
        || raw.eq_ignore_ascii_case("C:\\")
        || raw.eq_ignore_ascii_case("C:/")
}

pub(crate) fn sorted_capabilities(capabilities: &[String]) -> Vec<String> {
    let mut sorted = capabilities.to_vec();
    sorted.sort_by(|left, right| {
        capability_rank(left)
            .cmp(&capability_rank(right))
            .then_with(|| left.cmp(right))
    });
    sorted.dedup();
    sorted
}

fn capability_rank(name: &str) -> usize {
    CAPABILITY_ORDER
        .iter()
        .position(|known| *known == name)
        .unwrap_or(CAPABILITY_ORDER.len())
}

fn path_error(reason: impl Into<String>) -> BuildError {
    BuildError::Context {
        reason: reason.into(),
    }
}
