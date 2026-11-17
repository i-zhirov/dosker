use dosker_image::inject::{
    AGENT_PATH, AUTOEXEC_OLD_PATH, AgentInjection, NEW_AUTOEXEC_BYTES, WRAPPED_AUTOEXEC_BYTES,
    plan_agent_injection, starts_agent,
};
use dosker_image::{Attributes, DosPath, ImageError, LayerDelta, Tree, TreeFile, fixed_timestamp};

const AGENT: &[u8] = b"MZ synthetic agent";

fn path(value: &str) -> DosPath {
    DosPath::parse(value).unwrap()
}

fn tree_with(files: &[(&str, &[u8], Attributes)]) -> Tree {
    let mut tree = Tree::new();
    let mut delta = LayerDelta::new();
    for (path_text, bytes, attributes) in files {
        delta.add_file(TreeFile {
            path: path(path_text),
            attributes: attributes.clone(),
            timestamp: fixed_timestamp(),
            bytes: bytes.to_vec(),
        });
    }
    tree.apply_delta(&delta).unwrap();
    tree
}

fn delta_file<'a>(injection: &'a AgentInjection, path_text: &str) -> Option<&'a TreeFile> {
    injection
        .delta
        .files
        .iter()
        .find(|file| file.path == path(path_text))
}

#[test]
fn injection_creates_the_agent_directory_and_file() {
    let tree = Tree::new();
    let injection = plan_agent_injection(&tree, AGENT).unwrap();

    assert!(injection.applied);
    let dir = injection
        .delta
        .dirs
        .iter()
        .find(|dir| dir.path == path("DOSKER"))
        .expect("the agent directory is added");
    assert_eq!(dir.attributes, Attributes::default());
    assert_eq!(dir.timestamp, fixed_timestamp());

    let agent_file = delta_file(&injection, AGENT_PATH).expect("the agent file is added");
    assert_eq!(agent_file.bytes, AGENT);
    assert_eq!(agent_file.attributes, Attributes::default());
    assert_eq!(agent_file.timestamp, fixed_timestamp());

    let autoexec = delta_file(&injection, r"C:\AUTOEXEC.BAT").expect("AUTOEXEC.BAT is added");
    assert_eq!(autoexec.bytes, NEW_AUTOEXEC_BYTES);
    assert_eq!(autoexec.attributes, Attributes::default());
    assert_eq!(autoexec.timestamp, fixed_timestamp());
}

#[test]
fn injection_wraps_an_existing_autoexec() {
    let attributes = Attributes {
        read_only: true,
        archive: true,
        ..Attributes::default()
    };
    let tree = tree_with(&[("AUTOEXEC.BAT", b"@ECHO OFF\r\n", attributes.clone())]);
    let injection = plan_agent_injection(&tree, AGENT).unwrap();

    assert!(injection.applied);
    let old = delta_file(&injection, AUTOEXEC_OLD_PATH).expect("the old batch is added");
    assert_eq!(old.bytes, b"@ECHO OFF\r\n");
    assert_eq!(old.attributes, Attributes::default());
    assert_eq!(old.timestamp, fixed_timestamp());

    let autoexec = delta_file(&injection, r"C:\AUTOEXEC.BAT").expect("AUTOEXEC.BAT is replaced");
    assert_eq!(autoexec.bytes, WRAPPED_AUTOEXEC_BYTES);
    assert_eq!(autoexec.attributes, attributes);
    assert_eq!(autoexec.timestamp, fixed_timestamp());
}

#[test]
fn injection_keeps_the_wrap_out_of_an_absent_autoexec() {
    let tree = Tree::new();
    let injection = plan_agent_injection(&tree, AGENT).unwrap();

    assert!(delta_file(&injection, AUTOEXEC_OLD_PATH).is_none());
    assert!(delta_file(&injection, r"C:\AUTOEXEC.BAT").is_some());
}

#[test]
fn injection_detects_an_existing_agent_line() {
    let tree = tree_with(&[
        (
            "AUTOEXEC.BAT",
            b"@ECHO OFF\r\nc:\\dosker\\dosker.exe\r\n",
            Attributes::default(),
        ),
        ("DOSKER/DOSKER.EXE", AGENT, Attributes::default()),
    ]);
    let injection = plan_agent_injection(&tree, AGENT).unwrap();

    assert!(!injection.applied);
    assert!(injection.delta.files.is_empty());
    assert!(injection.delta.dirs.is_empty());
    assert!(delta_file(&injection, r"C:\AUTOEXEC.BAT").is_none());
    assert!(delta_file(&injection, AUTOEXEC_OLD_PATH).is_none());
}

