//! Image names, tags, and references.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::ImageError;

const DEFAULT_TAG: &str = "latest";
const MAX_NAME_BYTES: usize = 63;
const MAX_TAG_BYTES: usize = 128;

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
    name: ImageName,
    tag: ImageTag,
}

impl ImageReference {
    /// Parses `name[:tag]`, defaulting the tag to `latest`.
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

        let name =
            ImageName::parse(name).map_err(|_| ImageError::InvalidReference(value.to_owned()))?;
        let tag =
            ImageTag::parse(tag).map_err(|_| ImageError::InvalidReference(value.to_owned()))?;
        Ok(Self { name, tag })
    }

    /// Borrows the image name.
    pub fn name(&self) -> &ImageName {
        &self.name
    }

    /// Borrows the image tag.
    pub fn tag(&self) -> &ImageTag {
        &self.tag
    }
}

impl fmt::Display for ImageReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.name, self.tag)
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
