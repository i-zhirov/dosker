//! Command-line value parsers for published ports and volume mounts.

use std::net::Ipv4Addr;

use dosker_core::ContainerName;

/// A parsed `[addr:]host:guest` port publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublishSpec {
    pub host_address: Ipv4Addr,
    pub host_port: u16,
    pub guest_port: u16,
}

impl PublishSpec {
    /// Parses `[addr:]host:guest`, binding to loopback when no address is given.
    pub fn parse(value: &str) -> Result<Self, SpecError> {
        let parts: Vec<&str> = value.split(':').collect();
        let (host_address, host, guest) = match parts.as_slice() {
            [host, guest] => (Ipv4Addr::LOCALHOST, *host, *guest),
            [address, host, guest] => (parse_address(value, address)?, *host, *guest),
            _ => return Err(publish(value, "expected [addr:]host:guest")),
        };

        Ok(Self {
            host_address,
            host_port: parse_port(value, host)?,
            guest_port: parse_port(value, guest)?,
        })
    }
}

fn parse_address(value: &str, address: &str) -> Result<Ipv4Addr, SpecError> {
    address
        .parse::<Ipv4Addr>()
        .map_err(|_| publish(value, format!("invalid IPv4 address: {address}")))
}

fn parse_port(value: &str, port: &str) -> Result<u16, SpecError> {
    match port.parse::<u16>() {
        Ok(0) => Err(publish(value, "ports must be in 1..=65535")),
        Ok(port) => Ok(port),
        Err(_) => Err(publish(value, format!("invalid port: {port}"))),
    }
}

/// A parsed `name:drive[:ro]` volume mount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeSpec {
    pub name: String,
    pub drive: String,
    pub read_only: bool,
}

impl VolumeSpec {
    /// Parses `name:drive[:ro]`, normalizing the drive to an uppercase letter.
    pub fn parse(value: &str) -> Result<Self, SpecError> {
        let (rest, read_only) = match value.strip_suffix(":ro") {
            Some(rest) => (rest, true),
            None => (value, false),
        };
        let rest = rest.strip_suffix(':').unwrap_or(rest);
        let (name, drive) = rest
            .rsplit_once(':')
            .ok_or_else(|| volume(value, "expected name:drive"))?;
        let name = ContainerName::parse(name)
            .map_err(|_| volume(value, format!("invalid volume name: {name}")))?;

        Ok(Self {
            name: name.into(),
            drive: normalize_drive(value, drive)?,
            read_only,
        })
    }
}

fn normalize_drive(value: &str, drive: &str) -> Result<String, SpecError> {
    let letter = match drive.as_bytes() {
        [letter] if letter.is_ascii_alphabetic() => letter.to_ascii_uppercase(),
        _ => return Err(volume(value, format!("invalid drive: {drive}"))),
    };
    if letter == b'C' {
        return Err(volume(value, "the boot drive C: cannot hold a volume"));
    }

    Ok(format!("{}:", char::from(letter)))
}

fn publish(value: &str, reason: impl Into<String>) -> SpecError {
    SpecError::Publish {
        value: value.to_owned(),
        reason: reason.into(),
    }
}

fn volume(value: &str, reason: impl Into<String>) -> SpecError {
    SpecError::Volume {
        value: value.to_owned(),
        reason: reason.into(),
    }
}

/// A failure while parsing a command-line value.
#[derive(Debug, thiserror::Error)]
pub enum SpecError {
    /// A port publication is malformed.
    #[error("invalid publish mapping {value}: {reason}")]
    Publish { value: String, reason: String },

    /// A volume mount is malformed.
    #[error("invalid volume mapping {value}: {reason}")]
    Volume { value: String, reason: String },
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    #[test]
    fn publish_defaults_to_loopback() {
        let spec = PublishSpec::parse("8080:80").expect("a valid mapping");
        assert_eq!(spec.host_address, Ipv4Addr::LOCALHOST);
        assert_eq!(spec.host_port, 8080);
        assert_eq!(spec.guest_port, 80);
    }

    #[test]
    fn publish_accepts_an_explicit_address() {
        let spec = PublishSpec::parse("0.0.0.0:8080:80").expect("a valid mapping");
        assert_eq!(spec.host_address, Ipv4Addr::UNSPECIFIED);
        assert_eq!(spec.host_port, 8080);
        assert_eq!(spec.guest_port, 80);
    }

    #[test]
    fn publish_rejects_a_lone_port() {
        assert!(matches!(
            PublishSpec::parse("80"),
            Err(SpecError::Publish { .. })
        ));
    }

    #[test]
    fn publish_rejects_port_zero() {
        assert!(matches!(
            PublishSpec::parse("0:80"),
            Err(SpecError::Publish { .. })
        ));
        assert!(matches!(
            PublishSpec::parse("8080:0"),
            Err(SpecError::Publish { .. })
        ));
    }

    #[test]
    fn publish_rejects_a_non_ipv4_address() {
        assert!(matches!(
            PublishSpec::parse("not-an-address:8080:80"),
            Err(SpecError::Publish { .. })
        ));
    }

    #[test]
    fn volume_spec_accepts_the_canonical_form() {
        let spec = VolumeSpec::parse("data:D:").expect("a valid mount");
        assert_eq!(spec.name, "data");
        assert_eq!(spec.drive, "D:");
        assert!(!spec.read_only);
    }

    #[test]
    fn volume_spec_accepts_a_drive_without_a_colon() {
        let spec = VolumeSpec::parse("data:D").expect("a valid mount");
        assert_eq!(spec.name, "data");
        assert_eq!(spec.drive, "D:");
    }

    #[test]
    fn volume_spec_normalizes_a_lowercase_drive() {
        let spec = VolumeSpec::parse("data:d:").expect("a valid mount");
        assert_eq!(spec.name, "data");
        assert_eq!(spec.drive, "D:");
    }

    #[test]
    fn volume_spec_parses_read_only() {
        let spec = VolumeSpec::parse("data:D:ro").expect("a valid mount");
        assert_eq!(spec.drive, "D:");
        assert!(spec.read_only);

        let spec = VolumeSpec::parse("data:d:ro").expect("a valid mount");
        assert_eq!(spec.drive, "D:");
        assert!(spec.read_only);
    }

    #[test]
    fn volume_spec_rejects_the_boot_drive() {
        assert!(matches!(
            VolumeSpec::parse("data:C:"),
            Err(SpecError::Volume { .. })
        ));
    }

    #[test]
    fn volume_spec_rejects_an_unknown_option() {
        assert!(matches!(
            VolumeSpec::parse("data:D:rw"),
            Err(SpecError::Volume { .. })
        ));
    }

    #[test]
    fn volume_spec_rejects_a_missing_drive() {
        assert!(matches!(
            VolumeSpec::parse("data"),
            Err(SpecError::Volume { .. })
        ));
    }

    #[test]
    fn volume_spec_rejects_a_path_like_name() {
        for value in ["../x:D:", "/tmp/x:D:", "a/b:D:"] {
            assert!(
                matches!(VolumeSpec::parse(value), Err(SpecError::Volume { .. })),
                "{value} must be rejected"
            );
        }
    }
}
