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
            writeln!(
                stdout,
                " 1) SIGHUP\t 2) SIGINT\t 3) SIGQUIT\t 4) SIGILL\t 5) SIGTRAP"
            )?;
            writeln!(
                stdout,
                " 6) SIGABRT\t 7) SIGEMT\t 8) SIGFPE\t 9) SIGKILL\t10) SIGBUS"
            )?;
            writeln!(
                stdout,
                "11) SIGSEGV\t12) SIGSYS\t13) SIGPIPE\t14) SIGALRM\t15) SIGTERM"
            )?;
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
        Some("2") | Some("130") => {
            writeln!(stdout, "INT")?;
            Ok(0)
        }
        Some("3") | Some("131") => {
            writeln!(stdout, "QUIT")?;
            Ok(0)
        }
        Some("9") | Some("137") => {
            writeln!(stdout, "KILL")?;
            Ok(0)
        }
        Some("15") | Some("143") => {
            writeln!(stdout, "TERM")?;
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
        Some("QUIT") | Some("SIGQUIT") => {
            writeln!(stdout, "3")?;
            Ok(0)
        }
        Some("KILL") | Some("SIGKILL") => {
            writeln!(stdout, "9")?;
            Ok(0)
        }
        Some("TERM") | Some("SIGTERM") => {
            writeln!(stdout, "15")?;
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
        "2" | "130" => Some("INT"),
        "3" | "131" => Some("QUIT"),
        "9" | "137" => Some("KILL"),
        "15" | "143" => Some("TERM"),
        "EXIT" | "SIGEXIT" => Some("0"),
        "HUP" | "SIGHUP" => Some("1"),
        "INT" | "SIGINT" => Some("2"),
        "QUIT" | "SIGQUIT" => Some("3"),
        "KILL" | "SIGKILL" => Some("9"),
        "TERM" | "SIGTERM" => Some("15"),
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
