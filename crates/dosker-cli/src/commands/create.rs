//! `dosker create`: make a container and its private writable disk.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use dosker_core::{ContainerId, ContainerName};
use dosker_engine::{Capability, CapabilitySet, DisplayMode};
use dosker_image::{
    DEFAULT_VOLUME_SIZE, FLOPPY_1440_SIZE, Image, ImageRef, PortProtocol, VolumeLayout,
    validate_host_tree,
};
use dosker_platform::endpoint::control_endpoint;
use dosker_runtime::{ContainerRecord, PortMapping, VolumeError, VolumeKind, VolumeMount};

use crate::adapter;
use crate::cli::{CreateArgs, EXIT_OK};
use crate::context::Context;
use crate::error::CliError;
use crate::spec::{BindSpec, PublishSpec, VolumeSpec};

const MOUNTABLE_DRIVES: &str = "A:, B:, D:, E:, and F:";

/// The inputs `create` and `run` share when making a container.
pub struct CreateOptions {
    pub image: String,
    pub name: Option<String>,
    pub engine: Option<String>,
    pub display: DisplayMode,
    pub vnc_port: Option<u16>,
    pub publish: Vec<String>,
    pub volumes: Vec<String>,
    pub binds: Vec<String>,
}

pub fn execute(
    args: CreateArgs,
    context: &mut Context,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let display = DisplayMode::parse(&args.display).ok_or_else(|| {
        CliError::Usage(format!(
            "invalid display mode: {}; use none, window, or vnc",
            args.display
        ))
    })?;

    let record = create_container(
        context,
        CreateOptions {
            image: args.image,
            name: args.name,
            engine: args.engine,
            display,
            vnc_port: args.vnc_port,
            publish: args.publish,
            volumes: args.volumes,
            binds: args.bind,
        },
        now,
    )?;
    context.console.line(record.id.as_str())?;

    Ok(EXIT_OK)
}

pub fn create_container(
    context: &mut Context,
    options: CreateOptions,
    now: DateTime<Utc>,
) -> Result<ContainerRecord, CliError> {
    let reference = ImageRef::parse(&options.image)?;
    let image = context.images.load(&reference)?;

    let engine = match options.engine {
        Some(engine) => engine,
        None => match image.engine_pin.clone() {
            Some(engine) => engine,
            None => match adapter::default_engine() {
                Some(engine) => engine.to_owned(),
                None => {
                    return Err(CliError::Usage(String::from(
                        "no engine selected; pass --engine",
                    )));
                }
            },
        },
    };
    let engine = match dosker_core::EngineName::parse(&engine) {
        Some(name) => name.as_str().to_owned(),
        None => engine,
    };

    if adapter::declared_capabilities(&engine).is_none() {
        if adapter::is_engine_name(&engine) {
            return Err(CliError::Usage(format!(
                "engine {engine} is not available; supported engines are {}",
                adapter::supported_engines_message()
            )));
        }
        return Err(CliError::Usage(format!("unknown engine: {engine}")));
    }

    check_requires(&engine, &image.requires)?;

    let specs = options
        .volumes
        .iter()
        .map(|value| VolumeSpec::parse(value).map_err(|error| CliError::Usage(error.to_string())))
        .collect::<Result<Vec<_>, _>>()?;
    let binds = options
        .binds
        .iter()
        .map(|value| BindSpec::parse(value).map_err(|error| CliError::Usage(error.to_string())))
        .collect::<Result<Vec<_>, _>>()?;

    let capabilities = adapter::declared_capabilities(&engine)
        .ok_or_else(|| CliError::Usage(format!("unknown engine: {engine}")))?;
    if image.net == "ne2000" && !capabilities.contains(Capability::Nat) {
        return Err(CliError::Usage(dosker_engine::capability_message(
            &engine,
            Capability::Nat,
        )));
    }
    if options.display == DisplayMode::Vnc && !capabilities.contains(Capability::Vnc) {
        return Err(CliError::Usage(dosker_engine::capability_message(
            &engine,
            Capability::Vnc,
        )));
    }
    if engine == "dosbox-x" && (!specs.is_empty() || !image.volumes.is_empty()) {
        return Err(CliError::Usage(String::from(
            "named volumes are not supported by the dosbox-x engine; use qemu",
        )));
    }

    let id = ContainerId::generate();
    let name = match options.name {
        Some(name) => ContainerName::parse(&name)?,
        None => ContainerName::parse(&format!("dosker-{}", &id.as_str()[..8]))?,
    };

    let root = std::path::absolute(context.paths.root())?;
    let endpoint = control_endpoint(&root, &id);

    let bind_mounts = validate_binds(&image, &engine, &specs, &binds)?;
    let (mut mounts, anonymous) = resolve_volumes(context, &image, &specs, &id, now)?;
    mounts.extend(bind_mounts);

    let ports = match resolve_ports(&image, &engine, &options.publish) {
        Ok(ports) => ports,
        Err(error) => {
            remove_anonymous(context, &anonymous);
            return Err(error);
        }
    };

    let record = ContainerRecord::new(
        id.clone(),
        name,
        reference.to_string(),
        engine,
        endpoint.record(),
        now,
    )
    .with_image(
        image.manifest_digest.clone(),
        image.is_booter(),
        image.cmd.clone(),
    )
    .with_disk_interface(&image.disk_interface)
    .with_display(options.display, options.vnc_port)
    .with_network(image.net.clone(), ports)
    .with_engine_config(image.engine_config.clone())
    .with_volumes(mounts);

    if let Err(error) = context.store.create(&record) {
        remove_anonymous(context, &anonymous);
        return Err(error.into());
    }

    let disk = context.paths.disk_file(&id);
    if let Err(error) = context.images.copy_disk_to(&reference, &disk) {
        remove_anonymous(context, &anonymous);
        let _ = context.store.remove(&record);
        return Err(error.into());
    }

    let record = record.with_disk(disk);
    if let Err(error) = context.store.save(&record) {
        remove_anonymous(context, &anonymous);
        let _ = context.store.remove(&record);
        return Err(error.into());
    }

    Ok(record)
}

