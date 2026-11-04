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
    Inspect(InspectArgs),
    Logs(LogsArgs),
    Status(StatusArgs),
    Exec(ExecArgs),
    Attach(AttachArgs),
    Cp(CpArgs),
    Stop(StopArgs),
    Screenshot(ScreenshotArgs),
    Debug(DebugArgs),
    Create(CreateArgs),
    Start(StartArgs),
    Restart(RestartArgs),
    Rm(RmArgs),
    Image(ImageArgs),
    Engines(EnginesArgs),
    Doctor(DoctorArgs),
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

    #[arg(long = "display", default_value = "none")]
    pub display: String,

    #[arg(long = "vnc-port")]
    pub vnc_port: Option<u16>,

    #[arg(long = "boot-timeout", default_value_t = 60)]
    pub boot_timeout: u64,

    pub image: String,
}

/// Arguments for `create`.
#[derive(Debug, Args)]
pub struct CreateArgs {
    #[arg(long = "name")]
    pub name: Option<String>,

    #[arg(long = "engine")]
    pub engine: Option<String>,

    #[arg(long = "display", default_value = "none")]
    pub display: String,

    #[arg(long = "vnc-port")]
    pub vnc_port: Option<u16>,

    pub image: String,
}

/// Arguments for `start`.
#[derive(Debug, Args)]
pub struct StartArgs {
    #[arg(long = "boot-timeout", default_value_t = 60)]
    pub boot_timeout: u64,

    pub container: String,
}

/// Arguments for `restart`.
#[derive(Debug, Args)]
pub struct RestartArgs {
    #[arg(long = "boot-timeout", default_value_t = 60)]
    pub boot_timeout: u64,

    #[arg(long = "time", default_value_t = 10)]
    pub time: u64,

    pub container: String,
}

/// Arguments for `rm`.
#[derive(Debug, Args)]
pub struct RmArgs {
    pub container: String,
}

/// Arguments for `image`.
#[derive(Debug, Args)]
pub struct ImageArgs {
    #[command(subcommand)]
    pub command: ImageCommand,
}

/// The image subcommands.
#[derive(Debug, Subcommand)]
pub enum ImageCommand {
    Import(ImageImportArgs),
}

/// Arguments for `image import`.
#[derive(Debug, Args)]
pub struct ImageImportArgs {
    #[arg(long = "booter")]
    pub booter: bool,

    pub file: PathBuf,

    pub reference: String,
}

/// Arguments for `engines`.
#[derive(Debug, Args)]
pub struct EnginesArgs {}

/// Arguments for `doctor`.
#[derive(Debug, Args)]
pub struct DoctorArgs {}

/// Arguments for `ps`.
#[derive(Debug, Args)]
pub struct PsArgs {
    #[arg(short = 'a', long = "all")]
    pub all: bool,
}

/// Arguments for `inspect`.
#[derive(Debug, Args)]
pub struct InspectArgs {
    pub container: String,
}

/// Arguments for `logs`.
#[derive(Debug, Args)]
pub struct LogsArgs {
    #[arg(short = 'f', long = "follow")]
    pub follow: bool,

    #[arg(long = "engine", conflicts_with = "serial")]
    pub engine: bool,

    #[arg(long = "serial")]
    pub serial: bool,

    pub container: String,
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

/// Arguments for `attach`.
#[derive(Debug, Args)]
pub struct AttachArgs {
    pub container: String,
}

/// Arguments for `cp`.
#[derive(Debug, Args)]
pub struct CpArgs {
    pub source: String,
    pub destination: String,
}

/// Arguments for `screenshot`.
#[derive(Debug, Args)]
pub struct ScreenshotArgs {
    pub container: String,

    #[arg(short = 'o', long = "output")]
    pub output: PathBuf,
}

/// Arguments for `debug`.
#[derive(Debug, Args)]
pub struct DebugArgs {
    pub container: String,

    #[arg(long = "gdb")]
    pub gdb: bool,
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

    #[arg(long = "ready-timeout-ms", default_value_t = 60000)]
    pub ready_timeout_ms: u64,
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
        Command::Inspect(args) => {
            let mut context = Context::load()?;
            commands::inspect::execute(args, &mut context, &SystemProbe::new(), Utc::now())
        }
        Command::Logs(args) => {
            let mut context = Context::load()?;
            commands::logs::execute(args, &mut context, &SystemProbe::new(), Utc::now())
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
        Command::Attach(args) => {
            let mut context = Context::load()?;
            commands::attach::execute(args, &mut context, &SystemProbe::new(), Utc::now())
        }
        Command::Cp(args) => {
            let mut context = Context::load()?;
            commands::cp::execute(args, &mut context, &SystemProbe::new(), Utc::now())
        }
        Command::Screenshot(args) => {
            let mut context = Context::load()?;
            commands::screenshot::execute(args, &mut context, &SystemProbe::new(), Utc::now())
        }
        Command::Debug(args) => {
            let mut context = Context::load()?;
            commands::debug::execute(args, &mut context, &SystemProbe::new(), Utc::now())
        }
        Command::Stop(args) => {
            let mut context = Context::load()?;
            commands::stop::execute(args, &mut context, &SystemProbe::new(), Utc::now())
        }
        Command::Create(args) => {
            let mut context = Context::load()?;
            commands::create::execute(args, &mut context, Utc::now())
        }
        Command::Start(args) => {
            let mut context = Context::load()?;
            commands::start::execute(args, &mut context, &SystemProbe::new(), Utc::now())
        }
        Command::Restart(args) => {
            let mut context = Context::load()?;
            commands::restart::execute(args, &mut context, &SystemProbe::new(), Utc::now())
        }
        Command::Rm(args) => {
            let mut context = Context::load()?;
            commands::rm::execute(args, &mut context, &SystemProbe::new(), Utc::now())
        }
        Command::Image(args) => {
            let mut context = Context::load()?;
            commands::image::execute(args, &mut context, Utc::now())
        }
        Command::Engines(args) => {
            let mut context = Context::load()?;
            commands::engines::execute(args, &mut context)
        }
        Command::Doctor(args) => {
            let mut context = Context::load()?;
            commands::doctor::execute(args, &mut context)
        }
    }
}
