//! trap module.
//!
//! GNU Bash source ownership:
//! - builtins/trap.def

use std::collections::{BTreeSet, HashMap};
use std::io::{self, Write};

const TRAP_LIST: &str = "__RUBASH_TRAPS";
const TRAP_PREFIX: &str = "__RUBASH_TRAP_";

pub fn execute(args: &[String]) -> io::Result<i32> {
    let mut env_vars = HashMap::new();
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args, &mut env_vars, &mut stdout, &mut stderr)
}

pub fn execute_with_io<W, E>(
    args: &[String],
    env_vars: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    // TODO(builtins/trap.def/sig.c): Install real process signal handlers and
    // run EXIT/DEBUG/ERR/RETURN traps through Bash's unwind machinery. This
    // implements the shell-visible trap table and `trap -p` output.
    let mut index = 0;
    if args.first().map(String::as_str) == Some("--") {
        index = 1;
    }

    if index >= args.len() || args.get(index).map(String::as_str) == Some("-p") {
        if args.get(index).map(String::as_str) == Some("-p") {
            index += 1;
        }
        let signals = normalized_signals(&args[index..], stderr)?;
        let selected = if args[index..].is_empty() {
            None
        } else {
            Some(signals.as_slice())
        };
        print_traps(env_vars, selected, stdout)?;
        return Ok(0);
    }

    let action = args[index].as_str();
    index += 1;
    if index >= args.len() {
        return Ok(0);
    }

    let signals = normalized_signals(&args[index..], stderr)?;
    if action == "-" {
        for signal in signals {
            remove_trap(env_vars, &signal);
        }
        return Ok(0);
    }

    for signal in signals {
        set_trap(env_vars, &signal, action);
    }
    Ok(0)
}

pub fn list_first_signal_for_sed() -> &'static str {
    "SIGHUP"
}

fn normalized_signals<E>(args: &[String], stderr: &mut E) -> io::Result<Vec<String>>
where
    E: Write,
{
    let mut signals = Vec::new();
    for arg in args {
        match normalize_signal(arg) {
            Some(signal) => signals.push(signal.to_string()),
            None => {
                writeln!(stderr, "rubash: trap: {arg}: invalid signal specification")?;
            }
        }
    }
    Ok(signals)
}

fn normalize_signal(signal: &str) -> Option<&'static str> {
    match signal.to_ascii_uppercase().as_str() {
        "0" | "EXIT" => Some("EXIT"),
        "1" | "HUP" | "SIGHUP" => Some("SIGHUP"),
        "2" | "INT" | "SIGINT" => Some("SIGINT"),
        "3" | "QUIT" | "SIGQUIT" => Some("SIGQUIT"),
        "6" | "ABRT" | "SIGABRT" => Some("SIGABRT"),
        "15" | "TERM" | "SIGTERM" => Some("SIGTERM"),
        "USR1" | "SIGUSR1" => Some("SIGUSR1"),
        "USR2" | "SIGUSR2" => Some("SIGUSR2"),
        "CHLD" | "SIGCHLD" => Some("SIGCHLD"),
        "DEBUG" => Some("DEBUG"),
        "ERR" => Some("ERR"),
        "RETURN" => Some("RETURN"),
        _ => None,
    }
}

fn set_trap(env_vars: &mut HashMap<String, String>, signal: &str, action: &str) {
    env_vars.insert(trap_key(signal), action.to_string());
    let mut signals = trap_list(env_vars);
    signals.insert(signal.to_string());
    store_trap_list(env_vars, signals);
}

fn remove_trap(env_vars: &mut HashMap<String, String>, signal: &str) {
    env_vars.remove(&trap_key(signal));
    let mut signals = trap_list(env_vars);
    signals.remove(signal);
    store_trap_list(env_vars, signals);
}

fn print_traps<W>(
    env_vars: &HashMap<String, String>,
    selected: Option<&[String]>,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    let signals: Vec<String> = match selected {
        Some(signals) => signals.to_vec(),
        None => trap_list(env_vars).into_iter().collect(),
    };

    for signal in signals {
        if let Some(action) = env_vars.get(&trap_key(&signal)) {
            writeln!(stdout, "trap -- {} {}", shell_quote(action), signal)?;
        }
    }
    Ok(())
}

fn trap_key(signal: &str) -> String {
    format!("{TRAP_PREFIX}{signal}")
}

fn trap_list(env_vars: &HashMap<String, String>) -> BTreeSet<String> {
    env_vars
        .get(TRAP_LIST)
        .map(|value| value.split(':').map(str::to_string).collect())
        .unwrap_or_default()
}

fn store_trap_list(env_vars: &mut HashMap<String, String>, signals: BTreeSet<String>) {
    if signals.is_empty() {
        env_vars.remove(TRAP_LIST);
    } else {
        env_vars.insert(
            TRAP_LIST.to_string(),
            signals.into_iter().collect::<Vec<_>>().join(":"),
        );
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
