//! `dosker run`: create a container and start its detached supervisor.

use std::fs::OpenOptions;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use dosker_core::{ContainerId, ContainerName, Status};
use dosker_platform::endpoint::control_endpoint;
use dosker_platform::process::{ProcessSupervisor, system};
use dosker_runtime::ContainerRecord;

use crate::adapter;
use crate::cli::{EXIT_ERROR, EXIT_OK, RunArgs};
use crate::context::Context;
use crate::control_client;
use crate::error::CliError;

const RUN_WAIT: Duration = Duration::from_secs(65);
const POLL_STEP: Duration = Duration::from_millis(100);
const PING_BUDGET: Duration = Duration::from_millis(200);

pub fn execute(args: RunArgs, context: &mut Context, now: DateTime<Utc>) -> Result<u8, CliError> {
    if !args.detach {
        return Err(CliError::Usage(String::from(
            "run requires -d to start a container in the background",
        )));
    }

    let engine = match args.engine {
        Some(engine) => engine,
        None => match adapter::default_engine() {
            Some(engine) => engine.to_owned(),
            None => {
                return Err(CliError::Usage(String::from(
                    "no engine selected; pass --engine",
                )));
            }
        },
    };
    if !adapter::is_known(&engine) {
        return Err(CliError::Usage(format!("unknown engine: {engine}")));
    }

    let id = ContainerId::generate();
    let name = match args.name {
        Some(name) => ContainerName::parse(&name)?,
        None => ContainerName::parse(&format!("dosker-{}", &id.as_str()[..8]))?,
    };

    let root = std::path::absolute(context.paths.root())?;
    let endpoint = control_endpoint(&root, &id);
    let record = ContainerRecord::new(
        id.clone(),
        name,
        args.image,
        engine.clone(),
        endpoint.record(),
        now,
    );
    context.store.create(&record)?;

    let mut child = spawn_supervisor(context, &root, &id, &engine)?;

    let deadline = Instant::now() + RUN_WAIT;
    loop {
        let stored = context.store.load(&id)?;
        if matches!(stored.status, Status::Failed | Status::Exited) {
            context.console.line(stored.status.as_str())?;
            return Ok(EXIT_ERROR);
        }

        if child.try_wait()?.is_some() {
            context.console.line(stored.status.as_str())?;
            if let Ok(log) = std::fs::read_to_string(context.paths.container_log(&id)) {
                let log = log.trim_end();
                if !log.is_empty() {
                    context.console.error_line(log)?;
                }
            }
            return Ok(EXIT_ERROR);
        }

        if let Some(endpoint) = stored.control_endpoint()
            && control_client::ping(endpoint, PING_BUDGET).is_ok()
        {
            context.console.line(id.as_str())?;
            return Ok(EXIT_OK);
        }

        if Instant::now() >= deadline {
            let status = context.store.load(&id)?.status;
            context.console.line(status.as_str())?;
            return Ok(EXIT_ERROR);
        }

        std::thread::sleep(POLL_STEP);
    }
}

fn spawn_supervisor(
    context: &Context,
    root: &std::path::Path,
    id: &ContainerId,
    engine: &str,
) -> Result<Child, CliError> {
    let log_path = context.paths.container_log(id);
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let stderr = stdout.try_clone()?;

    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("__supervise")
        .arg("--root")
        .arg(root)
        .arg("--container")
        .arg(id.as_str())
        .arg("--engine")
        .arg(engine)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    system().prepare_detached(&mut command)?;
    Ok(command.spawn()?)
}
