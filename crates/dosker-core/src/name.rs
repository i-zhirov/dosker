//! Container names.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

const MAX_NAME_BYTES: usize = 63;

/// A container name: one to sixty-three bytes matching
/// `[a-z0-9][a-z0-9_.-]{0,62}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContainerName(String);

impl ContainerName {
    /// Parses a name, rejecting anything outside the allowed pattern.
    pub fn parse(value: &str) -> Result<Self, CoreError> {
        if is_valid(value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(CoreError::InvalidContainerName(value.to_owned()))
        }
    }

    /// Borrows the name text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_valid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_NAME_BYTES {
        return false;
    }

    let mut iter = bytes.iter().copied();
    let first = iter.next().expect("a non-empty name has a first byte");
    is_first(first) && iter.all(is_rest)
}

fn is_first(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit()
}

fn is_rest(byte: u8) -> bool {
    is_first(byte) || matches!(byte, b'.' | b'-' | b'_')
}

impl fmt::Display for ContainerName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ContainerName {
    type Error = CoreError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ContainerName> for String {
    fn from(value: ContainerName) -> Self {
        value.0
    }
}
