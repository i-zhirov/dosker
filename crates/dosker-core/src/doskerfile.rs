//! Doskerfile scanning and instruction grammar.

use std::collections::{BTreeMap, BTreeSet};

use crate::license::{LicenseBasis, LicenseEntry, LicenseStatus};

/// The conventional file name of a Doskerfile.
pub const DOSKERFILE_NAME: &str = "Doskerfile";

/// A virtual machine engine selectable from a Doskerfile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineName {
    Qemu,
    DosboxX,
    Bochs,
}

impl EngineName {
    /// Parses an engine name case-insensitively.
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "qemu" => Some(Self::Qemu),
            "dosbox-x" => Some(Self::DosboxX),
            "bochs" => Some(Self::Bochs),
            _ => None,
        }
    }

    /// Returns the canonical engine name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Qemu => "qemu",
            Self::DosboxX => "dosbox-x",
            Self::Bochs => "bochs",
        }
    }
}

/// The base image selected by `FROM`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromSource {
    /// A built-in FreeDOS base version.
    Freedos { version: String },
    /// A local image tag or digest reference.
    Image(String),
    /// A local disk image path.
    LocalPath(String),
    /// An opaque booter image path.
    BooterPath(String),
}

impl FromSource {
    /// Returns the text that denotes the source in a Doskerfile.
    pub fn text(&self) -> String {
        match self {
            Self::Freedos { version } => format!("freedos:{version}"),
            Self::Image(value) | Self::LocalPath(value) => value.clone(),
            Self::BooterPath(path) => format!("booter:{path}"),
        }
    }
}

/// One parsed Doskerfile instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instruction {
    /// The first physical line of the instruction.
    pub line: usize,
    /// The parsed instruction.
    pub kind: InstructionKind,
}

/// The parsed form of a Doskerfile instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstructionKind {
    From(FromSource),
    Copy {
        sources: Vec<String>,
        destination: String,
    },
    Run {
        host: bool,
        command: Vec<String>,
        /// The expanded argument text after the optional `--host` flag, before
        /// tokenization. It keeps the user's quotes and backslashes.
        raw: String,
    },
    Env {
        key: String,
        value: String,
    },
    Workdir {
        path: String,
    },
    Cmd {
        command: Vec<String>,
        /// The expanded argument text before tokenization. It keeps the user's
        /// quotes and backslashes.
        raw: String,
    },
    Machine {
        settings: BTreeMap<String, String>,
    },
    Engine {
        name: EngineName,
    },
    EngineConfig {
        engine: EngineName,
        raw: String,
    },
    Requires {
        capabilities: Vec<String>,
    },
    Expose {
        port: u16,
        protocol: String,
    },
    Volume {
        drive: String,
    },
    License(LicenseEntry),
}

impl InstructionKind {
    /// Returns the uppercase instruction name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::From(_) => "FROM",
            Self::Copy { .. } => "COPY",
            Self::Run { .. } => "RUN",
            Self::Env { .. } => "ENV",
            Self::Workdir { .. } => "WORKDIR",
            Self::Cmd { .. } => "CMD",
            Self::Machine { .. } => "MACHINE",
            Self::Engine { .. } => "ENGINE",
            Self::EngineConfig { .. } => "ENGINE-CONFIG",
            Self::Requires { .. } => "REQUIRES",
            Self::Expose { .. } => "EXPOSE",
            Self::Volume { .. } => "VOLUME",
            Self::License(_) => "LICENSE",
        }
    }
}

/// A parsed Doskerfile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doskerfile {
    pub instructions: Vec<Instruction>,
}

impl Doskerfile {
    /// Parses a Doskerfile from bytes.
    pub fn parse_bytes(bytes: &[u8]) -> Result<Self, DoskerfileError> {
        let text = std::str::from_utf8(bytes).map_err(|error| DoskerfileError::Utf8 {
            offset: error.valid_up_to(),
        })?;
        Self::parse_str(text)
    }

    /// Parses a Doskerfile from text that is already valid UTF-8.
    pub fn parse_str(text: &str) -> Result<Self, DoskerfileError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        Parser::default().parse(text)
    }
}

