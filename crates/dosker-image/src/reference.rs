//! Image names, tags, and references.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::ImageError;

const DEFAULT_TAG: &str = "latest";
const MAX_NAME_BYTES: usize = 63;
const MAX_TAG_BYTES: usize = 128;

/// The default registry base URL.
pub const DEFAULT_REGISTRY: &str = "https://i-zhirov.github.io/dosker-registry";

/// The environment variable that overrides the default registry base URL.
pub const REGISTRY_ENV: &str = "DOSKER_REGISTRY";

/// Returns the registry base URL from `DOSKER_REGISTRY`, or the default.
///
/// An empty value is ignored. A trailing slash is trimmed.
pub fn default_registry_url() -> String {
    let value = std::env::var(REGISTRY_ENV).unwrap_or_default();
    let value = value.trim();
    if value.is_empty() {
        DEFAULT_REGISTRY.to_owned()
    } else {
        value.trim_end_matches('/').to_owned()
    }
}

/// A registry namespace: a lowercase GitHub handle.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Namespace(String);

impl Namespace {
    /// Parses a namespace handle.
    pub fn parse(value: &str) -> Result<Self, ImageError> {
        if is_valid_namespace(value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(ImageError::InvalidReference(value.to_owned()))
        }
    }

    /// Borrows the handle text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Namespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Namespace {
    type Error = ImageError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<Namespace> for String {
    fn from(value: Namespace) -> Self {
        value.0
    }
}

/// An image name: one to sixty-three bytes matching
/// `[a-z0-9][a-z0-9_.-]{0,62}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ImageName(String);

impl ImageName {
    /// Parses a name, rejecting anything outside the allowed pattern.
    pub fn parse(value: &str) -> Result<Self, ImageError> {
        if is_valid(value, MAX_NAME_BYTES, is_name_first, is_name_rest) {
            Ok(Self(value.to_owned()))
        } else {
            Err(ImageError::InvalidReference(value.to_owned()))
        }
    }

    /// Borrows the name text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ImageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ImageName {
    type Error = ImageError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ImageName> for String {
    fn from(value: ImageName) -> Self {
        value.0
    }
}

/// A repository path: an optional namespace and a name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Repository {
    namespace: Option<Namespace>,
    name: ImageName,
}

impl Repository {
    /// Builds a repository from its parts.
    pub fn new(namespace: Option<Namespace>, name: ImageName) -> Self {
        Self { namespace, name }
    }

    /// Parses `[namespace/]name`.
    pub fn parse(value: &str) -> Result<Self, ImageError> {
        let mut parts = value.split('/');
        let first = parts.next().unwrap_or_default();
        let second = parts.next();
        let third = parts.next();
        match (first, second, third) {
            ("", _, _) => Err(ImageError::InvalidReference(value.to_owned())),
            (name, None, None) => Ok(Self {
                namespace: None,
                name: ImageName::parse(name)?,
            }),
            (namespace, Some(name), None) if !namespace.is_empty() && !name.is_empty() => {
                Ok(Self {
                    namespace: Some(Namespace::parse(namespace)?),
                    name: ImageName::parse(name)?,
                })
            }
            _ => Err(ImageError::InvalidReference(value.to_owned())),
        }
    }

    /// Borrows the namespace, if present.
    pub fn namespace(&self) -> Option<&Namespace> {
        self.namespace.as_ref()
    }

    /// Borrows the image name.
    pub fn name(&self) -> &ImageName {
        &self.name
    }

    /// Returns whether the path has a namespace.
    pub fn is_namespaced(&self) -> bool {
        self.namespace.is_some()
    }
}

impl fmt::Display for Repository {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(namespace) = &self.namespace {
            write!(f, "{namespace}/{}", self.name)
        } else {
            write!(f, "{}", self.name)
        }
    }
}

impl TryFrom<String> for Repository {
    type Error = ImageError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<Repository> for String {
    fn from(value: Repository) -> Self {
        value.to_string()
    }
}

/// An image tag: one to one hundred twenty-eight bytes matching
/// `[A-Za-z0-9][A-Za-z0-9_.-]{0,127}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ImageTag(String);

impl ImageTag {
    /// Parses a tag, rejecting anything outside the allowed pattern.
    pub fn parse(value: &str) -> Result<Self, ImageError> {
        if is_valid(value, MAX_TAG_BYTES, is_tag_first, is_tag_rest) {
            Ok(Self(value.to_owned()))
        } else {
            Err(ImageError::InvalidReference(value.to_owned()))
        }
    }

