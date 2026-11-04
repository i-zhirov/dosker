//! `dosker start`: start a stopped container and wait for it to be ready.

use std::fs::OpenOptions;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use dosker_core::{ContainerId, Status};
use dosker_platform::process::{ProcessSupervisor, system};
use dosker_runtime::stale::{HostProbe, SystemProbe};
use dosker_runtime::{ContainerRecord, StateStore, recover_stale};

use crate::cli::{EXIT_ERROR, EXIT_OK, StartArgs};
use crate::context::Context;
use crate::error::CliError;
use crate::resolve::resolve;

const START_WAIT: Duration = Duration::from_secs(65);
const START_MARGIN: Duration = Duration::from_secs(5);
const KILL_WAIT: Duration = Duration::from_secs(5);
const ENDPOINT_WAIT: Duration = Duration::from_secs(5);
const POLL_STEP: Duration = Duration::from_millis(100);

/// The result of waiting for a container start.
pub struct StartOutcome {
    pub record: ContainerRecord,
    pub timed_out: bool,
}

impl StartOutcome {
    /// Reports whether the caller should show the container log.
    pub fn needs_log(&self) -> bool {
        self.timed_out || self.record.status == Status::Failed
    }
}

pub fn execute(
    args: StartArgs,
    context: &mut Context,
    probe: &dyn HostProbe,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let record = resolve(&context.store, &args.container)?;
    recover_stale(&context.store, probe, now)?;
    let record = context.store.load(&record.id)?;

    match record.status {
        Status::Starting => {
            return Err(CliError::Message(format!(
                "container {} is starting",
                record.name
            )));
        }
        Status::Running => {
            return Err(CliError::Message(format!(
                "container {} is already running",
                record.name
            )));
        }
        Status::Stopping => {
            return Err(CliError::Message(format!(
                "container {} is stopping",
                record.name
            )));
        }
        Status::Failed => {
            return Err(CliError::Message(format!(
                "container {} failed; remove it with `dosker rm {}`",
                record.name, record.name
            )));
        }
        _ => {}
    }

    warn_if_dirty(context, &record)?;

    let outcome = start_container(context, &record, Duration::from_secs(args.boot_timeout))?;
    context.console.line(&format!(
        "{} {}",
        outcome.record.name, outcome.record.status
    ))?;

    if outcome.needs_log() {
        print_container_log(context, &outcome.record.id)?;
    }

    if outcome.record.status == Status::Running {
        Ok(EXIT_OK)
    } else {
        Ok(EXIT_ERROR)
    }
}

/// Starts the container and waits for its supervisor to report readiness.
pub fn start_container(
    context: &mut Context,
    record: &ContainerRecord,
    ready_timeout: Duration,
) -> Result<StartOutcome, CliError> {
    let root = std::path::absolute(context.paths.root())?;
    let probe = SystemProbe::new();

    wait_for_previous_supervisor(&probe, record, ENDPOINT_WAIT);

    let ready_timeout_ms = u64::try_from(ready_timeout.as_millis()).unwrap_or(u64::MAX);
    let mut child = spawn_supervisor(context, &root, &record.id, &record.engine, ready_timeout_ms)?;

    let deadline = Instant::now() + START_WAIT.max(ready_timeout.saturating_add(START_MARGIN));
    let processes = system();
    wait_for_start(
        &context.store,
        &record.id,
        deadline,
        &probe,
        &processes,
        Some(&mut child),
    )
}

/// Waits for the previous supervisor to release the control endpoint.
///
/// A record that still carries a supervisor PID waits for that process.
/// Otherwise only a running or stopped record can still own the endpoint
/// after a graceful stop, so other states skip the wait entirely.
fn wait_for_previous_supervisor(probe: &dyn HostProbe, record: &ContainerRecord, wait: Duration) {
    let deadline = Instant::now() + wait;

    if let Some(pid) = record.pid {
        while probe.process_alive(pid) && Instant::now() < deadline {
            std::thread::sleep(POLL_STEP);
        }
        return;
    }

    if !matches!(record.status, Status::Running | Status::Stopped) {
        return;
    }

    let Some(endpoint) = record.control_endpoint() else {
        return;
    };
    while probe.endpoint_alive(endpoint) && Instant::now() < deadline {
        std::thread::sleep(POLL_STEP);
    }
}

