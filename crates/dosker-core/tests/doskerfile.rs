use std::collections::BTreeMap;

use dosker_core::{Doskerfile, DoskerfileError, EngineName, FromSource, InstructionKind};

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn parse(text: &str) -> Doskerfile {
    Doskerfile::parse_str(text).expect("the Doskerfile parses")
}

fn line_of(error: &DoskerfileError) -> usize {
    match error {
        DoskerfileError::Line { line, .. } => *line,
        other => panic!("expected a line error, got {other}"),
    }
}

fn last_run(text: &str) -> (bool, Vec<String>) {
    let file = parse(text);
    match &file.instructions.last().expect("an instruction").kind {
        InstructionKind::Run { host, command, .. } => (*host, command.clone()),
        other => panic!("expected RUN, got {}", other.name()),
    }
}

fn only_from(text: &str) -> FromSource {
    let file = parse(text);
    match &file.instructions[0].kind {
        InstructionKind::From(source) => source.clone(),
        other => panic!("expected FROM, got {}", other.name()),
    }
}

#[test]
fn parse_ignores_comments_and_blank_lines() {
    let file = parse("# a comment\n\nFROM demo:latest\n   # another comment\n\n");

    assert_eq!(file.instructions.len(), 1);
    assert_eq!(
        only_from("# a comment\n\nFROM demo:latest\n"),
        FromSource::Image("demo:latest".to_owned())
    );
}

#[test]
fn parse_normalizes_crlf() {
    let lf = parse("FROM demo:latest\nRUN ECHO hello\n");
    let crlf = parse("FROM demo:latest\r\nRUN ECHO hello\r\n");

    assert_eq!(lf.instructions, crlf.instructions);
}

#[test]
fn parse_strips_a_utf8_bom() {
    let file = Doskerfile::parse_bytes(b"\xEF\xBB\xBFFROM demo:latest\n").unwrap();

    assert_eq!(file.instructions.len(), 1);
    assert_eq!(
        only_from("\u{feff}FROM demo:latest\n"),
        FromSource::Image("demo:latest".to_owned())
    );
}

#[test]
fn parse_joins_a_continued_line() {
    let (host, command) = last_run("FROM demo:latest\nRUN ECHO one \\\n  two\n");

    assert!(!host);
    assert_eq!(command, vec!["ECHO", "one", "two"]);
}

#[test]
fn parse_reports_the_first_line_of_a_continued_instruction() {
    let error = Doskerfile::parse_str("FROM demo:latest\nRUN ECHO \"a \\\nb\n").unwrap_err();

    assert_eq!(line_of(&error), 2);
}

#[test]
fn parse_groups_a_quoted_token() {
    let (_, command) = last_run("FROM demo:latest\nRUN ECHO \"a b\"\n");

    assert_eq!(command, vec!["ECHO", "a b"]);
}

#[test]
fn parse_concatenates_quoted_and_unquoted_parts() {
    let (_, command) = last_run("FROM demo:latest\nRUN ECHO a\"b c\"d\n");

    assert_eq!(command, vec!["ECHO", "ab cd"]);
}

#[test]
fn parse_handles_escaped_quote_and_backslash() {
    let (_, command) = last_run("FROM demo:latest\nRUN ECHO \"a\\\"b\" \"c\\\\d\"\n");

    assert_eq!(command, vec!["ECHO", "a\"b", "c\\d"]);
}

#[test]
fn parse_rejects_an_unterminated_quote() {
    let error = Doskerfile::parse_str("FROM demo:latest\nRUN ECHO \"a b\n").unwrap_err();

    assert_eq!(line_of(&error), 2);
    assert!(error.to_string().contains("line 2"));
}

#[test]
fn parse_rejects_an_invalid_quote_escape() {
    let error = Doskerfile::parse_str("FROM demo:latest\nRUN ECHO \"a\\nb\"\n").unwrap_err();

    assert_eq!(line_of(&error), 2);
}

#[test]
fn parse_keeps_a_literal_dollar() {
    let (_, command) = last_run("FROM demo:latest\nRUN ECHO \\$NAME\n");

    assert_eq!(command, vec!["ECHO", "$NAME"]);
}

#[test]
fn parse_rejects_an_undefined_placeholder() {
    let error = Doskerfile::parse_str("FROM demo:latest\nRUN ECHO $MISSING\n").unwrap_err();

    assert_eq!(line_of(&error), 2);
    assert!(error.to_string().contains("MISSING"), "{error}");
}

#[test]
fn from_must_be_the_first_instruction() {
    let error = Doskerfile::parse_str("COPY a b\n").unwrap_err();

    assert_eq!(line_of(&error), 1);
}

