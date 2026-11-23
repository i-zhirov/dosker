mod common;

use std::fs;
use std::process::Output;

use common::{CliRoot, abandonware_license, stderr_text};
use dosker_image::OciStore;
use dosker_test_support::registry_site::{StaticServer, build_site};

fn site(root: &CliRoot) -> (tempfile::TempDir, StaticServer) {
    let hello = root.registry_site_image_disk(
        "i-zhirov/hello-dos:1.0.0",
        "1.0.0",
        "latest",
        "Minimal DOS environment.",
    );
    let old = root.registry_site_image(
        "i-zhirov/old-game:1.0.0",
        "1.0.0",
        "latest",
        "Abandonware example.",
        vec![abandonware_license()],
    );
    let directory = tempfile::tempdir().expect("a site directory");
    let server = StaticServer::start(directory.path().to_path_buf()).expect("the server");
    build_site(
        directory.path(),
        &server.url(),
        "2026-11-23T12:00:00Z",
        &[hello, old],
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

fn has_tag(root: &CliRoot, name: &str, tag: &str) -> bool {
    OciStore::new(root.path().to_path_buf())
        .list()
        .expect("the list")
        .iter()
        .any(|image| image.name == name && image.tag == tag)
}

#[test]
fn run_pulls_a_missing_remote_image() {
    let root = CliRoot::new();
    let (_directory, server) = site(&root);

    let output = run(
        &root,
        &server.url(),
        &["run", "-d", "--engine", "mock", "i-zhirov/hello-dos:latest"],
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(has_tag(&root, "i-zhirov/hello-dos", "latest"));
}

#[test]
fn run_with_no_pull_does_not_pull() {
    let root = CliRoot::new();
    let (_directory, server) = site(&root);

    let output = run(
        &root,
        &server.url(),
        &[
            "run",
            "-d",
            "--engine",
            "mock",
            "--no-pull",
            "i-zhirov/hello-dos:latest",
        ],
    );

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(!has_tag(&root, "i-zhirov/hello-dos", "latest"));
}

#[test]
fn create_pulls_a_missing_remote_image() {
    let root = CliRoot::new();
    let (_directory, server) = site(&root);

    let output = run(
        &root,
        &server.url(),
        &["create", "--engine", "mock", "i-zhirov/hello-dos:latest"],
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(has_tag(&root, "i-zhirov/hello-dos", "latest"));
}

#[test]
fn build_pulls_a_missing_remote_base() {
    let root = CliRoot::new();
    let (_directory, server) = site(&root);
    let context = tempfile::tempdir().expect("a build context");
    fs::write(
        context.path().join("Doskerfile"),
        "FROM i-zhirov/hello-dos:latest\n",
    )
    .expect("write the Doskerfile");

    let output = run(
        &root,
        &server.url(),
        &[
            "build",
            "-t",
            "built:latest",
            context.path().to_str().unwrap(),
        ],
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(has_tag(&root, "i-zhirov/hello-dos", "latest"));
}

#[test]
fn build_with_no_pull_keeps_the_pull_hint() {
    let root = CliRoot::new();
    let (_directory, server) = site(&root);
    let context = tempfile::tempdir().expect("a build context");
    fs::write(
        context.path().join("Doskerfile"),
        "FROM i-zhirov/hello-dos:latest\n",
    )
    .expect("write the Doskerfile");

    let output = run(
        &root,
        &server.url(),
        &[
            "build",
            "--no-pull",
            "-t",
            "built:latest",
            context.path().to_str().unwrap(),
        ],
    );

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("pull it first"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn build_maps_a_license_refusal_to_exit_3() {
    let root = CliRoot::new();
    let (_directory, server) = site(&root);
    let context = tempfile::tempdir().expect("a build context");
    fs::write(
        context.path().join("Doskerfile"),
        "FROM i-zhirov/old-game:latest\n",
    )
    .expect("write the Doskerfile");

    let output = run(
        &root,
        &server.url(),
        &[
            "build",
            "-t",
            "built:latest",
            context.path().to_str().unwrap(),
        ],
    );

    assert_eq!(
        output.status.code(),
        Some(3),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("--accept-license"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn run_uses_a_pulled_image_offline() {
    let root = CliRoot::new();
    let (_directory, server) = site(&root);
    let pull = run(&root, &server.url(), &["pull", "i-zhirov/hello-dos:latest"]);
    assert_eq!(
        pull.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&pull)
    );
    drop(server);

    let output = root.run(&["run", "-d", "i-zhirov/hello-dos:latest"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
}
