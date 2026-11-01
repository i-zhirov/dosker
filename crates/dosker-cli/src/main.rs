use std::process::ExitCode;

use dosker_cli::cli;

fn main() -> ExitCode {
    ExitCode::from(cli::run(std::env::args_os()))
}