fn resolve_volumes(
    context: &Context,
    image: &Image,
    specs: &[VolumeSpec],
    id: &ContainerId,
    now: DateTime<Utc>,
) -> Result<(Vec<VolumeMount>, Vec<String>), CliError> {
    let mut created = Vec::new();
    let result = (|| -> Result<Vec<VolumeMount>, CliError> {
        let mut mounts = Vec::new();
        let mut drives = BTreeSet::new();
        let mut names = BTreeSet::new();

        for spec in specs {
            if !drives.insert(spec.drive.clone()) {
                return Err(CliError::Usage(format!(
                    "drive {} is mounted more than once",
                    spec.drive
                )));
            }
            if !names.insert(spec.name.clone()) {
                return Err(CliError::Usage(format!(
                    "volume {} is mounted more than once",
                    spec.name
                )));
            }
            check_drive(&spec.drive)?;
            let volume = match context.volumes.load(&spec.name) {
                Ok(volume) => volume,
                Err(VolumeError::NotFound(_)) => {
                    return Err(CliError::Message(format!(
                        "volume not found: {}",
                        spec.name
                    )));
                }
                Err(error) => return Err(error.into()),
            };
            validate_drive(&spec.drive, volume.meta.layout, image)?;
            mounts.push(VolumeMount {
                drive: spec.drive.clone(),
                kind: VolumeKind::Named,
                name: Some(spec.name.clone()),
                host_path: None,
                read_only: spec.read_only,
            });
        }

        for declaration in &image.volumes {
            let drive = declaration.drive.clone();
            if drives.contains(&drive) {
                continue;
            }
            check_drive(&drive)?;
            drives.insert(drive.clone());
            let name = anonymous_name(id, &drive);
            let volume = match context.volumes.load(&name) {
                Ok(volume) => volume,
                Err(VolumeError::NotFound(_)) => {
                    let size = if matches!(drive.as_str(), "A:" | "B:") {
                        FLOPPY_1440_SIZE
                    } else {
                        DEFAULT_VOLUME_SIZE
                    };
                    let volume = context.volumes.create(&name, size, now)?;
                    created.push(name.clone());
                    volume
                }
                Err(error) => return Err(error.into()),
            };
            validate_drive(&drive, volume.meta.layout, image)?;
            mounts.push(VolumeMount {
                drive,
                kind: VolumeKind::Named,
                name: Some(name),
                host_path: None,
                read_only: false,
            });
        }

        Ok(mounts)
    })();

    match result {
        Ok(mounts) => Ok((mounts, created)),
        Err(error) => {
            remove_anonymous(context, &created);
            Err(error)
        }
    }
}

fn validate_binds(
    image: &Image,
    engine: &str,
    volumes: &[VolumeSpec],
    binds: &[BindSpec],
) -> Result<Vec<VolumeMount>, CliError> {
    if binds.is_empty() {
        return Ok(Vec::new());
    }

    let capabilities = adapter::declared_capabilities(engine)
        .ok_or_else(|| CliError::Usage(format!("unknown engine: {engine}")))?;
    if !capabilities.contains(Capability::HostDirLive) {
        return Err(CliError::Usage(dosker_engine::capability_message(
            engine,
            Capability::HostDirLive,
        )));
    }

    let mut drives: BTreeSet<String> = volumes.iter().map(|spec| spec.drive.clone()).collect();
    let mut mounts = Vec::new();
    for bind in binds {
        if !drives.insert(bind.drive.clone()) {
            return Err(CliError::Usage(format!(
                "drive {} is mounted more than once",
                bind.drive
            )));
        }
        if image
            .volumes
            .iter()
            .any(|declaration| declaration.drive == bind.drive)
        {
            return Err(CliError::Usage(format!(
                "drive {} is declared by the image and cannot hold a bind mount",
                bind.drive
            )));
        }
        let host = std::path::absolute(&bind.path).map_err(|error| {
            CliError::Usage(format!("bind source {}: {error}", bind.path.display()))
        })?;
        validate_host_tree(&host).map_err(|error| CliError::Usage(error.to_string()))?;
        mounts.push(VolumeMount {
            drive: bind.drive.clone(),
            kind: VolumeKind::Bind,
            name: None,
            host_path: Some(host),
            read_only: bind.read_only,
        });
    }

    Ok(mounts)
}

