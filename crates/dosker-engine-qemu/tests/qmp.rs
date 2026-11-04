#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use dosker_engine::EngineError;
use dosker_engine_qemu::qmp::QmpClient;
use serde_json::{Value, json};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

struct ScriptedServer {
    _dir: tempfile::TempDir,
    path: PathBuf,
    commands: Arc<Mutex<Vec<Value>>>,
    _handle: JoinHandle<()>,
}

impl ScriptedServer {
    fn start(script: Vec<Vec<Value>>) -> Self {
        let dir = tempfile::tempdir().expect("create server directory");
        let path = dir.path().join("qmp.sock");
        let listener = UnixListener::bind(&path).expect("bind qmp socket");
        let commands = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&commands);

        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept qmp client");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set server read timeout");
            let mut reader = BufReader::new(stream);

            reader
                .get_mut()
                .write_all(
                    b"{\"QMP\":{\"version\":{\"qemu\":{\"major\":11,\"minor\":1,\"patch\":1}}}}\n",
                )
                .expect("write greeting");
            reader.get_mut().flush().expect("flush greeting");

            let mut line = String::new();
            if !read_command(&mut reader, &mut line, &recorded) {
                return;
            }
            reader
                .get_mut()
                .write_all(b"{\"return\":{}}\n")
                .expect("write capabilities response");

            for lines in script {
                line.clear();
                if !read_command(&mut reader, &mut line, &recorded) {
                    return;
                }

                for value in lines {
                    let mut encoded = serde_json::to_vec(&value).expect("encode response");
                    encoded.push(b'\n');
                    if reader.get_mut().write_all(&encoded).is_err() {
                        return;
                    }
                }
                if reader.get_mut().flush().is_err() {
                    return;
                }
            }
        });

        Self {
            _dir: dir,
            path,
            commands,
            _handle: handle,
        }
    }

    fn commands(&self) -> Vec<Value> {
        self.commands.lock().expect("lock commands").clone()
    }
}

fn read_command(
    reader: &mut BufReader<std::os::unix::net::UnixStream>,
    line: &mut String,
    recorded: &Arc<Mutex<Vec<Value>>>,
) -> bool {
    if reader.read_line(line).unwrap_or(0) == 0 {
        return false;
    }

    let value: Value = serde_json::from_str(line.trim()).expect("decode command");
    recorded.lock().expect("lock commands").push(value);
    true
}

fn client(server: &ScriptedServer) -> QmpClient {
    QmpClient::connect(&server.path, CONNECT_TIMEOUT).expect("connect to qmp")
}

#[test]
fn qmp_connect_sends_capabilities() {
    let server = ScriptedServer::start(Vec::new());

    let _client = client(&server);

    let commands = server.commands();
    assert_eq!(
        commands.first().and_then(|value| value.get("execute")),
        Some(&json!("qmp_capabilities"))
    );
}

#[test]
fn qmp_execute_returns_the_return_value() {
    let server = ScriptedServer::start(vec![vec![json!({
        "return": {"status": "running"}
    })]]);
    let mut client = client(&server);

    let value = client
        .execute("query-status", None)
        .expect("execute query-status");

    assert_eq!(value, json!({"status": "running"}));
    assert_eq!(
        server.commands()[1].get("execute"),
        Some(&json!("query-status"))
    );
}

#[test]
fn qmp_quit_sends_quit() {
    let server = ScriptedServer::start(vec![vec![json!({"return": {}})]]);
    let mut client = client(&server);

    client.quit().expect("quit");

    assert_eq!(server.commands()[1].get("execute"), Some(&json!("quit")));
}

#[test]
fn qmp_screendump_sends_filename_and_format() {
    let server = ScriptedServer::start(vec![vec![json!({"return": {}})]]);
    let mut client = client(&server);
    let destination = PathBuf::from("/tmp/dosker-shot.ppm");

    client
        .screendump(&destination, Some("ppm"))
        .expect("screendump");

    let command = &server.commands()[1];
    assert_eq!(command.get("execute"), Some(&json!("screendump")));
    assert_eq!(
        command
            .get("arguments")
            .and_then(|arguments| arguments.get("filename")),
        Some(&json!("/tmp/dosker-shot.ppm"))
    );
    assert_eq!(
        command
            .get("arguments")
            .and_then(|arguments| arguments.get("format")),
        Some(&json!("ppm"))
    );
}

#[test]
fn qmp_ignores_event_lines() {
    let server = ScriptedServer::start(vec![vec![
        json!({"event": "RESUME", "data": {}}),
        json!({"return": {"ok": true}}),
    ]]);
    let mut client = client(&server);

    let value = client.execute("cont", None).expect("execute cont");

    assert_eq!(value, json!({"ok": true}));
}

#[test]
fn qmp_surfaces_an_error_response() {
    let server = ScriptedServer::start(vec![vec![json!({
        "error": {"class": "GenericError", "desc": "the screen is not available"}
    })]]);
    let mut client = client(&server);

    match client.execute("screendump", None) {
        Err(EngineError::Protocol(message)) => {
            assert!(message.contains("the screen is not available"), "{message}");
        }
        other => panic!("expected a protocol error, got {other:?}"),
    }
}

#[test]
fn qmp_reports_a_closed_socket() {
    let server = ScriptedServer::start(vec![Vec::new()]);
    let mut client = client(&server);

    match client.execute("query-status", None) {
        Err(EngineError::Protocol(message)) => {
            assert!(message.contains("closed"), "{message}");
        }
        other => panic!("expected a protocol error, got {other:?}"),
    }
}
