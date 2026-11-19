//! `dosker volume`: create, list, and remove named volumes.

use std::time::Duration;

use chrono::{DateTime, Utc};
use dosker_core::ContainerName;
use dosker_runtime::{VolumeError, VolumeStore};

use crate::cli::{EXIT_OK, VolumeArgs, VolumeCommand, VolumeCreateArgs, VolumeRmArgs};
use crate::context::Context;
use crate::error::CliError;

const RM_RETRY_TIMEOUT: Duration = Duration::from_secs(5);
const RM_RETRY_STEP: Duration = Duration::from_millis(50);

pub fn execute(
    args: VolumeArgs,
    context: &mut Context,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    match args.command {
        VolumeCommand::Create(create) => create_volume(create, context, now),
        VolumeCommand::Ls(_) => list_volumes(context),
        VolumeCommand::Rm(remove) => remove_volume(remove, context),
    }
}

fn create_volume(
    args: VolumeCreateArgs,
    context: &mut Context,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let name = ContainerName::parse(&args.name)?;
    let size = parse_size(&args.size)?;
    context.volumes.create(name.as_str(), size, now)?;
    context.console.line(name.as_str())?;
    Ok(EXIT_OK)
}

fn list_volumes(context: &mut Context) -> Result<u8, CliError> {
    let volumes = context.volumes.list()?;
    context.console.line("NAME SIZE LAYOUT ATTACHED")?;
    for volume in volumes {
        let attached = volume
            .attached
            .as_ref()
            .map(|holder| holder.container_name.as_str())
            .unwrap_or("-");
        context.console.line(&format!(
            "{} {} {} {}",
            volume.meta.name,
            volume.meta.size,
            volume.meta.layout.as_str(),
            attached
        ))?;
    }
    Ok(EXIT_OK)
}

fn remove_volume(args: VolumeRmArgs, context: &mut Context) -> Result<u8, CliError> {
    let name = ContainerName::parse(&args.name)?;
    remove_retrying(
        &context.volumes,
        name.as_str(),
        RM_RETRY_TIMEOUT,
        RM_RETRY_STEP,
    )?;
    context.console.line(name.as_str())?;
    Ok(EXIT_OK)
}

fn remove_retrying(
    store: &VolumeStore,
    name: &str,
    timeout: Duration,
    step: Duration,
) -> Result<(), VolumeError> {
    let mut waited = Duration::ZERO;
    loop {
        match store.remove(name) {
            Ok(()) => return Ok(()),
            Err(VolumeError::InUse { .. }) if waited < timeout => {
                std::thread::sleep(step);
                waited += step;
            }
            Err(error) => return Err(error),
        }
    }
}

fn parse_size(value: &str) -> Result<u64, CliError> {
    let (digits, multiplier) = if let Some(digits) = value.strip_suffix('K') {
        (digits, 1024u64)
    } else if let Some(digits) = value.strip_suffix('M') {
        (digits, 1024u64 * 1024)
    } else if let Some(digits) = value.strip_suffix('G') {
        (digits, 1024u64 * 1024 * 1024)
    } else {
        (value, 1)
    };

    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_size(value));
    }
    let amount: u64 = digits.parse().map_err(|_| invalid_size(value))?;
    let size = amount
        .checked_mul(multiplier)
        .ok_or_else(|| invalid_size(value))?;
    if size == 0 {
        return Err(invalid_size(value));
    }
    Ok(size)
}

fn invalid_size(value: &str) -> CliError {
    CliError::Usage(format!(
        "invalid volume size: {value}; use bytes or a K, M, or G suffix"
    ))
}

#[cfg(test)]
mod tests {
    use super::parse_size;

    #[test]
    fn a_size_suffix_is_base_1024() {
        assert_eq!(parse_size("64M").expect("parse the size"), 67_108_864);
    }

    #[test]
    fn a_bare_size_is_bytes() {
        assert_eq!(parse_size("1048576").expect("parse the size"), 1_048_576);
    }

    #[test]
    fn a_zero_size_is_rejected() {
        assert!(parse_size("0").is_err());
        assert!(parse_size("0M").is_err());
    }

    #[test]
    fn a_size_with_an_unknown_suffix_is_rejected() {
        assert!(parse_size("64X").is_err());
        assert!(parse_size("12KB").is_err());
    }
}
