#![cfg(unix)]

mod support;

use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use dosker_agent_proto::{
    AgentClient, ErrorCode, ExecRequest, FrameType, ProtoError, encode_payload,
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);
const EXEC_TIMEOUT: Duration = Duration::from_secs(60);
const OUTPUT_LIMIT: u32 = 1_048_576;

fn start() -> (support::QemuAgent, AgentClient<UnixStream>) {
    let mut qemu = support::QemuAgent::start().expect("failed to start QEMU");
    let mut client = qemu.connect().expect("failed to connect to the agent");
    client
        .handshake(HANDSHAKE_TIMEOUT)
        .expect("failed to complete the handshake");

    (qemu, client)
}

fn request(command: &str, workdir: &str, max_output_bytes: u32) -> ExecRequest {
    ExecRequest {
        command: command.to_owned(),
        env: BTreeMap::new(),
        workdir: workdir.to_owned(),
        max_output_bytes,
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn exec_echo_hello_returns_hello_and_zero() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let outcome = client
        .exec(&request("ECHO HELLO", "C:\\", OUTPUT_LIMIT), EXEC_TIMEOUT)
        .expect("failed to execute ECHO HELLO");
    let output = text(&outcome.output);

    assert_eq!(outcome.exit_code, 0, "output: {output:?}");
    assert!(
        output.contains("HELLO"),
        "output does not contain HELLO: {output:?}"
    );
}

#[test]
fn exec_line_shell_sequence() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let first = client
        .exec(&request("CD \\", "C:\\APP", OUTPUT_LIMIT), EXEC_TIMEOUT)
        .expect("failed to execute CD");
    let second = client
        .exec(&request("ECHO ONE", "C:\\", OUTPUT_LIMIT), EXEC_TIMEOUT)
        .expect("failed to execute ECHO ONE");
    let third = client
        .exec(&request("ECHO TWO", "C:\\", OUTPUT_LIMIT), EXEC_TIMEOUT)
        .expect("failed to execute ECHO TWO");

    assert_eq!(first.exit_code, 0);
    assert_eq!(second.exit_code, 0);
    assert_eq!(third.exit_code, 0);

    let output = format!(
        "{}{}{}",
        text(&first.output),
        text(&second.output),
        text(&third.output)
    );
    let one = output.find("ONE").expect("output does not contain ONE");
    let two = output.find("TWO").expect("output does not contain TWO");

    assert!(one < two, "ONE and TWO are out of order: {output:?}");
}

#[test]
fn exec_exit_code_passthrough() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let outcome = client
        .exec(
            &request("C:\\DOSKER\\EXITCODE.EXE 7", "C:\\", OUTPUT_LIMIT),
            EXEC_TIMEOUT,
        )
        .expect("failed to execute the exit-code helper");
    let output = text(&outcome.output);

    assert_eq!(outcome.exit_code, 7, "output: {output:?}");
}

#[test]
fn exec_captures_multiple_lines() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let outcome = client
        .exec(
            &request("TYPE C:\\DOSKER\\TWO.TXT", "C:\\", OUTPUT_LIMIT),
            EXEC_TIMEOUT,
        )
        .expect("failed to execute TYPE");
    let output = text(&outcome.output);

    let one = output.find("ONE").expect("output does not contain ONE");
    let two = output.find("TWO").expect("output does not contain TWO");

    assert!(one < two, "ONE and TWO are out of order: {output:?}");
}

#[test]
fn exec_truncates_at_max_output_bytes() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let outcome = client
        .exec(
            &request("TYPE C:\\DOSKER\\BIG.TXT", "C:\\", 1024),
            EXEC_TIMEOUT,
        )
        .expect("failed to execute TYPE on BIG.TXT");

    assert_eq!(
        outcome.output.len(),
        1024,
        "captured {} bytes",
        outcome.output.len()
    );
    assert!(outcome.truncated, "output was not marked as truncated");
}

#[test]
fn exec_rejects_a_command_over_127_chars() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let mut command = String::from("ECHO ");
    command.push_str(&"A".repeat(123));

    let result = client.exec(&request(&command, "C:\\", OUTPUT_LIMIT), EXEC_TIMEOUT);

    match result {
        Err(ProtoError::CommandTooLong { length, max }) => {
            assert_eq!(length, 128);
            assert_eq!(max, 127);
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn exec_missing_command_sets_command_not_found() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let outcome = client
        .exec(&request("NOSUCH", "C:\\", OUTPUT_LIMIT), EXEC_TIMEOUT)
        .expect("failed to execute NOSUCH");
    let output = text(&outcome.output);

    assert!(
        outcome.command_not_found,
        "command_not_found was false; captured output: {output:?}"
    );
}

#[test]
fn exec_passes_environment_to_child() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let mut request = request("ECHO %GREETING%", "C:\\DOSKER", OUTPUT_LIMIT);
    request
        .env
        .insert("GREETING".to_owned(), "HELLO".to_owned());

    let outcome = client
        .exec(&request, EXEC_TIMEOUT)
        .expect("failed to execute with an environment");
    let output = text(&outcome.output);

    assert!(
        output.contains("HELLO"),
        "output does not contain the environment value: {output:?}"
    );
}

#[test]
fn exec_uses_the_request_workdir_for_a_relative_command() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let outcome = client
        .exec(
            &request("TYPE TWO.TXT", "C:\\DOSKER", OUTPUT_LIMIT),
            EXEC_TIMEOUT,
        )
        .expect("failed to execute in the request workdir");
    let output = text(&outcome.output);

    assert!(
        output.contains("ONE") && output.contains("TWO"),
        "relative command did not read the workdir file: {output:?}"
    );
}

#[test]
fn exec_rejects_an_invalid_workdir() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let result = client.exec(
        &request("ECHO X", "C:\\APP\\TOOLONGNAME", OUTPUT_LIMIT),
        EXEC_TIMEOUT,
    );

    match result {
        Err(ProtoError::Agent(error)) => assert_eq!(error.code, ErrorCode::BadPath),
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn exec_rejects_a_command_over_127_chars_on_the_agent() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let request = ExecRequest {
        command: "A".repeat(128),
        env: BTreeMap::new(),
        workdir: "C:\\".to_owned(),
        max_output_bytes: OUTPUT_LIMIT,
    };
    let payload = encode_payload(&request).expect("failed to encode the request");
    let session = client.session_mut();
    session.begin_request();

    match session.send(FrameType::ExecReq, &payload) {
        Err(ProtoError::Agent(error)) => assert_eq!(error.code, ErrorCode::TooLong),
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn exec_line_shell_sequence_proves_state() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let first = client
        .exec(
            &request(
                "COPY C:\\DOSKER\\TWO.TXT C:\\DOSKER\\SEQ.TMP",
                "C:\\",
                OUTPUT_LIMIT,
            ),
            EXEC_TIMEOUT,
        )
        .expect("failed to create the sequence file");
    let second = client
        .exec(
            &request("TYPE C:\\DOSKER\\SEQ.TMP", "C:\\", OUTPUT_LIMIT),
            EXEC_TIMEOUT,
        )
        .expect("failed to read the sequence file");

    assert_eq!(first.exit_code, 0);
    assert_eq!(second.exit_code, 0);

    let output = text(&second.output);
    let one = output.find("ONE").expect("output does not contain ONE");
    let two = output.find("TWO").expect("output does not contain TWO");
    assert!(one < two, "sequence is out of order: {output:?}");
}
