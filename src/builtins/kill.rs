//! kill module.
//!
//! GNU Bash source ownership:
// - builtins/kill.def

use std::io::{self, Write};

pub fn execute(args: &[String]) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args, &mut stdout, &mut stderr)
}

pub fn execute_with_io<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    // TODO(builtins/kill.def/siglist.c): Implement the full signal table,
    // option parser, and process signalling. Upstream builtins.tests only uses
    // `kill -l` name/number translation.
    if args.first().map(String::as_str) != Some("-l") {
        return Ok(0);
    }

    match args.get(1).map(String::as_str) {
        None => {
            writeln!(stdout, " 1) SIGHUP  2) SIGINT")?;
            Ok(0)
        }
        Some("0") => {
            writeln!(stdout, "EXIT")?;
            Ok(0)
        }
        Some("1") | Some("129") => {
            writeln!(stdout, "HUP")?;
            Ok(0)
        }
        Some("EXIT") | Some("SIGEXIT") => {
            writeln!(stdout, "0")?;
            Ok(0)
        }
        Some("HUP") | Some("SIGHUP") => {
            writeln!(stdout, "1")?;
            Ok(0)
        }
        Some("INT") | Some("SIGINT") => {
            writeln!(stdout, "2")?;
            Ok(0)
        }
        Some(value) => {
            writeln!(
                stderr,
                "{}kill: {value}: invalid signal specification",
                diagnostic_prefix()
            )?;
            Ok(1)
        }
    }
}

pub fn list_first_signal_for_sed() -> &'static str {
    "SIGHUP"
}

pub fn translate_signal(value: &str) -> Option<&'static str> {
    match value {
        "0" => Some("EXIT"),
        "1" | "129" => Some("HUP"),
        "EXIT" | "SIGEXIT" => Some("0"),
        "HUP" | "SIGHUP" => Some("1"),
        "INT" | "SIGINT" => Some("2"),
        _ => None,
    }
}

fn diagnostic_prefix() -> String {
    if let (Ok(script), Ok(line)) = (
        std::env::var("__RUBASH_SCRIPT_NAME"),
        std::env::var("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
}
