mod support;

use std::fs;

use dosker_image::ImageTag;
use dosker_image::ReferenceSelector;
use dosker_image::Repository;
use dosker_image::remote::{RegistryClient, RemoteCache, RemoteError, VersionRecord};
use dosker_test_support::registry_site::StaticServer;
use support::site::{SiteSpec, abandonware, fixture_site, redistributable};

fn fixture() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    StaticServer,
    RegistryClient,
    VersionRecord,
    RemoteCache,
) {
    let (store_dir, site_dir, server) = fixture_site(&[SiteSpec {
        reference: "i-zhirov/hello-dos:1.0.0".to_owned(),
        version: "1.0.0".to_owned(),
        tag: "latest".to_owned(),
        description: "Fixture image.".to_owned(),
        licenses: vec![redistributable()],
    }]);
    let client = RegistryClient::new(&server.url()).expect("a client");
    let index = client
        .image_index(&Repository::parse("i-zhirov/hello-dos").unwrap())
        .expect("the index");
    let version = client
        .resolve(
            &index,
            &ReferenceSelector::Tag(ImageTag::parse("latest").unwrap()),
        )
        .expect("the version")
        .clone();
    let cache = RemoteCache::new(store_dir.path().to_path_buf());
    (store_dir, site_dir, server, client, version, cache)
}

#[test]
fn a_second_pull_reuses_the_cached_asset() {
    let (_store, _site, server, client, version, cache) = fixture();

    let (_first, first_cached) = cache.get_or_download(&client, &version).expect("first");
    let (second_path, second_cached) = cache.get_or_download(&client, &version).expect("second");

    assert!(!first_cached);
    assert!(second_cached);
    assert!(second_path.is_file());
    let tar_requests = server
        .requests()
        .into_iter()
        .filter(|path| path.ends_with(".tar"))
        .count();
    assert_eq!(tar_requests, 1);
}

#[test]
fn a_tampered_cached_asset_is_downloaded_again() {
    let (_store, _site, server, client, version, cache) = fixture();
    let (path, _) = cache.get_or_download(&client, &version).expect("first");
    fs::write(&path, b"tampered").expect("tamper the cache");
    server.reset_requests();

    let (_, cached) = cache.get_or_download(&client, &version).expect("second");

    assert!(!cached);
    assert!(
        server
            .requests()
            .iter()
            .any(|request| request.ends_with(".tar"))
    );
}

#[test]
fn a_tampered_server_asset_fails() {
    let (_store, site_dir, _server, client, version, cache) = fixture();
    let asset = site_dir
        .path()
        .join("assets")
        .join(format!("{}.tar", version.asset_digest));
    fs::write(&asset, b"tampered").expect("tamper the asset");

    let error = cache
        .get_or_download(&client, &version)
        .expect_err("a mismatch");

    assert!(matches!(error, RemoteError::DigestMismatch { .. }));
}

#[test]
fn a_new_digest_uses_a_new_cache_entry() {
    let (_store, _site, _server, _client, version, cache) = fixture();
    let other = format!("sha256:{}", "0".repeat(64));

    let first = cache.asset_path(&version.asset_digest).expect("first path");
    let second = cache.asset_path(&other).expect("second path");

    assert_ne!(first, second);
}

#[test]
fn an_unsafe_digest_is_rejected() {
    let (_store, _site, _server, _client, _version, cache) = fixture();

    assert!(matches!(
        cache.asset_path("../escape"),
        Err(RemoteError::Store(_))
    ));
}

#[test]
fn an_abandonware_site_asset_is_available() {
    let (store_dir, _site, server) = fixture_site(&[SiteSpec {
        reference: "i-zhirov/old-game:1.0.0".to_owned(),
        version: "1.0.0".to_owned(),
        tag: "latest".to_owned(),
        description: "Old game.".to_owned(),
        licenses: vec![abandonware()],
    }]);
    let client = RegistryClient::new(&server.url()).expect("a client");
    let index = client
        .image_index(&Repository::parse("i-zhirov/old-game").unwrap())
        .expect("the index");
    let version = client
        .resolve(
            &index,
            &ReferenceSelector::Tag(ImageTag::parse("latest").unwrap()),
        )
        .expect("the version")
        .clone();
    let cache = RemoteCache::new(store_dir.path().to_path_buf());

    let (path, _) = cache.get_or_download(&client, &version).expect("download");

    assert!(path.is_file());
}
