//! Argument parsing and command dispatch.

use std::ffi::OsString;
use std::path::PathBuf;

use chrono::Utc;
use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand};
use dosker_runtime::SystemProbe;

use crate::commands;
use crate::context::Context;
use crate::error::CliError;

pub const EXIT_OK: u8 = 0;
pub const EXIT_ERROR: u8 = 1;
pub const EXIT_USAGE: u8 = 2;
pub const EXIT_TIMEOUT: u8 = 124;
pub const EXIT_NOT_RUNNING: u8 = 125;

/// The dosker command line.
#[derive(Debug, Parser)]
#[command(name = "dosker", version, about = "Run DOS systems as containers")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// The available commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    Run(RunArgs),
    Ps(PsArgs),
    Status(StatusArgs),
    Exec(ExecArgs),
    Stop(StopArgs),
    #[command(name = "__supervise", hide = true)]
    Supervise(SuperviseArgs),
}

/// Arguments for `run`.
#[derive(Debug, Args)]
pub struct RunArgs {
    #[arg(short = 'd', long = "detach")]
    pub detach: bool,

    #[arg(long = "name")]
    pub name: Option<String>,

    #[arg(long = "engine")]
    pub engine: Option<String>,

    pub image: String,
}

/// Arguments for `ps`.
#[derive(Debug, Args)]
pub struct PsArgs {
    #[arg(short = 'a', long = "all")]
    pub all: bool,
}

/// Arguments for `status`.
#[derive(Debug, Args)]
pub struct StatusArgs {
    pub container: String,
}

/// Arguments for `exec`.
#[derive(Debug, Args)]
pub struct ExecArgs {
    #[arg(short = 'e', long = "env")]
    pub env: Vec<String>,

    #[arg(short = 'w', long = "workdir", default_value = "C:\\")]
    pub workdir: String,

    #[arg(long = "timeout", default_value_t = 30)]
    pub timeout: u64,

    pub container: String,

    #[arg(required = true, trailing_var_arg = true)]
    pub command: Vec<String>,
}

/// Arguments for `stop`.
#[derive(Debug, Args)]
pub struct StopArgs {
    #[arg(long = "time", default_value_t = 10)]
    pub time: u64,

    pub container: String,
}

/// Arguments for the hidden supervisor entry.
#[derive(Debug, Args)]
pub struct SuperviseArgs {
    #[arg(long = "root")]
    pub root: PathBuf,

    #[arg(long = "container")]
    pub container: String,

    #[arg(long = "engine")]
    pub engine: String,
}

/// Parses the command line, runs the command, and returns an exit code.
pub fn run(args: impl IntoIterator<Item = OsString>) -> u8 {
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let code = match error.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => EXIT_OK,
                _ => EXIT_USAGE,
            };
            let _ = error.print();
            return code;
        }
    };

    match dispatch(cli.command) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("dosker: {error}");
            error.exit_code()
        }
    }
}

fn dispatch(command: Command) -> Result<u8, CliError> {
    match command {
        Command::Ps(args) => {
            let mut context = Context::load()?;
            commands::ps::execute(args, &mut context, &SystemProbe::new(), Utc::now())
        }
        Command::Status(args) => {
            let mut context = Context::load()?;
            commands::status::execute(args, &mut context, &SystemProbe::new(), Utc::now())
        }
        Command::Supervise(args) => commands::supervise::execute(args),
        Command::Run(args) => {
            let mut context = Context::load()?;
            commands::run::execute(args, &mut context, Utc::now())
        }
        Command::Exec(args) => {
            let mut context = Context::load()?;
            commands::exec::execute(args, &mut context, &SystemProbe::new(), Utc::now())
        }
        Command::Stop(args) => {
            let mut context = Context::load()?;
            commands::stop::execute(args, &mut context, &SystemProbe::new(), Utc::now())
        }
    }
}
