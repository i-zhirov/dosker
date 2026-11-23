use std::collections::BTreeMap;
use std::io::Cursor;

use chrono::{TimeZone, Utc};
use dosker_image::{
    Attributes, DosPath, LAYER_META_FILE, Layer, LayerDelta, TreeDir, TreeFile, fixed_timestamp,
};
use tar::{Archive, Builder, EntryType, Header};

fn time(second: i64, nanos: u32) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(second, nanos).unwrap()
}

fn file(path: &str, bytes: &[u8]) -> TreeFile {
    TreeFile {
        path: DosPath::parse(path).unwrap(),
        attributes: Attributes::default(),
        timestamp: fixed_timestamp(),
        bytes: bytes.to_vec(),
    }
}

fn dir(path: &str) -> TreeDir {
    TreeDir {
        path: DosPath::parse(path).unwrap(),
        attributes: Attributes::default(),
        timestamp: fixed_timestamp(),
    }
}

fn layer(delta: LayerDelta) -> Layer {
    Layer::from_delta(&delta, "test", None, None).unwrap()
}

fn first_entries(bytes: &[u8]) -> Vec<String> {
    let mut archive = Archive::new(Cursor::new(bytes));
    archive
        .entries()
        .unwrap()
        .map(|entry| {
            entry
                .unwrap()
                .path()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

#[test]
fn layer_tar_writes_the_sidecar_first() {
    let bytes = layer(LayerDelta {
        files: vec![file("A.TXT", b"a")],
        ..LayerDelta::new()
    })
    .write_tar()
    .unwrap();
    assert_eq!(first_entries(&bytes)[0], LAYER_META_FILE);
}

#[test]
fn layer_tar_writes_members_in_path_order() {
    let delta = LayerDelta {
        dirs: vec![dir("APP")],
        files: vec![file("APP/FOO.TXT", b"a")],
        ..LayerDelta::new()
    };
    let bytes = layer(delta).write_tar().unwrap();
    assert_eq!(
        first_entries(&bytes),
        vec![LAYER_META_FILE, "files/APP/", "files/APP/FOO.TXT"]
    );
}

#[test]
fn layer_tar_uses_the_stored_timestamps() {
    let timestamp = time(315_532_800, 0);
    let mut delta = LayerDelta::new();
    delta.add_file(TreeFile {
        timestamp,
        ..file("A.TXT", b"a")
    });
    let bytes = layer(delta).write_tar().unwrap();
    let mut archive = Archive::new(Cursor::new(bytes));
    let entry = archive.entries().unwrap().nth(1).unwrap().unwrap();
    assert_eq!(entry.header().mtime().unwrap(), 315_532_800);
}

#[test]
fn layer_tar_truncates_subsecond_timestamps() {
    let mut delta = LayerDelta::new();
    delta.add_file(TreeFile {
        timestamp: time(315_532_800, 999),
        ..file("A.TXT", b"a")
    });
    let bytes = layer(delta).write_tar().unwrap();
    let mut archive = Archive::new(Cursor::new(bytes));
    let entry = archive.entries().unwrap().nth(1).unwrap().unwrap();
    assert_eq!(entry.header().mtime().unwrap(), 315_532_800);
}

#[test]
fn layer_tar_uses_the_fixed_sidecar_timestamp() {
    let bytes = layer(LayerDelta {
        files: vec![file("A.TXT", b"a")],
        ..LayerDelta::new()
    })
    .write_tar()
    .unwrap();
    let mut archive = Archive::new(Cursor::new(bytes));
    let entry = archive.entries().unwrap().next().unwrap().unwrap();
    assert_eq!(entry.header().mtime().unwrap(), 315_532_800);
}

#[test]
fn layer_digest_is_stable_across_independent_writes() {
    let first = layer(LayerDelta {
        files: vec![file("A.TXT", b"a")],
        ..LayerDelta::new()
    });
    let second = layer(LayerDelta {
        files: vec![file("A.TXT", b"a")],
        ..LayerDelta::new()
    });
    assert_eq!(first.digest().unwrap(), second.digest().unwrap());
}

#[test]
fn layer_digest_changes_when_a_file_changes() {
    let first = layer(LayerDelta {
        files: vec![file("A.TXT", b"a")],
        ..LayerDelta::new()
    });
    let second = layer(LayerDelta {
        files: vec![file("A.TXT", b"b")],
        ..LayerDelta::new()
    });
    assert_ne!(first.digest().unwrap(), second.digest().unwrap());
}

#[test]
fn layer_round_trips_through_tar() {
    let mut delta = LayerDelta::new();
    delta.add_dir(dir("APP"));
    delta.add_file(file("APP/FOO.TXT", b"hello"));
    delta.whiteout(DosPath::parse("OLD.TXT").unwrap());
    let original = layer(delta);
    let decoded = Layer::read_tar(&original.write_tar().unwrap()).unwrap();
    assert_eq!(decoded.meta, original.meta);
    assert_eq!(decoded.to_delta(), original.to_delta());
}

#[test]
fn layer_parent_must_match_the_previous_digest() {
    let value = layer(LayerDelta {
        files: vec![file("A.TXT", b"a")],
        ..LayerDelta::new()
    });
    assert!(value.validate_parent(None).is_ok());
    assert!(value.validate_parent(Some("sha256:bad")).is_err());
    let with_parent = Layer::from_delta(
        &LayerDelta {
            files: vec![file("B.TXT", b"b")],
            ..LayerDelta::new()
        },
        "test",
        Some(value.digest().unwrap()),
        None,
    )
    .unwrap();
    assert!(
        with_parent
            .validate_parent(Some(&value.digest().unwrap()))
            .is_ok()
    );
}

#[test]
fn layer_reader_rejects_a_symlink_member() {
    let mut builder = Builder::new(Vec::new());
    let mut header = Header::new_gnu();
    header.set_size(0);
    header.set_mode(0o644);
    header.set_entry_type(EntryType::Symlink);
    builder
        .append_data(&mut header, "files/A", &[][..])
        .unwrap();
    assert!(Layer::read_tar(&builder.into_inner().unwrap()).is_err());
}

#[test]
fn layer_reader_rejects_a_traversal_member() {
    let mut builder = Builder::new(Vec::new());
    let mut header = Header::new_gnu();
    header.set_size(0);
    header.set_mode(0o644);
    header.set_entry_type(EntryType::Regular);
    header.set_path("files/APP").unwrap();
    let name = b"files/../APP";
    header.as_mut_bytes()[..name.len()].copy_from_slice(name);
    header.set_cksum();
    builder.append(&header, std::io::empty()).unwrap();
    assert!(Layer::read_tar(&builder.into_inner().unwrap()).is_err());
}

#[test]
fn layer_reader_rejects_a_missing_sidecar() {
    let mut builder = Builder::new(Vec::new());
    let mut header = Header::new_gnu();
    header.set_size(0);
    header.set_mode(0o644);
    header.set_entry_type(EntryType::Regular);
    builder
        .append_data(&mut header, "files/A", &[][..])
        .unwrap();
    assert!(matches!(
        Layer::read_tar(&builder.into_inner().unwrap()),
        Err(dosker_image::ImageError::MissingLayerSidecar)
    ));
}

#[test]
fn layer_reader_rejects_a_wrong_schema_version() {
    let mut value = layer(LayerDelta {
        files: vec![file("A.TXT", b"a")],
        ..LayerDelta::new()
    });
    value.meta.schema_version = 2;
    assert!(Layer::read_tar(&value.write_tar().unwrap()).is_err());
}

#[test]
fn layer_reader_rejects_a_size_mismatch() {
    let mut value = layer(LayerDelta {
        files: vec![file("A.TXT", b"a")],
        ..LayerDelta::new()
    });
    value.meta.entries[0].size = 99;
    assert!(Layer::read_tar(&value.write_tar().unwrap()).is_err());
}

#[test]
fn layer_writer_uses_a_gnu_long_name_for_a_long_path() {
    let long = (0..12).map(|_| "AAAAAAAA").collect::<Vec<_>>().join("/");
    let bytes = layer(LayerDelta {
        files: vec![file(&long, b"a")],
        ..LayerDelta::new()
    })
    .write_tar()
    .unwrap();
    assert!(
        bytes
            .chunks(512)
            .any(|block| block.len() == 512 && block[156] == b'L'),
        "missing GNU long-name header"
    );
    let decoded = Layer::read_tar(&bytes).unwrap();
    assert_eq!(decoded.meta.entries[0].path.as_str(), long);
}

#[test]
fn layer_reader_accepts_a_gnu_long_path() {
    let long = (0..12).map(|_| "BBBBBBBB").collect::<Vec<_>>().join("/");
    let value = layer(LayerDelta {
        files: vec![file(&long, b"a")],
        ..LayerDelta::new()
    });
    let decoded = Layer::read_tar(&value.write_tar().unwrap()).unwrap();
    assert_eq!(decoded.meta.entries[0].path.as_str(), long);
}

#[test]
fn layer_reader_accepts_a_pax_long_path() {
    let long = (0..12).map(|_| "CCCCCCCC").collect::<Vec<_>>().join("/");
    let value = layer(LayerDelta {
        files: vec![file(&long, b"a")],
        ..LayerDelta::new()
    });
    let sidecar = value.meta_json().unwrap();
    let mut builder = Builder::new(Vec::new());

    let mut sidecar_header = Header::new_gnu();
    sidecar_header.set_size(sidecar.len() as u64);
    sidecar_header.set_mode(0o644);
    sidecar_header.set_entry_type(EntryType::Regular);
    builder
        .append_data(&mut sidecar_header, LAYER_META_FILE, &sidecar[..])
        .unwrap();

    let pax = pax_record("path", &format!("files/{long}"));
    let mut pax_header = Header::new_gnu();
    pax_header.set_size(pax.len() as u64);
    pax_header.set_mode(0o644);
    pax_header.set_entry_type(EntryType::new(b'x'));
    builder
        .append_data(&mut pax_header, "PaxHeader", &pax[..])
        .unwrap();

    let mut member_header = Header::new_gnu();
    member_header.set_size(1);
    member_header.set_mode(0o644);
    member_header.set_entry_type(EntryType::Regular);
    builder
        .append_data(&mut member_header, "files/CCCCCCCC", &b"a"[..])
        .unwrap();

    let decoded = Layer::read_tar(&builder.into_inner().unwrap()).unwrap();
    assert_eq!(decoded.meta.entries[0].path.as_str(), long);
}

#[test]
fn layer_long_path_round_trips() {
    let long = (0..12).map(|_| "DDDDDDDD").collect::<Vec<_>>().join("/");
    let original = layer(LayerDelta {
        files: vec![file(&long, b"hello")],
        ..LayerDelta::new()
    });
    let decoded = Layer::read_tar(&original.write_tar().unwrap()).unwrap();
    assert_eq!(decoded.to_delta(), original.to_delta());
}

#[test]
fn layer_long_path_digest_is_stable() {
    let long = (0..12).map(|_| "EEEEEEEE").collect::<Vec<_>>().join("/");
    let first = layer(LayerDelta {
        files: vec![file(&long, b"a")],
        ..LayerDelta::new()
    });
    let second = layer(LayerDelta {
        files: vec![file(&long, b"a")],
        ..LayerDelta::new()
    });
    assert_eq!(first.digest().unwrap(), second.digest().unwrap());
}

#[test]
fn layer_apply_to_applies_whiteouts_before_entries() {
    let mut tree = dosker_image::Tree::new();
    tree.apply_delta(&LayerDelta {
        files: vec![file("A.TXT", b"old")],
        ..LayerDelta::new()
    })
    .unwrap();
    let mut delta = LayerDelta::new();
    delta.whiteout(DosPath::parse("A.TXT").unwrap());
    delta.add_file(file("A.TXT", b"new"));
    layer(delta).apply_to(&mut tree).unwrap();
    assert_eq!(tree.files().next().unwrap().bytes, b"new");
}

#[test]
fn effective_boot_uses_the_last_non_null_layer_override() {
    use dosker_image::{BootMetadata, DosFlavour, ImageConfig, MachineProfile};
    let config = ImageConfig {
        schema_version: 1,
        dos_flavour: DosFlavour::Other,
        booter: true,
        boot: BootMetadata::opaque(1),
        machine: MachineProfile::default(),
        env: Default::default(),
        workdir: String::new(),
        cmd: None,
        exposed_ports: Vec::new(),
        volumes: Vec::new(),
        engine_pin: None,
        engine_config: BTreeMap::new(),
        requires: Vec::new(),
        agent_version: None,
        created_by: "test".to_owned(),
        history: Vec::new(),
        licenses: Vec::new(),
    };
    let mut first = layer(LayerDelta::new());
    first.meta.boot = Some(BootMetadata::opaque(2));
    let mut second = layer(LayerDelta::new());
    second.meta.boot = Some(BootMetadata::opaque(3));
    assert_eq!(
        dosker_image::effective_boot(&config, &[first, second]).disk_size,
        3
    );
}

fn pax_record(key: &str, value: &str) -> Vec<u8> {
    let body = format!(" {key}={value}\n");
    let mut length = body.len() + 1;
    loop {
        let record = format!("{length}{body}");
        if record.len() == length {
            return record.into_bytes();
        }
        length = record.len();
    }
}
