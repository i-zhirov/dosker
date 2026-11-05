//! QEMU host endpoints for one container.

#[cfg(windows)]
use std::net::Ipv4Addr;

use dosker_engine::{EngineCommand, EngineEndpoint, EngineEndpointKind, EngineError, EngineSpec};

#[cfg(unix)]
use dosker_platform::endpoint::engine_socket_path;

/// The four host endpoints QEMU exposes for one container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QemuEndpoints {
    pub agent_serial: Option<EngineEndpointKind>,
    pub qmp: EngineEndpointKind,
    pub monitor: EngineEndpointKind,
    pub gdb: EngineEndpointKind,
}

impl QemuEndpoints {
    /// Resolves every endpoint for a spec.
    ///
    /// On Unix the endpoints are socket paths in the container directory.
    /// On Windows they are loopback TCP ports allocated while all listeners
    /// remain open, so the four ports cannot repeat.
    pub fn for_spec(spec: &EngineSpec) -> Result<Self, EngineError> {
        #[cfg(unix)]
        {
            let agent_serial = if spec.booter {
                None
            } else {
                Some(EngineEndpointKind::UnixSocket(engine_socket_path(
                    &spec.root,
                    &spec.container_id,
                    "agent",
                )))
            };

            Ok(Self {
                agent_serial,
                qmp: EngineEndpointKind::UnixSocket(engine_socket_path(
                    &spec.root,
                    &spec.container_id,
                    "qmp",
                )),
                monitor: EngineEndpointKind::UnixSocket(engine_socket_path(
                    &spec.root,
                    &spec.container_id,
                    "monitor",
                )),
                gdb: EngineEndpointKind::UnixSocket(engine_socket_path(
                    &spec.root,
                    &spec.container_id,
                    "gdb",
                )),
            })
        }

        #[cfg(windows)]
        {
            let count = if spec.booter { 3 } else { 4 };
            let mut ports = reserve_tcp_ports(count)?.into_iter();
            let agent_serial = if spec.booter {
                None
            } else {
                Some(tcp_endpoint(next_port(&mut ports, count)?))
            };

            Ok(Self {
                agent_serial,
                qmp: tcp_endpoint(next_port(&mut ports, count)?),
                monitor: tcp_endpoint(next_port(&mut ports, count)?),
                gdb: tcp_endpoint(next_port(&mut ports, count)?),
            })
        }
    }

    /// Reads the endpoints that a command carries.
    pub fn from_command(command: &EngineCommand) -> Result<Self, EngineError> {
        Ok(Self {
            agent_serial: find_endpoint(command, "serial_agent"),
            qmp: required_endpoint(command, "qmp")?,
            monitor: required_endpoint(command, "monitor")?,
            gdb: required_endpoint(command, "gdb")?,
        })
    }

    /// Returns the named endpoints.
    pub fn engine_endpoints(&self) -> Vec<EngineEndpoint> {
        let mut endpoints = Vec::with_capacity(4);

        if let Some(kind) = &self.agent_serial {
            endpoints.push(EngineEndpoint {
                name: "serial_agent".to_owned(),
                kind: kind.clone(),
            });
        }

        endpoints.push(EngineEndpoint {
            name: "qmp".to_owned(),
            kind: self.qmp.clone(),
        });
        endpoints.push(EngineEndpoint {
            name: "monitor".to_owned(),
            kind: self.monitor.clone(),
        });
        endpoints.push(EngineEndpoint {
            name: "gdb".to_owned(),
            kind: self.gdb.clone(),
        });

        endpoints
    }

    /// Removes every socket file that is left over from an earlier run.
    ///
    /// A missing socket is not an error. Windows uses TCP and has no files
    /// to remove.
    pub fn remove_stale(&self) -> Result<(), EngineError> {
        #[cfg(unix)]
        {
            for kind in [
                self.agent_serial.as_ref(),
                Some(&self.qmp),
                Some(&self.monitor),
                Some(&self.gdb),
            ]
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
        }

        #[cfg(windows)]
        {
            let _ = self;
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

fn required_endpoint(
    command: &EngineCommand,
    name: &str,
) -> Result<EngineEndpointKind, EngineError> {
    find_endpoint(command, name)
        .ok_or_else(|| EngineError::Setup(format!("the engine command has no {name} endpoint")))
}

#[cfg(windows)]
fn tcp_endpoint(port: u16) -> EngineEndpointKind {
    EngineEndpointKind::Tcp {
        host: Ipv4Addr::LOCALHOST,
        port,
    }
}

#[cfg(windows)]
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
#[cfg(windows)]
fn reserve_tcp_ports(count: usize) -> Result<Vec<u16>, EngineError> {
    use std::net::TcpListener;

    let mut listeners = Vec::with_capacity(count);
    let mut ports = Vec::with_capacity(count);

    for _ in 0..count {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        ports.push(listener.local_addr()?.port());
        listeners.push(listener);
    }

    ports.sort_unstable();
    drop(listeners);

    Ok(ports)
}
