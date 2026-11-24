//! The static registry client.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use dosker_core::{LicenseEntry, LicenseSummary, license_summary, validate_licenses};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::acceptance::{ACCEPTANCE_SCHEMA_VERSION, AcceptanceStore, AcceptedImage};
use crate::archive::{AssetCompression, load_image_as};
use crate::canonical::sha256_digest;
use crate::config::ImageConfig;
use crate::error::ImageError;
use crate::http::{self, HttpError};
use crate::manifest::Manifest;
use crate::oci::OciStore;
use crate::reference::{
    ImageDigest, ImageReference, ReferenceSelector, RegistryReference, Repository,
    default_registry_url,
};

/// The `/index.json` search catalog.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Catalog {
    pub schema: u32,
    pub generated_at: String,
    pub registry: String,
    pub images: Vec<CatalogImage>,
}

/// One catalog entry.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CatalogImage {
    pub namespace: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub deprecated: Option<String>,
    pub tags: BTreeMap<String, String>,
    pub license_summary: LicenseSummary,
    pub requires_acceptance: bool,
    pub latest: Latest,
}

/// The current version of one image.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Latest {
    pub version: String,
    pub digest: String,
}

/// One per-image index.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ImageIndex {
    pub schema: u32,
    pub image: String,
    pub namespace: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub deprecated: Option<String>,
    pub tags: BTreeMap<String, String>,
    pub versions: Vec<VersionRecord>,
}

/// One version in an image index.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VersionRecord {
    pub version: String,
    pub digest: String,
    pub config_digest: String,
    pub manifest_url: String,
    pub config_url: String,
    pub asset_url: String,
    pub asset_digest: String,
    pub asset_size: u64,
    pub asset_compression: AssetCompression,
    pub published_at: String,
    pub license_summary: LicenseSummary,
    pub licenses_url: String,
}

/// The local registry asset cache.
#[derive(Clone, Debug)]
pub struct RemoteCache {
    root: PathBuf,
}

impl RemoteCache {
    /// Creates a cache under `<root>/cache/registry/assets`.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: root.join("cache").join("registry").join("assets"),
        }
    }

    /// Returns the cache path for one asset digest.
    pub fn asset_path(&self, digest: &str) -> Result<PathBuf, RemoteError> {
        let parsed = ImageDigest::parse(digest).map_err(RemoteError::Store)?;
        let hex = parsed
            .as_str()
            .strip_prefix("sha256:")
            .expect("a sha256 digest");
        Ok(self.root.join(format!("{hex}.tar.zst")))
    }

    /// Returns the partial path for one asset path.
    fn part_path(&self, path: &Path) -> PathBuf {
        PathBuf::from(format!("{}.part", path.display()))
    }

    /// Takes the exclusive lock for one partial asset.
    fn lock_part(&self, part: &Path, digest: &str) -> Result<PartLock, RemoteError> {
        let path = PathBuf::from(format!("{}.lock", part.display()));
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|source| RemoteError::Io {
                path: path.clone(),
                source,
            })?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(PartLock { file }),
            Err(source) if source.kind() == io::ErrorKind::WouldBlock => {
                Err(RemoteError::DownloadInProgress(digest.to_owned()))
            }
            Err(source) => Err(RemoteError::Io { path, source }),
        }
    }

    /// Returns a verified cached asset, or resumes or downloads it.
    ///
    /// The returned flags are `(cached, resumed)`. `cached` means the completed
    /// asset was already verified; `resumed` means a partial file supplied the
    /// asset, either by a Range transfer or by completing it locally.
    pub fn get_or_download(
        &self,
        client: &RegistryClient,
        version: &VersionRecord,
    ) -> Result<(PathBuf, bool, bool), RemoteError> {
        let path = self.asset_path(&version.asset_digest)?;
        if path.is_file() {
            let (bytes, digest) = hash_file(&path).map_err(|source| RemoteError::Io {
                path: path.clone(),
                source,
            })?;
            if digest == version.asset_digest && bytes == version.asset_size {
                return Ok((path, true, false));
            }
            let _ = fs::remove_file(&path);
        }
        fs::create_dir_all(&self.root).map_err(|source| RemoteError::Io {
            path: self.root.clone(),
            source,
        })?;
        let part = self.part_path(&path);
        let _lock = self.lock_part(&part, &version.asset_digest)?;
        let mut offset = match fs::metadata(&part) {
            Ok(metadata) => metadata.len(),
            Err(source) if source.kind() == io::ErrorKind::NotFound => 0,
            Err(source) => {
                return Err(RemoteError::Io {
                    path: part.clone(),
                    source,
                });
            }
        };
        if offset > version.asset_size {
            let _ = fs::remove_file(&part);
            offset = 0;
        }
        if offset == version.asset_size {
            let (bytes, digest) = hash_file(&part).map_err(|source| RemoteError::Io {
                path: part.clone(),
                source,
            })?;
            if digest == version.asset_digest && bytes == version.asset_size {
                fs::rename(&part, &path).map_err(|source| RemoteError::Io {
                    path: path.clone(),
                    source,
                })?;
                return Ok((path, false, true));
            }
            let _ = fs::remove_file(&part);
            return Err(RemoteError::DigestMismatch {
                expected: version.asset_digest.clone(),
                actual: digest,
            });
        }
        let report = client.download_asset(version, &part, offset, &path)?;
        Ok((path, false, report.resumed))
    }
}

