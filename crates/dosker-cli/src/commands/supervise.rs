//! The hidden supervisor entry that owns one container.

use dosker_core::ContainerId;
use dosker_runtime::{DEFAULT_READY_TIMEOUT, Supervisor, SupervisorConfig};

use crate::adapter;
use crate::cli::SuperviseArgs;
use crate::error::CliError;

pub fn execute(args: SuperviseArgs) -> Result<u8, CliError> {
    let container_id = ContainerId::parse(&args.container)?;
    let engine = adapter::build(&args.engine)?;
    let config = SupervisorConfig {
        root: args.root,
        container_id,
        ready_timeout: DEFAULT_READY_TIMEOUT,
    };

    Supervisor::new(config, engine)
        .run()
        .map_err(|error| CliError::Message(error.to_string()))
}
