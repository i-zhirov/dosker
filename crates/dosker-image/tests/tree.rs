use chrono::{TimeZone, Utc};
use dosker_image::{Attributes, DosPath, LayerDelta, Tree, TreeDir, TreeFile, canonical_json};

fn path(value: &str) -> DosPath {
    DosPath::parse(value).unwrap()
}

fn time(second: i64, nanos: u32) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(second, nanos).unwrap()
}

fn file(path_text: &str) -> TreeFile {
    TreeFile {
        path: path(path_text),
        attributes: Attributes::default(),
        timestamp: time(315_532_800, 0),
        bytes: b"hello".to_vec(),
    }
}

fn dir(path_text: &str) -> TreeDir {
    TreeDir {
        path: path(path_text),
        attributes: Attributes::default(),
        timestamp: time(315_532_800, 0),
    }
}

#[test]
fn apply_delta_adds_files_and_directories() {
    let mut tree = Tree::new();
    let mut delta = LayerDelta::new();
    delta.add_dir(dir("APP"));
    delta.add_file(file("APP/FOO.TXT"));
    tree.apply_delta(&delta).unwrap();
    assert!(tree.get(&path("APP")).is_some());
    assert!(tree.get(&path("APP/FOO.TXT")).is_some());
}

#[test]
fn apply_delta_creates_missing_parent_directories() {
    let mut tree = Tree::new();
    let mut delta = LayerDelta::new();
    delta.add_file(file("APP/FOO.TXT"));
    tree.apply_delta(&delta).unwrap();
    assert!(matches!(
        tree.get(&path("APP")),
        Some(dosker_image::TreeNode::Dir(_))
    ));
}

#[test]
fn apply_delta_rejects_an_empty_path() {
    let mut tree = Tree::new();
    let mut delta = LayerDelta::new();
    delta.add_file(TreeFile {
        path: path(""),
        ..file("A")
    });
    assert!(tree.apply_delta(&delta).is_err());
}

#[test]
fn apply_delta_replaces_content_and_attributes() {
    let mut tree = Tree::new();
    let mut first = LayerDelta::new();
    first.add_file(file("APP/FOO.TXT"));
    tree.apply_delta(&first).unwrap();
    let mut second = LayerDelta::new();
    second.add_file(TreeFile {
        attributes: Attributes {
            read_only: true,
            ..Attributes::default()
        },
        bytes: b"new".to_vec(),
        ..file("APP/FOO.TXT")
    });
    tree.apply_delta(&second).unwrap();
    let node = tree.get(&path("APP/FOO.TXT")).unwrap();
    assert!(node.attributes().read_only);
}

#[test]
fn whiteout_removes_a_file() {
    let mut tree = Tree::new();
    tree.apply_delta(&LayerDelta {
        files: vec![file("APP/FOO.TXT")],
        ..LayerDelta::new()
    })
    .unwrap();
    let mut delta = LayerDelta::new();
    delta.whiteout(path("APP/FOO.TXT"));
    tree.apply_delta(&delta).unwrap();
    assert!(tree.get(&path("APP/FOO.TXT")).is_none());
}

#[test]
fn whiteout_removes_a_directory_subtree() {
    let mut tree = Tree::new();
    tree.apply_delta(&LayerDelta {
        files: vec![file("APP/FOO.TXT")],
        ..LayerDelta::new()
    })
    .unwrap();
    let mut delta = LayerDelta::new();
    delta.whiteout(path("APP"));
    tree.apply_delta(&delta).unwrap();
    assert!(tree.is_empty());
}

#[test]
fn whiteout_of_a_missing_path_is_ignored() {
    let mut tree = Tree::new();
    let mut delta = LayerDelta::new();
    delta.whiteout(path("MISSING"));
    tree.apply_delta(&delta).unwrap();
}

#[test]
fn file_and_directory_conflict_is_an_error() {
    let mut tree = Tree::new();
    tree.apply_delta(&LayerDelta {
        files: vec![file("APP")],
        ..LayerDelta::new()
    })
    .unwrap();
    let mut delta = LayerDelta::new();
    delta.add_dir(dir("APP/SUB"));
    assert!(tree.apply_delta(&delta).is_err());

    let mut tree = Tree::new();
    tree.apply_delta(&LayerDelta {
        dirs: vec![dir("APP")],
        ..LayerDelta::new()
    })
    .unwrap();
    let mut delta = LayerDelta::new();
    delta.add_file(file("APP"));
    assert!(tree.apply_delta(&delta).is_err());
}

#[test]
fn lowercase_paths_are_normalized_before_application() {
    let mut tree = Tree::new();
    let mut delta = LayerDelta::new();
    delta.add_file(file("app/foo.txt"));
    tree.apply_delta(&delta).unwrap();
    assert!(tree.get(&path("APP/FOO.TXT")).is_some());
}

#[test]
fn tree_digest_is_stable_across_insertion_order() {
    let mut first = Tree::new();
    let mut one = LayerDelta::new();
    one.add_file(file("B.TXT"));
    one.add_file(file("A.TXT"));
    first.apply_delta(&one).unwrap();

    let mut second = Tree::new();
    let mut two = LayerDelta::new();
    two.add_file(file("A.TXT"));
    two.add_file(file("B.TXT"));
    second.apply_delta(&two).unwrap();

    assert_eq!(first.digest().unwrap(), second.digest().unwrap());
}

#[test]
fn tree_digest_changes_when_file_content_changes() {
    let mut tree = Tree::new();
    tree.apply_delta(&LayerDelta {
        files: vec![file("A.TXT")],
        ..LayerDelta::new()
    })
    .unwrap();
    let before = tree.digest().unwrap();
    tree.apply_delta(&LayerDelta {
        files: vec![TreeFile {
            bytes: b"other".to_vec(),
            ..file("A.TXT")
        }],
        ..LayerDelta::new()
    })
    .unwrap();
    assert_ne!(before, tree.digest().unwrap());
}

#[test]
fn tree_digest_changes_when_attributes_change() {
    let mut tree = Tree::new();
    tree.apply_delta(&LayerDelta {
        files: vec![file("A.TXT")],
        ..LayerDelta::new()
    })
    .unwrap();
    let before = tree.digest().unwrap();
    tree.apply_delta(&LayerDelta {
        files: vec![TreeFile {
            attributes: Attributes {
                hidden: true,
                ..Attributes::default()
            },
            ..file("A.TXT")
        }],
        ..LayerDelta::new()
    })
    .unwrap();
    assert_ne!(before, tree.digest().unwrap());
}

#[test]
fn tree_digest_truncates_subsecond_timestamps() {
    let mut first = Tree::new();
    first
        .apply_delta(&LayerDelta {
            files: vec![TreeFile {
                timestamp: time(1, 1),
                ..file("A.TXT")
            }],
            ..LayerDelta::new()
        })
        .unwrap();
    let mut second = Tree::new();
    second
        .apply_delta(&LayerDelta {
            files: vec![TreeFile {
                timestamp: time(1, 999_999_999),
                ..file("A.TXT")
            }],
            ..LayerDelta::new()
        })
        .unwrap();
    assert_eq!(first.digest().unwrap(), second.digest().unwrap());
}

#[test]
fn canonical_json_sorts_keys_and_has_no_whitespace() {
    let value = serde_json::json!({"b": 1, "a": {"d": 3, "c": 2}});
    let bytes = canonical_json(&value).unwrap();
    assert_eq!(bytes, br#"{"a":{"c":2,"d":3},"b":1}"#);
}
