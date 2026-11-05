mod common;

use std::fs;

use common::{CliRoot, stderr_text, stdout_text};
use dosker_core::{ContainerId, Status};
use dosker_runtime::MAX_CP_BYTES;

#[cfg(unix)]
use std::io::BufReader;
#[cfg(unix)]
use std::process::Command;
#[cfg(unix)]
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use dosker_core::ContainerName;
#[cfg(unix)]
use dosker_platform::Endpoint;
#[cfg(unix)]
use dosker_platform::control::bind;
#[cfg(unix)]
use dosker_platform::endpoint::control_endpoint;
#[cfg(unix)]
use dosker_runtime::protocol::{FrameRead, Response, read_frame, write_message};
#[cfg(unix)]
use dosker_runtime::{ContainerRecord, StateStore};

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

#[cfg(unix)]
fn seed_running(root: &CliRoot, id: &str, name: &str) -> ContainerId {
    let id = ContainerId::parse(id).expect("parse the container id");
    let name = ContainerName::parse(name).expect("parse the container name");
    let endpoint = control_endpoint(root.path(), &id);
    let mut record = ContainerRecord::new(
        id.clone(),
        name,
        String::from("freedos:latest"),
        String::from("mock"),
        endpoint.record(),
        chrono::Utc::now(),
    );
    record.status = Status::Running;
    record.pid = Some(
        Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("start a stand-in supervisor")
            .id(),
    );
    StateStore::new(root.path().to_path_buf())
        .create(&record)
        .expect("create the running record");
    id
}

#[cfg(unix)]
fn spawn_stub(endpoint: &Endpoint, reply: Option<Response>) -> Arc<Mutex<Vec<String>>> {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);
    let listener = bind(endpoint).expect("bind the stub endpoint");
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            match listener.accept(Duration::from_millis(50)) {
                Ok(Some(connection)) => {
                    let mut reader = BufReader::new(connection);
                    if let Ok(FrameRead::Line(line)) = read_frame(&mut reader) {
                        recorded.lock().unwrap().push(line);
                        if let Some(response) = &reply {
                            let _ = write_message(reader.get_mut(), response);
                        }
                    }
                }
                Ok(None) => {}
                Err(_) => break,
            }
        }
    });
    requests
}

