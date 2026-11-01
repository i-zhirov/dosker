//! `dosker exec`: run a command through the container supervisor.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dosker_core::Status;
use dosker_runtime::protocol::{ErrorCode, Reply, Request};
use dosker_runtime::recover_stale;
use dosker_runtime::stale::HostProbe;

use crate::cli::{EXIT_NOT_RUNNING, EXIT_TIMEOUT, ExecArgs};
use crate::context::Context;
use crate::control_client;
use crate::error::CliError;
use crate::resolve::resolve;

pub fn execute(
    args: ExecArgs,
    context: &mut Context,
    probe: &dyn HostProbe,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let record = resolve(&context.store, &args.container)?;
    recover_stale(&context.store, probe, now)?;
    let record = context.store.load(&record.id)?;

    if record.status != Status::Running {
        return Ok(EXIT_NOT_RUNNING);
    }

    let env = parse_env(&args.env)?;
    let command = args.command.join(" ");

    let Some(endpoint) = record.control_endpoint() else {
        return Ok(EXIT_NOT_RUNNING);
    };

    let request = Request::Exec {
        command,
        env,
        workdir: args.workdir,
        timeout_ms: args.timeout.saturating_mul(1000),
    };

    let response =
        match control_client::request(endpoint, &request, Duration::from_secs(args.timeout)) {
            Ok(response) => response,
            Err(CliError::Platform(_)) => return Ok(EXIT_NOT_RUNNING),
            Err(error) => return Err(error),
        };

    if !response.ok {
        return match response.error {
            Some(error) => match error.code {
                ErrorCode::NotRunning | ErrorCode::EngineError => Ok(EXIT_NOT_RUNNING),
                _ => Err(CliError::Message(error.message)),
            },
            None => Err(CliError::Message(String::from(
                "unexpected supervisor reply",
            ))),
        };
    }

    match response.result {
        Some(Reply::Exec {
            exit_code,
            output,
            timed_out,
            ..
        }) => {
            write_output(context, &output)?;
            if timed_out {
                Ok(EXIT_TIMEOUT)
            } else {
                Ok(exit_code.clamp(0, 255) as u8)
            }
        }
        _ => Err(CliError::Message(String::from(
            "unexpected supervisor reply",
        ))),
    }
}

fn parse_env(values: &[String]) -> Result<BTreeMap<String, String>, CliError> {
    let mut env = BTreeMap::new();
    for value in values {
        let Some((key, value)) = value.split_once('=') else {
            return Err(CliError::Usage(format!(
                "invalid environment assignment: {value}"
            )));
        };
        env.insert(key.to_owned(), value.to_owned());
    }
    Ok(env)
}

fn write_output(context: &mut Context, output: &str) -> Result<(), CliError> {
    if output.is_empty() {
        return Ok(());
    }
    if output.ends_with('\n') {
        context.console.raw(output)?;
    } else {
        context.console.line(output)?;
    }
    Ok(())
}
