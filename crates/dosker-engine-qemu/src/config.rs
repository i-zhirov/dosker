//! QEMU engine configuration parsing.

use dosker_engine::EngineError;

/// Splits one engine configuration line into QEMU arguments.
///
/// Whitespace separates tokens. Single quotes group a token literally, and
/// double quotes group a token while a backslash escapes the next character.
/// Outside quotes a backslash escapes the next character. Adjacent segments
/// form one token.
pub fn split_engine_config(line: &str) -> Result<Vec<String>, EngineError> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut chars = line.chars();

    while let Some(character) = chars.next() {
        match character {
            character if character.is_whitespace() => {
                if in_token {
                    args.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            '\'' => {
                in_token = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(character) => current.push(character),
                        None => return Err(unterminated_quote()),
                    }
                }
            }
            '"' => {
                in_token = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(character) => current.push(character),
                            None => return Err(unterminated_quote()),
                        },
                        Some(character) => current.push(character),
                        None => return Err(unterminated_quote()),
                    }
                }
            }
            '\\' => {
                in_token = true;
                match chars.next() {
                    Some(character) => current.push(character),
                    None => {
                        return Err(EngineError::Setup(
                            "trailing backslash in the engine configuration".to_owned(),
                        ));
                    }
                }
            }
            character => {
                in_token = true;
                current.push(character);
            }
        }
    }

    if in_token {
        args.push(current);
    }

    Ok(args)
}

fn unterminated_quote() -> EngineError {
    EngineError::Setup("unterminated quote in the engine configuration".to_owned())
}
