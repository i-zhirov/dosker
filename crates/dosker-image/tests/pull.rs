mod support;

use std::fs;

use chrono::{TimeZone, Utc};
use dosker_core::{LicenseBasis, LicenseEntry, LicenseStatus, LicenseSummary};
use dosker_image::acceptance::AcceptanceStore;
use dosker_image::remote::{
    PullError, PullRequest, RegistryClient, RemoteCache, RemoteError, pull,
};
use dosker_image::{
    ImageDigest, ImageRef, ImageReference, OciStore, ReferenceSelector, RegistryReference,
    Repository,
};
use dosker_test_support::registry_site::StaticServer;
use support::site::{SiteSpec, abandonware, fixture_site, redistributable};

fn now() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_763_900_000, 0).unwrap()
}

fn request(value: &str, accept: Option<&str>) -> PullRequest {
    PullRequest {
        reference: RegistryReference::parse(value).expect("a reference"),
        accept: accept.map(|digest| ImageDigest::parse(digest).expect("a digest")),
    }
}

fn target() -> (tempfile::TempDir, OciStore, RemoteCache, AcceptanceStore) {
    let directory = tempfile::tempdir().expect("a target directory");
    let root = directory.path().to_path_buf();
    (
        directory,
        OciStore::new(root.clone()),
        RemoteCache::new(root.clone()),
        AcceptanceStore::new(root),
    )
}

fn fixture() -> (tempfile::TempDir, tempfile::TempDir, StaticServer) {
    fixture_site(&[
        SiteSpec {
            reference: "i-zhirov/hello-dos:1.0.0".to_owned(),
            version: "1.0.0".to_owned(),
            tag: "latest".to_owned(),
            description: "Hello.".to_owned(),
            licenses: vec![redistributable()],
        },
        SiteSpec {
            reference: "i-zhirov/old-game:1.0.0".to_owned(),
            version: "1.0.0".to_owned(),
            tag: "latest".to_owned(),
            description: "Old.".to_owned(),
            licenses: vec![abandonware()],
        },
    ])
}

fn apache_license() -> LicenseEntry {
    LicenseEntry {
        name: "APP".to_owned(),
        spdx: "Apache-2.0".to_owned(),
        holder: None,
        status: LicenseStatus::Redistributable,
        basis: Some(LicenseBasis::OpenSourceLicense),
        evidence: None,
        license_url: None,
        covers: Vec::new(),
        source_url: None,
        notice: None,
    }
}

#[test]
fn a_redistributable_tag_pull_stores_and_tags() {
    let (_fixture_store, _site, server) = fixture();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let (_target, store, cache, acceptance) = target();

    let report = pull(
        &client,
        &store,
        &cache,
        &acceptance,
        &request("i-zhirov/hello-dos:latest", None),
        now(),
    )
    .expect("the pull");

    assert_eq!(report.image, "i-zhirov/hello-dos");
    assert_eq!(report.tag.as_deref(), Some("latest"));
    assert_eq!(report.license_summary, LicenseSummary::Redistributable);
    assert!(!report.cached);
    let local = ImageReference::parse("i-zhirov/hello-dos:latest").expect("a local reference");
    let image = store.load(&ImageRef::Tag(local)).expect("the stored image");
    assert_eq!(image.manifest_digest, report.digest);
}

#[test]
fn a_digest_pull_tags_the_version() {
    let (_fixture_store, _site, server) = fixture();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let index = client
        .image_index(&Repository::parse("i-zhirov/hello-dos").unwrap())
        .expect("the index");
    let digest = index.versions[0].digest.clone();
    let (_target, store, cache, acceptance) = target();

    let report = pull(
        &client,
        &store,
        &cache,
        &acceptance,
        &request(&format!("i-zhirov/hello-dos@{digest}"), None),
        now(),
    )
    .expect("the pull");

    assert_eq!(report.tag.as_deref(), Some("1.0.0"));
    assert!(
        store
            .list()
            .expect("the list")
            .iter()
            .any(|image| image.name == "i-zhirov/hello-dos" && image.tag == "1.0.0")
    );
}