#[test]
fn a_second_from_is_rejected() {
    let error = Doskerfile::parse_str("FROM demo:latest\nFROM other:latest\n").unwrap_err();

    assert_eq!(line_of(&error), 2);
}

#[test]
fn from_does_not_expand_placeholders() {
    assert_eq!(
        only_from("FROM $BASE\n"),
        FromSource::Image("$BASE".to_owned())
    );
}

#[test]
fn from_classifies_a_tag_a_digest_a_path_and_a_booter() {
    assert_eq!(
        only_from("FROM demo:latest\n"),
        FromSource::Image("demo:latest".to_owned())
    );
    let digest = format!("demo@sha256:{DIGEST}");
    assert_eq!(
        only_from(&format!("FROM {digest}\n")),
        FromSource::Image(digest)
    );
    assert_eq!(
        only_from("FROM ./base.img\n"),
        FromSource::LocalPath("./base.img".to_owned())
    );
    assert_eq!(
        only_from("FROM booter:./base.bin\n"),
        FromSource::BooterPath("./base.bin".to_owned())
    );
}

#[test]
fn from_classifies_freedos() {
    assert_eq!(
        only_from("FROM freedos:1.3\n"),
        FromSource::Freedos {
            version: "1.3".to_owned()
        }
    );
}

#[test]
fn from_accepts_an_absolute_img_path() {
    assert_eq!(
        only_from("FROM /tmp/base.img\n"),
        FromSource::LocalPath("/tmp/base.img".to_owned())
    );
    assert_eq!(
        only_from("FROM C:\\base.img\n"),
        FromSource::LocalPath("C:\\base.img".to_owned())
    );
}

#[test]
fn from_rejects_a_non_img_path() {
    for text in ["FROM ./base\n", "FROM C:\\base\n"] {
        let error = Doskerfile::parse_str(text).unwrap_err();
        assert_eq!(line_of(&error), 1, "{text}");
    }
}

#[test]
fn parse_accepts_a_lowercase_instruction_name() {
    let file = parse("from demo:latest\ncopy a b\n");

    assert_eq!(
        file.instructions[0].kind,
        InstructionKind::From(FromSource::Image("demo:latest".to_owned()))
    );
    assert_eq!(
        file.instructions[1].kind,
        InstructionKind::Copy {
            sources: vec!["a".to_owned()],
            destination: "b".to_owned(),
        }
    );
}

#[test]
fn copy_requires_a_source_and_destination() {
    let error = Doskerfile::parse_str("FROM demo:latest\nCOPY only\n").unwrap_err();

    assert_eq!(line_of(&error), 2);
}

#[test]
fn run_recognizes_the_host_flag() {
    let (host, command) = last_run("FROM demo:latest\nRUN --host echo hi\n");

    assert!(host);
    assert_eq!(command, vec!["echo", "hi"]);
}

