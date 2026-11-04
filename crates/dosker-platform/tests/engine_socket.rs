#![cfg(unix)]

use std::collections::HashSet;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use dosker_core::ContainerId;
use dosker_platform::{
    Endpoint, MAX_UNIX_SOCKET_PATH_BYTES, control, engine_socket_path, short_socket_dir,
};

fn long_root(dir: &tempfile::TempDir) -> PathBuf {
    let root = dir.path().join("r".repeat(200));
    std::fs::create_dir_all(&root).expect("create long root");
    root
}

#[test]
fn engine_socket_uses_the_container_directory() {
    let dir = tempfile::Builder::new()
        .prefix("ds")
        .tempdir_in("/tmp")
        .expect("create short temp directory");
    let id = ContainerId::generate();
    let expected = dir
        .path()
        .join("containers")
        .join(id.as_str())
        .join("agent.sock");

    assert!(expected.as_os_str().as_bytes().len() <= MAX_UNIX_SOCKET_PATH_BYTES);
    assert_eq!(engine_socket_path(dir.path(), &id, "agent"), expected);
}

#[test]
fn engine_socket_falls_back_for_a_long_root() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let root = long_root(&dir);
    let id = ContainerId::generate();

    let path = engine_socket_path(&root, &id, "qmp");

    assert_eq!(
        path,
        short_socket_dir().join(format!("{}-qmp.sock", id.as_str()))
    );
    assert_eq!(path.parent(), Some(short_socket_dir().as_path()));
}

#[test]
fn engine_socket_fallback_is_at_most_100_bytes() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let root = long_root(&dir);
    let id = ContainerId::generate();

    let path = engine_socket_path(&root, &id, "monitor");

    assert!(path.as_os_str().as_bytes().len() <= MAX_UNIX_SOCKET_PATH_BYTES);
}

#[test]
fn engine_socket_names_are_unique() {
    let dir = tempfile::Builder::new()
        .prefix("ds")
        .tempdir_in("/tmp")
        .expect("create short temp directory");
    let id = ContainerId::generate();
    let other = ContainerId::generate();
    let names = ["agent", "qmp", "monitor", "gdb"];

    let mut paths = HashSet::new();
    for name in names {
        paths.insert(engine_socket_path(dir.path(), &id, name));
        paths.insert(engine_socket_path(dir.path(), &other, name));
    }

    assert_eq!(paths.len(), names.len() * 2);
}

#[test]
fn engine_socket_is_usable_by_bind_and_connect() {
    let dir = tempfile::Builder::new()
        .prefix("ds")
        .tempdir_in("/tmp")
        .expect("create short temp directory");
    let path = engine_socket_path(dir.path(), &ContainerId::generate(), "agent");
    let endpoint = Endpoint::UnixSocket(path.clone());

    let listener = control::bind(&endpoint).expect("bind");
    let mut client = control::connect(&endpoint, Duration::from_secs(2)).expect("connect");
    let mut server = listener
        .accept(Duration::from_secs(2))
        .expect("accept")
        .expect("accepted connection");

    client.write_all(b"ping\n").expect("write request");
    client.flush().expect("flush request");

    let mut request = [0u8; 5];
    server
        .read_exact(&mut request)
        .expect("read the request line");
    assert_eq!(&request, b"ping\n");

    server.write_all(b"pong\n").expect("write response");
    server.flush().expect("flush response");

    let mut response = [0u8; 5];
    client
        .read_exact(&mut response)
        .expect("read the response line");
    assert_eq!(&response, b"pong\n");

    control::remove_endpoint(&endpoint).expect("remove endpoint");
    assert!(!Path::new(&path).exists());
}