#[test]
fn an_unknown_tag_fails() {
    let (_fixture_store, _site, server) = fixture();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let (_target, store, cache, acceptance) = target();

    let error = pull(
        &client,
        &store,
        &cache,
        &acceptance,
        &request("i-zhirov/hello-dos:missing", None),
        now(),
    )
    .expect_err("an unknown tag");

    assert!(matches!(
        error,
        PullError::Remote(RemoteError::TagNotFound(_))
    ));
}

#[test]
fn a_gate_refuses_an_abandonware_image() {
    let (_fixture_store, _site, server) = fixture();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let (_target, store, cache, acceptance) = target();

    let error = pull(
        &client,
        &store,
        &cache,
        &acceptance,
        &request("i-zhirov/old-game:latest", None),
        now(),
    )
    .expect_err("the gate");

    assert!(matches!(error, PullError::LicenseNotAccepted(_)));
    assert!(
        !server
            .requests()
            .iter()
            .any(|request| request.ends_with(".tar"))
    );
}

#[test]
fn an_acceptance_unlocks_the_exact_digest() {
    let (_fixture_store, _site, server) = fixture();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let index = client
        .image_index(&Repository::parse("i-zhirov/old-game").unwrap())
        .expect("the index");
    let digest = index.versions[0].digest.clone();
    let (_target, store, cache, acceptance) = target();

    let report = pull(
        &client,
        &store,
        &cache,
        &acceptance,
        &request("i-zhirov/old-game:latest", Some(&digest)),
        now(),
    )
    .expect("the accepted pull");

    assert!(acceptance.is_accepted(&digest).expect("the acceptance"));
    let records = acceptance.list().expect("the records");
    assert_eq!(records[0].config_digest, report.config_digest);
}

#[test]
fn a_different_digest_needs_a_new_acceptance() {
    let (_fixture_store, _site, server) = fixture();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let (_target, store, cache, acceptance) = target();
    let other = format!("sha256:{}", "0".repeat(64));

    let error = pull(
        &client,
        &store,
        &cache,
        &acceptance,
        &request("i-zhirov/old-game:latest", Some(&other)),
        now(),
    )
    .expect_err("a mismatch");

    assert!(matches!(error, PullError::AcceptMismatch { .. }));
}