    /// Borrows the tag text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ImageTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ImageTag {
    type Error = ImageError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ImageTag> for String {
    fn from(value: ImageTag) -> Self {
        value.0
    }
}

/// A named and tagged image reference.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ImageReference {
    repository: Repository,
    tag: ImageTag,
}

impl ImageReference {
    /// Parses `[namespace/]name[:tag]`, defaulting the tag to `latest`.
    pub fn parse(value: &str) -> Result<Self, ImageError> {
        let (name, tag) = match value.split_once(':') {
            None => (value, DEFAULT_TAG),
            Some((name, tag)) => {
                if tag.is_empty() || tag.contains(':') {
                    return Err(ImageError::InvalidReference(value.to_owned()));
                }
                (name, tag)
            }
        };

        let repository =
            Repository::parse(name).map_err(|_| ImageError::InvalidReference(value.to_owned()))?;
        let tag =
            ImageTag::parse(tag).map_err(|_| ImageError::InvalidReference(value.to_owned()))?;
        Ok(Self { repository, tag })
    }

    /// Borrows the repository path.
    pub fn repository(&self) -> &Repository {
        &self.repository
    }

    /// Borrows the namespace, if present.
    pub fn namespace(&self) -> Option<&Namespace> {
        self.repository.namespace()
    }

    /// Borrows the image name (the last repository segment).
    pub fn name(&self) -> &ImageName {
        self.repository.name()
    }

    /// Borrows the image tag.
    pub fn tag(&self) -> &ImageTag {
        &self.tag
    }
}

impl fmt::Display for ImageReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.repository, self.tag)
    }
}

/// A content-addressed `sha256:<hex>` image digest.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ImageDigest(String);

impl ImageDigest {
    /// Parses a lowercase SHA-256 digest.
    pub fn parse(value: &str) -> Result<Self, ImageError> {
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(ImageError::InvalidReference(value.to_owned()));
        };
        if hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            Ok(Self(value.to_owned()))
        } else {
            Err(ImageError::InvalidReference(value.to_owned()))
        }
    }

    /// Returns the digest text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ImageDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for ImageDigest {
    type Error = ImageError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ImageDigest> for String {
    fn from(value: ImageDigest) -> Self {
        value.0
    }
}

/// A registry host, with an optional port.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RegistryHost(String);

impl RegistryHost {
    /// Parses a host name and an optional port.
    pub fn parse(value: &str) -> Result<Self, ImageError> {
        match normalize_host(value) {
            Some(host) => Ok(Self(host)),
            None => Err(ImageError::InvalidReference(value.to_owned())),
        }
    }

    /// Borrows the normalized host text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RegistryHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for RegistryHost {
    type Error = ImageError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<RegistryHost> for String {
    fn from(value: RegistryHost) -> Self {
        value.0
    }
}

/// The tag or digest selector of a registry reference.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ReferenceSelector {
    Tag(ImageTag),
    Digest(ImageDigest),
}

impl fmt::Display for ReferenceSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tag(tag) => write!(f, ":{tag}"),
            Self::Digest(digest) => write!(f, "@{digest}"),
        }
    }
}

/// A registry reference: an optional host, a repository, and a selector.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegistryReference {
    host: Option<RegistryHost>,
    repository: Repository,
    selector: ReferenceSelector,
}

impl RegistryReference {
    /// Parses `[host/][namespace/]name[:tag|@sha256:<hex>]`.
    ///
    /// A bare `sha256:<hex>` is a local digest, not a registry reference.
    pub fn parse(value: &str) -> Result<Self, ImageError> {
        if value.starts_with("sha256:") && ImageDigest::parse(value).is_ok() {
            return Err(ImageError::InvalidReference(value.to_owned()));
        }

        let (path, selector) = match value.split_once('@') {
            Some((path, digest)) => (path, ReferenceSelector::Digest(ImageDigest::parse(digest)?)),
            None => {
                let last_slash = value.rfind('/');
                let last_colon = value.rfind(':');
                match (last_slash, last_colon) {
                    (_, None) => (value, ReferenceSelector::Tag(ImageTag::parse(DEFAULT_TAG)?)),
                    (Some(slash), Some(colon)) if colon > slash => {
                        let tag = split_tag(value, colon)?;
                        (&value[..colon], ReferenceSelector::Tag(tag))
                    }
                    (None, Some(colon)) => {
                        let tag = split_tag(value, colon)?;
                        (&value[..colon], ReferenceSelector::Tag(tag))
                    }
                    _ => (value, ReferenceSelector::Tag(ImageTag::parse(DEFAULT_TAG)?)),
                }
            }
        };

        let parts: Vec<&str> = path.split('/').collect();
        let (host, repository) = match parts.as_slice() {
            [name] => (None, Repository::new(None, ImageName::parse(name)?)),
            [namespace, name] => (
                None,
                Repository::new(Some(Namespace::parse(namespace)?), ImageName::parse(name)?),
            ),
            [host, namespace, name] => (
                Some(RegistryHost::parse(host)?),
                Repository::new(Some(Namespace::parse(namespace)?), ImageName::parse(name)?),
            ),
            _ => return Err(ImageError::InvalidReference(value.to_owned())),
        };

        Ok(Self {
            host,
            repository,
            selector,
        })
    }

