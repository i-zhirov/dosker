//! QEMU command line construction.

use std::net::TcpListener;

use dosker_engine::{
    DisplayMode, EngineCommand, EngineEndpointKind, EngineError, EnginePort, EngineSpec,
    EngineVolume,
};

use crate::discovery::QemuBinary;
use crate::endpoints::QemuEndpoints;

const VNC_FIRST_PORT: u16 = 5900;

/// Builds the QEMU command line for one spec.
pub fn build(spec: &EngineSpec, binary: &QemuBinary) -> Result<EngineCommand, EngineError> {
    if spec.control_port == 0 || spec.control_port > spec.serial_ports {
        return Err(EngineError::Setup(format!(
            "the control port {} is outside the {} configured serial ports",
            spec.control_port, spec.serial_ports
        )));
    }

    let endpoints = QemuEndpoints::for_spec(spec)?;
    let engine_ports: Vec<u16> = endpoints
        .engine_endpoints()
        .iter()
        .filter_map(|endpoint| endpoint.kind.tcp_addr().map(|address| address.port()))
        .collect();
    let mut args = vec![
        "-machine".to_owned(),
        "pc".to_owned(),
        "-cpu".to_owned(),
        "486".to_owned(),
        "-m".to_owned(),
        "16".to_owned(),
    ];
    if !spec.allow_reboot {
        args.push("-no-reboot".to_owned());
    }
    args.extend(display_arguments(
        spec.display,
        spec.vnc_port,
        &engine_ports,
    )?);
    args.push("-drive".to_owned());
    let interface = if spec.disk_interface == "floppy" {
        "floppy"
    } else {
        "ide"
    };
    args.push(format!(
        "file={},format=raw,if={interface},index=0",
        spec.disk_path.display()
    ));
    for drive in volume_drives(&spec.volumes)? {
        args.push("-drive".to_owned());
        args.push(drive);
    }

    for port in 1..=spec.serial_ports {
        args.push("-serial".to_owned());
        if port == spec.control_port {
            match &endpoints.agent_serial {
                Some(kind) => args.push(socket_option(kind)),
                None => args.push("none".to_owned()),
            }
        } else {
            args.push("none".to_owned());
        }
    }

    args.push("-qmp".to_owned());
    args.push(socket_option(&endpoints.qmp));
    args.push("-monitor".to_owned());
    args.push(socket_option(&endpoints.monitor));
    args.push("-chardev".to_owned());
    args.push(gdb_chardev(&endpoints.gdb));
    args.push("-gdb".to_owned());
    args.push("chardev:gdb0".to_owned());
    args.push("-nic".to_owned());
    args.push(nic_option(spec)?);

    Ok(EngineCommand {
        program: binary.path.clone(),
        args,
        env: Vec::new(),
        endpoints: endpoints.engine_endpoints(),
    })
}

fn nic_option(spec: &EngineSpec) -> Result<String, EngineError> {
    let mut option = match spec.net.as_str() {
        "none" => return Ok("none".to_owned()),
        "ne2000" => String::from("user,model=ne2k_isa"),
        other => {
            return Err(EngineError::Setup(format!(
                "unknown network device {other}"
            )));
        }
    };

    let mut ports: Vec<EnginePort> = spec.ports.clone();
    ports.sort_by_key(|port| (port.host_port, port.guest_port));
    for port in ports {
        option.push_str(&format!(
            ",hostfwd=tcp:{}:{}-:{}",
            port.host_address, port.host_port, port.guest_port
        ));
    }

    Ok(option)
}

fn volume_drives(volumes: &[EngineVolume]) -> Result<Vec<String>, EngineError> {
    let mut sorted: Vec<&EngineVolume> = volumes.iter().collect();
    sorted.sort_by(|left, right| left.drive.cmp(&right.drive));

    let mut drives = Vec::with_capacity(sorted.len());
    for volume in sorted {
        let placement = match volume.drive.as_str() {
            "A:" => "if=floppy,index=0",
            "B:" => "if=floppy,index=1",
            "D:" => "if=ide,index=1",
            "E:" => "if=ide,index=2",
            "F:" => "if=ide,index=3",
            other => {
                return Err(EngineError::Setup(format!(
                    "unsupported volume drive {other}"
                )));
            }
        };

        let mut drive = format!("file={},format=raw,{placement}", volume.path.display());
        if volume.read_only {
            // QEMU rejects a read-only block node on an IDE hard disk, so a
            // read-only hard-disk volume gets a temporary overlay whose writes
            // are discarded. A floppy can be attached read-only directly.
            if matches!(volume.drive.as_str(), "A:" | "B:") {
                drive.push_str(",readonly=on");
            } else {
                drive.push_str(",snapshot=on");
            }
        }
        drives.push(drive);
    }

    Ok(drives)
}