/// An error raised while parsing a Doskerfile.
#[derive(Debug, thiserror::Error)]
pub enum DoskerfileError {
    #[error("Doskerfile line {line}: {reason}")]
    Line { line: usize, reason: String },
    #[error("Doskerfile is empty")]
    Empty,
    #[error("Doskerfile is not valid UTF-8 at byte {offset}")]
    Utf8 { offset: usize },
}

fn line_error(line: usize, reason: impl Into<String>) -> DoskerfileError {
    DoskerfileError::Line {
        line,
        reason: reason.into(),
    }
}

const MACHINE_KEYS: [&str; 9] = [
    "cpu",
    "memory",
    "video",
    "sound",
    "net",
    "serial",
    "control_port",
    "disk",
    "baud",
];

const LICENSE_KEYS: [&str; 10] = [
    "NAME",
    "SPDX",
    "STATUS",
    "HOLDER",
    "BASIS",
    "EVIDENCE",
    "LICENSE_URL",
    "COVERS",
    "SOURCE_URL",
    "NOTICE",
];

/// Capabilities sort in the adapter declaration order so that a normalized
/// `REQUIRES` list stays stable across engines.
const CAPABILITY_ORDER: [&str; 11] = [
    "boot",
    "serial",
    "stop",
    "headless",
    "host_dir_live",
    "debug_native",
    "gdbstub",
    "screenshot",
    "vnc",
    "nat",
    "audio",
];

#[derive(Default)]
struct Parser {
    env: BTreeMap<String, String>,
    license_names: BTreeSet<String>,
}

struct LogicalLine {
    line: usize,
    text: String,
}

impl Parser {
    fn parse(mut self, text: &str) -> Result<Doskerfile, DoskerfileError> {
        let lines = logical_lines(text)?;
        if lines.is_empty() {
            return Err(DoskerfileError::Empty);
        }

        let mut instructions = Vec::with_capacity(lines.len());
        for line in lines {
            let instruction = self.parse_instruction(&line)?;
            if instructions.is_empty() {
                if !matches!(&instruction.kind, InstructionKind::From(_)) {
                    return Err(line_error(line.line, "the first instruction must be FROM"));
                }
            } else if matches!(&instruction.kind, InstructionKind::From(_)) {
                return Err(line_error(line.line, "a Doskerfile has exactly one FROM"));
            }
            instructions.push(instruction);
        }

        Ok(Doskerfile { instructions })
    }

    fn parse_instruction(&mut self, line: &LogicalLine) -> Result<Instruction, DoskerfileError> {
        let (name, raw_args) = split_name(&line.text);
        let kind = match name.to_ascii_uppercase().as_str() {
            "FROM" => self.parse_from(raw_args, line.line)?,
            "COPY" => self.parse_copy(raw_args, line.line)?,
            "RUN" => self.parse_run(raw_args, line.line)?,
            "ENV" => self.parse_env(raw_args, line.line)?,
            "WORKDIR" => self.parse_workdir(raw_args, line.line)?,
            "CMD" => self.parse_cmd(raw_args, line.line)?,
            "MACHINE" => self.parse_machine(raw_args, line.line)?,
            "ENGINE" => self.parse_engine(raw_args, line.line)?,
            "ENGINE-CONFIG" => self.parse_engine_config(raw_args, line.line)?,
            "REQUIRES" => self.parse_requires(raw_args, line.line)?,
            "EXPOSE" => self.parse_expose(raw_args, line.line)?,
            "VOLUME" => self.parse_volume(raw_args, line.line)?,
            "LICENSE" => self.parse_license(raw_args, line.line)?,
            _ => {
                return Err(line_error(line.line, format!("unknown instruction {name}")));
            }
        };

        Ok(Instruction {
            line: line.line,
            kind,
        })
    }

    fn parse_from(&self, raw_args: &str, line: usize) -> Result<InstructionKind, DoskerfileError> {
        let tokens = tokenize(raw_args, line)?;
        if tokens.len() != 1 {
            return Err(line_error(line, "FROM requires exactly one source"));
        }
        Ok(InstructionKind::From(classify_from(&tokens[0], line)?))
    }

    fn parse_copy(
        &mut self,
        raw_args: &str,
        line: usize,
    ) -> Result<InstructionKind, DoskerfileError> {
        let expanded = self.expand(raw_args, line)?;
        let mut tokens = tokenize(&expanded, line)?;
        if tokens.len() < 2 {
            return Err(line_error(
                line,
                "COPY requires at least one source and a destination",
            ));
        }
        let destination = tokens.pop().expect("COPY has at least two tokens");
        Ok(InstructionKind::Copy {
            sources: tokens,
            destination,
        })
    }

