//! A stand-in engine process for the adapter and supervisor tests.

use std::env;
use std::io::{self, BufRead, Write};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

fn main() {
    if env::var("MOCK_FAIL_START").as_deref() == Ok("1") {
        std::process::exit(3);
    }

    let delay = env::var("MOCK_READY_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    if delay > 0 {
        thread::sleep(Duration::from_millis(delay));
    }

    let mut stdout = io::stdout().lock();
    write_line(
        &mut stdout,
        &json!({"event": "ready", "pid": std::process::id()}),
    );

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        let line = match lines.next() {
            Some(Ok(line)) => line,
            Some(Err(_)) => block_forever(),
            None => block_forever(),
        };

        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                write_line(&mut stdout, &json!({"error": "unknown_op"}));
                continue;
            }
        };

        match request.get("op").and_then(Value::as_str) {
            Some("exec") => {
                let command = request
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let id = request.get("id").and_then(Value::as_u64).unwrap_or(0);
                let outcome = run_command(command);
                write_line(
                    &mut stdout,
                    &json!({
                        "id": id,
                        "exit_code": outcome.exit_code,
                        "output": outcome.output,
                    }),
                );
                if outcome.exit {
                    std::process::exit(outcome.exit_code);
                }
            }
            Some("stop") => {
                write_line(&mut stdout, &json!({"event": "stopped"}));
                std::process::exit(0);
            }
            _ => write_line(&mut stdout, &json!({"error": "unknown_op"})),
        }
    }
}

struct Outcome {
    exit_code: i32,
    output: String,
    exit: bool,
}

fn run_command(command: &str) -> Outcome {
    if let Some(rest) = command.strip_prefix("ECHO ") {
        return Outcome {
            exit_code: 0,
            output: format!("{rest}\r\n"),
            exit: false,
        };
    }

    if let Some(rest) = command.strip_prefix("EXIT ") {
        let code = rest.trim().parse::<i32>().unwrap_or(0);
        return Outcome {
            exit_code: code,
            output: String::new(),
            exit: true,
        };
    }

    if let Some(rest) = command.strip_prefix("SLEEP ") {
        if let Ok(millis) = rest.trim().parse::<u64>() {
            thread::sleep(Duration::from_millis(millis));
        }
        return Outcome {
            exit_code: 0,
            output: String::new(),
            exit: false,
        };
    }

    Outcome {
        exit_code: 0,
        output: String::new(),
        exit: false,
    }
}

fn write_line(stdout: &mut impl Write, value: &Value) {
    let _ = writeln!(stdout, "{value}");
    let _ = stdout.flush();
}

fn block_forever() -> ! {
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}
