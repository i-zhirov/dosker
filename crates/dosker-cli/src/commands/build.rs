//! `dosker build`: build an image from a Doskerfile.

use std::io;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dosker_build::{BuildOptions, BuildReporter, Builder};
use dosker_image::ImageReference;
use dosker_platform::console::Console;

use crate::cli::{BuildArgs, EXIT_OK};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(args: BuildArgs, context: &mut Context, now: DateTime<Utc>) -> Result<u8, CliError> {
    let tag = match args.tag.as_deref() {
        Some(value) => Some(ImageReference::parse(value)?),
        None => None,
    };
    let options = BuildOptions {
        tag,
        update_lock: args.update,
        no_cache: args.no_cache,
        timeout: Duration::from_secs(args.timeout),
        now,
        no_pull: args.no_pull,
    };

    let outcome = {
        let mut reporter = ConsoleReporter {
            console: &mut context.console,
        };
        Builder::new(context.paths.root().to_path_buf()).build(
            &args.context,
            &options,
            &mut reporter,
        )?
    };

    match outcome.reference {
        Some(reference) => context
            .console
            .line(&format!("built {reference} {}", outcome.manifest_digest))?,
        None => context
            .console
            .line(&format!("built {}", outcome.manifest_digest))?,
    }

    Ok(EXIT_OK)
}

struct ConsoleReporter<'a> {
    console: &'a mut Console,
}

impl BuildReporter for ConsoleReporter<'_> {
    fn line(&mut self, text: &str) -> io::Result<()> {
        self.console.line(text)
    }

    fn output(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.console.bytes(bytes)
    }
}
