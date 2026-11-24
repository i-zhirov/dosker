mod common;

use std::fs;
use std::path::Path;
use std::process::Output;

use common::{CliRoot, abandonware_license, redistributable_license, stderr_text, stdout_text};
use dosker_image::OciStore;
use dosker_image::acceptance::AcceptanceStore;
use dosker_test_support::registry_site::{StaticServer, build_site};

fn digest(site: &Path, owner: &str, name: &str) -> String {
    let path = site.join(format!("v1/{owner}/{name}/index.json"));
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(path).expect("the index")).expect("the index json");
    value["versions"][0]["digest"]
        .as_str()
        .expect("a digest")
        .to_owned()
}

fn site(root: &CliRoot) -> (tempfile::TempDir, StaticServer, String, String) {
    let hello = root.registry_site_image(
        "i-zhirov/hello-dos:1.0.0",
        "1.0.0",
        "latest",
        "Minimal DOS environment.",
        vec![redistributable_license()],
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
    let hello_digest = digest(directory.path(), "i-zhirov", "hello-dos");
    let old_digest = digest(directory.path(), "i-zhirov", "old-game");
    (directory, server, hello_digest, old_digest)
}

fn run(root: &CliRoot, url: &str, args: &[&str]) -> Output {
    root.command_with_registry(url)
        .args(args)
        .output()
        .expect("the command runs")
}

#[test]
fn pull_accepts_a_redistributable_image() {
    let root = CliRoot::new();
    let (_directory, server, _hello, _old) = site(&root);

    let output = run(&root, &server.url(), &["pull", "i-zhirov/hello-dos:latest"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains("pulled i-zhirov/hello-dos:latest"),
        "stdout: {}",
        stdout_text(&output)
    );
}

#[test]
fn pull_requires_acceptance_for_an_abandonware_image() {
    let root = CliRoot::new();
    let (_directory, server, _hello, _old) = site(&root);

    let output = run(&root, &server.url(), &["pull", "i-zhirov/old-game:latest"]);

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
    assert!(
        !server
            .requests()
            .iter()
            .any(|request| request.ends_with(".tar.zst"))
    );
}

#[test]
fn pull_with_accept_license_stores_the_image() {
    let root = CliRoot::new();
    let (_directory, server, _hello, old_digest) = site(&root);

    let output = run(
        &root,
        &server.url(),
        &[
            "pull",
            "i-zhirov/old-game:latest",
            "--accept-license",
            &old_digest,
        ],
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let acceptance = AcceptanceStore::new(root.path().to_path_buf());
    assert!(acceptance.is_accepted(&old_digest).expect("the acceptance"));
    let store = OciStore::new(root.path().to_path_buf());
    assert!(
        store
            .list()
            .expect("the list")
            .iter()
            .any(|image| image.name == "i-zhirov/old-game" && image.tag == "latest")
    );
}

#[test]
fn pull_rejects_a_bare_local_reference() {
    let root = CliRoot::new();
    let (_directory, server, _hello, _old) = site(&root);

    let output = run(&root, &server.url(), &["pull", "hello:latest"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn search_lists_a_matching_image() {
    let root = CliRoot::new();
    let (_directory, server, _hello, _old) = site(&root);

    let output = run(&root, &server.url(), &["search", "hello"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(text.contains("i-zhirov/hello-dos"), "stdout: {text}");
    assert!(text.contains("latest"), "stdout: {text}");
}

#[test]
fn search_json_returns_the_catalog_entries() {
    let root = CliRoot::new();
    let (_directory, server, _hello, _old) = site(&root);

    let output = run(&root, &server.url(), &["search", "hello", "--json"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("the catalog json");
    let images = value["images"].as_array().expect("an image list");
    assert_eq!(images.len(), 1);
    assert_eq!(images[0]["name"], "hello-dos");
}

#[test]
fn a_resumed_pull_reports_the_resume() {
    let root = CliRoot::new();
    let (_directory, server, _hello, _old) = site(&root);
    server.reset_requests();
    server.fail_next_asset(64);

    let first = run(&root, &server.url(), &["pull", "i-zhirov/hello-dos:latest"]);
    assert!(!first.status.success(), "stdout: {}", stdout_text(&first));
    assert!(
        stderr_text(&first).contains("truncated response"),
        "stderr: {}",
        stderr_text(&first)
    );

    let second = run(&root, &server.url(), &["pull", "i-zhirov/hello-dos:latest"]);
    assert_eq!(
        second.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&second)
    );
    assert!(
        stdout_text(&second).contains(", resumed"),
        "stdout: {}",
        stdout_text(&second)
    );
}
