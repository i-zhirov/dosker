use dosker_engine::EngineError;
use dosker_engine_bochs::DiskGeometry;

#[test]
fn thirty_two_mib_is_128_16_32() {
    let geometry = DiskGeometry::from_size(33_554_432).expect("a 32 MiB image is valid");

    assert_eq!(geometry.cylinders, 128);
    assert_eq!(geometry.heads, 16);
    assert_eq!(geometry.sectors, 32);
}

#[test]
fn one_mib_is_4_16_32() {
    let geometry = DiskGeometry::from_size(1_048_576).expect("a 1 MiB image is valid");

    assert_eq!(geometry.cylinders, 4);
    assert_eq!(geometry.heads, 16);
    assert_eq!(geometry.sectors, 32);
}

#[test]
fn rejects_a_size_that_is_not_a_multiple_of_262144() {
    let error = DiskGeometry::from_size(300_000).expect_err("an unaligned size must be rejected");

    match error {
        EngineError::Setup(message) => {
            assert!(
                message.contains("300000"),
                "the message must name the size: {message}"
            );
            assert!(
                message.contains("262144"),
                "the message must name the required multiple: {message}"
            );
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn rejects_an_empty_image() {
    let error = DiskGeometry::from_size(0).expect_err("an empty image must be rejected");

    assert!(
        matches!(error, EngineError::Setup(_)),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_more_than_16383_cylinders() {
    let error =
        DiskGeometry::from_size(4_294_967_296).expect_err("too many cylinders must be rejected");

    assert!(
        matches!(error, EngineError::Setup(_)),
        "unexpected error: {error}"
    );
}

#[test]
fn accepts_one_cylinder() {
    let geometry = DiskGeometry::from_size(262_144).expect("one cylinder is valid");

    assert_eq!(geometry.cylinders, 1);
    assert_eq!(geometry.heads, 16);
    assert_eq!(geometry.sectors, 32);
}

#[test]
fn accepts_the_maximum_cylinder_count() {
    let geometry = DiskGeometry::from_size(4_294_705_152).expect("16383 cylinders are valid");

    assert_eq!(geometry.cylinders, 16_383);
}
