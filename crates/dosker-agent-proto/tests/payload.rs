use std::collections::{BTreeMap, BTreeSet};

use dosker_agent_proto::{
    Attributes, ErrorCode, ErrorPayload, ExecEnd, ExecRequest, GetEnd, GetRequest, Hello, HelloAck,
    Pong, PutEnd, PutRequest, ShutdownAck, ShutdownRequest, StatRequest, StatResponse,
    decode_payload, encode_payload,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

fn json(value: impl Serialize) -> Value {
    serde_json::to_value(value).expect("value serializes")
}

fn json_keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .expect("value is an object")
        .keys()
        .map(String::as_str)
        .collect()
}

fn expected<'a>(names: &'a [&'a str]) -> BTreeSet<&'a str> {
    names.iter().copied().collect()
}

fn hello() -> Hello {
    Hello {
        protocol_version: 1,
        agent_version: "1.0.0".to_owned(),
        dos: "FreeDOS 1.3".to_owned(),
        memory_kb: 640,
        control_port: 2,
        max_payload: 4096,
        features: vec![
            "exec".to_owned(),
            "put".to_owned(),
            "get".to_owned(),
            "stat".to_owned(),
        ],
    }
}

fn exec_request() -> ExecRequest {
    ExecRequest {
        command: "TEST.EXE".to_owned(),
        env: BTreeMap::from([("PATH".to_owned(), "C:\\APP;C:\\DOS".to_owned())]),
        workdir: "C:\\APP".to_owned(),
        max_output_bytes: 1_048_576,
    }
}

fn exec_end() -> ExecEnd {
    ExecEnd {
        exit_code: 0,
        output_bytes: 12,
        truncated: false,
        duration_ms: 340,
        command_not_found: false,
    }
}

fn put_request() -> PutRequest {
    PutRequest {
        path: "C:\\APP\\DATA.BIN".to_owned(),
        size: 12_345,
        overwrite: true,
    }
}

fn attributes() -> Attributes {
    Attributes {
        read_only: false,
        hidden: false,
        system: false,
        archive: true,
    }
}

fn stat_response() -> StatResponse {
    StatResponse {
        exists: true,
        size: 12_345,
        directory: false,
        attributes: attributes(),
        timestamp: Some("1996-03-14T12:00:00Z".to_owned()),
    }
}

fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) {
    let bytes = encode_payload(value).expect("payload encodes");
    let decoded: T = decode_payload(&bytes).expect("payload decodes");

    assert!(decoded == *value);
}

#[test]
fn hello_field_names_are_exact() {
    assert_eq!(
        json_keys(&json(hello())),
        expected(&[
            "protocol_version",
            "agent_version",
            "dos",
            "memory_kb",
            "control_port",
            "max_payload",
            "features",
        ])
    );
}

#[test]
fn hello_ack_field_name_is_exact() {
    let value = json(HelloAck {
        session_id: "b1f0c2".to_owned(),
    });

    assert_eq!(json_keys(&value), expected(&["session_id"]));
    assert_eq!(value["session_id"], "b1f0c2");
}

#[test]
fn exec_request_field_names_are_exact() {
    let value = json(exec_request());

    assert_eq!(
        json_keys(&value),
        expected(&["command", "env", "workdir", "max_output_bytes"])
    );
    assert_eq!(value["env"]["PATH"], "C:\\APP;C:\\DOS");
}

#[test]
fn exec_end_field_names_are_exact() {
    assert_eq!(
        json_keys(&json(exec_end())),
        expected(&[
            "exit_code",
            "output_bytes",
            "truncated",
            "duration_ms",
            "command_not_found",
        ])
    );
}

#[test]
fn put_request_field_names_are_exact() {
    assert_eq!(
        json_keys(&json(put_request())),
        expected(&["path", "size", "overwrite"])
    );
}

#[test]
fn put_end_field_names_are_exact() {
    assert_eq!(
        json_keys(&json(PutEnd {
            ok: true,
            bytes: 12_345,
        })),
        expected(&["ok", "bytes"])
    );
}

#[test]
fn get_request_and_end_field_names_are_exact() {
    assert_eq!(
        json_keys(&json(GetRequest {
            path: "C:\\APP\\OUT.BIN".to_owned(),
        })),
        expected(&["path"])
    );
    assert_eq!(
        json_keys(&json(GetEnd {
            ok: true,
            size: 12_345,
        })),
        expected(&["ok", "size"])
    );
}

#[test]
fn stat_response_field_names_are_exact() {
    let value = json(stat_response());

    assert_eq!(
        json_keys(&value),
        expected(&["exists", "size", "directory", "attributes", "timestamp",])
    );

    let without_timestamp = json(StatResponse {
        timestamp: None,
        ..stat_response()
    });
    assert!(!json_keys(&without_timestamp).contains("timestamp"));
}

