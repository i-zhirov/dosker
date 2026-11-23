//! `dosker run`: create a container and start its supervisor.

use std::io::{self, BufReader};
use std::time::Duration;

use chrono::{DateTime, Utc};
use dosker_core::Status;
use dosker_engine::DisplayMode;

use crate::cli::{EXIT_ERROR, EXIT_NOT_RUNNING, EXIT_OK, RunArgs};
use crate::commands::attach;
use crate::commands::create::{CreateOptions, create_container};
use crate::commands::start::{print_container_log, start_container};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(args: RunArgs, context: &mut Context, now: DateTime<Utc>) -> Result<u8, CliError> {
    let display = DisplayMode::parse(&args.display).ok_or_else(|| {
        CliError::Usage(format!(
            "invalid display mode: {}; use none, window, or vnc",
            args.display
        ))
    })?;

    let record = create_container(
        context,
        CreateOptions {
            image: args.image,
            name: args.name,
            engine: args.engine,
            display,
            vnc_port: args.vnc_port,
            publish: args.publish,
            volumes: args.volumes,
            binds: args.bind,
            no_pull: args.no_pull,
        },
        now,
    )?;
    let outcome = start_container(context, &record, Duration::from_secs(args.boot_timeout))?;

    if outcome.record.status == Status::Running {
        if args.detach {
            context.console.line(outcome.record.id.as_str())?;
            return Ok(EXIT_OK);
        }

        if outcome.record.booter {
            context
                .console
                .line("the image has no agent; the container is running")?;
            return Ok(EXIT_OK);
        }

        let Some(endpoint) = outcome.record.control_endpoint() else {
            return Ok(EXIT_NOT_RUNNING);
        };
        let stdin = io::stdin();
        let mut input = BufReader::new(stdin.lock());
        return attach::run_shell(endpoint, r"C:\", &mut input, &mut context.console);
    }

    context.console.line(&format!(
        "{} {}",
        outcome.record.name, outcome.record.status
    ))?;
    print_container_log(context, &outcome.record.id)?;
    Ok(EXIT_ERROR)
}