    /// Borrows the explicit host, if present.
    pub fn host(&self) -> Option<&RegistryHost> {
        self.host.as_ref()
    }

    /// Borrows the repository path.
    pub fn repository(&self) -> &Repository {
        &self.repository
    }

    /// Borrows the selector.
    pub fn selector(&self) -> &ReferenceSelector {
        &self.selector
    }

    /// Returns the local reference for a tag selector, dropping the host.
    pub fn local_reference(&self) -> Option<ImageReference> {
        match &self.selector {
            ReferenceSelector::Tag(tag) => Some(ImageReference {
                repository: self.repository.clone(),
                tag: tag.clone(),
            }),
            ReferenceSelector::Digest(_) => None,
        }
    }

    /// Returns `host/repository` or `repository`, without the selector.
    pub fn identity(&self) -> String {
        match &self.host {
            Some(host) => format!("{host}/{}", self.repository),
            None => self.repository.to_string(),
        }
    }
}

impl fmt::Display for RegistryReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(host) = &self.host {
            write!(f, "{host}/{}", self.repository)?;
        } else {
            write!(f, "{}", self.repository)?;
        }
        write!(f, "{}", self.selector)
    }
}

/// A tag or digest reference.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ImageRef {
    Tag(ImageReference),
    Digest(ImageDigest),
}

impl ImageRef {
    /// Parses a bare name, `[namespace/]name[:tag]`, `name@sha256:...`, or `sha256:...`.
    pub fn parse(value: &str) -> Result<Self, ImageError> {
        if value.starts_with("sha256:") {
            return ImageDigest::parse(value).map(Self::Digest);
        }
        if let Some((name, digest)) = value.split_once('@') {
            Repository::parse(name)?;
            return ImageDigest::parse(digest).map(Self::Digest);
        }
        ImageReference::parse(value).map(Self::Tag)
    }
}

impl fmt::Display for ImageRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tag(reference) => reference.fmt(formatter),
            Self::Digest(digest) => digest.fmt(formatter),
        }
    }
}

impl TryFrom<String> for ImageReference {
    type Error = ImageError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ImageReference> for String {
    fn from(value: ImageReference) -> Self {
        value.to_string()
    }
}

fn split_tag(value: &str, colon: usize) -> Result<ImageTag, ImageError> {
    let tag = &value[colon + 1..];
    if tag.is_empty() {
        return Err(ImageError::InvalidReference(value.to_owned()));
    }
    ImageTag::parse(tag)
}

fn is_valid_namespace(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 39 {
        return false;
    }
    let first = bytes[0];
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    if bytes.len() == 1 {
        return true;
    }
    let last = bytes[bytes.len() - 1];
    if !last.is_ascii_lowercase() && !last.is_ascii_digit() {
        return false;
    }
    bytes[1..bytes.len() - 1]
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn normalize_host(value: &str) -> Option<String> {
    if value.is_empty() || value.contains('/') {
        return None;
    }

    let (host, port) = match value.rsplit_once(':') {
        Some((host, port)) => {
            if host.is_empty() || port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit())
            {
                return None;
            }
            let port = port.parse::<u16>().ok()?;
            if port == 0 {
                return None;
            }
            (host, Some(port))
        }
        None => (value, None),
    };

    if host.is_empty()
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return None;
    }

    let mut normalized = host.to_ascii_lowercase();
    if let Some(port) = port {
        normalized.push(':');
        normalized.push_str(&port.to_string());
    }
    Some(normalized)
}

fn is_valid(
    value: &str,
    max_bytes: usize,
    is_first: fn(u8) -> bool,
    is_rest: fn(u8) -> bool,
) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > max_bytes {
        return false;
    }

    let mut iter = bytes.iter().copied();
    let first = iter.next().expect("a non-empty value has a first byte");
    is_first(first) && iter.all(is_rest)
}

fn is_name_first(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit()
}

fn is_name_rest(byte: u8) -> bool {
    is_name_first(byte) || matches!(byte, b'.' | b'-' | b'_')
}

fn is_tag_first(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
}

fn is_tag_rest(byte: u8) -> bool {
    is_tag_first(byte) || matches!(byte, b'.' | b'-' | b'_')
}
