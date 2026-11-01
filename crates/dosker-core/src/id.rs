//! Container identifiers.

use std::fmt;
use std::fmt::Write;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

const ID_LEN: usize = 12;
const ID_BYTES: usize = 6;

/// A container identifier: twelve lowercase hexadecimal characters.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContainerId(String);

impl ContainerId {
    /// Generates a random identifier from six bytes.
    pub fn generate() -> Self {
        let mut bytes = [0u8; ID_BYTES];
        getrandom::fill(&mut bytes).expect("failed to generate container id bytes");

        let mut text = String::with_capacity(ID_LEN);
        for byte in bytes {
            write!(&mut text, "{byte:02x}").expect("writing to a string cannot fail");
        }
        Self(text)
    }

    /// Parses an identifier, rejecting anything that is not twelve lowercase
    /// hexadecimal characters.
    pub fn parse(value: &str) -> Result<Self, CoreError> {
        let valid = value.len() == ID_LEN
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));

        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(CoreError::InvalidContainerId(value.to_owned()))
        }
    }

    /// Borrows the identifier text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContainerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ContainerId {
    type Error = CoreError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ContainerId> for String {
    fn from(value: ContainerId) -> Self {
        value.0
    }
}