/// Polls a starting container until it is ready, terminal, or out of time.
///
/// A timeout kills the supervisor and engine groups without sending a stop
/// request, because a supervisor blocked in readiness cannot accept one.
pub fn wait_for_start(
    store: &StateStore,
    id: &ContainerId,
    deadline: Instant,
    probe: &dyn HostProbe,
    processes: &dyn ProcessSupervisor,
    mut supervisor: Option<&mut Child>,
) -> Result<StartOutcome, CliError> {
    let mut saw_starting = false;
    loop {
        let record = store.load(id)?;
        match record.status {
            Status::Created => {}
            Status::Starting => saw_starting = true,
            Status::Running | Status::Failed => {
                return Ok(StartOutcome {
                    record,
                    timed_out: false,
                });
            }
            Status::Stopped | Status::Exited if !saw_starting => {
                // The record has not been picked up by the new supervisor yet.
            }
            Status::Stopped | Status::Exited | Status::Stopping | Status::Removed => {
                return Ok(StartOutcome {
                    record,
                    timed_out: false,
                });
            }
        }

        if record.status == Status::Starting
            && let Some(pid) = record.pid
            && !probe.process_alive(pid)
        {
            recover_stale(store, probe, Utc::now())?;
            let record = store.load(id)?;
            return Ok(StartOutcome {
                record,
                timed_out: false,
            });
        }

        // A supervisor that exits before it writes `starting` can make no
        // further progress, so the current record is final even when it is
        // still in its pre-spawn state.
        if let Some(child) = supervisor.as_deref_mut()
            && child.try_wait()?.is_some()
        {
            let record = store.load(id)?;
            if record.status.is_active() {
                recover_stale(store, probe, Utc::now())?;
                let record = store.load(id)?;
                return Ok(StartOutcome {
                    record,
                    timed_out: false,
                });
            }
            return Ok(StartOutcome {
                record,
                timed_out: false,
            });
        }

        if Instant::now() >= deadline {
            return time_out(store, id, probe, processes);
        }

        std::thread::sleep(POLL_STEP);
    }
}

fn time_out(
    store: &StateStore,
    id: &ContainerId,
    probe: &dyn HostProbe,
    processes: &dyn ProcessSupervisor,
) -> Result<StartOutcome, CliError> {
    // The supervisor may have reached a terminal state between the last poll
    // and the deadline. Reload and never kill a container that is no longer
    // starting.
    let record = store.load(id)?;

    if matches!(
        record.status,
        Status::Running | Status::Failed | Status::Stopped | Status::Exited
    ) {
        return Ok(StartOutcome {
            record,
            timed_out: false,
        });
    }

    if record.status == Status::Starting {
        if let Some(pgid) = record.pid {
            let _ = processes.kill_group(pgid);
        }
        if let Some(pgid) = record.engine_pgid {
            let _ = processes.kill_group(pgid);
        }

        let kill_deadline = Instant::now() + KILL_WAIT;
        loop {
            let supervisor_alive = record.pid.is_some_and(|pid| probe.process_alive(pid));
            let engine_alive = record
                .engine_pid
                .is_some_and(|pid| probe.process_alive(pid));
            if (!supervisor_alive && !engine_alive) || Instant::now() >= kill_deadline {
                break;
            }
            std::thread::sleep(POLL_STEP);
        }
    }

    recover_stale(store, probe, Utc::now())?;
    let mut record = store.load(id)?;
    if matches!(record.status, Status::Created | Status::Starting) {
        if record.status == Status::Created {
            record.transition(Status::Starting, Utc::now())?;
        }
        record.transition(Status::Exited, Utc::now())?;
        record.pid = None;
        record.engine_pid = None;
        record.engine_pgid = None;
        record.dirty = true;
        store.save(&record)?;
    }

    Ok(StartOutcome {
        record,
        timed_out: true,
    })
}

