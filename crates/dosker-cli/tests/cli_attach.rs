mod common;

use std::fs;
use std::io::{self, BufReader, Cursor, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{CliRoot, stderr_text, stdout_text};
use dosker_cli::commands::attach::{LineShell, ShellAction, run_shell};
use dosker_core::{ContainerId, Status};
use dosker_platform::console::Console;
use dosker_platform::control::bind;
use dosker_platform::endpoint::control_endpoint;
use dosker_runtime::protocol::{FrameRead, Reply, Request, Response, read_frame, write_message};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Capture {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

fn console() -> (Console, Capture) {
    let stdout = Capture::default();
    let stderr = Capture::default();
    (
        Console::new(Box::new(stdout.clone()), Box::new(stderr)),
        stdout,
    )
}

fn import_booter(root: &CliRoot) -> String {
    let source = root.path().join("booter.img");
    fs::write(&source, b"not a filesystem").expect("write a booter blob");
    let output = root.run(&[
        "image",
        "import",
        "--booter",
        source.to_str().expect("source path is utf-8"),
        "booter-test:latest",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    String::from("booter-test:latest")
}

fn start_booter(root: &CliRoot) -> ContainerId {
    let image = import_booter(root);
    let output = root.run(&["run", "-d", image.as_str()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    ContainerId::parse(stdout_text(&output).trim()).expect("parse the container id")
}

#[test]
fn attach_runs_a_command_and_exits_on_exit() {
    let root = CliRoot::new();
    let id = root.start();

    let output = root.spawn_with_stdin(
        root.command(),
        &["attach", id.as_str()],
        "ECHO HELLO\nexit\n",
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(stdout_text(&output).contains("HELLO"));

    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.status, Status::Running);
}

#[test]
fn attach_survives_an_idle_pause() {
    let root = CliRoot::new();
    let id = root.start();

    let output = root.spawn_with_stdin_chunks(
        root.command(),
        &["attach", id.as_str()],
        &[
            (Duration::from_secs(6), "ECHO HELLO\n"),
            (Duration::ZERO, "exit\n"),
        ],
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(stdout_text(&output).contains("HELLO"));
}

#[test]
fn attach_runs_a_slow_command() {
    let root = CliRoot::new();
    let id = root.start();

    let output = root.spawn_with_stdin(
        root.command(),
        &["attach", id.as_str()],
        "SLEEP 1000\nexit\n",
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn attach_rejects_a_booter() {
    let root = CliRoot::new();
    let id = start_booter(&root);

    let output = root.spawn_with_stdin(root.command(), &["attach", id.as_str()], "exit\n");

    assert_eq!(
        output.status.code(),
        Some(125),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
}

#[test]
fn attach_rejects_a_stopped_container() {
    let root = CliRoot::new();
    let id = root.start();
    let stopped = root.run(&["stop", id.as_str()]);
    assert_eq!(stopped.status.code(), Some(0));

    let output = root.spawn_with_stdin(root.command(), &["attach", id.as_str()], "exit\n");

    assert_eq!(
        output.status.code(),
        Some(125),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn cd_updates_the_workdir() {
    let mut shell = LineShell::new(r"C:\");

    assert_eq!(
        shell.handle(r"cd C:\APP"),
        ShellAction::Exec {
            command: String::new(),
            workdir: String::from(r"C:\APP"),
        }
    );
    assert_eq!(
        shell.handle("DIR"),
        ShellAction::Exec {
            command: String::from("DIR"),
            workdir: String::from(r"C:\APP"),
        }
    );
}

#[test]
fn exit_detaches() {
    let mut shell = LineShell::new(r"C:\");

    assert_eq!(shell.handle("exit"), ShellAction::Detach);
    assert_eq!(shell.handle("quit"), ShellAction::Detach);
    assert_eq!(shell.handle("detach"), ShellAction::Detach);
}

#[test]
fn blank_line_is_ignored() {
    let mut shell = LineShell::new(r"C:\APP");

    assert_eq!(
        shell.handle(""),
        ShellAction::Exec {
            command: String::new(),
            workdir: String::from(r"C:\APP"),
        }
    );
}

#[test]
fn run_shell_returns_timeout_when_the_guest_times_out() {
    let dir = tempfile::tempdir().expect("create a temp directory");
    let id = ContainerId::parse("000000000102").expect("parse the id");
    let endpoint = control_endpoint(dir.path(), &id);
    let record = endpoint.record();
    let listener = bind(&endpoint).expect("bind the scripted endpoint");

    let worker = std::thread::spawn(move || {
        if let Ok(Some(connection)) = listener.accept(Duration::from_secs(5)) {
            let mut reader = BufReader::new(connection);
            let Ok(FrameRead::Line(_)) = read_frame(&mut reader) else {
                return;
            };
            let _ = write_message(reader.get_mut(), &Response::ok(Reply::Attach));

            let Ok(FrameRead::Line(_)) = read_frame(&mut reader) else {
                return;
            };
            let response = Response::ok(Reply::Exec {
                exit_code: 0,
                output: String::new(),
                timed_out: true,
                truncated: false,
            });
            let _ = write_message(reader.get_mut(), &response);
        }
    });

    let mut input = Cursor::new("SLEEP 1000\nexit\n");
    let (mut console, _capture) = console();
    let code = run_shell(&record, r"C:\", &mut input, &mut console).expect("run the shell");

    assert_eq!(code, 124);
    worker.join().expect("the scripted endpoint finishes");
}

#[test]
fn run_shell_writes_guest_output_verbatim() {
    let dir = tempfile::tempdir().expect("create a temp directory");
    let id = ContainerId::parse("000000000103").expect("parse the id");
    let endpoint = control_endpoint(dir.path(), &id);
    let record = endpoint.record();
    let listener = bind(&endpoint).expect("bind the scripted endpoint");

    let worker = std::thread::spawn(move || {
        if let Ok(Some(connection)) = listener.accept(Duration::from_secs(5)) {
            let mut reader = BufReader::new(connection);
            let Ok(FrameRead::Line(_)) = read_frame(&mut reader) else {
                return;
            };
            let _ = write_message(reader.get_mut(), &Response::ok(Reply::Attach));

            let Ok(FrameRead::Line(_)) = read_frame(&mut reader) else {
                return;
            };
            let response = Response::ok(Reply::Exec {
                exit_code: 0,
                output: String::from("HELLO"),
                timed_out: false,
                truncated: false,
            });
            let _ = write_message(reader.get_mut(), &response);
        }
    });

    let mut input = Cursor::new("ECHO HELLO\nexit\n");
    let (mut console, capture) = console();
    let code = run_shell(&record, r"C:\", &mut input, &mut console).expect("run the shell");

    assert_eq!(code, 0);
    assert!(
        capture.text().contains(r"HELLOC:\>"),
        "the guest output was not written verbatim: {:?}",
        capture.text()
    );
    worker.join().expect("the scripted endpoint finishes");
}

#[cfg(unix)]
#[test]
fn run_shell_reports_an_unreachable_endpoint_as_not_running() {
    let dir = tempfile::tempdir().expect("create a temp directory");
    let id = ContainerId::parse("000000000104").expect("parse the id");
    let endpoint = control_endpoint(dir.path(), &id);
    let record = endpoint.record();

    let mut input = Cursor::new("exit\n");
    let (mut console, _capture) = console();
    let code = run_shell(&record, r"C:\", &mut input, &mut console).expect("run the shell");

    assert_eq!(code, 125);
}

#[test]
fn run_shell_reads_scripted_input() {
    let dir = tempfile::tempdir().expect("create a temp directory");
    let id = ContainerId::parse("000000000101").expect("parse the id");
    let endpoint = control_endpoint(dir.path(), &id);
    let record = endpoint.record();
    let listener = bind(&endpoint).expect("bind the scripted endpoint");

    let worker = std::thread::spawn(move || {
        if let Ok(Some(connection)) = listener.accept(Duration::from_secs(5)) {
            let mut reader = BufReader::new(connection);
            let Ok(FrameRead::Line(line)) = read_frame(&mut reader) else {
                return;
            };
            assert_eq!(
                serde_json::from_str::<Request>(&line).expect("parse the attach request"),
                Request::Attach
            );
            let _ = write_message(reader.get_mut(), &Response::ok(Reply::Attach));

            let Ok(FrameRead::Line(line)) = read_frame(&mut reader) else {
                return;
            };
            match serde_json::from_str::<Request>(&line).expect("parse the exec request") {
                Request::Exec { command, .. } => assert_eq!(command, "ECHO HELLO"),
                other => panic!("expected an exec request, got {other:?}"),
            }
            let response = Response::ok(Reply::Exec {
                exit_code: 0,
                output: String::from("HELLO\r\n"),
                timed_out: false,
                truncated: false,
            });
            let _ = write_message(reader.get_mut(), &response);
        }
    });

    let mut input = Cursor::new("ECHO HELLO\nexit\n");
    let (mut console, capture) = console();
    let code = run_shell(&record, r"C:\", &mut input, &mut console).expect("run the shell");

    assert_eq!(code, 0);
    assert!(capture.text().contains("HELLO"));
    worker.join().expect("the scripted endpoint finishes");
}