#[test]
fn an_acceptance_is_recorded_after_success() {
    let (_fixture_store, _site, server) = fixture();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let index = client
        .image_index(&Repository::parse("i-zhirov/old-game").unwrap())
        .expect("the index");
    let digest = index.versions[0].digest.clone();
    let (_target, store, cache, acceptance) = target();

    pull(
        &client,
        &store,
        &cache,
        &acceptance,
        &request("i-zhirov/old-game:latest", Some(&digest)),
        now(),
    )
    .expect("the pull");

    let records = acceptance.list().expect("the records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].digest, digest);
    assert_eq!(records[0].license_summary, "abandonware");
}

#[test]
fn a_catalog_summary_does_not_bypass_the_gate() {
    let (_fixture_store, site_dir, server) = fixture();
    let path = site_dir.path().join("index.json");
    let mut catalog: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for image in catalog["images"].as_array_mut().unwrap() {
        if image["name"] == "old-game" {
            image["license_summary"] = serde_json::json!("redistributable");
            image["requires_acceptance"] = serde_json::json!(false);
        }
    }
    fs::write(&path, serde_json::to_vec(&catalog).unwrap()).unwrap();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let (_target, store, cache, acceptance) = target();

    let error = pull(
        &client,
        &store,
        &cache,
        &acceptance,
        &request("i-zhirov/old-game:latest", None),
        now(),
    )
    .expect_err("the gate");

    assert!(matches!(error, PullError::LicenseNotAccepted(_)));
}

#[test]
fn a_missing_license_list_fails() {
    let (_fixture_store, _site, server) = fixture_site(&[SiteSpec {
        reference: "i-zhirov/empty:1.0.0".to_owned(),
        version: "1.0.0".to_owned(),
        tag: "latest".to_owned(),
        description: "Empty.".to_owned(),
        licenses: Vec::new(),
    }]);
    let client = RegistryClient::new(&server.url()).expect("a client");
    let (_target, store, cache, acceptance) = target();

    let error = pull(
        &client,
        &store,
        &cache,
        &acceptance,
        &request("i-zhirov/empty:latest", None),
        now(),
    )
    .expect_err("no licenses");

    assert!(matches!(error, PullError::MissingLicense { .. }));
}

#[test]
fn an_asset_with_a_different_manifest_fails() {
    let (_fixture_store, site_dir, server) = fixture();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let hello_path = site_dir.path().join("v1/i-zhirov/hello-dos/index.json");
    let old_path = site_dir.path().join("v1/i-zhirov/old-game/index.json");
    let old: serde_json::Value = serde_json::from_slice(&fs::read(&old_path).unwrap()).unwrap();
    let old_version = &old["versions"][0];
    let mut hello: serde_json::Value =
        serde_json::from_slice(&fs::read(&hello_path).unwrap()).unwrap();
    hello["versions"][0]["asset_url"] = old_version["asset_url"].clone();
    hello["versions"][0]["asset_digest"] = old_version["asset_digest"].clone();
    hello["versions"][0]["asset_size"] = old_version["asset_size"].clone();
    fs::write(&hello_path, serde_json::to_vec(&hello).unwrap()).unwrap();
    let (_target, store, cache, acceptance) = target();

    let error = pull(
        &client,
        &store,
        &cache,
        &acceptance,
        &request("i-zhirov/hello-dos:latest", None),
        now(),
    )
    .expect_err("the archive mismatch");

    assert!(matches!(error, PullError::Store(_)));
}

#[test]
fn a_moved_tag_replaces_the_local_tag() {
    let (_first_store, _first_site, first_server) = fixture();
    let first_client = RegistryClient::new(&first_server.url()).expect("a client");
    let (_target, store, cache, acceptance) = target();
    let first = pull(
        &first_client,
        &store,
        &cache,
        &acceptance,
        &request("i-zhirov/hello-dos:latest", None),
        now(),
    )
    .expect("the first pull");

    let (_second_store, _second_site, second_server) = fixture_site(&[SiteSpec {
        reference: "i-zhirov/hello-dos:2.0.0".to_owned(),
        version: "2.0.0".to_owned(),
        tag: "latest".to_owned(),
        description: "Hello two.".to_owned(),
        licenses: vec![apache_license()],
    }]);
    let second_client = RegistryClient::new(&second_server.url()).expect("a client");
    let second = pull(
        &second_client,
        &store,
        &cache,
        &acceptance,
        &request("i-zhirov/hello-dos:latest", None),
        now(),
    )
    .expect("the second pull");

    assert_ne!(first.digest, second.digest);
    let local = ImageReference::parse("i-zhirov/hello-dos:latest").expect("a local reference");
    let image = store.load(&ImageRef::Tag(local)).expect("the image");
    assert_eq!(image.manifest_digest, second.digest);
}

#[test]
fn a_digest_resolves_from_the_index() {
    let (_fixture_store, _site, server) = fixture();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let index = client
        .image_index(&Repository::parse("i-zhirov/hello-dos").unwrap())
        .expect("the index");
    let digest = ImageDigest::parse(&index.versions[0].digest).expect("a digest");

    let version = client
        .resolve(&index, &ReferenceSelector::Digest(digest))
        .expect("the version");

    assert_eq!(version.version, "1.0.0");
}

#[test]
fn an_interrupted_pull_resumes_the_asset() {
    let (_fixture_store, _site, server) = fixture();
    server.reset_requests();
    server.fail_next_asset(512);
    let client = RegistryClient::new(&server.url()).expect("a client");
    let (_target, store, cache, acceptance) = target();
    let request = request("i-zhirov/hello-dos:latest", None);

    let error =
        pull(&client, &store, &cache, &acceptance, &request, now()).expect_err("a truncated pull");

    assert!(matches!(error, PullError::Remote(RemoteError::Http(_))));

    let report =
        pull(&client, &store, &cache, &acceptance, &request, now()).expect("the resumed pull");

    assert!(!report.cached);
    assert!(report.resumed);
    let local = ImageReference::parse("i-zhirov/hello-dos:latest").expect("a local reference");
    assert!(store.load(&ImageRef::Tag(local)).is_ok());
    assert!(server.ranges().contains(&Some("bytes=512-".to_owned())));
}