#[test]
fn cp_uploads_a_host_file_to_the_guest() {
    let root = CliRoot::new();
    let id = root.start();
    let host = tempfile::tempdir().expect("create a host directory");
    let source = host.path().join("hello.txt");
    fs::write(&source, b"HELLO").expect("write the host file");

    let guest = r"C:\DOSKER\HELLO.TXT";
    let output = root.run(&[
        "cp",
        source.to_str().expect("a utf-8 path"),
        &format!("{id}:{guest}"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(stdout_text(&output).contains("copied 5 bytes"));

    let destination = host.path().join("round-trip.txt");
    let output = root.run(&[
        "cp",
        &format!("{id}:{guest}"),
        destination.to_str().expect("a utf-8 path"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(fs::read(&destination).expect("read the download"), b"HELLO");
}

#[test]
fn cp_downloads_a_guest_file_to_the_host() {
    let root = CliRoot::new();
    let id = root.start();
    let host = tempfile::tempdir().expect("create a host directory");
    let source = host.path().join("round.txt");
    fs::write(&source, b"ROUND TRIP").expect("write the host file");

    let guest = r"C:\DOSKER\ROUND.TXT";
    let upload = root.run(&[
        "cp",
        source.to_str().expect("a utf-8 path"),
        &format!("{id}:{guest}"),
    ]);
    assert_eq!(
        upload.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&upload)
    );

    let destination = host.path().join("round-download.txt");
    let download = root.run(&[
        "cp",
        &format!("{id}:{guest}"),
        destination.to_str().expect("a utf-8 path"),
    ]);
    assert_eq!(
        download.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&download)
    );
    assert_eq!(
        fs::read(&destination).expect("read the download"),
        b"ROUND TRIP"
    );
}

#[test]
fn cp_rejects_two_host_paths() {
    let root = CliRoot::new();
    let output = root.run(&["cp", "one.txt", "two.txt"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn cp_rejects_two_container_paths() {
    let root = CliRoot::new();
    let id = root.start();
    let output = root.run(&[
        "cp",
        &format!("{id}:C:\\ONE.TXT"),
        &format!("{id}:C:\\TWO.TXT"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn cp_rejects_an_empty_container_path() {
    let root = CliRoot::new();
    let id = root.start();

    let source = root.run(&["cp", &format!("{id}:"), "host.txt"]);
    assert_eq!(
        source.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&source)
    );

    let destination = root.run(&["cp", "host.txt", &format!("{id}:")]);
    assert_eq!(
        destination.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&destination)
    );
}

#[cfg(not(windows))]
#[test]
fn cp_reports_a_missing_container_reference() {
    use dosker_cli::commands::cp::parse_operand;
    use dosker_cli::context::Context;
    use dosker_platform::console::Console;

    let root = CliRoot::new();
    let context = Context::with_console(root.path().to_path_buf(), Console::std());

    let error = parse_operand(&context, r"000000000999:C:\MISSING.TXT")
        .expect_err("a missing container must not become a host path");

    assert!(
        error.to_string().contains("container not found"),
        "error: {error}"
    );
}

#[cfg(not(windows))]
#[test]
fn cp_treats_a_non_reference_colon_path_as_host() {
    use std::path::Path;

    use dosker_cli::commands::cp::{CpOperand, parse_operand};
    use dosker_cli::context::Context;
    use dosker_platform::console::Console;

    let root = CliRoot::new();
    let context = Context::with_console(root.path().to_path_buf(), Console::std());

    let operand = parse_operand(&context, "/tmp/a:b").expect("parse the host path");

    assert!(
        matches!(operand, CpOperand::Host(path) if path == Path::new("/tmp/a:b")),
        "a non-reference prefix must stay a host path"
    );
}

#[cfg(unix)]
#[test]
fn cp_missing_host_file_exits_one_without_a_request() {
    let root = CliRoot::new();
    let id = seed_running(&root, "000000000201", "cp-missing-host");
    let endpoint = control_endpoint(root.path(), &id);
    let requests = spawn_stub(&endpoint, None);
    let missing = root.path().join("missing.txt");

    let output = root.run(&[
        "cp",
        missing.to_str().expect("a utf-8 path"),
        &format!("{id}:C:\\MISSING.TXT"),
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_text(&output).contains("missing.txt"),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        requests.lock().unwrap().is_empty(),
        "a request was sent: {:?}",
        requests.lock().unwrap()
    );
}

#[test]
fn cp_rejects_a_file_over_the_limit() {
    let root = CliRoot::new();
    let id = root.start();
    let host = tempfile::tempdir().expect("create a host directory");
    let source = host.path().join("large.bin");
    let file = fs::File::create(&source).expect("create the source file");
    file.set_len(MAX_CP_BYTES + 1)
        .expect("extend the source file");

    let output = root.run(&[
        "cp",
        source.to_str().expect("a utf-8 path"),
        &format!("{id}:C:\\LARGE.BIN"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(stderr_text(&output).contains("larger than"));
}

#[cfg(unix)]
#[test]
fn cp_rejects_an_oversized_source_before_sending() {
    let root = CliRoot::new();
    let id = seed_running(&root, "000000000202", "cp-oversized");
    let endpoint = control_endpoint(root.path(), &id);
    let requests = spawn_stub(&endpoint, None);
    let host = tempfile::tempdir().expect("create a host directory");
    let source = host.path().join("large.bin");
    let file = fs::File::create(&source).expect("create the source file");
    file.set_len(MAX_CP_BYTES + 1)
        .expect("extend the source file");

    let output = root.run(&[
        "cp",
        source.to_str().expect("a utf-8 path"),
        &format!("{id}:C:\\LARGE.BIN"),
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn cp_transfers_a_non_trivial_file() {
    let root = CliRoot::new();
    let id = root.start();
    let host = tempfile::tempdir().expect("create a host directory");
    let source = host.path().join("block.bin");
    let payload: Vec<u8> = (0..256 * 1024).map(|index| (index % 251) as u8).collect();
    fs::write(&source, &payload).expect("write the source file");

    let guest = r"C:\DOSKER\BLOCK.BIN";
    let upload = root.run(&[
        "cp",
        source.to_str().expect("a utf-8 path"),
        &format!("{id}:{guest}"),
    ]);
    assert_eq!(
        upload.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&upload)
    );

    let destination = host.path().join("block-download.bin");
    let download = root.run(&[
        "cp",
        &format!("{id}:{guest}"),
        destination.to_str().expect("a utf-8 path"),
    ]);
    assert_eq!(
        download.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&download)
    );
    assert_eq!(fs::read(&destination).expect("read the download"), payload);
}

#[cfg(windows)]
#[test]
fn cp_treats_a_windows_drive_path_as_host() {
    use dosker_cli::commands::cp::{CpOperand, parse_operand};
    use dosker_cli::context::Context;
    use dosker_core::{ContainerId, ContainerName};
    use dosker_platform::console::Console;
    use dosker_platform::endpoint::control_endpoint;
    use dosker_runtime::ContainerRecord;

    let root = CliRoot::new();
    let id = ContainerId::parse("000000000203").expect("parse the container id");
    let name = ContainerName::parse("c").expect("parse the container name");
    let endpoint = control_endpoint(root.path(), &id);
    let record = ContainerRecord::new(
        id,
        name,
        String::from("freedos:latest"),
        String::from("mock"),
        endpoint.record(),
        chrono::Utc::now(),
    );
    root.store().create(&record).expect("create the record");

    let context = Context::with_console(root.path().to_path_buf(), Console::std());
    let operand = parse_operand(&context, r"C:\TEMP\FILE.TXT").expect("parse the operand");

    assert!(
        matches!(operand, CpOperand::Host(path) if path.as_path() == std::path::Path::new(r"C:\TEMP\FILE.TXT"))
    );
}

#[test]
fn cp_rejects_a_booter_with_no_agent() {
    let root = CliRoot::new();
    let id = start_booter(&root);
    let host = tempfile::tempdir().expect("create a host directory");
    let source = host.path().join("hello.txt");
    fs::write(&source, b"HELLO").expect("write the host file");

    let output = root.run(&[
        "cp",
        source.to_str().expect("a utf-8 path"),
        &format!("{id}:C:\\HELLO.TXT"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(125),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
}

#[test]
fn cp_missing_guest_file_exits_one_and_keeps_running() {
    let root = CliRoot::new();
    let id = root.start();
    let host = tempfile::tempdir().expect("create a host directory");
    let destination = host.path().join("missing.txt");

    let output = root.run(&[
        "cp",
        &format!("{id}:C:\\MISSING.TXT"),
        destination.to_str().expect("a utf-8 path"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains(r"C:\MISSING.TXT"),
        "stderr: {}",
        stderr_text(&output)
    );
    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.status, Status::Running);
}

#[test]
fn cp_host_destination_write_failure_exits_one_and_keeps_running() {
    let root = CliRoot::new();
    let id = root.start();
    let host = tempfile::tempdir().expect("create a host directory");
    let source = host.path().join("hello.txt");
    fs::write(&source, b"HELLO").expect("write the host file");
    let guest = r"C:\DOSKER\HELLO.TXT";
    let upload = root.run(&[
        "cp",
        source.to_str().expect("a utf-8 path"),
        &format!("{id}:{guest}"),
    ]);
    assert_eq!(upload.status.code(), Some(0));

    let destination = host.path().join("missing").join("hello.txt");
    let output = root.run(&[
        "cp",
        &format!("{id}:{guest}"),
        destination.to_str().expect("a utf-8 path"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.status, Status::Running);
}
