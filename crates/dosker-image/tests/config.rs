use chrono::{TimeZone, Utc};
use dosker_image::{
    BootMetadata, DosFlavour, ImageConfig, MachineProfile, PortDeclaration, PortProtocol,
    VolumeDeclaration,
};
use std::collections::BTreeMap;

fn config(boot: BootMetadata) -> ImageConfig {
    let booter = boot.layout == dosker_image::BootLayout::Opaque;
    ImageConfig {
        schema_version: 1,
        dos_flavour: DosFlavour::Other,
        booter,
        boot,
        machine: MachineProfile::default(),
        env: BTreeMap::new(),
        workdir: if booter {
            String::new()
        } else {
            r"C:\".to_owned()
        },
        cmd: None,
        exposed_ports: Vec::new(),
        volumes: Vec::new(),
        engine_pin: None,
        requires: Vec::new(),
        agent_version: None,
        created_by: "import".to_owned(),
        history: vec![dosker_image::HistoryEntry {
            created_by: "import".to_owned(),
            created_at: Utc.timestamp_opt(315_532_800, 0).unwrap(),
            empty_layer: false,
            comment: None,
        }],
    }
}

fn disk() -> BootMetadata {
    BootMetadata {
        layout: dosker_image::BootLayout::MbrPartitioned,
        disk_size: 1024,
        geometry: dosker_image::Geometry {
            cylinders: 1,
            heads: 1,
            sectors: 1,
            sector_size: 512,
        },
        partition_start: Some(1),
        partition_sectors: Some(1),
        mbr: Some(dosker_image::BlobRef::from_bytes(b"mbr")),
        boot_sector: Some(dosker_image::BlobRef::from_bytes(b"boot")),
        fat_type: Some(dosker_image::FatType::Fat16),
        volume_label: Some("DOSKER".to_owned()),
        media_descriptor: Some(0xF8),
    }
}

#[test]
fn default_machine_profile_matches_the_pinned_values() {
    let machine = MachineProfile::default();
    assert_eq!(machine.cpu, "486dx2");
    assert_eq!(machine.memory_mb, 16);
    assert_eq!(machine.video, "vga");
    assert_eq!(machine.sound, "sb16");
    assert_eq!(machine.net, "ne2000");
    assert_eq!(machine.serial_ports, 2);
    assert_eq!(machine.control_port, 2);
    assert_eq!(machine.disk_interface, "ide");
    assert_eq!(machine.baud, 115200);
}

#[test]
fn machine_profile_rejects_an_out_of_range_value() {
    assert!(
        MachineProfile {
            memory_mb: 0,
            ..MachineProfile::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        MachineProfile {
            serial_ports: 0,
            ..MachineProfile::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        MachineProfile {
            disk_interface: "scsi".to_owned(),
            ..MachineProfile::default()
        }
        .validate()
        .is_err()
    );
}

#[test]
fn image_config_round_trips_through_json() {
    let value = config(disk());
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(serde_json::from_str::<ImageConfig>(&json).unwrap(), value);
}

#[test]
fn config_digest_is_stable_across_independent_construction() {
    assert_eq!(
        config(disk()).digest().unwrap(),
        config(disk()).digest().unwrap()
    );
}

#[test]
fn config_digest_changes_when_env_changes() {
    let first = config(disk());
    let mut second = first.clone();
    second.env.insert("A".to_owned(), "B".to_owned());
    assert_ne!(first.digest().unwrap(), second.digest().unwrap());
}

#[test]
fn config_digest_changes_when_workdir_changes() {
    let first = config(disk());
    let mut second = first.clone();
    second.workdir = r"C:\APP".to_owned();
    assert_ne!(first.digest().unwrap(), second.digest().unwrap());
}

#[test]
fn config_validate_rejects_a_booter_with_a_tree_field() {
    let mut value = config(BootMetadata::opaque(1024));
    value.env.insert("A".to_owned(), "B".to_owned());
    assert!(value.validate().is_err());
}

#[test]
fn config_validate_rejects_a_booter_with_a_non_opaque_layout() {
    let mut value = config(BootMetadata::opaque(1024));
    value.boot = disk();
    assert!(value.validate().is_err());
}

#[test]
fn config_validate_rejects_a_non_booter_with_an_opaque_layout() {
    let mut value = config(disk());
    value.boot = BootMetadata::opaque(1024);
    assert!(value.validate().is_err());
}

#[test]
fn config_validate_rejects_a_disk_interface_that_does_not_match_the_layout() {
    let mut value = config(disk());
    value.machine.disk_interface = "floppy".to_owned();
    assert!(value.validate().is_err());
}

#[test]
fn port_declaration_rejects_port_zero() {
    let mut value = config(disk());
    value.exposed_ports.push(PortDeclaration {
        port: 0,
        protocol: PortProtocol::Tcp,
    });
    assert!(value.validate().is_err());
    value.exposed_ports[0].port = 65535;
    assert!(value.validate().is_ok());
}

#[test]
fn volume_declaration_requires_a_drive_shape() {
    let mut value = config(disk());
    value.volumes.push(VolumeDeclaration {
        drive: "D:".to_owned(),
    });
    assert!(value.validate().is_ok());
    value.volumes[0].drive = "D".to_owned();
    assert!(value.validate().is_err());
}
