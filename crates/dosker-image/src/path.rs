//! DOS 8.3 names and paths.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::ImageError;

/// The maximum number of characters in a DOS name stem.
pub const DOS_NAME_MAX: usize = 8;
/// The maximum number of characters in a DOS extension.
pub const DOS_EXTENSION_MAX: usize = 3;
/// The only drive letter accepted by a DOS path.
pub const DOS_DRIVE: char = 'C';

/// One DOS 8.3 name component.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DosName(String);

impl DosName {
    /// Uppercases and validates one DOS 8.3 name.
    pub fn normalize(value: &str) -> Result<Self, ImageError> {
        normalize_name(value).map(Self)
    }

    /// Alias for [`DosName::normalize`].
    pub fn parse(value: &str) -> Result<Self, ImageError> {
        Self::normalize(value)
    }

    /// Returns the normalized name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DosName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for DosName {
    type Error = ImageError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::normalize(&value)
    }
}

impl From<DosName> for String {
    fn from(value: DosName) -> Self {
        value.0
    }
}

/// A path relative to `C:\`, with normalized uppercase 8.3 components.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DosPath(String);

impl DosPath {
    /// Parses an optional-drive DOS path. A drive root is the empty path.
    pub fn parse(value: &str) -> Result<Self, ImageError> {
        let value = strip_drive(value);
        let value = value
            .strip_prefix('\\')
            .or_else(|| value.strip_prefix('/'))
            .unwrap_or(value);
        Self::parse_relative(value)
    }

    /// Parses a path relative to `C:\`.
    pub fn parse_relative(value: &str) -> Result<Self, ImageError> {
        if value.starts_with('\\') || value.starts_with('/') {
            return Err(invalid(value, "a relative path has no leading separator"));
        }
        if value.is_empty() {
            return Ok(Self(String::new()));
        }

        let mut components = Vec::new();
        for component in value.split(['/', '\\']) {
            if component.is_empty() {
                return Err(invalid(value, "empty path component"));
            }
            if component == "." || component == ".." {
                return Err(invalid(value, "path traversal component"));
            }
            components.push(normalize_name(component)?);
        }
        Ok(Self(components.join("/")))
    }

    /// Returns the canonical `/`-separated path without a drive.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the normalized path components.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|component| !component.is_empty())
    }

    /// Returns the parent path, or `None` for the root.
    pub fn parent(&self) -> Option<Self> {
        if self.0.is_empty() {
            return None;
        }
        match self.0.rsplit_once('/') {
            Some((parent, _)) => Some(Self(parent.to_owned())),
            None => Some(Self(String::new())),
        }
    }

    /// Returns the final component, or `None` for the root.
    pub fn file_name(&self) -> Option<&str> {
        if self.0.is_empty() {
            return None;
        }
        Some(self.0.rsplit('/').next().unwrap_or(self.0.as_str()))
    }

    /// Appends one component.
    pub fn join(&self, name: &DosName) -> Self {
        if self.0.is_empty() {
            Self(name.as_str().to_owned())
        } else {
            Self(format!("{}/{}", self.0, name.as_str()))
        }
    }

    /// Returns true for the root path.
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Renders the DOS form, for example `C:\APP\FOO.TXT`.
    pub fn to_dos_string(&self) -> String {
        if self.0.is_empty() {
            String::from(r"C:\")
        } else {
            format!(r"C:\{}", self.0.replace('/', r"\"))
        }
    }

    /// Renders the tar member path, for example `files/APP/FOO.TXT`.
    pub fn to_tar_member(&self) -> String {
        format!("files/{}", self.0)
    }
}

impl fmt::Display for DosPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_dos_string())
    }
}

impl TryFrom<String> for DosPath {
    type Error = ImageError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<DosPath> for String {
    fn from(value: DosPath) -> Self {
        value.0
    }
}

/// Parses a tar member path under `files/`.
pub fn normalize_tree_member(value: &str) -> Result<DosPath, ImageError> {
    let Some(value) = value.strip_prefix("files/") else {
        return Err(invalid(value, "a tree member must start with files/"));
    };
    let value = value.strip_suffix('/').unwrap_or(value);
    if value.is_empty() {
        return Err(invalid(value, "a tree member path is empty"));
    }
    DosPath::parse_relative(value)
}

fn strip_drive(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[0].eq_ignore_ascii_case(&(DOS_DRIVE as u8)) && bytes[1] == b':' {
        &value[2..]
    } else {
        value
    }
}

fn normalize_name(value: &str) -> Result<String, ImageError> {
    if value.is_empty() {
        return Err(invalid(value, "a DOS name is empty"));
    }
    if value.starts_with(' ') || value.starts_with('.') {
        return Err(invalid(
            value,
            "a DOS name cannot start with a space or period",
        ));
    }

    let (stem, extension) = match value.split_once('.') {
        Some((stem, extension)) => {
            if extension.is_empty() || extension.contains('.') {
                return Err(invalid(value, "a DOS name has one optional extension"));
            }
            (stem, Some(extension))
        }
        None => (value, None),
    };

    if stem.is_empty() || stem.len() > DOS_NAME_MAX {
        return Err(invalid(
            value,
            "the DOS name stem must be 1 to 8 characters",
        ));
    }
    if let Some(extension) = extension
        && extension.len() > DOS_EXTENSION_MAX
    {
        return Err(invalid(
            value,
            "the DOS extension must be 1 to 3 characters",
        ));
    }

    let mut normalized = String::with_capacity(value.len());
    for byte in stem.bytes() {
        normalized
            .push(normalize_byte(byte).ok_or_else(|| invalid(value, "invalid DOS character"))?);
    }
    if let Some(extension) = extension {
        normalized.push('.');
        for byte in extension.bytes() {
            normalized
                .push(normalize_byte(byte).ok_or_else(|| invalid(value, "invalid DOS character"))?);
        }
    }
    Ok(normalized)
}

fn normalize_byte(byte: u8) -> Option<char> {
    if byte.is_ascii_alphanumeric() {
        return Some(byte.to_ascii_uppercase() as char);
    }
    if matches!(
        byte,
        b'$' | b'#'
            | b'@'
            | b'!'
            | b'%'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'-'
            | b'{'
            | b'}'
            | b'~'
            | b'^'
            | b'_'
            | b'`'
    ) {
        return Some(byte as char);
    }
    None
}

fn invalid(path: &str, reason: &str) -> ImageError {
    ImageError::InvalidDosPath {
        path: path.to_owned(),
        reason: reason.to_owned(),
    }
}
