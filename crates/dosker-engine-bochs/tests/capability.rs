use dosker_engine::Capability;
use dosker_engine_bochs::capabilities;

#[test]
fn declares_exactly_the_proven_rows() {
    let expected: [Capability; 5] = [
        Capability::Boot,
        Capability::Serial,
        Capability::Stop,
        Capability::Headless,
        Capability::DebugNative,
    ];

    assert_eq!(capabilities().iter().collect::<Vec<_>>(), expected);
}

#[test]
fn declares_boot_serial_stop_headless_and_debug_native() {
    let declared = capabilities();

    assert!(declared.contains(Capability::Boot));
    assert!(declared.contains(Capability::Serial));
    assert!(declared.contains(Capability::Stop));
    assert!(declared.contains(Capability::Headless));
    assert!(declared.contains(Capability::DebugNative));
}

#[test]
fn never_declares_the_six_capabilities_outside_the_proven_set() {
    let declared = capabilities();

    assert!(!declared.contains(Capability::GdbStub));
    assert!(!declared.contains(Capability::HostDirLive));
    assert!(!declared.contains(Capability::Screenshot));
    assert!(!declared.contains(Capability::Vnc));
    assert!(!declared.contains(Capability::Nat));
    assert!(!declared.contains(Capability::Audio));
}
