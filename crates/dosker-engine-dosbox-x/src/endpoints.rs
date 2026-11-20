//! DOSBox-X host endpoints for one container.

use std::net::{Ipv4Addr, TcpListener};

use dosker_engine::{
    Capability, EngineCommand, EngineEndpoint, EngineEndpointKind, EngineError, EngineSpec,
};

use crate::adapter::capabilities;

/// The host endpoints DOSBox-X exposes for one container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DosboxXEndpoints {
    /// The serial line the control agent listens on.
    pub agent_serial: Option<EngineEndpointKind>,
    /// The debugger control channel.
    pub debug: Option<EngineEndpointKind>,
}

impl DosboxXEndpoints {
    /// Resolves every endpoint for a spec.
    ///
    /// DOSBox-X listens on the agent serial line, so the host connects to it.
    /// The debugger channel reverses the direction: DOSBox-X connects back to
    /// a host listener. Both endpoints are loopback TCP ports.
    pub fn for_spec(spec: &EngineSpec) -> Result<Self, EngineError> {
        let wants_agent = !spec.booter;
        let wants_debug = capabilities().contains(Capability::DebugNative);
        let count = usize::from(wants_agent) + usize::from(wants_debug);
        let mut ports = reserve_tcp_ports(count)?.into_iter();

        let agent_serial = if wants_agent {
            Some(tcp_endpoint(next_port(&mut ports, count)?))
        } else {
            None
        };
        let debug = if wants_debug {
            Some(tcp_endpoint(next_port(&mut ports, count)?))
        } else {
            None
        };

        Ok(Self {
            agent_serial,
            debug,
        })
    }

    /// Reads the endpoints that a command carries.
    pub fn from_command(command: &EngineCommand) -> Result<Self, EngineError> {
        Ok(Self {
            agent_serial: find_endpoint(command, "serial_agent"),
            debug: find_endpoint(command, "debug"),
        })
    }

    /// Returns the named endpoints.
    pub fn engine_endpoints(&self) -> Vec<EngineEndpoint> {
        let mut endpoints = Vec::with_capacity(2);

        if let Some(kind) = &self.agent_serial {
            endpoints.push(EngineEndpoint {
                name: "serial_agent".to_owned(),
                kind: kind.clone(),
            });
        }

        if let Some(kind) = &self.debug {
            endpoints.push(EngineEndpoint {
                name: "debug".to_owned(),
                kind: kind.clone(),
            });
        }

        endpoints
    }

    /// Removes every socket file that is left over from an earlier run.
    ///
    /// A missing socket is not an error. TCP endpoints have no files to
    /// remove.
    pub fn remove_stale(&self) -> Result<(), EngineError> {
        for kind in [self.agent_serial.as_ref(), self.debug.as_ref()]
            .into_iter()
            .flatten()
        {
            let Some(path) = kind.unix_path() else {
                continue;
            };

            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(EngineError::Io(error)),
            }
        }

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

fn next_port(ports: &mut impl Iterator<Item = u16>, count: usize) -> Result<u16, EngineError> {
    ports.next().ok_or_else(|| {
        EngineError::Setup(format!(
            "the engine reserved fewer than {count} loopback ports"
        ))
    })
}

/// Reserves distinct loopback TCP ports.
///
/// Every listener stays open until all ports are collected, so no selected
/// port can repeat. The listeners drop before the function returns.
fn reserve_tcp_ports(count: usize) -> Result<Vec<u16>, EngineError> {
    let mut listeners = Vec::with_capacity(count);
    let mut ports = Vec::with_capacity(count);

    for _ in 0..count {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(EngineError::Io)?;
        ports.push(listener.local_addr().map_err(EngineError::Io)?.port());
        listeners.push(listener);
    }

    drop(listeners);

    Ok(ports)
}
