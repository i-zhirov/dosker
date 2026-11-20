use dosker_engine::{
    Capability, CapabilitySet, capability_message, check_requires, declared_capabilities,
    providers, supported_engines_message,
};

#[test]
fn declared_capabilities_match_the_qemu_set() {
    let expected = [
        Capability::Boot,
        Capability::Serial,
        Capability::Stop,
        Capability::Headless,
        Capability::HostDirLive,
        Capability::DebugNative,
        Capability::GdbStub,
        Capability::Screenshot,
        Capability::Vnc,
        Capability::Nat,
    ];
    let capabilities = declared_capabilities("qemu").expect("qemu is an implemented engine");

    assert_eq!(capabilities.iter().collect::<Vec<_>>(), expected.to_vec());
}

#[test]
fn declared_capabilities_match_the_dosbox_x_proven_rows() {
    let expected = [
        Capability::Boot,
        Capability::Serial,
        Capability::Stop,
        Capability::Headless,
        Capability::DebugNative,
    ];
    let capabilities =
        declared_capabilities("dosbox-x").expect("dosbox-x is an implemented engine");

    assert_eq!(capabilities.iter().collect::<Vec<_>>(), expected.to_vec());
}

#[test]
fn providers_are_in_table_order() {
    assert_eq!(providers(Capability::Headless), vec!["qemu", "dosbox-x"]);
}

#[test]
fn capability_message_uses_the_one_provider_form() {
    assert_eq!(
        capability_message("dosbox-x", Capability::GdbStub),
        "engine dosbox-x does not provide the gdbstub capability; the qemu engine provides it"
    );
}

#[test]
fn capability_message_uses_the_two_provider_form() {
    assert_eq!(
        capability_message("mock", Capability::Headless),
        "engine mock does not provide the headless capability; the qemu and dosbox-x engines provide it"
    );
}

#[test]
fn capability_message_uses_the_zero_provider_form() {
    assert_eq!(
        capability_message("dosbox-x", Capability::Audio),
        "engine dosbox-x does not provide the audio capability; no engine provides it"
    );
}

#[test]
fn check_requires_rejects_a_missing_capability() {
    let result = check_requires(
        "dosbox-x",
        declared_capabilities("dosbox-x").expect("dosbox-x is an implemented engine"),
        &["gdbstub".to_owned()],
    );

    assert_eq!(
        result,
        Err(
            "engine dosbox-x does not provide the gdbstub capability; the qemu engine provides it"
                .to_owned()
        )
    );
}

#[test]
fn check_requires_rejects_an_unknown_capability() {
    let result = check_requires(
        "qemu",
        declared_capabilities("qemu").expect("qemu is an implemented engine"),
        &["nope".to_owned()],
    );

    assert_eq!(result, Err("unknown capability: nope".to_owned()));
}

#[test]
fn check_requires_honors_the_supplied_capability_set() {
    let mock_set: CapabilitySet = [
        Capability::Boot,
        Capability::Serial,
        Capability::Stop,
        Capability::Nat,
    ]
    .into_iter()
    .collect();

    assert_eq!(
        check_requires("mock", mock_set, &["nat".to_owned()]),
        Ok(())
    );
    assert_eq!(
        check_requires("mock", mock_set, &["gdbstub".to_owned()]),
        Err(
            "engine mock does not provide the gdbstub capability; the qemu engine provides it"
                .to_owned()
        )
    );
}

#[test]
fn supported_engines_message_lists_the_implemented_engines() {
    assert_eq!(supported_engines_message(), "qemu and dosbox-x");
}