    fn parse_run(
        &mut self,
        raw_args: &str,
        line: usize,
    ) -> Result<InstructionKind, DoskerfileError> {
        let expanded = self.expand(raw_args, line)?;
        let tokens = tokenize_spans(&expanded, line)?;
        let host = tokens.first().is_some_and(|token| token.value == "--host");
        let raw = if host {
            let end = tokens.first().expect("the host flag token exists").end;
            expanded[end..].trim_start().to_owned()
        } else {
            expanded.clone()
        };
        let mut command: Vec<String> = tokens.into_iter().map(|token| token.value).collect();
        if host {
            command.remove(0);
        }
        if command.is_empty() {
            return Err(line_error(line, "RUN requires a command"));
        }
        Ok(InstructionKind::Run { host, command, raw })
    }

    fn parse_env(
        &mut self,
        raw_args: &str,
        line: usize,
    ) -> Result<InstructionKind, DoskerfileError> {
        let expanded = self.expand(raw_args, line)?;
        let (key, value) = expanded
            .split_once('=')
            .ok_or_else(|| line_error(line, "ENV requires a KEY=VALUE argument"))?;
        let key = key.trim();
        let valid_key = key.chars().next().is_some_and(is_identifier_start)
            && key.chars().all(is_identifier_char);
        if !valid_key {
            return Err(line_error(line, format!("invalid ENV key {key}")));
        }

        let value = value.trim_start().to_owned();
        self.env.insert(key.to_owned(), value.clone());
        Ok(InstructionKind::Env {
            key: key.to_owned(),
            value,
        })
    }

    fn parse_workdir(
        &self,
        raw_args: &str,
        line: usize,
    ) -> Result<InstructionKind, DoskerfileError> {
        let expanded = self.expand(raw_args, line)?;
        let mut tokens = tokenize(&expanded, line)?;
        if tokens.len() != 1 {
            return Err(line_error(line, "WORKDIR requires exactly one path"));
        }
        Ok(InstructionKind::Workdir {
            path: tokens.remove(0),
        })
    }

    fn parse_cmd(&self, raw_args: &str, line: usize) -> Result<InstructionKind, DoskerfileError> {
        let expanded = self.expand(raw_args, line)?;
        let command = tokenize(&expanded, line)?;
        if command.is_empty() {
            return Err(line_error(line, "CMD requires a command"));
        }
        Ok(InstructionKind::Cmd {
            command,
            raw: expanded,
        })
    }

    fn parse_machine(
        &self,
        raw_args: &str,
        line: usize,
    ) -> Result<InstructionKind, DoskerfileError> {
        let expanded = self.expand(raw_args, line)?;
        let tokens = tokenize(&expanded, line)?;
        if tokens.is_empty() {
            return Err(line_error(line, "MACHINE requires at least one setting"));
        }

        let mut settings = BTreeMap::new();
        for token in tokens {
            let (key, value) = token.split_once('=').ok_or_else(|| {
                line_error(line, format!("MACHINE setting {token} must be key=value"))
            })?;
            if !MACHINE_KEYS.contains(&key) {
                return Err(line_error(line, format!("unknown MACHINE key {key}")));
            }
            settings.insert(key.to_owned(), value.to_owned());
        }

        Ok(InstructionKind::Machine { settings })
    }

    fn parse_engine(
        &self,
        raw_args: &str,
        line: usize,
    ) -> Result<InstructionKind, DoskerfileError> {
        let expanded = self.expand(raw_args, line)?;
        let tokens = tokenize(&expanded, line)?;
        if tokens.len() != 1 {
            return Err(line_error(line, "ENGINE requires exactly one engine name"));
        }
        let name = EngineName::parse(&tokens[0])
            .ok_or_else(|| line_error(line, format!("unknown engine {}", tokens[0])))?;
        Ok(InstructionKind::Engine { name })
    }

