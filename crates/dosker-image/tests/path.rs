use dosker_image::{DosName, DosPath, normalize_tree_member};

#[test]
fn normalize_uppercases_a_lowercase_name() {
    assert_eq!(DosName::normalize("foo.txt").unwrap().as_str(), "FOO.TXT");
    assert_eq!(
        DosPath::parse("app/foo.txt").unwrap().as_str(),
        "APP/FOO.TXT"
    );
}

#[test]
fn normalize_accepts_each_allowed_symbol() {
    for symbol in "$#@!%&'()-{}~^_`".chars() {
        let name = format!("A{symbol}B.TXT");
        assert!(DosName::normalize(&name).is_ok(), "rejected {name}");
    }
}

#[test]
fn normalize_rejects_a_name_longer_than_eight() {
    assert!(DosName::normalize("ABCDEFGHI.TXT").is_err());
}

#[test]
fn normalize_rejects_an_extension_longer_than_three() {
    assert!(DosName::normalize("FILE.TEXT").is_err());
}

#[test]
fn normalize_rejects_a_leading_period_or_space() {
    assert!(DosName::normalize(".FOO").is_err());
    assert!(DosName::normalize(" FOO").is_err());
}

#[test]
fn normalize_rejects_a_second_dot() {
    assert!(DosName::normalize("A.B.C").is_err());
}

#[test]
fn normalize_rejects_an_empty_component() {
    assert!(DosPath::parse("A//B").is_err());
}

#[test]
fn normalize_rejects_a_dot_dot_component() {
    assert!(DosPath::parse("A/../B").is_err());
}

#[test]
fn normalize_accepts_a_leading_separator() {
    assert_eq!(DosPath::parse("/A").unwrap().as_str(), "A");
    assert_eq!(DosPath::parse(r"\A").unwrap().as_str(), "A");
}

#[test]
fn normalize_rejects_a_non_c_drive() {
    assert!(DosPath::parse(r"D:\APP").is_err());
}

#[test]
fn normalize_accepts_a_c_drive_and_either_separator() {
    assert_eq!(
        DosPath::parse(r"C:\app\foo.txt").unwrap().as_str(),
        "APP/FOO.TXT"
    );
    assert_eq!(
        DosPath::parse("c:/app/foo.txt").unwrap().as_str(),
        "APP/FOO.TXT"
    );
}

#[test]
fn normalize_accepts_the_drive_root() {
    for root in [r"C:\", "C:/", "C:"] {
        let path = DosPath::parse(root).unwrap();
        assert_eq!(path.as_str(), "");
        assert_eq!(path.to_dos_string(), r"C:\");
    }
}

#[test]
fn tree_member_requires_the_files_prefix() {
    assert!(normalize_tree_member("APP/FOO.TXT").is_err());
    assert_eq!(
        normalize_tree_member("files/APP/FOO.TXT").unwrap().as_str(),
        "APP/FOO.TXT"
    );
}

#[test]
fn tree_member_rejects_traversal_and_absolute_paths() {
    assert!(normalize_tree_member("files/../APP").is_err());
    assert!(normalize_tree_member("files//APP").is_err());
}

#[test]
fn dos_path_round_trips_through_serde() {
    let path = DosPath::parse(r"C:\APP\FOO.TXT").unwrap();
    let json = serde_json::to_string(&path).unwrap();
    assert_eq!(json, "\"APP/FOO.TXT\"");
    let decoded: DosPath = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, path);
}
