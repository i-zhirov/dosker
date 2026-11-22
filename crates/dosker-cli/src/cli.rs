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
    Build(BuildArgs),
    Run(RunArgs),
    Ps(PsArgs),
    Inspect(InspectArgs),
    Port(PortArgs),
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
    Volume(VolumeArgs),
    Images(ImagesArgs),
    Rmi(RmiArgs),
    Tag(TagArgs),
    Save(SaveArgs),
    Load(LoadArgs),
    System(SystemArgs),
    Engines(EnginesArgs),
    Doctor(DoctorArgs),
    #[command(name = "__supervise", hide = true)]
    Supervise(SuperviseArgs),
}

/// Build an image from a Doskerfile.
#[derive(Debug, Args)]
pub struct BuildArgs {
    #[arg(short = 't', long = "tag")]
    pub tag: Option<String>,

    #[arg(long = "update")]
    pub update: bool,

    #[arg(long = "no-cache")]
    pub no_cache: bool,

    #[arg(long = "timeout", default_value_t = 300)]
    pub timeout: u64,

    #[arg(default_value = ".")]
    pub context: PathBuf,
}

/// Create, start, and attach to a container.
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

    #[arg(short = 'p', long = "publish", value_name = "[addr:]host:guest")]
    pub publish: Vec<String>,

    #[arg(short = 'v', long = "volume", value_name = "name:drive[:ro]")]
    pub volumes: Vec<String>,

    #[arg(long = "bind", value_name = "drive:path[:ro]")]
    pub bind: Vec<String>,

    #[arg(long = "boot-timeout", default_value_t = 60)]
    pub boot_timeout: u64,

    pub image: String,
}

/// Create a container without starting it.
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

    #[arg(short = 'p', long = "publish", value_name = "[addr:]host:guest")]
    pub publish: Vec<String>,

    #[arg(short = 'v', long = "volume", value_name = "name:drive[:ro]")]
    pub volumes: Vec<String>,

    #[arg(long = "bind", value_name = "drive:path[:ro]")]
    pub bind: Vec<String>,

    pub image: String,
}

/// Start a created container.
#[derive(Debug, Args)]
pub struct StartArgs {
    #[arg(long = "boot-timeout", default_value_t = 60)]
    pub boot_timeout: u64,

    pub container: String,
}

/// Restart a container.
#[derive(Debug, Args)]
pub struct RestartArgs {
    #[arg(long = "boot-timeout", default_value_t = 60)]
    pub boot_timeout: u64,

    #[arg(long = "time", default_value_t = 10)]
    pub time: u64,

    pub container: String,
}

/// Remove a container.
#[derive(Debug, Args)]
pub struct RmArgs {
    pub container: String,
}

/// Manage images.
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

/// Import a disk image.
#[derive(Debug, Args)]
pub struct ImageImportArgs {
    #[arg(long = "booter")]
    pub booter: bool,

    pub file: PathBuf,

    pub reference: String,
}

/// Manage volumes.
#[derive(Debug, Args)]
pub struct VolumeArgs {
    #[command(subcommand)]
    pub command: VolumeCommand,
}

/// The volume subcommands.
#[derive(Debug, Subcommand)]
pub enum VolumeCommand {
    Create(VolumeCreateArgs),
    Ls(VolumeLsArgs),
    Rm(VolumeRmArgs),
}

/// Create a volume.
#[derive(Debug, Args)]
pub struct VolumeCreateArgs {
    #[arg(long = "size", default_value = "64M")]
    pub size: String,

    pub name: String,
}

/// List volumes.
#[derive(Debug, Args)]
pub struct VolumeLsArgs {}

/// Remove a volume.
#[derive(Debug, Args)]
pub struct VolumeRmArgs {
    pub name: String,
}

/// List images.
#[derive(Debug, Args)]
pub struct ImagesArgs {}

/// Remove or untag an image.
#[derive(Debug, Args)]
pub struct RmiArgs {
    #[arg(help = "A tag untags; a sha256 digest removes an untagged image")]
    pub reference: String,
}

/// Tag an image.
#[derive(Debug, Args)]
pub struct TagArgs {
    pub source: String,
    pub target: String,
}

/// Save an image to an archive.
#[derive(Debug, Args)]
pub struct SaveArgs {
    #[arg(short = 'o', long = "output")]
    pub output: Option<PathBuf>,