/// An exclusive lock on one partial asset.
struct PartLock {
    file: fs::File,
}

impl Drop for PartLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// Hashes one file while counting its bytes.
fn hash_file(path: &Path) -> io::Result<(u64, String)> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((total, format!("sha256:{}", hex(&hasher.finalize()))))
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// The data for a refused non-redistributable image.
#[derive(Debug)]
pub struct LicenseRefusal {
    pub image: String,
    pub reference: String,
    pub digest: String,
    pub config_digest: String,
    pub summary: String,
    pub entries: Vec<LicenseEntry>,
}

impl std::fmt::Display for LicenseRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "image {} needs license acceptance ({})\nimage digest: {}\nconfig digest: {}\nTo accept: dosker pull {} --accept-license {}",
            self.image, self.summary, self.digest, self.config_digest, self.reference, self.digest
        )
    }
}

/// One pull request.
#[derive(Clone, Debug)]
pub struct PullRequest {
    pub reference: RegistryReference,
    pub accept: Option<ImageDigest>,
}

/// The result of one pull.
#[derive(Clone, Debug)]
pub struct PullReport {
    pub image: String,
    pub tag: Option<String>,
    pub digest: String,
    pub config_digest: String,
    pub license_summary: LicenseSummary,
    pub asset_bytes: u64,
    pub cached: bool,
    pub resumed: bool,
}