#[test]
fn pong_field_names_are_exact() {
    assert_eq!(
        json_keys(&json(Pong {
            agent_version: "1.0.0".to_owned(),
            busy: false,
        })),
        expected(&["agent_version", "busy"])
    );
}

#[test]
fn shutdown_field_names_are_exact() {
    assert_eq!(
        json_keys(&json(ShutdownRequest { grace_ms: 10_000 })),
        expected(&["grace_ms"])
    );
    assert_eq!(
        json_keys(&json(ShutdownAck { ready: true })),
        expected(&["ready"])
    );
}

#[test]
fn error_payload_field_names_and_codes_are_exact() {
    let value = json(ErrorPayload {
        code: ErrorCode::Busy,
        message: "a command is already running".to_owned(),
        request_id: 17,
    });

    assert_eq!(
        json_keys(&value),
        expected(&["code", "message", "request_id"])
    );
    assert_eq!(value["code"], "BUSY");

    let codes = [
        (ErrorCode::Busy, "BUSY"),
        (ErrorCode::NoFile, "NO_FILE"),
        (ErrorCode::NoSpace, "NO_SPACE"),
        (ErrorCode::BadPath, "BAD_PATH"),
        (ErrorCode::TooLong, "TOO_LONG"),
        (ErrorCode::TooLarge, "TOO_LARGE"),
        (ErrorCode::Unsupported, "UNSUPPORTED"),
        (ErrorCode::Crc, "CRC"),
        (ErrorCode::Session, "SESSION"),
        (ErrorCode::Internal, "INTERNAL"),
    ];

    for (code, name) in codes {
        assert_eq!(json(code), Value::String(name.to_owned()));
    }
}

#[test]
fn attributes_field_names_are_exact() {
    assert_eq!(
        json_keys(&json(attributes())),
        expected(&["read_only", "hidden", "system", "archive"])
    );
}

#[test]
fn payload_round_trip_for_every_message() {
    round_trip(&hello());
    round_trip(&HelloAck {
        session_id: "b1f0c2".to_owned(),
    });
    round_trip(&exec_request());
    round_trip(&exec_end());
    round_trip(&put_request());
    round_trip(&PutEnd {
        ok: true,
        bytes: 12_345,
    });
    round_trip(&GetRequest {
        path: "C:\\APP\\OUT.BIN".to_owned(),
    });
    round_trip(&GetEnd {
        ok: true,
        size: 12_345,
    });
    round_trip(&StatRequest {
        path: "C:\\APP\\APP.EXE".to_owned(),
    });
    round_trip(&attributes());
    round_trip(&stat_response());
    round_trip(&Pong {
        agent_version: "1.0.0".to_owned(),
        busy: false,
    });
    round_trip(&ShutdownRequest { grace_ms: 10_000 });
    round_trip(&ShutdownAck { ready: true });
    round_trip(&ErrorPayload {
        code: ErrorCode::NoFile,
        message: "path does not exist".to_owned(),
        request_id: 17,
    });
}

#[test]
fn exec_request_rejects_a_command_over_127_chars() {
    let request = ExecRequest {
        command: "A".repeat(128),
        ..exec_request()
    };

    assert!(request.validate().is_err());

    let at_limit = ExecRequest {
        command: "A".repeat(127),
        ..exec_request()
    };
    assert!(at_limit.validate().is_ok());
}

#[test]
fn exec_request_rejects_max_output_over_one_mib() {
    let request = ExecRequest {
        max_output_bytes: 1_048_577,
        ..exec_request()
    };

    assert!(request.validate().is_err());

    let at_limit = ExecRequest {
        max_output_bytes: 1_048_576,
        ..exec_request()
    };
    assert!(at_limit.validate().is_ok());
}

#[test]
fn put_request_rejects_size_over_16_mib() {
    let request = PutRequest {
        size: 16 * 1024 * 1024 + 1,
        ..put_request()
    };

    assert!(request.validate().is_err());

    let at_limit = PutRequest {
        size: 16 * 1024 * 1024,
        ..put_request()
    };
    assert!(at_limit.validate().is_ok());
}

#[test]
fn hello_validate_rejects_protocol_version_other_than_one() {
    let hello = Hello {
        protocol_version: 2,
        ..hello()
    };

    assert!(hello.validate().is_err());
}

#[test]
fn hello_validate_rejects_max_payload_other_than_4096() {
    let hello = Hello {
        max_payload: 2048,
        ..hello()
    };

    assert!(hello.validate().is_err());
}

#[test]
fn command_not_found_defaults_to_false() {
    let json = r#"{
        "exit_code": 1,
        "output_bytes": 0,
        "truncated": false,
        "duration_ms": 12
    }"#;

    let end: ExecEnd = decode_payload(json.as_bytes()).expect("payload decodes");

    assert!(!end.command_not_found);
}
