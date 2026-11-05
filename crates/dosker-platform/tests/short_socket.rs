#![cfg(unix)]

use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use dosker_core::ContainerId;
use dosker_platform::endpoint::control_endpoint;
use dosker_platform::{
    Endpoint, MAX_UNIX_SOCKET_PATH_BYTES, control, short_socket_dir, short_socket_path,
};

fn long_root(dir: &tempfile::TempDir) -> PathBuf {
    let root = dir.path().join("r".repeat(200));
    std::fs::create_dir_all(&root).expect("create long root");
    root
}

fn unix_path(endpoint: &Endpoint) -> &Path {
    match endpoint {
        Endpoint::UnixSocket(path) => path,
        Endpoint::NamedPipe(name) => panic!("expected a unix socket endpoint, got {name}"),
    }
}

fn default_path(root: &Path, id: &ContainerId) -> PathBuf {
    root.join("containers")
        .join(id.as_str())
        .join("control.sock")
}

#[test]
fn short_default_path_is_used_when_it_fits() {
    let dir = tempfile::Builder::new()
        .prefix("ds")
        .tempdir_in("/tmp")
        .expect("create short temp directory");
    let id = ContainerId::generate();
    let expected = default_path(dir.path(), &id);

    assert!(expected.as_os_str().as_bytes().len() <= MAX_UNIX_SOCKET_PATH_BYTES);

    let endpoint = control_endpoint(dir.path(), &id);

    assert_eq!(endpoint, Endpoint::UnixSocket(expected));
}

#[test]
fn fallback_path_is_used_when_default_exceeds_limit() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let root = long_root(&dir);
    let id = ContainerId::generate();

    let endpoint = control_endpoint(&root, &id);

    let euid = unsafe { libc::geteuid() };
    let expected_dir = Path::new("/tmp").join(format!("dosker-{euid}"));
    assert_eq!(unix_path(&endpoint).parent(), Some(expected_dir.as_path()));
    assert_eq!(unix_path(&endpoint), short_socket_path(&id));
}

#[test]
fn fallback_path_is_at_most_100_bytes() {
    let path = short_socket_path(&ContainerId::generate());

    assert!(path.as_os_str().as_bytes().len() <= MAX_UNIX_SOCKET_PATH_BYTES);
}

#[test]
fn fallback_is_unique_per_id() {
    let first = short_socket_path(&ContainerId::generate());
    let second = short_socket_path(&ContainerId::generate());

    assert_ne!(first, second);
}

#[test]
fn fallback_round_trips_a_line() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let root = long_root(&dir);
    let endpoint = control_endpoint(&root, &ContainerId::generate());

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
}

#[test]
fn fallback_dir_is_owner_only() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let root = long_root(&dir);
    let endpoint = control_endpoint(&root, &ContainerId::generate());

    let _listener = control::bind(&endpoint).expect("bind");

    let mode = std::fs::metadata(short_socket_dir())
        .expect("fallback directory metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
}

#[test]
fn fallback_socket_file_is_owner_only() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let root = long_root(&dir);
    let id = ContainerId::generate();
    let endpoint = control_endpoint(&root, &id);

    let _listener = control::bind(&endpoint).expect("bind");

    let mode = std::fs::metadata(short_socket_path(&id))
        .expect("fallback socket metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn remove_endpoint_removes_the_fallback() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let root = long_root(&dir);
    let id = ContainerId::generate();
    let endpoint = control_endpoint(&root, &id);
    let path = short_socket_path(&id);

    let _listener = control::bind(&endpoint).expect("bind");
    assert!(control::endpoint_alive(&endpoint));

    control::remove_endpoint(&endpoint).expect("remove endpoint");

    assert!(!control::endpoint_alive(&endpoint));
    assert!(!path.exists());
}

#[test]
fn fallback_endpoint_record_round_trips() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let root = long_root(&dir);
    let endpoint = control_endpoint(&root, &ContainerId::generate());

    let record = endpoint.record();

    assert_eq!(
        Endpoint::try_from(record).expect("control endpoint"),
        endpoint
    );
}

#[test]
fn default_endpoint_with_a_long_root_is_not_used() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let root = long_root(&dir);
    let id = ContainerId::generate();
    let default = default_path(&root, &id);
    assert!(default.as_os_str().as_bytes().len() > MAX_UNIX_SOCKET_PATH_BYTES);

    let endpoint = control_endpoint(&root, &id);

    assert_ne!(endpoint, Endpoint::UnixSocket(default));
}