#[test]
fn injection_replaces_a_different_agent_executable() {
    let attributes = Attributes {
        hidden: true,
        archive: true,
        ..Attributes::default()
    };
    let tree = tree_with(&[("DOSKER/DOSKER.EXE", b"old agent", attributes.clone())]);
    let injection = plan_agent_injection(&tree, AGENT).unwrap();

    let file = delta_file(&injection, AGENT_PATH).expect("the agent file is replaced");
    assert_eq!(file.bytes, AGENT);
    assert_eq!(file.attributes, attributes);
    assert_eq!(file.timestamp, fixed_timestamp());
}

#[test]
fn injection_is_idempotent() {
    let mut tree = Tree::new();
    let first = plan_agent_injection(&tree, AGENT).unwrap();
    assert!(first.applied);
    tree.apply_delta(&first.delta).unwrap();

    let second = plan_agent_injection(&tree, AGENT).unwrap();
    assert!(!second.applied);
    assert!(second.delta.files.is_empty());
    assert!(second.delta.dirs.is_empty());
    assert!(second.delta.whiteouts.is_empty());
}

#[test]
fn injection_rejects_a_dosker_file() {
    let tree = tree_with(&[("DOSKER", b"not a directory", Attributes::default())]);

    match plan_agent_injection(&tree, AGENT) {
        Err(ImageError::AgentInjection { reason }) => {
            assert!(reason.contains(r"C:\DOSKER"), "reason: {reason}");
        }
        other => panic!("expected AgentInjection, got {other:?}"),
    }
}

#[test]
fn injection_rejects_an_existing_old_batch() {
    let tree = tree_with(&[
        ("AUTOEXEC.BAT", b"@ECHO OFF\r\n", Attributes::default()),
        (r"C:\DOSKER\AUTOEXEC.OLD", b"old", Attributes::default()),
    ]);

    match plan_agent_injection(&tree, AGENT) {
        Err(ImageError::AgentInjection { reason }) => {
            assert!(
                reason.contains(r"C:\DOSKER\AUTOEXEC.OLD"),
                "reason: {reason}"
            );
        }
        other => panic!("expected AgentInjection, got {other:?}"),
    }
}

#[test]
fn starts_agent_matches_the_agent_command() {
    assert!(starts_agent(b"  c:\\dosker\\dosker.exe  \r\n"));
    assert!(starts_agent(b"@ECHO OFF\r\nC:\\DOSKER\\DOSKER.EXE\r\n"));
    assert!(starts_agent(b"C:\\DOSKER\\DOSKER.EXE /TEST\r\n"));
    assert!(starts_agent(b"CALL C:\\DOSKER\\DOSKER.EXE\r\n"));
    assert!(starts_agent(b"CALL C:\\DOSKER\\DOSKER.EXE /TEST\r\n"));

    assert!(!starts_agent(b"C:\\DOSKER\\AUTOEXEC.OLD\r\n"));
    assert!(!starts_agent(b"C:\\DOSKER\\DOSKER.EXE.BAK\r\n"));
    assert!(!starts_agent(b"REM C:\\DOSKER\\DOSKER.EXE\r\n"));
}

#[test]
fn injection_detects_the_self_test_agent_command() {
    let tree = tree_with(&[
        (
            "AUTOEXEC.BAT",
            b"@ECHO OFF\r\nC:\\DOSKER\\DOSKER.EXE /TEST\r\n",
            Attributes::default(),
        ),
        ("DOSKER/DOSKER.EXE", AGENT, Attributes::default()),
    ]);
    let injection = plan_agent_injection(&tree, AGENT).unwrap();

    assert!(!injection.applied);
    assert!(delta_file(&injection, r"C:\AUTOEXEC.BAT").is_none());
    assert!(delta_file(&injection, AUTOEXEC_OLD_PATH).is_none());
}

#[test]
fn injection_detects_a_call_with_arguments() {
    let tree = tree_with(&[
        (
            "AUTOEXEC.BAT",
            b"CALL C:\\DOSKER\\DOSKER.EXE /TEST\r\n",
            Attributes::default(),
        ),
        ("DOSKER/DOSKER.EXE", AGENT, Attributes::default()),
    ]);
    let injection = plan_agent_injection(&tree, AGENT).unwrap();

    assert!(!injection.applied);
    assert!(delta_file(&injection, r"C:\AUTOEXEC.BAT").is_none());
    assert!(delta_file(&injection, AUTOEXEC_OLD_PATH).is_none());
}
