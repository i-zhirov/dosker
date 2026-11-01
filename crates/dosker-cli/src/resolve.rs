//! Turning a container reference into a stored record.

use dosker_core::{ContainerId, ContainerName};
use dosker_runtime::{ContainerRecord, StateStore, StoreError};

use crate::error::CliError;

/// A container reference: a full identifier or a name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    Id(ContainerId),
    Name(ContainerName),
}

/// Parses a container reference, preferring a full identifier.
pub fn parse_selector(value: &str) -> Result<Selector, CliError> {
    if let Ok(id) = ContainerId::parse(value) {
        return Ok(Selector::Id(id));
    }

    match ContainerName::parse(value) {
        Ok(name) => Ok(Selector::Name(name)),
        Err(_) => Err(CliError::Usage(format!(
            "invalid container reference: {value}"
        ))),
    }
}

/// Resolves a container reference against the state store.
pub fn resolve(store: &StateStore, value: &str) -> Result<ContainerRecord, CliError> {
    match parse_selector(value)? {
        Selector::Id(id) => match store.load(&id) {
            Ok(record) => Ok(record),
            Err(StoreError::NotFound(_)) => Err(not_found(value)),
            Err(error) => Err(error.into()),
        },
        Selector::Name(name) => match store.find_by_name(&name)? {
            Some(record) => Ok(record),
            None => Err(not_found(value)),
        },
    }
}

fn not_found(value: &str) -> CliError {
    CliError::Message(format!("container not found: {value}"))
}
