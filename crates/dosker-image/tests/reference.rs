use dosker_image::ImageReference;

#[test]
fn parse_accepts_name_and_tag() {
    let reference = ImageReference::parse("freedos:freedos-test").unwrap();

    assert_eq!(reference.name().as_str(), "freedos");
    assert_eq!(reference.tag().as_str(), "freedos-test");
}

#[test]
fn parse_defaults_to_latest_when_the_tag_is_absent() {
    let reference = ImageReference::parse("freedos-test").unwrap();

    assert_eq!(reference.name().as_str(), "freedos-test");
    assert_eq!(reference.tag().as_str(), "latest");
    assert_eq!(reference.to_string(), "freedos-test:latest");
}

#[test]
fn parse_rejects_an_empty_tag() {
    assert!(ImageReference::parse("freedos-test:").is_err());
}

#[test]
fn parse_rejects_an_uppercase_name() {
    assert!(ImageReference::parse("FreeDOS:latest").is_err());
}

#[test]
fn parse_rejects_an_empty_name() {
    assert!(ImageReference::parse(":latest").is_err());
}

#[test]
fn parse_rejects_a_second_colon() {
    assert!(ImageReference::parse("freedos:latest:extra").is_err());
}

#[test]
fn display_round_trips() {
    let reference = ImageReference::parse("freedos:freedos-test").unwrap();

    assert_eq!(reference.to_string(), "freedos:freedos-test");
    assert_eq!(
        ImageReference::parse(&reference.to_string()).unwrap(),
        reference
    );
}

#[test]
fn serde_round_trips_as_a_bare_string() {
    let reference = ImageReference::parse("freedos:latest").unwrap();

    let json = serde_json::to_string(&reference).unwrap();
    assert_eq!(json, "\"freedos:latest\"");

    let parsed: ImageReference = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, reference);
}

use dosker_image::{ImageDigest, ImageRef};

#[test]
fn parse_accepts_a_bare_name() {
    let reference = ImageRef::parse("freedos").unwrap();
    assert_eq!(reference.to_string(), "freedos:latest");
}

#[test]
fn parse_accepts_a_name_at_digest() {
    let digest = format!("sha256:{}", "a".repeat(64));
    let reference = ImageRef::parse(&format!("freedos@{digest}")).unwrap();
    assert_eq!(reference.to_string(), digest);
}

#[test]
fn parse_accepts_a_bare_digest() {
    let digest = format!("sha256:{}", "b".repeat(64));
    let reference = ImageRef::parse(&digest).unwrap();
    assert!(matches!(reference, ImageRef::Digest(_)));
}

#[test]
fn parse_rejects_an_uppercase_digest() {
    assert!(ImageDigest::parse(&format!("sha256:{}", "A".repeat(64))).is_err());
}

#[test]
fn parse_rejects_a_short_digest() {
    assert!(ImageDigest::parse(&format!("sha256:{}", "a".repeat(63))).is_err());
}

#[test]
fn parse_rejects_an_empty_name_before_at() {
    assert!(ImageRef::parse(&format!("@sha256:{}", "a".repeat(64))).is_err());
}

#[test]
fn display_round_trips_a_digest() {
    let digest = format!("sha256:{}", "c".repeat(64));
    assert_eq!(ImageRef::parse(&digest).unwrap().to_string(), digest);
}
