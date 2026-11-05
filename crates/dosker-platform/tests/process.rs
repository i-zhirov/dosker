//! Process cleanup and detached spawn behavior.

use std::process::Command;
#[cfg(windows)]
use std::process::Stdio;
#[cfg(any(unix, windows))]
use std::thread;
#[cfg(any(unix, windows))]
use std::time::{Duration, Instant};

use dosker_platform::process::{ProcessSupervisor, system};

#[cfg(unix)]
#[test]
fn engine_child_gets_its_own_process_group() {
    let processes = system();
    let mut command = Command::new("sleep");
    command.arg("30");
    processes
        .prepare_engine(&mut command)
        .expect("prepare engine command");

    let mut child = command.spawn().expect("spawn sleep");
    let pid = child.id();

    let group = processes.process_group(pid).expect("read process group");
    assert_eq!(group, Some(pid));

    processes.kill_group(pid).expect("kill engine group");
    let _ = child.wait();
}

#[cfg(unix)]
#[test]
fn kill_group_terminates_every_member() {
    let processes = system();
    let directory = tempfile::tempdir().expect("temp directory");
    let pid_file = directory.path().join("sleep.pid");

    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("sleep 30 & echo $! > \"$1\"; wait")
        .arg("sh")
        .arg(&pid_file);
    processes
        .prepare_engine(&mut command)
        .expect("prepare engine command");

    let mut child = command.spawn().expect("spawn shell");
    let leader = child.id();
    let member = wait_for_pid(&pid_file);

    processes.kill_group(leader).expect("kill process group");

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut leader_reaped = false;
    while Instant::now() < deadline {
        if !leader_reaped {
            leader_reaped = child.try_wait().expect("poll group leader").is_some();
        }
        if leader_reaped && !processes.process_alive(leader) && !processes.process_alive(member) {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let _ = child.kill();
    let _ = child.wait();

    assert!(!processes.process_alive(leader), "group leader is alive");
    assert!(!processes.process_alive(member), "group member is alive");
}

#[cfg(unix)]
#[test]
fn kill_group_is_idempotent() {
    let processes = system();
    let mut command = Command::new("sleep");
    command.arg("30");
    processes
        .prepare_engine(&mut command)
        .expect("prepare engine command");

    let mut child = command.spawn().expect("spawn sleep");
    let pid = child.id();

    processes.kill_group(pid).expect("first kill");
    let _ = child.wait();
    processes.kill_group(pid).expect("second kill");
}

#[test]
fn process_alive_is_false_after_reap() {
    let processes = system();
    let mut command = trivial_command();
    let mut child = command.spawn().expect("spawn trivial child");
    let pid = child.id();

    let status = child.wait().expect("wait for trivial child");
    assert!(status.success());

    assert!(!processes.process_alive(pid));
}

#[test]
fn process_alive_is_true_for_current_process() {
    assert!(system().process_alive(std::process::id()));
}

#[cfg(windows)]
#[test]
fn job_object_kills_an_assigned_process_when_dropped() {
    use dosker_platform::process::JobObject;

    let job = JobObject::kill_on_close().expect("create the job object");
    let mut child = windows_long_running_command()
        .spawn()
        .expect("spawn a long-running child");
    let pid = child.id();
    job.assign_child(&child).expect("assign the child");

    assert!(
        system().process_alive(pid),
        "the assigned process is not alive"
    );

    drop(job);

    assert!(
        wait_until_process_exits(pid, Duration::from_secs(5)),
        "the job object did not kill the assigned process"
    );
    let _ = child.wait();
}

#[cfg(windows)]
#[test]
fn job_object_keeps_an_assigned_process_alive_while_open() {
    use dosker_platform::process::JobObject;

    let job = JobObject::kill_on_close().expect("create the job object");
    let mut child = windows_long_running_command()
        .spawn()
        .expect("spawn a long-running child");
    let pid = child.id();
    job.assign_child(&child).expect("assign the child");

    thread::sleep(Duration::from_millis(200));

    assert!(
        system().process_alive(pid),
        "the open job object ended the assigned process"
    );
    assert!(
        child.try_wait().expect("poll the child").is_none(),
        "the assigned process exited while the job object was open"
    );

    let _ = child.kill();
    let _ = child.wait();
    drop(job);
}

#[cfg(windows)]
#[test]
fn kill_group_terminates_the_named_process_on_windows() {
    let processes = system();
    let mut child = windows_long_running_command()
        .spawn()
        .expect("spawn a long-running child");
    let pid = child.id();

    processes.kill_group(pid).expect("kill the named process");

    assert!(
        wait_until_process_exits(pid, Duration::from_secs(5)),
        "kill_group did not terminate the process"
    );
    let _ = child.wait();
}

#[cfg(unix)]
fn trivial_command() -> Command {
    Command::new("true")
}

#[cfg(windows)]
fn trivial_command() -> Command {
    let mut command = Command::new("cmd");
    command.args(["/C", "exit", "0"]);
    command
}

#[cfg(windows)]
fn windows_long_running_command() -> Command {
    let mut command = Command::new("cmd");
    command.args(["/C", "ping -n 30 127.0.0.1 > NUL"]);
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command
}

#[cfg(windows)]
fn wait_until_process_exits(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !system().process_alive(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }

    !system().process_alive(pid)
}

#[cfg(unix)]
fn wait_for_pid(path: &std::path::Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(text) = std::fs::read_to_string(path)
            && let Ok(pid) = text.trim().parse::<u32>()
        {
            return pid;
        }
        assert!(Instant::now() < deadline, "child pid file was not written");
        thread::sleep(Duration::from_millis(10));
    }
}