/// A pull failure.
#[derive(Debug, thiserror::Error)]
pub enum PullError {
    /// The registry request failed.
    #[error(transparent)]
    Remote(#[from] RemoteError),

    /// The local store failed.
    #[error(transparent)]
    Store(#[from] ImageError),

    /// The reference needs a namespace.
    #[error("pull needs a namespaced reference")]
    NotNamespaced,

    /// The config has no license entries.
    #[error("image {image} has no license entries")]
    MissingLicense { image: String },

    /// A license entry is invalid.
    #[error("the image has an invalid license entry: {reason}")]
    InvalidLicense { reason: String },

    /// The acceptance digest does not match the resolved digest.
    #[error("the acceptance digest {provided} does not match {expected}")]
    AcceptMismatch { expected: String, provided: String },

    /// The image needs a license acceptance.
    #[error("{0}")]
    LicenseNotAccepted(Box<LicenseRefusal>),
}

/// Pulls one image through the registry and stores it.
pub fn pull(
    client: &RegistryClient,
    store: &OciStore,
    cache: &RemoteCache,
    acceptance: &AcceptanceStore,
    request: &PullRequest,
    now: DateTime<Utc>,
) -> Result<PullReport, PullError> {
    let repository = request.reference.repository();
    let namespace = repository.namespace().ok_or(PullError::NotNamespaced)?;
    let index = client.image_index(repository)?;
    let version = client
        .resolve(&index, request.reference.selector())?
        .clone();

    client.manifest(&version)?;
    let config = client.config(&version)?;
    let summary = license_summary(&config.licenses).ok_or_else(|| PullError::MissingLicense {
        image: index.image.clone(),
    })?;
    validate_licenses(&config.licenses).map_err(|error| PullError::InvalidLicense {
        reason: error.to_string(),
    })?;

    if let Some(provided) = &request.accept
        && provided.as_str() != version.digest
    {
        return Err(PullError::AcceptMismatch {
            expected: version.digest.clone(),
            provided: provided.as_str().to_owned(),
        });
    }
    let already_accepted = acceptance.is_accepted(&version.digest)?;
    if summary != LicenseSummary::Redistributable && !already_accepted && request.accept.is_none() {
        return Err(PullError::LicenseNotAccepted(Box::new(LicenseRefusal {
            image: index.image.clone(),
            reference: request.reference.to_string(),
            digest: version.digest.clone(),
            config_digest: version.config_digest.clone(),
            summary: summary.as_str().to_owned(),
            entries: config.licenses.clone(),
        })));
    }

    let (asset, cached, resumed) = cache.get_or_download(client, &version)?;
    let local = local_reference(repository, namespace, request, &version)?;
    load_image_as(
        store,
        &asset,
        &local,
        &version.digest,
        &version.config_digest,
        version.asset_compression,
    )?;

    if summary != LicenseSummary::Redistributable && !already_accepted {
        acceptance.record(&AcceptedImage {
            schema_version: ACCEPTANCE_SCHEMA_VERSION,
            digest: version.digest.clone(),
            config_digest: version.config_digest.clone(),
            image: index.image.clone(),
            license_summary: summary.as_str().to_owned(),
            accepted_at: now,
        })?;
    }

    Ok(PullReport {
        image: index.image,
        tag: Some(local.tag().as_str().to_owned()),
        digest: version.digest,
        config_digest: version.config_digest,
        license_summary: summary,
        asset_bytes: version.asset_size,
        cached,
        resumed,
    })
}

fn local_reference(
    repository: &Repository,
    namespace: &crate::reference::Namespace,
    request: &PullRequest,
    version: &VersionRecord,
) -> Result<ImageReference, PullError> {
    match request.reference.local_reference() {
        Some(reference) => Ok(reference),
        None => ImageReference::parse(&format!(
            "{namespace}/{}:{}",
            repository.name(),
            version.version
        ))
        .map_err(PullError::Store),
    }
}

/// A registry client failure.
#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    /// The HTTP request failed.
    #[error(transparent)]
    Http(#[from] HttpError),

    /// The image or the requested document does not exist.
    #[error("image not found: {0}")]
    ImageNotFound(String),

    /// The tag is not in the image index.
    #[error("tag not found: {0}")]
    TagNotFound(String),

    /// The digest is not in the image index.
    #[error("digest not found: {0}")]
    DigestNotFound(String),

    /// The bytes do not match the expected digest.
    #[error("digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch { expected: String, actual: String },

    /// A document is malformed or inconsistent.
    #[error("invalid registry document at {0}: {1}")]
    InvalidDocument(String, String),

    /// The URL is not safe.
    #[error("unsafe URL: {0}")]
    UnsafeUrl(String),

    /// A local file operation failed.
    #[error("i/o error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },

    /// Another pull is downloading the same asset.
    #[error("another pull is downloading {0}")]
    DownloadInProgress(String),

    /// The local image store failed.
    #[error(transparent)]
    Store(#[from] ImageError),
}

/// A blocking client for one registry base.
#[derive(Clone, Debug)]
pub struct RegistryClient {
    base: String,
}

impl RegistryClient {
    /// Builds a client for one base URL.
    pub fn new(base: &str) -> Result<Self, RemoteError> {
        let base = base.trim_end_matches('/');
        let url = Url::parse(base).map_err(|_| RemoteError::UnsafeUrl(base.to_owned()))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(RemoteError::UnsafeUrl(base.to_owned()));
        }
        Ok(Self {
            base: format!("{base}/"),
        })
    }

    /// Builds a client for one reference.
    pub fn for_reference(reference: &RegistryReference) -> Result<Self, RemoteError> {
        match reference.host() {
            Some(host) => {
                let scheme = if host_is_loopback(host.as_str()) {
                    "http"
                } else {
                    "https"
                };
                Self::new(&format!("{scheme}://{host}"))
            }
            None => Self::new(&default_registry_url()),
        }
    }

    /// Returns the base URL, with a trailing slash.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Fetches the catalog.
    pub fn catalog(&self) -> Result<Catalog, RemoteError> {
        let url = self.url("index.json")?;
        let bytes = self.document(url.as_str())?;
        serde_json::from_slice(&bytes)
            .map_err(|error| RemoteError::InvalidDocument(url.to_string(), error.to_string()))
    }

    /// Fetches one image index.
    pub fn image_index(&self, repository: &Repository) -> Result<ImageIndex, RemoteError> {
        let namespace = repository
            .namespace()
            .ok_or_else(|| RemoteError::ImageNotFound(repository.to_string()))?;
        let name = repository.name();
        let url = self.url(&format!("v1/{namespace}/{name}/index.json"))?;
        let bytes = self.document(url.as_str())?;
        let index: ImageIndex = serde_json::from_slice(&bytes)
            .map_err(|error| RemoteError::InvalidDocument(url.to_string(), error.to_string()))?;
        if index.namespace != namespace.as_str() || index.name != name.as_str() {
            return Err(RemoteError::InvalidDocument(
                url.to_string(),
                "the index names a different image".to_owned(),
            ));
        }
        Ok(index)
    }

    /// Resolves a tag or digest to one version.
    pub fn resolve<'a>(
        &self,
        index: &'a ImageIndex,
        selector: &ReferenceSelector,
    ) -> Result<&'a VersionRecord, RemoteError> {
        match selector {
            ReferenceSelector::Tag(tag) => {
                let digest = index
                    .tags
                    .get(tag.as_str())
                    .ok_or_else(|| RemoteError::TagNotFound(tag.as_str().to_owned()))?;
                index
                    .versions
                    .iter()
                    .find(|version| &version.digest == digest)
                    .ok_or_else(|| RemoteError::DigestNotFound(digest.clone()))
            }
            ReferenceSelector::Digest(digest) => index
                .versions
                .iter()
                .find(|version| version.digest == digest.as_str())
                .ok_or_else(|| RemoteError::DigestNotFound(digest.as_str().to_owned())),
        }
    }

