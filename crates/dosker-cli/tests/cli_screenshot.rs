mod common;

use common::{CliRoot, stderr_text};

#[test]
fn screenshot_reports_unsupported_for_the_mock_engine() {
    let root = CliRoot::new();
    let id = root.start();
    let output_dir = tempfile::tempdir().expect("create an output directory");
    let output_path = output_dir.path().join("shot.ppm");

    let output = root.run(&[
        "screenshot",
        id.as_str(),
        "-o",
        output_path.to_str().expect("a utf-8 path"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("screenshot is not supported by this engine"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn screenshot_rejects_a_stopped_container() {
    let root = CliRoot::new();
    let id = root.start();
    let stopped = root.run(&["stop", id.as_str()]);
    assert_eq!(stopped.status.code(), Some(0));
    let output_dir = tempfile::tempdir().expect("create an output directory");
    let output_path = output_dir.path().join("shot.ppm");

    let output = root.run(&[
        "screenshot",
        id.as_str(),
        "-o",
        output_path.to_str().expect("a utf-8 path"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(125),
        "stderr: {}",
        stderr_text(&output)
    );
}
