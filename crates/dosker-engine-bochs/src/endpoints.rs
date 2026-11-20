//! Bochs host endpoints for one container.

use std::net::{Ipv4Addr, TcpListener};

use dosker_engine::{EngineCommand, EngineEndpoint, EngineEndpointKind, EngineError, EngineSpec};

/// The host endpoints Bochs exposes for one container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BochsEndpoints {
    /// The serial line the control agent listens on.
    pub agent_serial: Option<EngineEndpointKind>,
}

impl BochsEndpoints {
    /// Resolves every endpoint for a spec.
    ///
    /// Bochs listens on the agent serial line, so the host connects to it. A
    /// booter has no agent and therefore no endpoint.
    pub fn for_spec(spec: &EngineSpec) -> Result<Self, EngineError> {
        let agent_serial = if spec.booter {
            None
        } else {
            Some(tcp_endpoint(reserve_tcp_port()?))
        };

        Ok(Self { agent_serial })
    }

    /// Reads the endpoints that a command carries.
    pub fn from_command(command: &EngineCommand) -> Result<Self, EngineError> {
        Ok(Self {
            agent_serial: find_endpoint(command, "serial_agent"),
        })
    }

    /// Returns the named endpoints.
    pub fn engine_endpoints(&self) -> Vec<EngineEndpoint> {
        let mut endpoints = Vec::with_capacity(1);

        if let Some(kind) = &self.agent_serial {
            endpoints.push(EngineEndpoint {
                name: "serial_agent".to_owned(),
                kind: kind.clone(),
            });
        }

        endpoints
    }

    /// Removes every socket file that is left over from an earlier run.
    ///
    /// TCP endpoints have no files to remove.
    pub fn remove_stale(&self) -> Result<(), EngineError> {
        Ok(())
    }
}

fn find_endpoint(command: &EngineCommand, name: &str) -> Option<EngineEndpointKind> {
    command
        .endpoints
        .iter()
        .find(|endpoint| endpoint.name == name)
        .map(|endpoint| endpoint.kind.clone())
}

fn tcp_endpoint(port: u16) -> EngineEndpointKind {
    EngineEndpointKind::Tcp {
        host: Ipv4Addr::LOCALHOST,
        port,
    }
}

/// Reserves one free loopback TCP port.
///
/// The listener drops before the function returns, so the port is free for
/// Bochs to bind.
fn reserve_tcp_port() -> Result<u16, EngineError> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(EngineError::Io)?;
    let port = listener.local_addr().map_err(EngineError::Io)?.port();
    drop(listener);

    Ok(port)
}
