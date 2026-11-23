//! Fixture site images built from the real image store.

use std::collections::BTreeMap;

use dosker_core::{LicenseBasis, LicenseEntry, LicenseStatus, license_summary};
use dosker_image::{ImageConfig, ImageRef, ImageReference, Layer, OciStore, Tree, save_image};
use dosker_test_support::registry_site::{SiteImage, StaticServer, build_site};

/// Builds one site image from an image in the store.
pub fn site_image(
    store: &OciStore,
    reference: &ImageReference,
    version: &str,
    tag: &str,
    description: &str,
    published_at: &str,
) -> SiteImage {
    let image = store
        .load(&ImageRef::Tag(reference.clone()))
        .expect("the fixture image loads");
    let manifest_bytes = store
        .read_blob(&image.manifest_digest)
        .expect("the manifest reads");
    let config_bytes = store
        .read_blob(&image.config_digest)
        .expect("the config reads");
    let config: ImageConfig = serde_json::from_slice(&config_bytes).expect("the config parses");
    let summary = license_summary(&config.licenses)
        .map(|summary| summary.as_str().to_owned())
        .unwrap_or_else(|| "redistributable".to_owned());

    let tar = tempfile::NamedTempFile::new().expect("a temporary tar");
    save_image(store, reference, tar.path()).expect("the image saves");
    let asset_tar = std::fs::read(tar.path()).expect("the tar reads");

    SiteImage {
        owner: reference
            .repository()
            .namespace()
            .expect("a namespaced fixture")
            .to_string(),
        name: reference.repository().name().to_string(),
        version: version.to_owned(),
        tag: tag.to_owned(),
        description: description.to_owned(),
        homepage: None,
        deprecated: None,
        license_summary: summary,
        manifest_bytes,
        config_bytes,
        asset_tar,
        published_at: published_at.to_owned(),
    }
}

/// One image specification for a fixture site.
pub struct SiteSpec {
    pub reference: String,
    pub version: String,
    pub tag: String,
    pub description: String,
    pub licenses: Vec<LicenseEntry>,
}

/// Returns one redistributable license entry.
pub fn redistributable() -> LicenseEntry {
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

/// Returns one abandonware license entry.
pub fn abandonware() -> LicenseEntry {
    LicenseEntry {
        name: "OLD".to_owned(),
        spdx: "NONE".to_owned(),
        holder: None,
        status: LicenseStatus::Abandonware,
        basis: Some(LicenseBasis::GoodFaith),
        evidence: Some("https://example.test/evidence".to_owned()),
        license_url: None,
        covers: Vec::new(),
        source_url: None,
        notice: Some("Old content.".to_owned()),
    }
}

/// Builds a store and a served fixture site for the specifications.
pub fn fixture_site(specs: &[SiteSpec]) -> (tempfile::TempDir, tempfile::TempDir, StaticServer) {
    let store_dir = tempfile::tempdir().expect("a store directory");
    let store = OciStore::new(store_dir.path().to_path_buf());
    let mut images = Vec::new();
    for spec in specs {
        let layer =
            Layer::from_delta(&Tree::new().to_delta(), "test", None, None).expect("a layer");
        let descriptor = store.put_layer(&layer).expect("the layer stores");
        let mut config = super::sample_config(super::sample_boot());
        config.licenses = spec.licenses.clone();
        let reference = ImageReference::parse(&spec.reference).expect("a reference");
        store
            .commit_image(Some(&reference), &config, vec![descriptor], BTreeMap::new())
            .expect("the image commits");
        images.push(site_image(
            &store,
            &reference,
            &spec.version,
            &spec.tag,
            &spec.description,
            "2026-11-23T12:00:00Z",
        ));
    }

    let site_dir = tempfile::tempdir().expect("a site directory");
    let server = StaticServer::start(site_dir.path().to_path_buf()).expect("the server");
    build_site(
        site_dir.path(),
        &server.url(),
        "2026-11-23T12:00:00Z",
        &images,
    )
    .expect("the site builds");
    (store_dir, site_dir, server)
}
