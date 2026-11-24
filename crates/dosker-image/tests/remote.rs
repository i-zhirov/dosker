mod support;

use std::collections::BTreeMap;
use std::fs;

use dosker_core::{LicenseBasis, LicenseEntry, LicenseStatus};
use dosker_image::remote::{RegistryClient, RemoteError};
use dosker_image::{
    AssetCompression, ImageDigest, ImageReference, ImageTag, Layer, OciStore, ReferenceSelector,
    RegistryReference, Repository, Tree,
};
use dosker_test_support::registry_site::{StaticServer, build_site};
use support::{sample_boot, sample_config, site::site_image};

fn license() -> LicenseEntry {
    LicenseEntry {
        name: "APP".to_owned(),
        spdx: "MIT".to_owned(),
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

fn site() -> (tempfile::TempDir, tempfile::TempDir, StaticServer) {
    let store_dir = tempfile::tempdir().expect("a store directory");
    let store = OciStore::new(store_dir.path().to_path_buf());
    let layer = Layer::from_delta(&Tree::new().to_delta(), "test", None, None).expect("a layer");
    let descriptor = store.put_layer(&layer).expect("the layer stores");
    let mut config = sample_config(sample_boot());
    config.licenses = vec![license()];
    let reference = ImageReference::parse("i-zhirov/hello-dos:1.0.0").expect("a reference");
    store
        .commit_image(Some(&reference), &config, vec![descriptor], BTreeMap::new())
        .expect("the image commits");
    let image = site_image(
        &store,
        &reference,
        "1.0.0",
        "latest",
        "Fixture image.",
        "2026-11-23T12:00:00Z",
    );

    let site_dir = tempfile::tempdir().expect("a site directory");
    let server = StaticServer::start(site_dir.path().to_path_buf()).expect("the server");
    build_site(
        site_dir.path(),
        &server.url(),
        "2026-11-23T12:00:00Z",
        &[image],
    )
    .expect("the site builds");
    (store_dir, site_dir, server)
}

#[test]
fn a_catalog_is_read() {
    let (_store, _site, server) = site();
    let client = RegistryClient::new(&server.url()).expect("a client");

    let catalog = client.catalog().expect("the catalog");

    assert_eq!(catalog.images.len(), 1);
    assert_eq!(catalog.images[0].namespace, "i-zhirov");
    assert_eq!(catalog.images[0].name, "hello-dos");
    assert_eq!(
        catalog.images[0].tags["latest"],
        catalog.images[0].latest.digest
    );
}

#[test]
fn an_image_index_resolves_a_tag() {
    let (_store, _site, server) = site();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let repository = Repository::parse("i-zhirov/hello-dos").expect("a repository");
    let index = client.image_index(&repository).expect("the index");

    let version = client
        .resolve(
            &index,
            &ReferenceSelector::Tag(ImageTag::parse("latest").unwrap()),
        )
        .expect("the tag");

    assert_eq!(version.version, "1.0.0");
    assert_eq!(version.license_summary.as_str(), "redistributable");
    assert_eq!(version.asset_compression, AssetCompression::Zstd);
}

#[test]
fn a_digest_selects_the_version() {
    let (_store, _site, server) = site();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let repository = Repository::parse("i-zhirov/hello-dos").expect("a repository");
    let index = client.image_index(&repository).expect("the index");
    let digest = ImageDigest::parse(&index.versions[0].digest).expect("a digest");

    let version = client
        .resolve(&index, &ReferenceSelector::Digest(digest))
        .expect("the digest");

    assert_eq!(version.version, "1.0.0");
}

#[test]
fn a_missing_asset_compression_fails() {
    let (_store, site_dir, server) = site();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let path = site_dir.path().join("v1/i-zhirov/hello-dos/index.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["versions"][0]
        .as_object_mut()
        .unwrap()
        .remove("asset_compression");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();

    let error = client
        .image_index(&Repository::parse("i-zhirov/hello-dos").unwrap())
        .expect_err("a missing compression");

    assert!(matches!(error, RemoteError::InvalidDocument(_, _)));
}

#[test]
fn a_tampered_manifest_fails() {
    let (_store, site_dir, server) = site();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let repository = Repository::parse("i-zhirov/hello-dos").expect("a repository");
    let index = client.image_index(&repository).expect("the index");
    let version = index.versions[0].clone();
    let hex_digest = version.digest.trim_start_matches("sha256:");
    let path = site_dir
        .path()
        .join("v1/i-zhirov/hello-dos/versions")
        .join(format!("sha256-{hex_digest}"))
        .join("manifest.json");
    fs::write(&path, b"{}").expect("tamper the manifest");

    let error = client.manifest(&version).expect_err("a mismatch");

    assert!(matches!(error, RemoteError::DigestMismatch { .. }));
}

#[test]
fn a_tampered_config_fails() {
    let (_store, site_dir, server) = site();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let repository = Repository::parse("i-zhirov/hello-dos").expect("a repository");
    let index = client.image_index(&repository).expect("the index");
    let version = index.versions[0].clone();
    let hex_digest = version.digest.trim_start_matches("sha256:");
    let path = site_dir
        .path()
        .join("v1/i-zhirov/hello-dos/versions")
        .join(format!("sha256-{hex_digest}"))
        .join("config.json");
    fs::write(&path, b"{}").expect("tamper the config");

    let error = client.config(&version).expect_err("a mismatch");

    assert!(matches!(error, RemoteError::DigestMismatch { .. }));
}

#[test]
fn a_missing_image_reports_not_found() {
    let (_store, _site, server) = site();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let repository = Repository::parse("i-zhirov/missing").expect("a repository");

    let error = client.image_index(&repository).expect_err("not found");

    assert!(matches!(error, RemoteError::ImageNotFound(_)));
}

#[test]
fn an_explicit_loopback_host_uses_http() {
    let (_store, _site, server) = site();
    let host = server.url().trim_start_matches("http://").to_owned();
    let reference = RegistryReference::parse(&format!("{host}/i-zhirov/hello-dos:latest"))
        .expect("a reference");

    let client = RegistryClient::for_reference(&reference).expect("a client");

    assert_eq!(client.base(), format!("{}/", server.url()));
}

#[test]
fn the_fixture_documents_deserialize() {
    let (_store, _site, server) = site();
    let client = RegistryClient::new(&server.url()).expect("a client");
    let repository = Repository::parse("i-zhirov/hello-dos").expect("a repository");

    let index = client.image_index(&repository).expect("the index");

    assert_eq!(index.image, "i-zhirov/hello-dos");
    assert!(index.versions[0].asset_size > 0);
}
