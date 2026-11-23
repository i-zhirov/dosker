mod common;

use common::{CliRoot, stderr_text};
use dosker_image::{ImageRef, ImageReference, OciStore};
use dosker_test_support::registry_site::{StaticServer, build_site};

fn site(root: &CliRoot) -> (tempfile::TempDir, StaticServer) {
    let hello = root.registry_site_image_disk(
        "i-zhirov/hello-dos:1.0.0",
        "1.0.0",
        "latest",
        "Minimal DOS environment.",
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

fn empty_site() -> (tempfile::TempDir, StaticServer) {
    let directory = tempfile::tempdir().expect("a site directory");
    let server = StaticServer::start(directory.path().to_path_buf()).expect("the server");
    build_site(directory.path(), &server.url(), "2026-11-23T12:00:00Z", &[])
        .expect("the site builds");
    (directory, server)
}

fn has_tag(root: &CliRoot, name: &str, tag: &str) -> bool {
    OciStore::new(root.path().to_path_buf())
        .list()
        .expect("the list")
        .iter()
        .any(|image| image.name == name && image.tag == tag)
}

#[test]
fn pull_after_a_takedown_fails_and_keeps_the_local_copy() {
    let root = CliRoot::new();
    let (first_directory, first_server) = site(&root);
    let pull = root
        .command_with_registry(&first_server.url())
        .args(["pull", "i-zhirov/hello-dos:latest"])
        .output()
        .expect("the pull runs");
    assert_eq!(
        pull.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&pull)
    );
    drop(first_server);
    drop(first_directory);

    let (_empty_directory, empty_server) = empty_site();
    let pull = root
        .command_with_registry(&empty_server.url())
        .args(["pull", "i-zhirov/hello-dos:latest"])
        .output()
        .expect("the pull runs");
    assert_ne!(pull.status.code(), Some(0));
    let stderr = stderr_text(&pull);
    assert!(stderr.contains("image not found"), "stderr: {stderr}");
    assert!(has_tag(&root, "i-zhirov/hello-dos", "latest"));

    let output = root.run(&["run", "-d", "i-zhirov/hello-dos:latest"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn pull_a_revoked_digest_fails() {
    let root = CliRoot::new();
    let (first_directory, first_server) = site(&root);
    let pull = root
        .command_with_registry(&first_server.url())
        .args(["pull", "i-zhirov/hello-dos:latest"])
        .output()
        .expect("the pull runs");
    assert_eq!(
        pull.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&pull)
    );
    let reference = ImageReference::parse("i-zhirov/hello-dos:latest").expect("a reference");
    let digest = OciStore::new(root.path().to_path_buf())
        .load(&ImageRef::Tag(reference))
        .expect("the local image")
        .manifest_digest;
    drop(first_server);
    drop(first_directory);

    let (_empty_directory, empty_server) = empty_site();
    let pull = root
        .command_with_registry(&empty_server.url())
        .args(["pull", &format!("i-zhirov/hello-dos@{digest}")])
        .output()
        .expect("the pull runs");
    assert_ne!(pull.status.code(), Some(0));
    let stderr = stderr_text(&pull);
    assert!(stderr.contains("image not found"), "stderr: {stderr}");
    assert!(has_tag(&root, "i-zhirov/hello-dos", "latest"));
}