fn resolve_ports(
    image: &Image,
    engine: &str,
    values: &[String],
) -> Result<Vec<PortMapping>, CliError> {
    let mut ports = Vec::new();
    let mut seen = BTreeSet::new();

    for value in values {
        let spec = PublishSpec::parse(value).map_err(|error| CliError::Usage(error.to_string()))?;
        if !seen.insert((spec.host_address, spec.host_port)) {
            return Err(CliError::Usage(format!(
                "host port {}:{} is published more than once",
                spec.host_address, spec.host_port
            )));
        }
        ports.push(PortMapping {
            host_address: spec.host_address,
            host_port: spec.host_port,
            guest_port: spec.guest_port,
            protocol: PortProtocol::Tcp,
        });
    }

    let capabilities = adapter::declared_capabilities(engine)
        .ok_or_else(|| CliError::Usage(format!("unknown engine: {engine}")))?;
    validate_network(engine, capabilities, &image.net, &ports).map_err(CliError::Usage)?;

    Ok(ports)
}

pub fn validate_network(
    engine: &str,
    capabilities: CapabilitySet,
    net: &str,
    ports: &[PortMapping],
) -> Result<(), String> {
    if ports.is_empty() {
        return Ok(());
    }
    if net != "ne2000" {
        return Err(String::from(
            "the image disables the network device; published ports need MACHINE net=ne2000",
        ));
    }
    if !capabilities.contains(Capability::Nat) {
        return Err(dosker_engine::capability_message(engine, Capability::Nat));
    }
    Ok(())
}

pub fn check_requires(engine: &str, requires: &[String]) -> Result<(), CliError> {
    let capabilities = adapter::declared_capabilities(engine)
        .ok_or_else(|| CliError::Usage(format!("unknown engine: {engine}")))?;
    dosker_engine::check_requires(engine, capabilities, requires).map_err(CliError::Usage)
}

fn check_drive(drive: &str) -> Result<(), CliError> {
    match drive {
        "A:" | "B:" | "D:" | "E:" | "F:" => Ok(()),
        _ => Err(unsupported_drive(drive)),
    }
}

fn validate_drive(drive: &str, layout: VolumeLayout, image: &Image) -> Result<(), CliError> {
    match drive {
        "A:" | "B:" => {
            if image.disk_interface != "ide" {
                return Err(CliError::Usage(format!(
                    "drive {drive} needs an IDE boot image"
                )));
            }
            if layout != VolumeLayout::Superfloppy {
                return Err(CliError::Usage(format!(
                    "drive {drive} needs a superfloppy volume"
                )));
            }
        }
        "D:" | "E:" | "F:" => {
            if layout != VolumeLayout::MbrPartitioned {
                return Err(CliError::Usage(format!(
                    "drive {drive} needs an mbr_partitioned volume"
                )));
            }
        }
        _ => return Err(unsupported_drive(drive)),
    }
    Ok(())
}

fn unsupported_drive(drive: &str) -> CliError {
    CliError::Usage(format!(
        "unsupported volume drive {drive}; supported drives are {MOUNTABLE_DRIVES}"
    ))
}

fn anonymous_name(id: &ContainerId, drive: &str) -> String {
    let letter = drive.chars().next().unwrap_or('?').to_ascii_lowercase();
    format!("anon-{}-{letter}", id.as_str())
}

fn remove_anonymous(context: &Context, names: &[String]) {
    for name in names {
        let _ = context.volumes.remove(name);
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use dosker_engine::{Capability, CapabilitySet};
    use dosker_image::PortProtocol;
    use dosker_runtime::PortMapping;

    use super::*;

    fn mapping() -> PortMapping {
        PortMapping {
            host_address: Ipv4Addr::LOCALHOST,
            host_port: 8080,
            guest_port: 80,
            protocol: PortProtocol::Tcp,
        }
    }

    #[test]
    fn no_ports_do_not_require_the_nat_capability() {
        assert_eq!(
            validate_network("qemu", CapabilitySet::EMPTY, "ne2000", &[]),
            Ok(())
        );
    }

    #[test]
    fn a_published_port_requires_the_nat_capability() {
        let error = validate_network("mock", CapabilitySet::EMPTY, "ne2000", &[mapping()])
            .expect_err("a port needs the nat capability");
        assert!(error.contains("nat"), "{error}");
        assert!(error.contains("qemu"), "{error}");
    }

    #[test]
    fn publish_is_rejected_when_the_image_disables_the_network() {
        let error = validate_network("qemu", CapabilitySet::EMPTY, "none", &[mapping()])
            .expect_err("a port needs the network device");
        assert!(error.contains("net=ne2000"), "{error}");
    }

    #[test]
    fn publish_is_accepted_for_the_ne2000_network() {
        let capabilities: CapabilitySet = [Capability::Nat].into_iter().collect();
        assert_eq!(
            validate_network("qemu", capabilities, "ne2000", &[mapping()]),
            Ok(())
        );
    }
}
