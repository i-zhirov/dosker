use dosker_engine::Capability;
use dosker_engine_dosbox_x::capabilities;

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
fn declares_boot_serial_and_headless() {
    let declared = capabilities();

    assert!(declared.contains(Capability::Boot));
    assert!(declared.contains(Capability::Serial));
    assert!(declared.contains(Capability::Headless));
}

#[test]
fn never_declares_gdbstub_screenshot_or_vnc() {
    let declared = capabilities();

    assert!(!declared.contains(Capability::GdbStub));
    assert!(!declared.contains(Capability::Screenshot));
    assert!(!declared.contains(Capability::Vnc));
}