    pub image: String,
}

/// Load an image from an archive.
#[derive(Debug, Args)]
pub struct LoadArgs {
    pub file: PathBuf,
}

/// Manage the DOSker data root.
#[derive(Debug, Args)]
pub struct SystemArgs {
    #[command(subcommand)]
    pub command: SystemCommand,
}

/// The system subcommands.
#[derive(Debug, Subcommand)]
pub enum SystemCommand {
    Prune(PruneArgs),
}

/// Remove unused data.
#[derive(Debug, Args)]
pub struct PruneArgs {}

/// List and inspect engines.
#[derive(Debug, Args)]
pub struct EnginesArgs {}

/// Check the data root and installed engines.
#[derive(Debug, Args)]
pub struct DoctorArgs {}

/// List containers.
#[derive(Debug, Args)]
pub struct PsArgs {
    #[arg(short = 'a', long = "all")]
    pub all: bool,
}

/// Show container details.
#[derive(Debug, Args)]
pub struct InspectArgs {
    pub container: String,
}

/// Show a container's published ports.
#[derive(Debug, Args)]
pub struct PortArgs {
    pub container: String,
}

/// Show container logs.
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

/// Show container status.
#[derive(Debug, Args)]
pub struct StatusArgs {
    pub container: String,
}

/// Run a command in a running container.
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

/// Attach to a running container.
#[derive(Debug, Args)]
pub struct AttachArgs {
    pub container: String,
}

/// Copy files between the host and a container.
#[derive(Debug, Args)]
pub struct CpArgs {
    pub source: String,
    pub destination: String,
}

/// Capture a container screenshot.
#[derive(Debug, Args)]
pub struct ScreenshotArgs {
    pub container: String,

    #[arg(short = 'o', long = "output")]
    pub output: PathBuf,
}

/// Start a debugger session.
#[derive(Debug, Args)]
pub struct DebugArgs {
    pub container: String,

    #[arg(long = "gdb")]
    pub gdb: bool,
}

/// Stop a running container.
#[derive(Debug, Args)]
pub struct StopArgs {
    #[arg(long = "time", default_value_t = 10)]
    pub time: u64,

    pub container: String,
}

/// Run the internal supervisor.
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
        Command::Build(args) => {
            let mut context = Context::load()?;
            commands::build::execute(args, &mut context, Utc::now())
        }
        Command::Ps(args) => {
            let mut context = Context::load()?;
            commands::ps::execute(args, &mut context, &SystemProbe::new(), Utc::now())
        }
        Command::Inspect(args) => {
            let mut context = Context::load()?;
            commands::inspect::execute(args, &mut context, &SystemProbe::new(), Utc::now())
        }
        Command::Port(args) => {
            let mut context = Context::load()?;
            commands::port::execute(args, &mut context, &SystemProbe::new(), Utc::now())
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
        Command::Volume(args) => {
            let mut context = Context::load()?;
            commands::volume::execute(args, &mut context, Utc::now())
        }
        Command::Images(args) => {
            let mut context = Context::load()?;
            commands::images::execute(args, &mut context)
        }
        Command::Rmi(args) => {
            let mut context = Context::load()?;
            commands::rmi::execute(args, &mut context)
        }
        Command::Tag(args) => {
            let mut context = Context::load()?;
            commands::tag::execute(args, &mut context)
        }
        Command::Save(args) => {
            let mut context = Context::load()?;
            commands::save::execute(args, &mut context)
        }
        Command::Load(args) => {
            let mut context = Context::load()?;
            commands::load::execute(args, &mut context)
        }
        Command::System(args) => {
            let mut context = Context::load()?;
            commands::system::execute(args, &mut context, &SystemProbe::new(), Utc::now())
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

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn help_describes_each_command() {
        let mut command = Cli::command();
        let help = command.render_help().to_string();
        assert!(!help.contains("Arguments for"), "{help}");
        for description in [
            "Build an image from a Doskerfile",
            "Create, start, and attach to a container",
            "Manage volumes",
            "Check the data root and installed engines",
        ] {
            assert!(help.contains(description), "missing {description}: {help}");
        }

        let mut command = Cli::command();
        let image = command
            .find_subcommand_mut("image")
            .expect("find the image subcommand");
        let help = image.render_help().to_string();
        assert!(help.contains("Import a disk image"), "{help}");
    }
}
