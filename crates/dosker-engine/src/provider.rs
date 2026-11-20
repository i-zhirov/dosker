//! The table of implemented engines and the capability checks built on it.

use dosker_core::EngineName;

use crate::{Capability, CapabilitySet};

/// An implemented engine and the capabilities it declares.
pub struct EngineProvider {
    /// The canonical engine name.
    pub name: &'static str,
    /// The declared capabilities, in declaration order.
    pub capabilities: &'static [Capability],
}

/// Every engine that has an adapter, in table order.
pub const IMPLEMENTED_ENGINES: &[EngineProvider] = &[
    EngineProvider {
        name: "qemu",
        capabilities: &[
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
        ],
    },
    EngineProvider {
        name: "dosbox-x",
        capabilities: &[
            Capability::Boot,
            Capability::Serial,
            Capability::Stop,
            Capability::Headless,
            Capability::DebugNative,
        ],
    },
    EngineProvider {
        name: "bochs",
        capabilities: &[
            Capability::Boot,
            Capability::Serial,
            Capability::Stop,
            Capability::Headless,
            Capability::DebugNative,
        ],
    },
];

/// Returns the capabilities the named implemented engine declares.
pub fn declared_capabilities(engine: &str) -> Option<CapabilitySet> {
    IMPLEMENTED_ENGINES
        .iter()
        .find(|provider| provider.name == engine)
        .map(|provider| provider.capabilities.iter().copied().collect())
}

/// Reports whether an engine name is known.
pub fn is_known(engine: &str) -> bool {
    EngineName::parse(engine).is_some()
}

/// Returns the names of the implemented engines, in table order.
pub fn implemented_engine_names() -> Vec<&'static str> {
    IMPLEMENTED_ENGINES
        .iter()
        .map(|provider| provider.name)
        .collect()
}

/// Returns the supported-engine list.
pub fn supported_engines_message() -> String {
    join_engine_names(&implemented_engine_names())
}

/// Returns the engines that provide a capability, in table order.
pub fn providers(capability: Capability) -> Vec<&'static str> {
    IMPLEMENTED_ENGINES
        .iter()
        .filter(|provider| provider.capabilities.contains(&capability))
        .map(|provider| provider.name)
        .collect()
}

/// Returns the message for a capability the named engine does not provide.
pub fn capability_message(engine: &str, capability: Capability) -> String {
    format_capability_message(engine, capability, &providers(capability))
}

/// Checks that a declared capability set satisfies every requirement.
pub fn check_requires(
    engine: &str,
    capabilities: CapabilitySet,
    requires: &[String],
) -> Result<(), String> {
    for requirement in requires {
        let Some(capability) = Capability::from_name(requirement) else {
            return Err(format!("unknown capability: {requirement}"));
        };
        if !capabilities.contains(capability) {
            return Err(capability_message(engine, capability));
        }
    }
    Ok(())
}

fn format_capability_message(engine: &str, capability: Capability, providers: &[&str]) -> String {
    let name = capability.name();
    match providers {
        [] => {
            format!("engine {engine} does not provide the {name} capability; no engine provides it")
        }
        [provider] => format!(
            "engine {engine} does not provide the {name} capability; the {provider} engine provides it"
        ),
        [first, second] => format!(
            "engine {engine} does not provide the {name} capability; the {first} and {second} engines provide it"
        ),
        _ => {
            let (last, rest) = providers.split_last().expect("providers is not empty");
            format!(
                "engine {engine} does not provide the {name} capability; the {}, and {last} engines provide it",
                rest.join(", ")
            )
        }
    }
}

fn join_engine_names(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [only] => (*only).to_owned(),
        [first, second] => format!("{first} and {second}"),
        _ => {
            let (last, rest) = names.split_last().expect("names is not empty");
            format!("{}, and {last}", rest.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_message_uses_the_two_provider_form() {
        assert_eq!(
            format_capability_message("mock", Capability::Headless, &["qemu", "dosbox-x"]),
            "engine mock does not provide the headless capability; the qemu and dosbox-x engines provide it"
        );
    }

    #[test]
    fn capability_message_uses_the_three_provider_form() {
        assert_eq!(
            format_capability_message("mock", Capability::GdbStub, &["qemu", "dosbox-x", "bochs"]),
            "engine mock does not provide the gdbstub capability; the qemu, dosbox-x, and bochs engines provide it"
        );
    }
}
