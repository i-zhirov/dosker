//! Canonical JSON and content digests.

use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::error::ImageError;

/// Serializes a value as compact JSON with recursively sorted object keys.
pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ImageError> {
    let value = serde_json::to_value(value)
        .map_err(|error| ImageError::CanonicalJson(error.to_string()))?;
    serde_json::to_vec(&sort_value(value))
        .map_err(|error| ImageError::CanonicalJson(error.to_string()))
}

/// Returns the `sha256:<hex>` digest of bytes.
pub fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(bytes)))
}

/// Returns the digest of a value's canonical JSON.
pub fn canonical_digest<T: Serialize>(value: &T) -> Result<String, ImageError> {
    canonical_json(value).map(|bytes| sha256_digest(&bytes))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        write!(&mut text, "{byte:02x}").expect("writing to a String cannot fail");
    }
    text
}

fn sort_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut sorted = Map::new();
            for (key, value) in entries {
                sorted.insert(key, sort_value(value));
            }
            Value::Object(sorted)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(sort_value).collect()),
        other => other,
    }
}
