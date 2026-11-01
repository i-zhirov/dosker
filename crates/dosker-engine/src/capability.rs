//! Engine capabilities and the set an adapter declares.

use serde::{Deserialize, Serialize};

/// A behaviour an engine adapter may declare.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Boot,
    Serial,
    Stop,
    Headless,
    HostDirLive,
    DebugNative,
    #[serde(rename = "gdbstub")]
    GdbStub,
    Screenshot,
    Vnc,
    Nat,
    Audio,
}

impl Capability {
    /// Every capability, in declaration order.
    pub const ALL: [Capability; 11] = [
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
        Capability::Audio,
    ];

    /// Resolves a capability from its stable name.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|capability| capability.name() == name)
    }

    /// Returns the stable name.
    pub fn name(self) -> &'static str {
        match self {
            Capability::Boot => "boot",
            Capability::Serial => "serial",
            Capability::Stop => "stop",
            Capability::Headless => "headless",
            Capability::HostDirLive => "host_dir_live",
            Capability::DebugNative => "debug_native",
            Capability::GdbStub => "gdbstub",
            Capability::Screenshot => "screenshot",
            Capability::Vnc => "vnc",
            Capability::Nat => "nat",
            Capability::Audio => "audio",
        }
    }
}

/// A compact set of declared capabilities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CapabilitySet(u16);

impl CapabilitySet {
    /// No capabilities.
    pub const EMPTY: Self = Self(0);

    /// Adds a capability.
    pub fn insert(&mut self, capability: Capability) {
        self.0 |= 1 << capability as u16;
    }

    /// Reports whether a capability is present.
    pub fn contains(self, capability: Capability) -> bool {
        self.0 & (1 << capability as u16) != 0
    }

    /// Iterates the present capabilities in declaration order.
    pub fn iter(self) -> impl Iterator<Item = Capability> {
        Capability::ALL
            .into_iter()
            .filter(move |capability| self.contains(*capability))
    }

    /// Iterates the stable names of the present capabilities in declaration
    /// order.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        self.iter().map(Capability::name)
    }
}

impl FromIterator<Capability> for CapabilitySet {
    fn from_iter<I: IntoIterator<Item = Capability>>(iter: I) -> Self {
        let mut set = Self::EMPTY;
        for capability in iter {
            set.insert(capability);
        }
        set
    }
}
