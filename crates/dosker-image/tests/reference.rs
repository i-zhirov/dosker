use std::sync::Mutex;

use dosker_image::{
    DEFAULT_REGISTRY, ImageDigest, ImageRef, ImageReference, ReferenceSelector, RegistryReference,
    default_registry_url,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());

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

#[test]
fn parse_accepts_a_namespaced_reference() {
    let reference = ImageReference::parse("owner/name:tag").unwrap();

    assert_eq!(reference.namespace().unwrap().as_str(), "owner");
    assert_eq!(reference.name().as_str(), "name");
    assert_eq!(reference.repository().to_string(), "owner/name");
}

#[test]
fn parse_defaults_the_namespace_tag() {
    let reference = ImageReference::parse("owner/name").unwrap();

    assert_eq!(reference.tag().as_str(), "latest");
    assert_eq!(reference.to_string(), "owner/name:latest");
}

#[test]
fn display_round_trips_a_namespaced_reference() {
    let reference = ImageReference::parse("owner/name:tag").unwrap();

    assert_eq!(reference.to_string(), "owner/name:tag");
    assert_eq!(
        ImageReference::parse(&reference.to_string()).unwrap(),
        reference
    );
}

#[test]
fn parse_rejects_a_second_slash() {
    assert!(ImageReference::parse("owner/name/extra:tag").is_err());
}

#[test]
fn parse_rejects_an_uppercase_namespace() {
    assert!(ImageReference::parse("Owner/name:tag").is_err());
}

#[test]
fn registry_reference_parses_a_default_registry() {
    let reference = RegistryReference::parse("owner/name:tag").unwrap();

    assert!(reference.host().is_none());
    assert_eq!(reference.repository().to_string(), "owner/name");
    assert_eq!(reference.identity(), "owner/name");
}

#[test]
fn registry_reference_parses_an_explicit_host() {
    let reference = RegistryReference::parse("example.com/owner/name:tag").unwrap();

    assert_eq!(reference.host().unwrap().as_str(), "example.com");
    assert_eq!(reference.identity(), "example.com/owner/name");
}

#[test]
fn registry_reference_parses_a_host_port() {
    let reference = RegistryReference::parse("example.com:5000/owner/name:tag").unwrap();

    assert_eq!(reference.host().unwrap().as_str(), "example.com:5000");
    assert_eq!(
        reference.local_reference().unwrap().to_string(),
        "owner/name:tag"
    );
}

#[test]
fn registry_reference_parses_a_digest_selector() {
    let digest = format!("sha256:{}", "a".repeat(64));
    let reference = RegistryReference::parse(&format!("owner/name@{digest}")).unwrap();

    assert!(matches!(reference.selector(), ReferenceSelector::Digest(_)));
    assert!(reference.local_reference().is_none());
}

#[test]
fn registry_reference_rejects_an_extra_segment() {
    assert!(RegistryReference::parse("host/owner/name/extra:tag").is_err());
}

#[test]
fn registry_reference_rejects_a_bare_digest() {
    let digest = format!("sha256:{}", "b".repeat(64));
    assert!(RegistryReference::parse(&digest).is_err());
}

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
fn image_ref_accepts_a_namespaced_tag_and_digest() {
    let tag = ImageRef::parse("owner/name:tag").unwrap();
    assert_eq!(tag.to_string(), "owner/name:tag");

    let digest = format!("sha256:{}", "d".repeat(64));
    let by_digest = ImageRef::parse(&format!("owner/name@{digest}")).unwrap();
    assert!(matches!(by_digest, ImageRef::Digest(_)));
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

#[test]
fn default_registry_url_reads_the_environment() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let original = std::env::var("DOSKER_REGISTRY").ok();

    unsafe { std::env::remove_var("DOSKER_REGISTRY") };
    assert_eq!(default_registry_url(), DEFAULT_REGISTRY);

    unsafe { std::env::set_var("DOSKER_REGISTRY", "https://example.com/registry/") };
    assert_eq!(default_registry_url(), "https://example.com/registry");

    match original {
        Some(value) => unsafe { std::env::set_var("DOSKER_REGISTRY", value) },
        None => unsafe { std::env::remove_var("DOSKER_REGISTRY") },
    }
}
