mod support;

use std::fs;
use std::path::PathBuf;

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

    let (_first, first_cached, first_resumed) =
        cache.get_or_download(&client, &version).expect("first");
    let (second_path, second_cached, second_resumed) =
        cache.get_or_download(&client, &version).expect("second");

    assert!(!first_cached);
    assert!(!first_resumed);
    assert!(second_cached);
    assert!(!second_resumed);
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
    let (path, _, _) = cache.get_or_download(&client, &version).expect("first");
    fs::write(&path, b"tampered").expect("tamper the cache");
    server.reset_requests();

    let (_, cached, resumed) = cache.get_or_download(&client, &version).expect("second");

    assert!(!cached);
    assert!(!resumed);
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

    let (path, _, _) = cache.get_or_download(&client, &version).expect("download");

    assert!(path.is_file());
}

#[test]
fn an_interrupted_download_is_resumed() {
    let (_store, _site, server, client, version, cache) = fixture();
    server.reset_requests();
    server.fail_next_asset(512);
    let path = cache
        .asset_path(&version.asset_digest)
        .expect("the asset path");
    let part = PathBuf::from(format!("{}.part", path.display()));

    let error = cache
        .get_or_download(&client, &version)
        .expect_err("a truncated asset");

    assert!(matches!(error, RemoteError::Http(_)));
    assert_eq!(fs::metadata(&part).expect("the partial file").len(), 512);
    assert!(!path.exists());

    let (final_path, cached, resumed) = cache
        .get_or_download(&client, &version)
        .expect("the resumed pull");

    assert_eq!(final_path, path);
    assert!(!cached);
    assert!(resumed);
    assert!(path.is_file());
    assert!(!part.exists());
    assert_eq!(server.ranges(), vec![None, Some("bytes=512-".to_owned())]);
}

#[test]
fn a_complete_but_corrupt_partial_is_deleted() {
    let (_store, _site, _server, client, version, cache) = fixture();
    let path = cache
        .asset_path(&version.asset_digest)
        .expect("the asset path");
    let part = PathBuf::from(format!("{}.part", path.display()));
    fs::create_dir_all(part.parent().expect("the cache directory")).expect("the cache directory");
    fs::write(&part, vec![0u8; version.asset_size as usize]).expect("the partial file");

    let error = cache
        .get_or_download(&client, &version)
        .expect_err("a corrupt partial");

    assert!(matches!(error, RemoteError::DigestMismatch { .. }));
    assert!(!part.exists());
    assert!(!path.exists());

    let (_, cached, resumed) = cache
        .get_or_download(&client, &version)
        .expect("the fresh pull");

    assert!(!cached);
    assert!(!resumed);
    assert!(path.is_file());
}

#[test]
fn a_partial_with_a_corrupt_prefix_fails() {
    let (_store, _site, _server, client, version, cache) = fixture();
    let path = cache
        .asset_path(&version.asset_digest)
        .expect("the asset path");
    let part = PathBuf::from(format!("{}.part", path.display()));
    fs::create_dir_all(part.parent().expect("the cache directory")).expect("the cache directory");
    fs::write(&part, vec![0u8; 512]).expect("the partial file");

    let error = cache
        .get_or_download(&client, &version)
        .expect_err("a corrupt prefix");

    assert!(matches!(error, RemoteError::DigestMismatch { .. }));
    assert!(!part.exists());
    assert!(!path.exists());
}

#[test]
fn an_oversize_partial_is_restarted() {
    let (_store, _site, _server, client, version, cache) = fixture();
    let path = cache
        .asset_path(&version.asset_digest)
        .expect("the asset path");
    let part = PathBuf::from(format!("{}.part", path.display()));
    fs::create_dir_all(part.parent().expect("the cache directory")).expect("the cache directory");
    fs::write(&part, vec![0u8; version.asset_size as usize + 1]).expect("the partial file");

    let (_, cached, resumed) = cache
        .get_or_download(&client, &version)
        .expect("the fresh pull");

    assert!(!cached);
    assert!(!resumed);
    assert!(path.is_file());
    assert!(!part.exists());
}

#[test]
fn a_concurrent_download_is_refused() {
    use fs2::FileExt;

    let (_store, _site, _server, client, version, cache) = fixture();
    let path = cache
        .asset_path(&version.asset_digest)
        .expect("the asset path");
    let part = PathBuf::from(format!("{}.part", path.display()));
    fs::create_dir_all(part.parent().expect("the cache directory")).expect("the cache directory");
    let lock_path = PathBuf::from(format!("{}.lock", part.display()));
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("the lock file");
    lock.lock_exclusive().expect("the lock");

    let error = cache
        .get_or_download(&client, &version)
        .expect_err("a locked asset");

    assert!(matches!(error, RemoteError::DownloadInProgress(_)));
    FileExt::unlock(&lock).expect("the unlock");
}
