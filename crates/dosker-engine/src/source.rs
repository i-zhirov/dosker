//! Where an engine binary was found.

/// The source that supplied an engine binary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineSource {
    /// An environment variable override.
    Env,
    /// A binary managed under the data root.
    Managed,
    /// A Homebrew installation.
    Homebrew,
    /// A binary found on the host path.
    Path,
}

impl EngineSource {
    /// Returns the lowercase name recorded in a lock file.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::Managed => "managed",
            Self::Homebrew => "homebrew",
            Self::Path => "path",
        }
    }

    /// Parses the lowercase name recorded in a lock file.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "env" => Some(Self::Env),
            "managed" => Some(Self::Managed),
            "homebrew" => Some(Self::Homebrew),
            "path" => Some(Self::Path),
            _ => None,
        }
    }
}
