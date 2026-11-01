use dosker_engine::{Capability, CapabilitySet};

#[test]
fn from_name_round_trips_every_variant() {
    for capability in Capability::ALL {
        let name = capability.name();
        assert_eq!(
            Capability::from_name(name),
            Some(capability),
            "name: {name}"
        );
        assert!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
            "name is not snake case: {name}"
        );
        assert_eq!(
            serde_json::to_value(capability).unwrap(),
            serde_json::Value::String(name.to_owned())
        );
    }
}

#[test]
fn names_match_the_documented_names() {
    let expected = [
        (Capability::Boot, "boot"),
        (Capability::Serial, "serial"),
        (Capability::Stop, "stop"),
        (Capability::Headless, "headless"),
        (Capability::HostDirLive, "host_dir_live"),
        (Capability::DebugNative, "debug_native"),
        (Capability::GdbStub, "gdbstub"),
        (Capability::Screenshot, "screenshot"),
        (Capability::Vnc, "vnc"),
        (Capability::Nat, "nat"),
        (Capability::Audio, "audio"),
    ];

    assert_eq!(Capability::ALL.len(), expected.len());
    assert_eq!(Capability::ALL, expected.map(|(capability, _)| capability));

    for (capability, name) in expected {
        assert_eq!(capability.name(), name);
        assert_eq!(Capability::from_name(name), Some(capability));
        assert_eq!(
            serde_json::to_value(capability).unwrap(),
            serde_json::Value::String(name.to_owned())
        );
    }
}

#[test]
fn unknown_name_is_none() {
    assert_eq!(Capability::from_name("nope"), None);
}

#[test]
fn contains_reports_only_inserted() {
    let mut set = CapabilitySet::EMPTY;
    set.insert(Capability::Boot);
    set.insert(Capability::Serial);

    assert!(set.contains(Capability::Boot));
    assert!(set.contains(Capability::Serial));
    assert!(!set.contains(Capability::Stop));
}

#[test]
fn empty_contains_nothing() {
    let set = CapabilitySet::EMPTY;
    for capability in Capability::ALL {
        assert!(!set.contains(capability));
    }
}

#[test]
fn iter_yields_inserted_in_declaration_order() {
    let set: CapabilitySet = [Capability::Stop, Capability::Boot, Capability::Serial]
        .into_iter()
        .collect();

    let observed: Vec<Capability> = set.iter().collect();
    assert_eq!(
        observed,
        vec![Capability::Boot, Capability::Serial, Capability::Stop]
    );

    let names: Vec<&str> = set.names().collect();
    assert_eq!(names, vec!["boot", "serial", "stop"]);
}

#[test]
fn set_serializes_as_u16() {
    let set: CapabilitySet = [Capability::Boot, Capability::Serial].into_iter().collect();
    let value = serde_json::to_value(set).unwrap();

    assert!(value.is_number());
    assert_eq!(value, serde_json::json!(3));
}
