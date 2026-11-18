use std::collections::BTreeMap;
use std::io;
use std::time::{Duration, Instant};

use dosker_build::{BuildError, BuildReporter, HostRunRequest, run_host};
use dosker_image::{
    Attributes, DosPath, LayerDelta, Tree, TreeDir, TreeFile, TreeNode, fixed_timestamp,
};

const LINE: usize = 4;

#[derive(Default)]
struct TestReporter {
    output: Vec<u8>,
}

impl BuildReporter for TestReporter {
    fn line(&mut self, _text: &str) -> io::Result<()> {
        Ok(())
    }

    fn output(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.output.extend_from_slice(bytes);
        Ok(())
    }
}

fn dos(path: &str) -> DosPath {
    DosPath::parse(path).expect("the DOS path parses")
}

fn root() -> DosPath {
    dos(r"C:\")
}

fn tree(dirs: &[&str], files: &[(&str, &[u8])]) -> Tree {
    let mut delta = LayerDelta::new();
    for dir in dirs {
        delta.add_dir(TreeDir {
            path: dos(dir),
            attributes: Attributes::default(),
            timestamp: fixed_timestamp(),
        });
    }
    for (path, bytes) in files {
        delta.add_file(TreeFile {
            path: dos(path),
            attributes: Attributes {
                archive: true,
                ..Attributes::default()
            },
            timestamp: fixed_timestamp(),
            bytes: bytes.to_vec(),
        });
    }
    let mut tree = Tree::new();
    tree.apply_delta(&delta).expect("the tree applies");
    tree
}

fn run(tree: &Tree, command: &str) -> (Result<LayerDelta, BuildError>, TestReporter) {
    run_in(
        tree,
        command,
        &root(),
        &BTreeMap::new(),
        Duration::from_secs(30),
    )
}

fn run_in(
    tree: &Tree,
    command: &str,
    workdir: &DosPath,
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> (Result<LayerDelta, BuildError>, TestReporter) {
    let work_root = tempfile::tempdir().expect("the work root is created");
    let mut reporter = TestReporter::default();
    let request = HostRunRequest {
        line: LINE,
        command,
        workdir,
        env,
        timeout,
    };
    let result = run_host(tree, &request, work_root.path(), &mut reporter);
    (result, reporter)
}

fn assert_instruction_error(result: Result<LayerDelta, BuildError>, needles: &[&str]) {
    match result {
        Ok(delta) => panic!("expected an error, got {delta:?}"),
        Err(BuildError::Instruction {
            line,
            instruction,
            reason,
        }) => {
            assert_eq!(line, LINE);
            assert_eq!(instruction, "RUN --host");
            for needle in needles {
                assert!(
                    reason.contains(needle),
                    "{reason:?} does not name {needle:?}"
                );
            }
        }
        Err(other) => panic!("expected a RUN --host instruction error, got {other:?}"),
    }
}

fn file_bytes(delta: &LayerDelta) -> BTreeMap<String, Vec<u8>> {
    delta
        .files
        .iter()
        .map(|file| (file.path.as_str().to_owned(), file.bytes.clone()))
        .collect()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_owned()
}

#[test]
fn run_host_adds_a_file() {
    let command = if cfg!(windows) {
        "echo hello> NEW.TXT"
    } else {
        "printf hello > NEW.TXT"
    };
    let (result, _) = run(&Tree::new(), command);
    let delta = result.expect("the command adds a file");

    assert_eq!(delta.whiteouts, Vec::new());
    assert_eq!(delta.dirs, Vec::new());
    assert_eq!(delta.files.len(), 1);
    assert_eq!(delta.files[0].path, dos(r"C:\NEW.TXT"));
    assert_eq!(text(&delta.files[0].bytes), "hello");
    assert_eq!(
        delta.files[0].attributes,
        Attributes {
            archive: true,
            ..Attributes::default()
        }
    );
    assert_eq!(delta.files[0].timestamp, fixed_timestamp());
}

#[test]
fn run_host_replaces_a_file() {
    let tree = tree(&[], &[(r"C:\OLD.TXT", b"old")]);
    let command = if cfg!(windows) {
        "echo new> OLD.TXT"
    } else {
        "printf new > OLD.TXT"
    };
    let (result, _) = run(&tree, command);
    let delta = result.expect("the command replaces the file");

    assert!(delta.whiteouts.is_empty());
    assert_eq!(delta.files.len(), 1);
    assert_eq!(delta.files[0].path, dos(r"C:\OLD.TXT"));
    assert_eq!(text(&delta.files[0].bytes), "new");
    assert_eq!(delta.files[0].timestamp, fixed_timestamp());
}

#[test]
fn run_host_deletes_a_file_with_a_whiteout() {
    let tree = tree(&[], &[(r"C:\DELETE.TXT", b"delete")]);
    let command = if cfg!(windows) {
        "del DELETE.TXT"
    } else {
        "rm DELETE.TXT"
    };
    let (result, _) = run(&tree, command);
    let delta = result.expect("the command deletes the file");

    assert_eq!(delta.whiteouts, vec![dos(r"C:\DELETE.TXT")]);
    assert!(delta.files.is_empty());
    assert!(delta.dirs.is_empty());
}

#[test]
fn run_host_deletes_a_directory_with_a_whiteout() {
    let tree = tree(
        &[r"C:\DATA", r"C:\DATA\SUB"],
        &[
            (r"C:\DATA\ONE.TXT", b"one"),
            (r"C:\DATA\SUB\TWO.TXT", b"two"),
        ],
    );
    let command = if cfg!(windows) {
        "rmdir /S /Q DATA"
    } else {
        "rm -rf DATA"
    };
    let (result, _) = run(&tree, command);
    let delta = result.expect("the command deletes the directory");

    assert_eq!(delta.whiteouts, vec![dos(r"C:\DATA")]);
    assert!(delta.files.is_empty());
    assert!(delta.dirs.is_empty());
}

#[test]
fn run_host_discards_changes_under_dosker() {
    let tree = tree(
        &[r"C:\DOSKER"],
        &[
            (r"C:\DOSKER\AGENT.EXE", b"agent"),
            (r"C:\KEEP.TXT", b"keep"),
        ],
    );
    let command = if cfg!(windows) {
        "echo changed> DOSKER\\AGENT.EXE & echo added> DOSKER\\NEW.TXT & del DOSKER\\AGENT.EXE"
    } else {
        "printf changed > DOSKER/AGENT.EXE; printf added > DOSKER/NEW.TXT; rm DOSKER/AGENT.EXE"
    };
    let (result, _) = run(&tree, command);
    let delta = result.expect("changes under DOSKER are ignored");

    assert!(delta.whiteouts.is_empty());
    assert!(delta.files.is_empty());
    assert!(delta.dirs.is_empty());
}

#[test]
fn run_host_provides_root_workdir_and_env() {
    let tree = tree(&[], &[(r"C:\HELLO.TXT", b"staged")]);
    let workdir = dos(r"C:\PROJECT");
    let env = BTreeMap::from([("GREETING".to_owned(), "hello world".to_owned())]);
    let command = if cfg!(windows) {
        "pushd \"%DOSKER_ROOT%\" && type HELLO.TXT > ROOT.TXT & popd & echo %DOSKER_WORKDIR%> WORKDIR.TXT & echo %DOSKER_ENV_GREETING%> ENV.TXT"
    } else {
        "cat \"$DOSKER_ROOT/HELLO.TXT\" > ROOT.TXT; printf '%s' \"$DOSKER_WORKDIR\" > WORKDIR.TXT; printf '%s' \"$DOSKER_ENV_GREETING\" > ENV.TXT"
    };
    let (result, _) = run_in(&tree, command, &workdir, &env, Duration::from_secs(30));
    let delta = result.expect("the environment variables are provided");
    let files = file_bytes(&delta);

    assert_eq!(
        text(files.get("ROOT.TXT").expect("ROOT.TXT exists")),
        "staged"
    );
    assert_eq!(
        text(files.get("WORKDIR.TXT").expect("WORKDIR.TXT exists")),
        r"C:\PROJECT"
    );
    assert_eq!(
        text(files.get("ENV.TXT").expect("ENV.TXT exists")),
        "hello world"
    );
}

#[test]
fn run_host_uses_the_staged_tree_as_the_working_directory() {
    let tree = tree(&[], &[(r"C:\HELLO.TXT", b"hello from tree")]);
    let command = if cfg!(windows) {
        "type HELLO.TXT"
    } else {
        "cat HELLO.TXT"
    };
    let (result, reporter) = run(&tree, command);
    let delta = result.expect("the command reads the staged tree");

    assert!(delta.whiteouts.is_empty());
    assert!(delta.files.is_empty());
    assert!(delta.dirs.is_empty());
    assert!(
        String::from_utf8_lossy(&reporter.output).contains("hello from tree"),
        "the reporter did not receive the command output"
    );
}

#[cfg(unix)]
#[test]
fn run_host_rejects_a_symlink() {
    let (result, _) = run(&Tree::new(), "ln -s target LINK");
    assert_instruction_error(result, &["LINK"]);
}

#[test]
fn run_host_rejects_an_invalid_8_3_name() {
    let command = if cfg!(windows) {
        "echo x> LONGFILENAME.TXT"
    } else {
        "printf x > LONGFILENAME.TXT"
    };
    let (result, _) = run(&Tree::new(), command);
    assert_instruction_error(result, &["LONGFILENAME.TXT"]);
}

#[test]
fn run_host_fails_on_a_nonzero_exit() {
    let command = if cfg!(windows) {
        "echo failure 1>&2 & exit /b 3"
    } else {
        "printf failure >&2; exit 3"
    };
    let (result, _) = run(&Tree::new(), command);
    assert_instruction_error(result, &["3", "failure"]);
}

#[test]
fn run_host_times_out() {
    let command = if cfg!(windows) {
        "ping -n 5 127.0.0.1 > NUL"
    } else {
        "sleep 5"
    };
    let started = Instant::now();
    let (result, _) = run_in(
        &Tree::new(),
        command,
        &root(),
        &BTreeMap::new(),
        Duration::from_millis(500),
    );

    assert_instruction_error(result, &["timed out"]);
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "the command was not terminated by the timeout"
    );
}

#[test]
fn run_host_streams_output_to_the_reporter() {
    let command = if cfg!(windows) {
        "echo out & echo err 1>&2"
    } else {
        "printf out; printf err >&2"
    };
    let (result, reporter) = run(&Tree::new(), command);
    result.expect("the command writes output");

    let output = String::from_utf8_lossy(&reporter.output);
    assert!(output.contains("out"), "{output:?} does not contain stdout");
    assert!(output.contains("err"), "{output:?} does not contain stderr");
}

#[test]
fn run_host_returns_no_change_for_an_untouched_tree() {
    let tree = tree(
        &[r"C:\APP", r"C:\APP\DATA"],
        &[(r"C:\APP\ONE.TXT", b"one"), (r"C:\TOP.TXT", b"top")],
    );
    let command = if cfg!(windows) { "exit /b 0" } else { "true" };
    let (result, _) = run(&tree, command);
    let delta = result.expect("the command leaves the tree untouched");

    assert!(delta.whiteouts.is_empty());
    assert!(delta.files.is_empty());
    assert!(delta.dirs.is_empty());
    assert!(matches!(
        tree.get(&dos(r"C:\APP\ONE.TXT")),
        Some(TreeNode::File(_))
    ));
}