    fn parse_engine_config(
        &self,
        raw_args: &str,
        line: usize,
    ) -> Result<InstructionKind, DoskerfileError> {
        let expanded = self.expand(raw_args, line)?;
        let (engine, raw) = split_name(&expanded);
        if raw.is_empty() {
            return Err(line_error(line, "ENGINE-CONFIG requires raw engine text"));
        }
        let engine = EngineName::parse(engine)
            .ok_or_else(|| line_error(line, format!("unknown engine {engine}")))?;
        Ok(InstructionKind::EngineConfig {
            engine,
            raw: raw.to_owned(),
        })
    }

    fn parse_requires(
        &self,
        raw_args: &str,
        line: usize,
    ) -> Result<InstructionKind, DoskerfileError> {
        let expanded = self.expand(raw_args, line)?;
        let mut capabilities = Vec::new();
        for part in expanded.split(',') {
            let capability = part.trim();
            if capability.is_empty() {
                return Err(line_error(line, "REQUIRES requires a capability"));
            }
            capabilities.push(capability.to_owned());
        }

        capabilities.sort_by(|left, right| {
            capability_rank(left)
                .cmp(&capability_rank(right))
                .then_with(|| left.cmp(right))
        });
        capabilities.dedup();
        Ok(InstructionKind::Requires { capabilities })
    }

    fn parse_expose(
        &self,
        raw_args: &str,
        line: usize,
    ) -> Result<InstructionKind, DoskerfileError> {
        let expanded = self.expand(raw_args, line)?;
        let mut tokens = tokenize(&expanded, line)?;
        if tokens.len() != 1 {
            return Err(line_error(line, "EXPOSE requires exactly one port"));
        }
        let token = tokens.remove(0);
        let (port, protocol) = match token.split_once('/') {
            Some((port, protocol)) => (port, protocol),
            None => (token.as_str(), "tcp"),
        };
        if !protocol.eq_ignore_ascii_case("tcp") {
            return Err(line_error(
                line,
                format!("unsupported EXPOSE protocol {protocol}"),
            ));
        }
        let port = port
            .parse::<u16>()
            .map_err(|_| line_error(line, format!("invalid EXPOSE port {port}")))?;
        if port == 0 {
            return Err(line_error(line, "EXPOSE port must be between 1 and 65535"));
        }
        Ok(InstructionKind::Expose {
            port,
            protocol: "tcp".to_owned(),
        })
    }

    fn parse_volume(
        &self,
        raw_args: &str,
        line: usize,
    ) -> Result<InstructionKind, DoskerfileError> {
        let expanded = self.expand(raw_args, line)?;
        let mut tokens = tokenize(&expanded, line)?;
        if tokens.len() != 1 {
            return Err(line_error(line, "VOLUME requires exactly one drive"));
        }

        let drive = tokens.remove(0).to_ascii_uppercase();
        let bytes = drive.as_bytes();
        if bytes.len() != 2 || !bytes[0].is_ascii_uppercase() || bytes[1] != b':' {
            return Err(line_error(line, format!("invalid VOLUME drive {drive}")));
        }
        if drive == "C:" {
            return Err(line_error(line, "C: is reserved for the boot disk"));
        }
        Ok(InstructionKind::Volume { drive })
    }

    fn parse_license(
        &mut self,
        raw_args: &str,
        line: usize,
    ) -> Result<InstructionKind, DoskerfileError> {
        let expanded = self.expand(raw_args, line)?;
        let tokens = tokenize(&expanded, line)?;
        let mut values: BTreeMap<String, &str> = BTreeMap::new();
        for token in &tokens {
            let (key, value) = token.split_once('=').ok_or_else(|| {
                line_error(line, format!("LICENSE field {token} must be key=value"))
            })?;
            if !LICENSE_KEYS.contains(&key) {
                return Err(line_error(line, format!("unknown LICENSE key {key}")));
            }
            if values.insert(key.to_owned(), value).is_some() {
                return Err(line_error(line, format!("duplicate LICENSE key {key}")));
            }
        }

        let name = values
            .remove("NAME")
            .ok_or_else(|| line_error(line, "LICENSE requires NAME"))?
            .to_owned();
        let spdx = values
            .remove("SPDX")
            .ok_or_else(|| line_error(line, "LICENSE requires SPDX"))?
            .to_owned();
        let status_value = values
            .remove("STATUS")
            .ok_or_else(|| line_error(line, "LICENSE requires STATUS"))?;
        let status = LicenseStatus::parse(status_value)
            .ok_or_else(|| line_error(line, format!("unknown LICENSE STATUS {status_value}")))?;
        let basis = match values.remove("BASIS") {
            Some(value) => Some(
                LicenseBasis::parse(value)
                    .ok_or_else(|| line_error(line, format!("unknown LICENSE BASIS {value}")))?,
            ),
            None => None,
        };
        let covers = match values.remove("COVERS") {
            Some(value) => split_covers(value, line)?,
            None => Vec::new(),
        };

        let entry = LicenseEntry {
            name: name.clone(),
            spdx,
            holder: values.remove("HOLDER").map(str::to_owned),
            status,
            basis,
            evidence: values.remove("EVIDENCE").map(str::to_owned),
            license_url: values.remove("LICENSE_URL").map(str::to_owned),
            covers,
            source_url: values.remove("SOURCE_URL").map(str::to_owned),
            notice: values.remove("NOTICE").map(str::to_owned),
        };
        entry
            .validate()
            .map_err(|error| line_error(line, error.to_string()))?;
        if !self.license_names.insert(name.clone()) {
            return Err(line_error(line, format!("duplicate LICENSE name {name}")));
        }

        Ok(InstructionKind::License(entry))
    }