pub fn spawn_supervisor(
    context: &Context,
    root: &Path,
    id: &ContainerId,
    engine: &str,
    ready_timeout_ms: u64,
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
        .arg("--ready-timeout-ms")
        .arg(ready_timeout_ms.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    system().prepare_detached(&mut command)?;
    Ok(command.spawn()?)
}

pub fn warn_if_dirty(context: &mut Context, record: &ContainerRecord) -> Result<(), CliError> {
    if record.dirty {
        context.console.error_line(&format!(
            "warning: the disk of {} is dirty; the guest was not shut down cleanly",
            record.name
        ))?;
    }
    Ok(())
}

pub fn print_container_log(context: &mut Context, id: &ContainerId) -> Result<(), CliError> {
    let log = std::fs::read_to_string(context.paths.container_log(id))?;
    let log = log.trim_end();
    if !log.is_empty() {
        context.console.error_line(log)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use chrono::Utc;
    use dosker_core::{ContainerId, ContainerName};
    use dosker_platform::endpoint::control_endpoint;
    use dosker_platform::{EndpointRecord, PlatformError};

    use super::*;

    struct FakeProbe {
        alive: Arc<AtomicBool>,
        killed: Arc<Mutex<Vec<u32>>>,
        endpoint_checks: Arc<Mutex<u32>>,
    }

    impl HostProbe for FakeProbe {
        fn process_alive(&self, _pid: u32) -> bool {
            self.alive.load(Ordering::SeqCst)
        }

        fn endpoint_alive(&self, _endpoint: &EndpointRecord) -> bool {
            let mut checks = self.endpoint_checks.lock().unwrap();
            *checks += 1;
            true
        }

        fn kill_group(&self, pgid: u32) -> Result<(), PlatformError> {
            self.killed.lock().unwrap().push(pgid);
            self.alive.store(false, Ordering::SeqCst);
            Ok(())
        }
    }

    struct FakeProcesses {
        alive: Arc<AtomicBool>,
        killed: Arc<Mutex<Vec<u32>>>,
    }

    impl ProcessSupervisor for FakeProcesses {
        fn prepare_detached(&self, _cmd: &mut std::process::Command) -> Result<(), PlatformError> {
            Ok(())
        }

        fn prepare_engine(&self, _cmd: &mut std::process::Command) -> Result<(), PlatformError> {
            Ok(())
        }

        fn kill_group(&self, pgid: u32) -> Result<(), PlatformError> {
            self.killed.lock().unwrap().push(pgid);
            self.alive.store(false, Ordering::SeqCst);
            Ok(())
        }

        fn process_alive(&self, _pid: u32) -> bool {
            self.alive.load(Ordering::SeqCst)
        }

        fn process_group(&self, _pid: u32) -> Result<Option<u32>, PlatformError> {
            Ok(None)
        }
    }

    #[test]
    fn start_container_times_out_without_stopping_from_starting() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().to_path_buf());
        let id = ContainerId::parse("000000000001").unwrap();
        let name = ContainerName::parse("stuck").unwrap();
        let endpoint = control_endpoint(dir.path(), &id);
        let mut record = ContainerRecord::new(
            id.clone(),
            name,
            String::from("freedos:latest"),
            String::from("mock"),
            endpoint.record(),
            Utc::now(),
        );
        record.status = Status::Starting;
        record.pid = Some(4242);
        record.engine_pid = Some(4343);
        record.engine_pgid = Some(4343);
        store.create(&record).unwrap();

        let alive = Arc::new(AtomicBool::new(true));
        let killed = Arc::new(Mutex::new(Vec::new()));
        let endpoint_checks = Arc::new(Mutex::new(0));
        let probe = FakeProbe {
            alive: Arc::clone(&alive),
            killed: Arc::clone(&killed),
            endpoint_checks: Arc::clone(&endpoint_checks),
        };
        let processes = FakeProcesses {
            alive: Arc::clone(&alive),
            killed: Arc::clone(&killed),
        };

        let outcome =
            wait_for_start(&store, &id, Instant::now(), &probe, &processes, None).unwrap();

        assert!(outcome.timed_out);
        assert_eq!(outcome.record.status, Status::Exited);
        assert!(outcome.record.dirty);
        assert_eq!(*killed.lock().unwrap(), vec![4242, 4343]);
        assert_eq!(*endpoint_checks.lock().unwrap(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_starting_record_without_an_engine_pgid_kills_the_supervisor_group() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().to_path_buf());
        let id = ContainerId::parse("000000000005").unwrap();
        let name = ContainerName::parse("pre-engine").unwrap();
        let endpoint = control_endpoint(dir.path(), &id);

        let mut supervisor = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("start a stand-in supervisor");

        let mut record = ContainerRecord::new(
            id.clone(),
            name,
            String::from("freedos:latest"),
            String::from("mock"),
            endpoint.record(),
            Utc::now(),
        );
        record.status = Status::Starting;
        record.pid = Some(supervisor.id());
        store.create(&record).unwrap();

        let alive = Arc::new(AtomicBool::new(true));
        let killed = Arc::new(Mutex::new(Vec::new()));
        let endpoint_checks = Arc::new(Mutex::new(0));
        let probe = FakeProbe {
            alive: Arc::clone(&alive),
            killed: Arc::clone(&killed),
            endpoint_checks,
        };
        let processes = FakeProcesses {
            alive: Arc::clone(&alive),
            killed: Arc::clone(&killed),
        };

        let outcome = wait_for_start(
            &store,
            &id,
            Instant::now(),
            &probe,
            &processes,
            Some(&mut supervisor),
        )
        .unwrap();

        assert!(outcome.timed_out);
        assert_eq!(outcome.record.status, Status::Exited);
        assert_eq!(outcome.record.engine_pid, None);
        assert_eq!(outcome.record.engine_pgid, None);
        assert_eq!(*killed.lock().unwrap(), vec![supervisor.id()]);

        let _ = supervisor.kill();
        let _ = supervisor.wait();
    }

    #[test]
    fn a_record_that_became_running_at_the_deadline_is_not_killed() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().to_path_buf());
        let id = ContainerId::parse("000000000006").unwrap();
        let name = ContainerName::parse("just-ready").unwrap();
        let endpoint = control_endpoint(dir.path(), &id);

        let mut record = ContainerRecord::new(
            id.clone(),
            name,
            String::from("freedos:latest"),
            String::from("mock"),
            endpoint.record(),
            Utc::now(),
        );
        record.status = Status::Running;
        record.pid = Some(4242);
        record.engine_pid = Some(4343);
        record.engine_pgid = Some(4343);
        store.create(&record).unwrap();

        let alive = Arc::new(AtomicBool::new(true));
        let killed = Arc::new(Mutex::new(Vec::new()));
        let probe = FakeProbe {
            alive: Arc::clone(&alive),
            killed: Arc::clone(&killed),
            endpoint_checks: Arc::new(Mutex::new(0)),
        };
        let processes = FakeProcesses {
            alive: Arc::clone(&alive),
            killed: Arc::clone(&killed),
        };

        let outcome = time_out(&store, &id, &probe, &processes).unwrap();

        assert!(!outcome.timed_out);
        assert_eq!(outcome.record.status, Status::Running);
        assert_eq!(outcome.record.pid, Some(4242));
        assert!(killed.lock().unwrap().is_empty());
    }

    #[test]
    fn a_stopped_record_waits_for_its_old_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let id = ContainerId::parse("000000000002").unwrap();
        let name = ContainerName::parse("stopped").unwrap();
        let endpoint = control_endpoint(dir.path(), &id);
        let mut record = ContainerRecord::new(
            id,
            name,
            String::from("freedos:latest"),
            String::from("mock"),
            endpoint.record(),
            Utc::now(),
        );
        record.status = Status::Stopped;

        let endpoint_checks = Arc::new(Mutex::new(0));
        let probe = FakeProbe {
            alive: Arc::new(AtomicBool::new(false)),
            killed: Arc::new(Mutex::new(Vec::new())),
            endpoint_checks: Arc::clone(&endpoint_checks),
        };

        let started = Instant::now();
        wait_for_previous_supervisor(&probe, &record, Duration::from_millis(50));

        assert!(*endpoint_checks.lock().unwrap() > 0);
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn an_old_supervisor_pid_is_preferred_to_the_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let id = ContainerId::parse("000000000003").unwrap();
        let name = ContainerName::parse("running").unwrap();
        let endpoint = control_endpoint(dir.path(), &id);
        let mut record = ContainerRecord::new(
            id,
            name,
            String::from("freedos:latest"),
            String::from("mock"),
            endpoint.record(),
            Utc::now(),
        );
        record.status = Status::Running;
        record.pid = Some(4242);

        let endpoint_checks = Arc::new(Mutex::new(0));
        let probe = FakeProbe {
            alive: Arc::new(AtomicBool::new(true)),
            killed: Arc::new(Mutex::new(Vec::new())),
            endpoint_checks: Arc::clone(&endpoint_checks),
        };

        let started = Instant::now();
        wait_for_previous_supervisor(&probe, &record, Duration::from_millis(50));

        assert_eq!(*endpoint_checks.lock().unwrap(), 0);
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn an_exited_start_only_needs_the_log_when_it_timed_out() {
        let dir = tempfile::tempdir().unwrap();
        let id = ContainerId::parse("000000000004").unwrap();
        let name = ContainerName::parse("exited").unwrap();
        let endpoint = control_endpoint(dir.path(), &id);
        let mut record = ContainerRecord::new(
            id,
            name,
            String::from("freedos:latest"),
            String::from("mock"),
            endpoint.record(),
            Utc::now(),
        );
        record.status = Status::Exited;

        let fast_exit = StartOutcome {
            record: record.clone(),
            timed_out: false,
        };
        assert!(!fast_exit.needs_log());

        let timed_out = StartOutcome {
            record,
            timed_out: true,
        };
        assert!(timed_out.needs_log());
    }
}
