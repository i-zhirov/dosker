use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

use dosker_core::ContainerId;
use dosker_platform::{Endpoint, EndpointRecord};

#[cfg(unix)]
#[test]
fn unix_endpoint_uses_container_directory() {
    let root = Path::new("/tmp/dosker-test-root");
    let id = ContainerId::parse("0123456789ab").unwrap();

    assert_eq!(
        dosker_platform::endpoint::control_endpoint(root, &id),
        Endpoint::UnixSocket(
            root.join("containers")
                .join("0123456789ab")
                .join("control.sock")
        )
    );
}

#[cfg(windows)]
#[test]
fn named_pipe_name_uses_container_id() {
    let root = Path::new(r"C:\dosker");
    let id = ContainerId::parse("0123456789ab").unwrap();

    assert_eq!(
        dosker_platform::endpoint::control_endpoint(root, &id),
        Endpoint::NamedPipe(r"\\.\pipe\dosker-0123456789ab-control".to_owned())
    );
}

#[test]
fn endpoint_record_round_trip() {
    #[cfg(unix)]
    let endpoint = Endpoint::UnixSocket(PathBuf::from(
        "/tmp/dosker/containers/0123456789ab/control.sock",
    ));
    #[cfg(windows)]
    let endpoint = Endpoint::NamedPipe(r"\\.\pipe\dosker-0123456789ab-control".to_owned());

    let record = endpoint.record();

    assert_eq!(Endpoint::from(record), endpoint);
}

#[test]
fn endpoint_record_serializes_with_kind_tag() {
    #[cfg(unix)]
    {
        let record: EndpointRecord =
            Endpoint::UnixSocket(PathBuf::from("/tmp/dosker/control.sock")).record();

        assert_eq!(
            serde_json::to_value(record).unwrap(),
            serde_json::json!({
                "kind": "unix_socket",
                "path": "/tmp/dosker/control.sock",
            })
        );
    }

    #[cfg(windows)]
    {
        let record: EndpointRecord =
            Endpoint::NamedPipe(r"\\.\pipe\dosker-0123456789ab-control".to_owned()).record();

        assert_eq!(
            serde_json::to_value(record).unwrap(),
            serde_json::json!({
                "kind": "named_pipe",
                "name": r"\\.\pipe\dosker-0123456789ab-control",
            })
        );
    }
}
