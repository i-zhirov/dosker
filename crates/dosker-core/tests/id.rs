use std::collections::HashSet;

use dosker_core::{ContainerId, CoreError};

#[test]
fn generated_id_is_twelve_lowercase_hex() {
    for _ in 0..1000 {
        let id = ContainerId::generate();
        let text = id.as_str();
        assert_eq!(text.len(), 12, "id {text:?} is not twelve bytes");
        assert!(
            text.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "id {text:?} contains a character outside 0-9a-f"
        );
        assert_eq!(ContainerId::parse(text).unwrap(), id);
    }
}

#[test]
fn generated_ids_are_unique() {
    let ids: HashSet<ContainerId> = (0..1000).map(|_| ContainerId::generate()).collect();
    assert_eq!(ids.len(), 1000);
}

#[test]
fn parse_accepts_lowercase_hex() {
    let id = ContainerId::parse("0123456789ab").unwrap();
    assert_eq!(id.as_str(), "0123456789ab");
}

#[test]
fn parse_rejects_uppercase() {
    assert!(matches!(
        ContainerId::parse("0123456789AB"),
        Err(CoreError::InvalidContainerId(_))
    ));
}

#[test]
fn parse_rejects_wrong_length() {
    for value in ["", "0123456789a", "0123456789abc"] {
        assert!(
            matches!(
                ContainerId::parse(value),
                Err(CoreError::InvalidContainerId(_))
            ),
            "{value:?} was accepted"
        );
    }
}

#[test]
fn parse_rejects_non_hex() {
    for value in ["0123456789ag", "0123456789a-"] {
        assert!(
            matches!(
                ContainerId::parse(value),
                Err(CoreError::InvalidContainerId(_))
            ),
            "{value:?} was accepted"
        );
    }
}

#[test]
fn parse_rejects_whitespace() {
    for value in [" 123456789ab", "0123456789ab "] {
        assert!(
            matches!(
                ContainerId::parse(value),
                Err(CoreError::InvalidContainerId(_))
            ),
            "{value:?} was accepted"
        );
    }
}

#[test]
fn serde_round_trip() {
    let id = ContainerId::parse("0123456789ab").unwrap();
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, "\"0123456789ab\"");
    let back: ContainerId = serde_json::from_str(&json).unwrap();
    assert_eq!(back, id);
    assert!(serde_json::from_str::<ContainerId>("\"NOT-A-VALID\"").is_err());
}
