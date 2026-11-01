use dosker_core::{ContainerName, CoreError};

#[test]
fn parse_accepts_single_character() {
    let name = ContainerName::parse("a").unwrap();
    assert_eq!(name.as_str(), "a");
}

#[test]
fn parse_accepts_max_length_63() {
    let value = "a".repeat(63);
    let name = ContainerName::parse(&value).unwrap();
    assert_eq!(name.as_str(), value);
}

#[test]
fn parse_rejects_length_64() {
    let value = "a".repeat(64);
    assert!(matches!(
        ContainerName::parse(&value),
        Err(CoreError::InvalidContainerName(_))
    ));
}

#[test]
fn parse_rejects_uppercase() {
    assert!(matches!(
        ContainerName::parse("Ab"),
        Err(CoreError::InvalidContainerName(_))
    ));
}

#[test]
fn parse_rejects_leading_dash() {
    assert!(matches!(
        ContainerName::parse("-a"),
        Err(CoreError::InvalidContainerName(_))
    ));
}

#[test]
fn parse_rejects_leading_dot() {
    assert!(matches!(
        ContainerName::parse(".a"),
        Err(CoreError::InvalidContainerName(_))
    ));
}

#[test]
fn parse_rejects_leading_underscore() {
    assert!(matches!(
        ContainerName::parse("_a"),
        Err(CoreError::InvalidContainerName(_))
    ));
}

#[test]
fn parse_rejects_space() {
    for value in [" a", "a b"] {
        assert!(
            matches!(
                ContainerName::parse(value),
                Err(CoreError::InvalidContainerName(_))
            ),
            "{value:?} was accepted"
        );
    }
}

#[test]
fn parse_accepts_dots_dashes_underscores_after_first() {
    let name = ContainerName::parse("a.b-c_d").unwrap();
    assert_eq!(name.as_str(), "a.b-c_d");
}

#[test]
fn serde_round_trip() {
    let name = ContainerName::parse("a.b-c_d").unwrap();
    let json = serde_json::to_string(&name).unwrap();
    assert_eq!(json, "\"a.b-c_d\"");
    let back: ContainerName = serde_json::from_str(&json).unwrap();
    assert_eq!(back, name);
    assert!(serde_json::from_str::<ContainerName>("\"Bad Name\"").is_err());
}