fn socket_option(kind: &EngineEndpointKind) -> String {
    match kind {
        EngineEndpointKind::UnixSocket(path) => {
            format!("unix:{},server=on,wait=off", path.display())
        }
        EngineEndpointKind::Tcp { host, port } => {
            format!("tcp:{host}:{port},server=on,wait=off")
        }
    }
}

fn gdb_chardev(kind: &EngineEndpointKind) -> String {
    match kind {
        EngineEndpointKind::UnixSocket(path) => {
            format!("socket,id=gdb0,path={},server=on,wait=off", path.display())
        }
        EngineEndpointKind::Tcp { host, port } => {
            format!("socket,id=gdb0,host={host},port={port},server=on,wait=off")
        }
    }
}

/// Builds the display options for a display mode.
///
/// A VNC port is translated to a QEMU display number. Port `5905` becomes
/// display `5`.
pub fn display_arguments(
    display: DisplayMode,
    vnc_port: Option<u16>,
    excluded_ports: &[u16],
) -> Result<Vec<String>, EngineError> {
    match display {
        DisplayMode::None => Ok(vec!["-display".to_owned(), "none".to_owned()]),
        DisplayMode::Window => {
            #[cfg(windows)]
            {
                Err(EngineError::Unsupported("window display on Windows"))
            }
            #[cfg(not(windows))]
            {
                Ok(vec!["-display".to_owned(), window_backend().to_owned()])
            }
        }
        DisplayMode::Vnc => {
            let port = match vnc_port {
                Some(port) => port,
                None => free_vnc_port_excluding(excluded_ports)?,
            };
            if port < VNC_FIRST_PORT {
                return Err(EngineError::Setup(format!(
                    "VNC port {port} is below {VNC_FIRST_PORT}"
                )));
            }
            let display_number = port - VNC_FIRST_PORT;

            Ok(vec![
                "-display".to_owned(),
                "none".to_owned(),
                "-vnc".to_owned(),
                format!("127.0.0.1:{display_number}"),
            ])
        }
    }
}

#[cfg(not(windows))]
#[cfg(target_os = "macos")]
fn window_backend() -> &'static str {
    "cocoa"
}

#[cfg(not(windows))]
#[cfg(not(target_os = "macos"))]
fn window_backend() -> &'static str {
    "gtk"
}

/// Allocates a free local VNC port at or above `5900`.
pub fn free_vnc_port() -> Result<u16, EngineError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);

    if port >= VNC_FIRST_PORT {
        return Ok(port);
    }

    for candidate in VNC_FIRST_PORT..=u16::MAX {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", candidate)) {
            drop(listener);
            return Ok(candidate);
        }
    }

    Err(EngineError::Setup(
        "could not find a free VNC port".to_owned(),
    ))
}

/// Allocates a free local VNC port that is not in `excluded`.
pub fn free_vnc_port_excluding(excluded: &[u16]) -> Result<u16, EngineError> {
    let port = free_vnc_port()?;
    if !excluded.contains(&port) {
        return Ok(port);
    }

    for candidate in VNC_FIRST_PORT..=u16::MAX {
        if excluded.contains(&candidate) {
            continue;
        }

        if let Ok(listener) = TcpListener::bind(("127.0.0.1", candidate)) {
            drop(listener);
            return Ok(candidate);
        }
    }

    Err(EngineError::Setup(
        "could not find a free VNC port".to_owned(),
    ))
}

/// Allocates a free local TCP port.
pub fn free_tcp_port() -> Result<u16, EngineError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);

    Ok(port)
}
