use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener};
#[cfg(unix)]
use std::os::unix::net::UnixListener;
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use dosker_engine::EngineEndpointKind;
#[cfg(unix)]
use dosker_engine::EngineError;
use dosker_engine_qemu::qmp::QmpClient;
use serde_json::{Value, json};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

struct ScriptedServer {
    _dir: Option<tempfile::TempDir>,
    kind: EngineEndpointKind,
    commands: Arc<Mutex<Vec<Value>>>,
    _handle: JoinHandle<()>,
}

impl ScriptedServer {
    #[cfg(unix)]
    fn start_unix(script: Vec<Vec<Value>>) -> Self {
        let dir = tempfile::tempdir().expect("create server directory");
        let path = dir.path().join("qmp.sock");
        let listener = UnixListener::bind(&path).expect("bind qmp socket");
        let commands = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&commands);

        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept qmp client");
            serve(stream, script, recorded);
        });

        Self {
            _dir: Some(dir),
            kind: EngineEndpointKind::UnixSocket(path),
            commands,
            _handle: handle,
        }
    }

    fn start_tcp(script: Vec<Vec<Value>>) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind qmp tcp socket");
        let port = listener.local_addr().expect("read tcp address").port();
        let commands = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&commands);

        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept qmp client");
            serve(stream, script, recorded);
        });

        Self {
            _dir: None,
            kind: EngineEndpointKind::Tcp {
                host: Ipv4Addr::LOCALHOST,
                port,
            },
            commands,
            _handle: handle,
        }
    }

    fn kind(&self) -> EngineEndpointKind {
        self.kind.clone()
    }

    fn commands(&self) -> Vec<Value> {
        self.commands.lock().expect("lock commands").clone()
    }
}

fn serve<S>(stream: S, script: Vec<Vec<Value>>, recorded: Arc<Mutex<Vec<Value>>>)
where
    S: Read + Write,
{
    let mut reader = BufReader::new(stream);

    reader
        .get_mut()
        .write_all(b"{\"QMP\":{\"version\":{\"qemu\":{\"major\":11,\"minor\":1,\"patch\":1}}}}\n")
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
}

fn read_command<R: BufRead>(
    reader: &mut R,
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
    QmpClient::connect(&server.kind(), CONNECT_TIMEOUT).expect("connect to qmp")
}

#[cfg(unix)]
#[test]
fn qmp_connect_sends_capabilities() {
    let server = ScriptedServer::start_unix(Vec::new());

    let _client = client(&server);

    let commands = server.commands();
    assert_eq!(
        commands.first().and_then(|value| value.get("execute")),
        Some(&json!("qmp_capabilities"))
    );
}

#[cfg(unix)]
#[test]
fn qmp_execute_returns_the_return_value() {
    let server = ScriptedServer::start_unix(vec![vec![json!({
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

#[cfg(unix)]
#[test]
fn qmp_quit_sends_quit() {
    let server = ScriptedServer::start_unix(vec![vec![json!({"return": {}})]]);
    let mut client = client(&server);

    client.quit().expect("quit");

    assert_eq!(server.commands()[1].get("execute"), Some(&json!("quit")));
}

#[cfg(unix)]
#[test]
fn qmp_screendump_sends_filename_and_format() {
    let server = ScriptedServer::start_unix(vec![vec![json!({"return": {}})]]);
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

#[cfg(unix)]
#[test]
fn qmp_ignores_event_lines() {
    let server = ScriptedServer::start_unix(vec![vec![
        json!({"event": "RESUME", "data": {}}),
        json!({"return": {"ok": true}}),
    ]]);
    let mut client = client(&server);

    let value = client.execute("cont", None).expect("execute cont");

    assert_eq!(value, json!({"ok": true}));
}

#[cfg(unix)]
#[test]
fn qmp_surfaces_an_error_response() {
    let server = ScriptedServer::start_unix(vec![vec![json!({
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

#[cfg(unix)]
#[test]
fn qmp_reports_a_closed_socket() {
    let server = ScriptedServer::start_unix(vec![Vec::new()]);
    let mut client = client(&server);

    match client.execute("query-status", None) {
        Err(EngineError::Protocol(message)) => {
            assert!(message.contains("closed"), "{message}");
        }
        other => panic!("expected a protocol error, got {other:?}"),
    }
}

#[test]
fn qmp_connects_over_loopback_tcp() {
    let server = ScriptedServer::start_tcp(vec![vec![json!({
        "return": {"status": "running"}
    })]]);
    let mut client = client(&server);

    let value = client
        .execute("query-status", None)
        .expect("execute query-status over TCP");

    assert_eq!(value, json!({"status": "running"}));
    assert_eq!(
        server
            .commands()
            .first()
            .and_then(|value| value.get("execute")),
        Some(&json!("qmp_capabilities"))
    );
}
