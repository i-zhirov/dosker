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