    fn expand(&self, raw: &str, line: usize) -> Result<String, DoskerfileError> {
        let mut output = String::with_capacity(raw.len());
        let mut chars = raw.chars().peekable();

        while let Some(ch) = chars.next() {
            match ch {
                '\\' if chars.peek() == Some(&'$') => {
                    chars.next();
                    output.push('$');
                }
                '\\' => output.push('\\'),
                '$' => {
                    let placeholder = read_placeholder(&mut chars, line)?;
                    let value = self.env.get(&placeholder.name).ok_or_else(|| {
                        line_error(
                            line,
                            format!("undefined placeholder ${{{}}}", placeholder.name),
                        )
                    })?;
                    output.push_str(value);
                }
                _ => output.push(ch),
            }
        }

        Ok(output)
    }
}

fn split_name(text: &str) -> (&str, &str) {
    match text.find(char::is_whitespace) {
        Some(index) => (&text[..index], text[index..].trim_start()),
        None => (text, ""),
    }
}

fn split_covers(value: &str, line: usize) -> Result<Vec<String>, DoskerfileError> {
    let mut covers = Vec::new();
    for item in value.split(',') {
        let item = item.trim();
        if item.is_empty() {
            return Err(line_error(line, "COVERS must not contain an empty glob"));
        }
        covers.push(item.to_owned());
    }
    Ok(covers)
}

fn classify_from(value: &str, line: usize) -> Result<FromSource, DoskerfileError> {
    if let Some(version) = value.strip_prefix("freedos:") {
        if version.is_empty() {
            return Err(line_error(line, "freedos requires a version"));
        }
        return Ok(FromSource::Freedos {
            version: version.to_owned(),
        });
    }

    if let Some(path) = value.strip_prefix("booter:") {
        if path.is_empty() {
            return Err(line_error(line, "booter requires a path"));
        }
        return Ok(FromSource::BooterPath(path.to_owned()));
    }

    if let Some((name, digest)) = value.split_once('@') {
        if name.is_empty() || !is_sha256_digest(digest) {
            return Err(line_error(line, format!("invalid image reference {value}")));
        }
        return Ok(FromSource::Image(value.to_owned()));
    }

    if is_path_like(value) || value.ends_with(".img") {
        if value.ends_with(".img") && value.len() > ".img".len() {
            return Ok(FromSource::LocalPath(value.to_owned()));
        }
        return Err(line_error(
            line,
            format!("local path {value} must end with .img"),
        ));
    }

    if value.is_empty() {
        return Err(line_error(line, "FROM requires an image reference"));
    }

    Ok(FromSource::Image(value.to_owned()))
}

