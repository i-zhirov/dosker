//! End-to-end checks for `dosker port`.

mod common;

use common::{CliRoot, stderr_text, stdout_text};

fn create_id(root: &CliRoot, args: &[&str]) -> String {
    let output = root.run(args);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    stdout_text(&output).trim().to_owned()
}

#[test]
fn port_prints_the_effective_mappings() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let first = dosker_engine_qemu::command::free_tcp_port().expect("allocate a free port");
    let mut second = dosker_engine_qemu::command::free_tcp_port().expect("allocate a free port");
    while second == first {
        second = dosker_engine_qemu::command::free_tcp_port().expect("allocate a free port");
    }
    // Publish the higher host port first so a sorted implementation cannot
    // pass the recorded-order assertion.
    let (high, low) = if first > second {
        (first, second)
    } else {
        (second, first)
    };
    let first_mapping = format!("{high}:80");
    let second_mapping = format!("127.0.0.1:{low}:81");

    let id = create_id(
        &root,
        &[
            "create",
            "-p",
            &first_mapping,
            "-p",
            &second_mapping,
            "freedos:latest",
        ],
    );

    let output = root.run(&["port", &id]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let expected = vec![
        format!("127.0.0.1:{high}->80/tcp"),
        format!("127.0.0.1:{low}->81/tcp"),
    ];
    assert_eq!(lines, expected);
}

#[test]
fn port_prints_nothing_without_mappings() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let id = create_id(&root, &["create", "freedos:latest"]);

    let output = root.run(&["port", &id]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "");
}

#[test]
fn port_reports_an_unknown_container() {
    let root = CliRoot::new();

    let output = root.run(&["port", "missing-container"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("container not found: missing-container"),
        "stderr: {}",
        stderr_text(&output)
    );
}
