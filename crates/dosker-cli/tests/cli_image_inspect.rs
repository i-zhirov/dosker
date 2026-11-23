mod common;

use std::process::Output;

use common::{CliRoot, redistributable_license, stderr_text, stdout_text};
use dosker_test_support::registry_site::{StaticServer, build_site};

fn site(root: &CliRoot) -> (tempfile::TempDir, StaticServer) {
    let hello = root.registry_site_image(
        "i-zhirov/hello-dos:1.0.0",
        "1.0.0",
        "latest",
        "Minimal DOS environment.",
        vec![redistributable_license()],
    );
    let directory = tempfile::tempdir().expect("a site directory");
    let server = StaticServer::start(directory.path().to_path_buf()).expect("the server");
    build_site(
        directory.path(),
        &server.url(),
        "2026-11-23T12:00:00Z",
        &[hello],
    )
    .expect("the site builds");
    (directory, server)
}

fn run(root: &CliRoot, url: &str, args: &[&str]) -> Output {
    root.command_with_registry(url)
        .args(args)
        .output()
        .expect("the command runs")
}

#[test]
fn inspect_shows_a_remote_image() {
    let root = CliRoot::new();
    let (_directory, server) = site(&root);

    let output = run(
        &root,
        &server.url(),
        &["image", "inspect", "i-zhirov/hello-dos:latest"],
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(text.contains("image i-zhirov/hello-dos"), "stdout: {text}");
    assert!(text.contains("digest sha256:"), "stdout: {text}");
    assert!(text.contains("license redistributable"), "stdout: {text}");
    assert!(text.contains("APP MIT"), "stdout: {text}");
}

#[test]
fn inspect_shows_a_local_image() {
    let root = CliRoot::new();
    root.store_final_image("hello:latest", vec![redistributable_license()], &[]);

    let output = run(
        &root,
        "http://127.0.0.1:1",
        &["image", "inspect", "hello:latest"],
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(text.contains("image hello:latest"), "stdout: {text}");
    assert!(text.contains("digest sha256:"), "stdout: {text}");
    assert!(text.contains("APP MIT"), "stdout: {text}");
}

#[test]
fn inspect_reports_an_unknown_tag() {
    let root = CliRoot::new();
    let (_directory, server) = site(&root);

    let output = run(
        &root,
        &server.url(),
        &["image", "inspect", "i-zhirov/hello-dos:missing"],
    );

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("tag not found"),
        "stderr: {}",
        stderr_text(&output)
    );
}