fn is_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn is_path_like(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.starts_with('/')
        || value.starts_with('\\')
        || value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with(".\\")
        || value.starts_with("..\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/'))
}

struct Placeholder {
    name: String,
}

fn read_placeholder(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    line: usize,
) -> Result<Placeholder, DoskerfileError> {
    match chars.peek().copied() {
        Some('{') => {
            chars.next();
            let mut name = String::new();
            loop {
                match chars.next() {
                    Some('}') => break,
                    Some(ch) if is_identifier_char(ch) => name.push(ch),
                    Some(_) => return Err(line_error(line, "invalid placeholder name")),
                    None => return Err(line_error(line, "unterminated placeholder")),
                }
            }
            if name.is_empty() || !name.chars().next().is_some_and(is_identifier_start) {
                return Err(line_error(line, "invalid placeholder name"));
            }
            Ok(Placeholder { name })
        }
        Some(ch) if is_identifier_start(ch) => {
            let mut name = String::new();
            name.push(ch);
            chars.next();
            while let Some(&next) = chars.peek() {
                if is_identifier_char(next) {
                    name.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            Ok(Placeholder { name })
        }
        _ => Err(line_error(line, "invalid placeholder")),
    }
}

fn is_identifier_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn capability_rank(name: &str) -> usize {
    CAPABILITY_ORDER
        .iter()
        .position(|known| *known == name)
        .unwrap_or(CAPABILITY_ORDER.len())
}

fn tokenize(text: &str, line: usize) -> Result<Vec<String>, DoskerfileError> {
    Ok(tokenize_spans(text, line)?
        .into_iter()
        .map(|token| token.value)
        .collect())
}

struct Token {
    value: String,
    /// The byte offset just past this token in the source text.
    end: usize,
}

fn tokenize_spans(text: &str, line: usize) -> Result<Vec<Token>, DoskerfileError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut in_quotes = false;
    let mut offset = 0;
    let mut token_end = 0;
    let mut chars = text.chars();

    while let Some(ch) = chars.next() {
        offset += ch.len_utf8();
        if in_quotes {
            match ch {
                '"' => in_quotes = false,
                '\\' => match chars.next() {
                    Some('"') => {
                        current.push('"');
                        offset += '"'.len_utf8();
                    }
                    Some('\\') => {
                        current.push('\\');
                        offset += '\\'.len_utf8();
                    }
                    Some(other) => {
                        return Err(line_error(line, format!("invalid quoted escape \\{other}")));
                    }
                    None => return Err(line_error(line, "unterminated quoted escape")),
                },
                _ => current.push(ch),
            }
            token_end = offset;
            continue;
        }

        match ch {
            '"' => {
                in_quotes = true;
                in_token = true;
                token_end = offset;
            }
            ch if ch.is_whitespace() => {
                if in_token {
                    tokens.push(Token {
                        value: std::mem::take(&mut current),
                        end: token_end,
                    });
                    in_token = false;
                }
            }
            _ => {
                current.push(ch);
                in_token = true;
                token_end = offset;
            }
        }
    }

    if in_quotes {
        return Err(line_error(line, "unterminated quoted string"));
    }
    if in_token {
        tokens.push(Token {
            value: current,
            end: token_end,
        });
    }

    Ok(tokens)
}

fn logical_lines(text: &str) -> Result<Vec<LogicalLine>, DoskerfileError> {
    let physical = normalize_lines(text)?;
    let mut logical = Vec::new();
    let mut pending: Option<(usize, String)> = None;

    for (index, raw) in physical.iter().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        match pending.take() {
            Some((start, mut accumulated)) => {
                if let Some(content) = trimmed.strip_suffix('\\') {
                    accumulated.push_str(content);
                    pending = Some((start, accumulated));
                } else {
                    accumulated.push_str(trimmed);
                    logical.push(LogicalLine {
                        line: start,
                        text: accumulated,
                    });
                }
            }
            None => {
                if let Some(content) = trimmed.strip_suffix('\\') {
                    pending = Some((line, content.to_owned()));
                } else {
                    logical.push(LogicalLine {
                        line,
                        text: trimmed.to_owned(),
                    });
                }
            }
        }
    }

    if let Some((line, _)) = pending {
        return Err(line_error(line, "unterminated line continuation"));
    }

    Ok(logical)
}

fn normalize_lines(text: &str) -> Result<Vec<&str>, DoskerfileError> {
    let segments: Vec<&str> = text.split('\n').collect();
    let last = segments.len() - 1;
    let mut lines = Vec::with_capacity(segments.len());

    for (index, segment) in segments.into_iter().enumerate() {
        let line = if index == last {
            segment
        } else {
            segment.strip_suffix('\r').unwrap_or(segment)
        };
        if line.contains('\r') {
            return Err(line_error(
                index + 1,
                "a carriage return must be followed by a line feed",
            ));
        }
        lines.push(line);
    }

    Ok(lines)
}