#[test]
fn parse_preserves_quotes_and_backslashes_in_the_raw_run_command() {
    let file = parse("FROM demo:latest\nRUN --host ECHO \"*.TXT\" C:\\APP\\BIN\n");

    match &file.instructions[1].kind {
        InstructionKind::Run { host, command, raw } => {
            assert!(*host);
            assert_eq!(
                command,
                &vec![
                    "ECHO".to_owned(),
                    "*.TXT".to_owned(),
                    r"C:\APP\BIN".to_owned()
                ]
            );
            assert_eq!(raw, r#"ECHO "*.TXT" C:\APP\BIN"#);
        }
        other => panic!("expected RUN, got {}", other.name()),
    }
}

#[test]
fn parse_preserves_the_raw_cmd_text() {
    let file = parse("FROM demo:latest\nCMD ECHO C:\\APP\\BIN\\APP.EXE\n");

    match &file.instructions[1].kind {
        InstructionKind::Cmd { command, raw } => {
            assert_eq!(
                command,
                &vec!["ECHO".to_owned(), r"C:\APP\BIN\APP.EXE".to_owned()]
            );
            assert_eq!(raw, r"ECHO C:\APP\BIN\APP.EXE");
        }
        other => panic!("expected CMD, got {}", other.name()),
    }
}

#[test]
fn run_requires_a_command() {
    let error = Doskerfile::parse_str("FROM demo:latest\nRUN\n").unwrap_err();

    assert_eq!(line_of(&error), 2);
}

#[test]
fn an_unknown_instruction_names_the_line() {
    let error = Doskerfile::parse_str("FROM demo:latest\nBOGUS x\n").unwrap_err();

    assert_eq!(line_of(&error), 2);
    assert!(error.to_string().contains("BOGUS"), "{error}");
}

#[test]
fn parse_rejects_invalid_utf8() {
    let error = Doskerfile::parse_bytes(b"FROM demo:latest\nBOGUS \xFF\n").unwrap_err();

    match error {
        DoskerfileError::Utf8 { offset } => assert_eq!(offset, 23),
        other => panic!("expected a UTF-8 error, got {other}"),
    }
}

#[test]
fn parse_rejects_an_empty_file() {
    assert!(matches!(
        Doskerfile::parse_str("# only a comment\n\n"),
        Err(DoskerfileError::Empty)
    ));
}

#[test]
fn parse_rejects_a_lone_carriage_return() {
    let error = Doskerfile::parse_str("FROM demo:latest\rFROM other:latest\n").unwrap_err();

    assert_eq!(line_of(&error), 1);
}

#[test]
fn parse_accepts_every_config_instruction() {
    let file = parse(
        "FROM demo:latest\n\
         ENV GREETING=hello\n\
         WORKDIR C:\\APP\n\
         CMD ECHO hello\n\
         MACHINE cpu=486dx2\n\
         ENGINE qemu\n\
         ENGINE-CONFIG qemu -m 32\n\
         REQUIRES headless,gdbstub\n\
         EXPOSE 80\n\
         VOLUME D:\n",
    );

    let names: Vec<&str> = file
        .instructions
        .iter()
        .map(|instruction| instruction.kind.name())
        .collect();
    assert_eq!(
        names,
        vec![
            "FROM",
            "ENV",
            "WORKDIR",
            "CMD",
            "MACHINE",
            "ENGINE",
            "ENGINE-CONFIG",
            "REQUIRES",
            "EXPOSE",
            "VOLUME"
        ]
    );
}

#[test]
fn parse_expands_dollar_name() {
    let (_, command) = last_run("FROM demo:latest\nENV NAME=world\nRUN ECHO $NAME\n");

    assert_eq!(command, vec!["ECHO", "world"]);
}

#[test]
fn parse_expands_braced_name() {
    let (_, command) = last_run("FROM demo:latest\nENV NAME=world\nRUN ECHO ${NAME}\n");

    assert_eq!(command, vec!["ECHO", "world"]);
}

#[test]
fn parse_accepts_a_placeholder_inside_quotes() {
    let (_, command) = last_run("FROM demo:latest\nENV NAME=hello world\nRUN ECHO \"$NAME\"\n");

    assert_eq!(command, vec!["ECHO", "hello world"]);
}

#[test]
fn unquoted_placeholder_with_space_splits_into_tokens() {
    let (_, command) = last_run("FROM demo:latest\nENV WORDS=one two\nRUN ECHO $WORDS\n");

    assert_eq!(command, vec!["ECHO", "one", "two"]);
}

#[test]
fn env_value_keeps_its_spaces() {
    let file = parse("FROM demo:latest\nENV GREETING=hello world\n");

    match &file.instructions[1].kind {
        InstructionKind::Env { key, value } => {
            assert_eq!(key, "GREETING");
            assert_eq!(value, "hello world");
        }
        other => panic!("expected ENV, got {}", other.name()),
    }
}

#[test]
fn env_key_must_match_the_identifier_pattern() {
    for text in [
        "FROM demo:latest\nENV BAD-KEY=value\n",
        "FROM demo:latest\nENV 1KEY=value\n",
    ] {
        let error = Doskerfile::parse_str(text).unwrap_err();
        assert_eq!(line_of(&error), 2, "{text}");
    }
}

#[test]
fn a_later_env_replaces_the_value() {
    let (_, command) = last_run("FROM demo:latest\nENV A=one\nENV A=two\nRUN ECHO $A\n");

    assert_eq!(command, vec!["ECHO", "two"]);
}

#[test]
fn env_expands_an_earlier_value() {
    let file = parse("FROM demo:latest\nENV A=hello\nENV B=$A world\n");

    match &file.instructions[2].kind {
        InstructionKind::Env { key, value } => {
            assert_eq!(key, "B");
            assert_eq!(value, "hello world");
        }
        other => panic!("expected ENV, got {}", other.name()),
    }
}

#[test]
fn workdir_takes_one_path() {
    let error = Doskerfile::parse_str("FROM demo:latest\nWORKDIR C:\\ONE C:\\TWO\n").unwrap_err();

    assert_eq!(line_of(&error), 2);
}

#[test]
fn cmd_takes_a_command() {
    let file = parse("FROM demo:latest\nCMD ECHO hello\n");

    match &file.instructions[1].kind {
        InstructionKind::Cmd { command, .. } => {
            assert_eq!(command, &vec!["ECHO".to_owned(), "hello".to_owned()]);
        }
        other => panic!("expected CMD, got {}", other.name()),
    }
}

#[test]
fn machine_keys_are_sorted_and_duplicates_keep_the_last() {
    let file = parse("FROM demo:latest\nMACHINE cpu=386 memory=8M cpu=486dx2\n");

    match &file.instructions[1].kind {
        InstructionKind::Machine { settings } => {
            let expected = BTreeMap::from([
                ("cpu".to_owned(), "486dx2".to_owned()),
                ("memory".to_owned(), "8M".to_owned()),
            ]);
            assert_eq!(settings, &expected);
            assert_eq!(
                settings.keys().map(String::as_str).collect::<Vec<_>>(),
                vec!["cpu", "memory"]
            );
        }
        other => panic!("expected MACHINE, got {}", other.name()),
    }
}

#[test]
fn machine_rejects_an_unknown_key() {
    let error = Doskerfile::parse_str("FROM demo:latest\nMACHINE foo=bar\n").unwrap_err();

    assert_eq!(line_of(&error), 2);
}

#[test]
fn engine_accepts_the_three_names() {
    for (text, expected) in [
        ("ENGINE qemu", EngineName::Qemu),
        ("ENGINE DOSBOX-X", EngineName::DosboxX),
        ("ENGINE Bochs", EngineName::Bochs),
    ] {
        let file = parse(&format!("FROM demo:latest\n{text}\n"));
        assert_eq!(
            file.instructions[1].kind,
            InstructionKind::Engine { name: expected }
        );
    }
}

#[test]
fn engine_rejects_an_unknown_name() {
    let error = Doskerfile::parse_str("FROM demo:latest\nENGINE virtualbox\n").unwrap_err();

    assert_eq!(line_of(&error), 2);
    assert!(error.to_string().contains("virtualbox"), "{error}");
}

#[test]
fn engine_config_keeps_the_raw_text() {
    let file =
        parse("FROM demo:latest\nENGINE-CONFIG qemu -m 32  -net \"user,hostfwd=tcp::8080-:80\"\n");

    match &file.instructions[1].kind {
        InstructionKind::EngineConfig { engine, raw } => {
            assert_eq!(*engine, EngineName::Qemu);
            assert_eq!(raw, "-m 32  -net \"user,hostfwd=tcp::8080-:80\"");
        }
        other => panic!("expected ENGINE-CONFIG, got {}", other.name()),
    }
}

#[test]
fn engine_config_requires_a_raw_text() {
    let error = Doskerfile::parse_str("FROM demo:latest\nENGINE-CONFIG qemu\n").unwrap_err();

    assert_eq!(line_of(&error), 2);
}

#[test]
fn requires_sorts_and_deduplicates() {
    let file = parse("FROM demo:latest\nREQUIRES gdbstub,headless,gdbstub\n");

    match &file.instructions[1].kind {
        InstructionKind::Requires { capabilities } => {
            assert_eq!(
                capabilities,
                &vec!["headless".to_owned(), "gdbstub".to_owned()]
            );
        }
        other => panic!("expected REQUIRES, got {}", other.name()),
    }
}

#[test]
fn expose_defaults_to_tcp() {
    let file = parse("FROM demo:latest\nEXPOSE 80\n");

    match &file.instructions[1].kind {
        InstructionKind::Expose { port, protocol } => {
            assert_eq!(*port, 80);
            assert_eq!(protocol, "tcp");
        }
        other => panic!("expected EXPOSE, got {}", other.name()),
    }
}

#[test]
fn expose_rejects_a_non_tcp_protocol() {
    let error = Doskerfile::parse_str("FROM demo:latest\nEXPOSE 80/udp\n").unwrap_err();

    assert_eq!(line_of(&error), 2);
}

#[test]
fn volume_rejects_c() {
    let error = Doskerfile::parse_str("FROM demo:latest\nVOLUME C:\n").unwrap_err();

    assert_eq!(line_of(&error), 2);
}

#[test]
fn volume_rejects_an_invalid_drive() {
    let error = Doskerfile::parse_str("FROM demo:latest\nVOLUME 1:\n").unwrap_err();

    assert_eq!(line_of(&error), 2);
}

#[test]
fn config_instructions_expand_placeholders() {
    let file = parse("FROM demo:latest\nENV DIR=C:\\APP\nWORKDIR $DIR\n");

    match &file.instructions[2].kind {
        InstructionKind::Workdir { path } => assert_eq!(path, "C:\\APP"),
        other => panic!("expected WORKDIR, got {}", other.name()),
    }
}

#[test]
fn undefined_placeholder_reports_the_line() {
    let error =
        Doskerfile::parse_str("FROM demo:latest\nENV A=one\nWORKDIR $MISSING\n").unwrap_err();

    assert_eq!(line_of(&error), 3);
    assert!(error.to_string().contains("MISSING"), "{error}");
}