    /// Fetches and verifies the manifest.
    pub fn manifest(&self, version: &VersionRecord) -> Result<Vec<u8>, RemoteError> {
        let bytes = http::get(&version.manifest_url, http::MAX_DOCUMENT_BYTES)?;
        verify(&bytes, &version.digest)?;
        let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|error| {
            RemoteError::InvalidDocument(version.manifest_url.clone(), error.to_string())
        })?;
        if manifest.config.digest != version.config_digest {
            return Err(RemoteError::DigestMismatch {
                expected: version.config_digest.clone(),
                actual: manifest.config.digest,
            });
        }
        Ok(bytes)
    }

    /// Fetches and verifies the config.
    pub fn config(&self, version: &VersionRecord) -> Result<ImageConfig, RemoteError> {
        let bytes = http::get(&version.config_url, http::MAX_DOCUMENT_BYTES)?;
        verify(&bytes, &version.config_digest)?;
        let config: ImageConfig = serde_json::from_slice(&bytes).map_err(|error| {
            RemoteError::InvalidDocument(version.config_url.clone(), error.to_string())
        })?;
        config.validate().map_err(|error| {
            RemoteError::InvalidDocument(version.config_url.clone(), error.to_string())
        })?;
        Ok(config)
    }

    /// Resumes or downloads and verifies the asset, then installs it.
    pub fn download_asset(
        &self,
        version: &VersionRecord,
        part: &Path,
        offset: u64,
        destination: &Path,
    ) -> Result<http::DownloadReport, RemoteError> {
        let report = http::download_part(
            &version.asset_url,
            http::MAX_ASSET_BYTES,
            part,
            offset,
            version.asset_size,
        )?;
        if report.digest != version.asset_digest {
            let _ = fs::remove_file(part);
            return Err(RemoteError::DigestMismatch {
                expected: version.asset_digest.clone(),
                actual: report.digest,
            });
        }
        if report.bytes != version.asset_size {
            let _ = fs::remove_file(part);
            return Err(RemoteError::DigestMismatch {
                expected: format!("{} bytes", version.asset_size),
                actual: format!("{} bytes", report.bytes),
            });
        }
        fs::rename(part, destination).map_err(|source| RemoteError::Io {
            path: destination.to_path_buf(),
            source,
        })?;
        Ok(report)
    }

    fn url(&self, relative: &str) -> Result<Url, RemoteError> {
        let base = Url::parse(&self.base).map_err(|_| RemoteError::UnsafeUrl(self.base.clone()))?;
        base.join(relative)
            .map_err(|_| RemoteError::UnsafeUrl(relative.to_owned()))
    }

    fn document(&self, url: &str) -> Result<Vec<u8>, RemoteError> {
        match http::get(url, http::MAX_DOCUMENT_BYTES) {
            Ok(bytes) => Ok(bytes),
            Err(HttpError::Status(404)) => Err(RemoteError::ImageNotFound(url.to_owned())),
            Err(error) => Err(RemoteError::Http(error)),
        }
    }
}

fn verify(bytes: &[u8], expected: &str) -> Result<(), RemoteError> {
    let actual = sha256_digest(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(RemoteError::DigestMismatch {
            expected: expected.to_owned(),
            actual,
        })
    }
}

fn host_is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
        || host.starts_with("localhost:")
        || host.starts_with("127.0.0.1:")
        || host.starts_with("[::1]:")
}
