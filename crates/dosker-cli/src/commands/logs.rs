//! `dosker logs`: print a container log and optionally follow it.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dosker_core::ContainerId;
use dosker_platform::console::Console;
use dosker_runtime::stale::HostProbe;
use dosker_runtime::{RuntimePaths, recover_stale};

use crate::cli::{EXIT_OK, LogsArgs};
use crate::context::Context;
use crate::error::CliError;
use crate::resolve::resolve;

const FOLLOW_INTERVAL: Duration = Duration::from_millis(200);
const CHUNK_SIZE: usize = 64 * 1024;

pub fn execute(
    args: LogsArgs,
    context: &mut Context,
    probe: &dyn HostProbe,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let record = resolve(&context.store, &args.container)?;
    recover_stale(&context.store, probe, now)?;
    let record = context.store.load(&record.id)?;

    let path = log_path(&context.paths, &record.id, &args);
    let mut file = File::open(&path)?;
    let mut offset = 0;
    if !stream_new(&mut context.console, &path, &mut file, &mut offset)? {
        return Ok(EXIT_OK);
    }

    if !args.follow {
        return Ok(EXIT_OK);
    }

    loop {
        thread::sleep(FOLLOW_INTERVAL);
        if context.console.stdout_hung_up() {
            return Ok(EXIT_OK);
        }
        if !stream_new(&mut context.console, &path, &mut file, &mut offset)? {
            return Ok(EXIT_OK);
        }
    }
}

fn log_path(paths: &RuntimePaths, id: &ContainerId, args: &LogsArgs) -> PathBuf {
    if args.engine {
        paths.engine_log(id)
    } else if args.serial {
        paths.serial_log(id)
    } else {
        paths.container_log(id)
    }
}

/// Streams the bytes written since `offset` to the console in bounded chunks.
///
/// Returns `false` when stdout closed (a broken pipe) so a follow loop can
/// stop. The offset advances with every chunk that reaches the console, so a
/// later call reads only newly appended bytes.
fn stream_new(
    console: &mut Console,
    path: &Path,
    file: &mut File,
    offset: &mut u64,
) -> io::Result<bool> {
    let length = fs::metadata(path)?.len();
    if length < *offset {
        *offset = 0;
    }
    if length == *offset {
        return Ok(true);
    }

    file.seek(SeekFrom::Start(*offset))?;
    let mut remaining = length - *offset;
    let mut buffer = vec![0u8; CHUNK_SIZE];
    while remaining > 0 {
        let chunk = remaining.min(CHUNK_SIZE as u64) as usize;
        let read = file.read(&mut buffer[..chunk])?;
        if read == 0 {
            break;
        }
        match console.try_bytes(&buffer[..read]) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => return Ok(false),
            Err(error) => return Err(error),
        }
        *offset += read as u64;
        remaining -= read as u64;
    }
    Ok(true)
}
