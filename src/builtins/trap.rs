//! trap module.
//!
//! GNU Bash source ownership:
//! - builtins/trap.def

use std::collections::{BTreeSet, HashMap};
use std::io::{self, Write};

const TRAP_LIST: &str = "__RUBASH_TRAPS";
const TRAP_PREFIX: &str = "__RUBASH_TRAP_";
const EX_USAGE: i32 = 2;
const SIGNALS: [&str; 64] = [
    "SIGHUP",
    "SIGINT",
    "SIGQUIT",
    "SIGILL",
    "SIGTRAP",
    "SIGABRT",
    "SIGEMT",
    "SIGFPE",
    "SIGKILL",
    "SIGBUS",
    "SIGSEGV",
    "SIGSYS",
    "SIGPIPE",
    "SIGALRM",
    "SIGTERM",
    "SIGURG",
    "SIGSTOP",
    "SIGTSTP",
    "SIGCONT",
    "SIGCHLD",
    "SIGTTIN",
    "SIGTTOU",
    "SIGIO",
    "SIGXCPU",
    "SIGXFSZ",
    "SIGVTALRM",
    "SIGPROF",
    "SIGWINCH",
    "SIGPWR",
    "SIGUSR1",
    "SIGUSR2",
    "SIGRTMIN",
    "SIGRTMIN+1",
    "SIGRTMIN+2",
    "SIGRTMIN+3",
    "SIGRTMIN+4",
    "SIGRTMIN+5",
    "SIGRTMIN+6",
    "SIGRTMIN+7",
    "SIGRTMIN+8",
    "SIGRTMIN+9",
    "SIGRTMIN+10",
    "SIGRTMIN+11",
    "SIGRTMIN+12",
    "SIGRTMIN+13",
    "SIGRTMIN+14",
    "SIGRTMIN+15",
    "SIGRTMIN+16",
    "SIGRTMAX-15",
    "SIGRTMAX-14",
    "SIGRTMAX-13",
    "SIGRTMAX-12",
    "SIGRTMAX-11",
    "SIGRTMAX-10",
    "SIGRTMAX-9",
    "SIGRTMAX-8",
    "SIGRTMAX-7",
    "SIGRTMAX-6",
    "SIGRTMAX-5",
    "SIGRTMAX-4",
    "SIGRTMAX-3",
    "SIGRTMAX-2",
    "SIGRTMAX-1",
    "SIGRTMAX",
];

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

    if args
        .get(index)
        .is_some_and(|arg| arg.starts_with('-') && arg[1..].contains('l'))
    {
        print_signal_list(stdout)?;
        return Ok(0);
    }

    if index >= args.len() || args.get(index).map(String::as_str) == Some("-p") {
        if args.get(index).map(String::as_str) == Some("-p") {
            index += 1;
        }
        let signals = normalized_signals(&args[index..], stderr)?;
        let selected = if args[index..].is_empty() {
            None
        } else {
            Some(signals.signals.as_slice())
        };
        print_traps(env_vars, selected, stdout)?;
        return Ok(i32::from(signals.invalid));
    }

    let action = args[index].as_str();
    index += 1;
    if index >= args.len() {
        print_usage(stderr)?;
        return Ok(EX_USAGE);
    }

    let signals = normalized_signals(&args[index..], stderr)?;
    if action == "-" {
        for signal in signals.signals {
            remove_trap(env_vars, &signal);
        }
        return Ok(i32::from(signals.invalid));
    }

    for signal in signals.signals {
        set_trap(env_vars, &signal, action);
    }
    Ok(i32::from(signals.invalid))
}

pub fn list_first_signal_for_sed() -> &'static str {
    "SIGHUP"
}

pub(crate) fn take_exit_trap(env_vars: &mut HashMap<String, String>) -> Option<String> {
    let action = env_vars.remove(&trap_key("EXIT"));
    let mut signals = trap_list(env_vars);
    signals.remove("EXIT");
    store_trap_list(env_vars, signals);
    action
}

pub(crate) fn get_trap_action(env_vars: &HashMap<String, String>, signal: &str) -> Option<String> {
    env_vars.get(&trap_key(signal)).cloned()
}

struct NormalizedSignals {
    signals: Vec<String>,
    invalid: bool,
}

fn normalized_signals<E>(args: &[String], stderr: &mut E) -> io::Result<NormalizedSignals>
where
    E: Write,
{
    let mut signals = Vec::new();
    let mut invalid = false;
    for arg in args {
        match normalize_signal(arg) {
            Some(signal) => signals.push(signal.to_string()),
            None => {
                invalid = true;
                writeln!(stderr, "rubash: trap: {arg}: invalid signal specification")?;
            }
        }
    }
    Ok(NormalizedSignals { signals, invalid })
}

fn print_usage<E>(stderr: &mut E) -> io::Result<()>
where
    E: Write,
{
    writeln!(stderr, "trap: usage: trap [-lp] [[arg] signal_spec ...]")
}

fn print_signal_list<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    for (index, signal) in SIGNALS.iter().enumerate() {
        write!(stdout, "{:>2}) {:<10}", index + 1, signal)?;
        if (index + 1) % 5 == 0 || index + 1 == SIGNALS.len() {
            writeln!(stdout)?;
        } else {
            write!(stdout, "\t")?;
        }
    }
    Ok(())
}

fn normalize_signal(signal: &str) -> Option<&'static str> {
    let signal = signal.to_ascii_uppercase();
    match signal.as_str() {
        "0" | "EXIT" => return Some("EXIT"),
        "DEBUG" => return Some("DEBUG"),
        "ERR" => return Some("ERR"),
        "RETURN" => return Some("RETURN"),
        _ => {}
    }

    if let Ok(number) = signal.parse::<usize>() {
        return number
            .checked_sub(1)
            .and_then(|index| SIGNALS.get(index).copied());
    }

    let name = signal.strip_prefix("SIG").unwrap_or(&signal);
    SIGNALS
        .iter()
        .copied()
        .find(|candidate| candidate.strip_prefix("SIG") == Some(name))
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
