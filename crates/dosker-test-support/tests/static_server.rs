use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;

use dosker_test_support::registry_site::StaticServer;

fn get(server: &StaticServer, path: &str) -> (u16, Vec<u8>) {
    let url = server.url();
    let authority = url.trim_start_matches("http://");
    let mut stream = TcpStream::connect(authority).expect("connect to the server");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .expect("write the request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read the response");

    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a header terminator");
    let head = String::from_utf8_lossy(&response[..split]);
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .expect("a status code");
    (status, response[split + 4..].to_vec())
}

fn fixture() -> (tempfile::TempDir, StaticServer) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    fs::write(dir.path().join("index.json"), b"{\"schema\":1}\n").expect("write the document");
    let server = StaticServer::start(PathBuf::from(dir.path())).expect("start the server");
    (dir, server)
}

#[test]
fn a_document_is_served() {
    let (_dir, server) = fixture();
    let (status, body) = get(&server, "/index.json");
    assert_eq!(status, 200);
    assert_eq!(body, b"{\"schema\":1}\n");
    assert_eq!(server.requests(), vec!["/index.json".to_owned()]);
}

#[test]
fn a_missing_path_is_404() {
    let (_dir, server) = fixture();
    let (status, _) = get(&server, "/missing.json");
    assert_eq!(status, 404);
}

#[test]
fn a_parent_path_is_404() {
    let (_dir, server) = fixture();
    let (status, _) = get(&server, "/../Cargo.toml");
    assert_eq!(status, 404);
}

#[test]
fn the_request_log_can_be_reset() {
    let (_dir, server) = fixture();
    let _ = get(&server, "/index.json");
    server.reset_requests();
    assert!(server.requests().is_empty());
}
