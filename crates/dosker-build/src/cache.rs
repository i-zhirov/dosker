//! Build-cache lookup and record writing.

use chrono::{DateTime, Utc};
use dosker_image::cache::CACHE_SCHEMA_VERSION;
use dosker_image::{CacheRecord, CacheStore, Descriptor, ImageError, Layer, OciStore};

use crate::error::BuildError;

/// The result of looking up one cache key.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum CacheLookup {
    /// A filesystem step whose layer is still valid.
    HitLayer {
        layer: Layer,
        descriptor: Descriptor,
    },
    /// A config-only step whose head layer, if any, is still valid.
    HitConfig,
    /// No usable record exists.
    Miss,
}

/// Looks up and stores build-cache records against the OCI store.
pub struct BuildCache<'a> {
    store: &'a OciStore,
    cache: &'a CacheStore,
    no_cache: bool,
}

impl<'a> BuildCache<'a> {
    /// Creates a cache view over one OCI store and cache directory.
    pub fn new(store: &'a OciStore, cache: &'a CacheStore, no_cache: bool) -> Self {
        Self {
            store,
            cache,
            no_cache,
        }
    }

    /// Looks up one record and validates it against the stored blobs.
    pub fn lookup(
        &self,
        key: &str,
        normalized: &str,
        previous: Option<&str>,
        tree_step: bool,
    ) -> Result<CacheLookup, BuildError> {
        if self.no_cache {
            return Ok(CacheLookup::Miss);
        }

        let record = match self.cache.load(key) {
            Ok(Some(record)) => record,
            Ok(None) => return Ok(CacheLookup::Miss),
            Err(_) => {
                self.invalidate(key)?;
                return Ok(CacheLookup::Miss);
            }
        };

        if record.instruction != normalized {
            self.invalidate(key)?;
            return Ok(CacheLookup::Miss);
        }

        if tree_step {
            match self.load_layer(&record.layer_digest, previous) {
                Ok((layer, descriptor)) => Ok(CacheLookup::HitLayer { layer, descriptor }),
                Err(_) => {
                    self.invalidate(key)?;
                    Ok(CacheLookup::Miss)
                }
            }
        } else {
            if !record.layer_digest.is_empty()
                && self.store.read_blob(&record.layer_digest).is_err()
            {
                self.invalidate(key)?;
                return Ok(CacheLookup::Miss);
            }
            Ok(CacheLookup::HitConfig)
        }
    }

    /// Writes one version 1 record.
    pub fn store(
        &self,
        key: &str,
        layer_digest: &str,
        normalized: &str,
        now: DateTime<Utc>,
    ) -> Result<(), BuildError> {
        let record = CacheRecord {
            schema_version: CACHE_SCHEMA_VERSION,
            key: key.to_owned(),
            layer_digest: layer_digest.to_owned(),
            instruction: normalized.to_owned(),
            created_at: now,
        };
        self.cache
            .store(&record)
            .map_err(|error| BuildError::Cache {
                message: error.to_string(),
            })
    }

    fn invalidate(&self, key: &str) -> Result<(), BuildError> {
        self.cache.remove(key).map_err(|error| BuildError::Cache {
            message: error.to_string(),
        })
    }

    fn load_layer(
        &self,
        digest: &str,
        previous: Option<&str>,
    ) -> Result<(Layer, Descriptor), ImageError> {
        let layer = self.verify_layer(digest)?;
        let descriptor = self.store.layer_descriptor(digest)?;
        layer.validate_parent(previous)?;
        Ok((layer, descriptor))
    }

    fn verify_layer(&self, digest: &str) -> Result<Layer, ImageError> {
        let layer = self.store.load_layer(digest)?;
        let actual = layer.digest()?;
        if actual != digest {
            return Err(ImageError::DigestMismatch {
                expected: digest.to_owned(),
                actual,
            });
        }
        Ok(layer)
    }
}
