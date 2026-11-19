//! Detection of a published host port that another container already holds.

use std::net::Ipv4Addr;

use dosker_core::ContainerId;

use crate::record::PortMapping;
use crate::store::{StateStore, StoreError};

/// A published host port that another container already holds.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PortConflict {
    #[error("port {host_address}:{host_port}/tcp is already published by container {holder}")]
    Taken {
        host_address: Ipv4Addr,
        host_port: u16,
        holder: String,
    },
}

/// Returns the first requested port that an active container already holds.
///
/// The container `id` is ignored, so a container never conflicts with itself.
/// Only starting, running, and stopping records hold a port.
pub fn find_conflict(
    store: &StateStore,
    id: &ContainerId,
    ports: &[PortMapping],
) -> Result<Option<PortConflict>, StoreError> {
    if ports.is_empty() {
        return Ok(None);
    }

    for record in store.list()? {
        if &record.id == id || !record.status.is_active() {
            continue;
        }
        for held in &record.ports {
            let taken = ports.iter().any(|requested| {
                requested.host_address == held.host_address
                    && requested.host_port == held.host_port
                    && requested.protocol == held.protocol
            });
            if taken {
                return Ok(Some(PortConflict::Taken {
                    host_address: held.host_address,
                    host_port: held.host_port,
                    holder: record.name.to_string(),
                }));
            }
        }
    }

    Ok(None)
}
