use dosker_engine::EngineError;
use dosker_engine_qemu::split_engine_config;

#[test]
fn splits_plain_tokens() {
    assert_eq!(
        split_engine_config("-smp 2").expect("split the line"),
        vec!["-smp".to_owned(), "2".to_owned()]
    );
}

#[test]
fn splits_quoted_tokens() {
    assert_eq!(
        split_engine_config("-drive \"file=a b.img\"").expect("split the line"),
        vec!["-drive".to_owned(), "file=a b.img".to_owned()]
    );
}

#[test]
fn splits_single_quoted_tokens() {
    assert_eq!(
        split_engine_config("-drive 'file=a b.img'").expect("split the line"),
        vec!["-drive".to_owned(), "file=a b.img".to_owned()]
    );
}

#[test]
fn escapes_a_space_outside_quotes() {
    assert_eq!(
        split_engine_config("-drive file=a\\ b.img").expect("split the line"),
        vec!["-drive".to_owned(), "file=a b.img".to_owned()]
    );
}

#[test]
fn escapes_a_quote_inside_double_quotes() {
    assert_eq!(
        split_engine_config("-drive \"file=a\\\"b\"").expect("split the line"),
        vec!["-drive".to_owned(), "file=a\"b".to_owned()]
    );
}

#[test]
fn rejects_an_unterminated_quote() {
    match split_engine_config("-drive \"file=a b.img") {
        Err(EngineError::Setup(reason)) => assert!(
            reason.contains("unterminated quote"),
            "unexpected setup reason: {reason}"
        ),
        other => panic!("expected a setup error for an unterminated quote, got {other:?}"),
    }
}

#[test]
fn rejects_a_trailing_backslash() {
    match split_engine_config("-drive file=a\\") {
        Err(EngineError::Setup(_)) => {}
        other => panic!("expected a setup error for a trailing backslash, got {other:?}"),
    }
}
