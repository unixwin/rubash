//! trap module.
//!
//! GNU Bash source ownership:
//! - builtins/trap.def

use std::collections::{BTreeSet, HashMap};
use std::io::{self, Write};

const TRAP_LIST: &str = "__RUBASH_TRAPS";
const TRAP_PREFIX: &str = "__RUBASH_TRAP_";
/// Signals whose disposition was SIG_IGN when this shell started (trap.c
/// sigmodes SIG_HARD_IGNORE). They cannot be trapped or reset, and POSIX
/// reports no error when attempted (trap.c set_signal/ignore_signal).
pub(crate) const TRAP_ORIG_IGNORES: &str = "__RUBASH_TRAP_ORIG_IGN";
/// Signals whose dispositions were reset on subshell entry. GNU
/// reset_signal_handlers (trap.c:1480 -> reset_or_restore_signal_handlers:1433)
/// restores the original handler for trapped signals but deliberately does
/// not free trap_list strings, so `trap` in a subshell still lists the
/// parent's traps; the reset set records which entries may not fire.
const TRAP_RESET: &str = "__RUBASH_TRAP_RESET";
const EX_USAGE: i32 = 2;
/// Positional signal-name table: entry i-1 is signal i's name (GNU's
/// `signal_names[]` is indexed by signal number — support/signames.c:69
/// initialize_signames, generated per target by support/mksignames.c from
/// the TARGET machine's `<signal.h>`; Makefile.in:599-617/:775-804). Empty
/// strings mark numbers the platform has no name for; `trap -l` and
/// trap.c:236 `decode_signal` skip them.
///
/// * `not(unix)` (Windows mailbox): the literal Linux x86 table is the wire
///   contract — `trap 17`/`kill -l 10`/BASH_TRAPSIG all speak Linux
///   numbering there, and the GNU 5.3.0 baseline runs on WSL. Slots 32/33
///   do not exist on Linux; GNU's `trap -l` skips them and accepts the
///   specifiers silently. DO NOT renumber.
/// * `target_os = "linux"`: same Linux table (RTMIN..RTMAX arithmetic names
///   are GNU-generated at signames.c:92-139 and are Linux-only, matching
///   the `#if defined (SIGRTMIN)` guard there).
/// * other unix (macos/freebsd): the BSD-family numbering where
///   7=EMT, 10=BUS, 12=SYS, 16=URG, 17=STOP, 18=TSTP, 19=CONT, 20=CHLD,
///   23=IO, 29=INFO, 30=USR1, 31=USR2 (libc resolves these per target; the
///   tests below pin every slot against libc::SIGxxx). No RT signals.
#[cfg(not(unix))]
pub(crate) const SIGNALS: [&str; 64] = [
    "SIGHUP",
    "SIGINT",
    "SIGQUIT",
    "SIGILL",
    "SIGTRAP",
    "SIGABRT",
    "SIGBUS",
    "SIGFPE",
    "SIGKILL",
    "SIGUSR1",
    "SIGSEGV",
    "SIGUSR2",
    "SIGPIPE",
    "SIGALRM",
    "SIGTERM",
    "SIGSTKFLT",
    "SIGCHLD",
    "SIGCONT",
    "SIGSTOP",
    "SIGTSTP",
    "SIGTTIN",
    "SIGTTOU",
    "SIGURG",
    "SIGXCPU",
    "SIGXFSZ",
    "SIGVTALRM",
    "SIGPROF",
    "SIGWINCH",
    "SIGIO",
    "SIGPWR",
    "SIGSYS",
    "",
    "",
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

/// Linux table: identical layout to the non-unix wire table above (it IS
/// the Linux numbering; see the shared doc comment).
#[cfg(target_os = "linux")]
pub(crate) const SIGNALS: [&str; 64] = [
    "SIGHUP",
    "SIGINT",
    "SIGQUIT",
    "SIGILL",
    "SIGTRAP",
    "SIGABRT",
    "SIGBUS",
    "SIGFPE",
    "SIGKILL",
    "SIGUSR1",
    "SIGSEGV",
    "SIGUSR2",
    "SIGPIPE",
    "SIGALRM",
    "SIGTERM",
    "SIGSTKFLT",
    "SIGCHLD",
    "SIGCONT",
    "SIGSTOP",
    "SIGTSTP",
    "SIGTTIN",
    "SIGTTOU",
    "SIGURG",
    "SIGXCPU",
    "SIGXFSZ",
    "SIGVTALRM",
    "SIGPROF",
    "SIGWINCH",
    "SIGIO",
    "SIGPWR",
    "SIGSYS",
    "",
    "",
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

/// BSD-family table (macos/freebsd): positional names for signals 1..31 in
/// the target's numbering (see the shared doc comment). Positional layout
/// means no libc constants appear here; the tests pin each slot against
/// libc::SIGxxx so CI fails on numbering drift.
#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) const SIGNALS: [&str; 31] = [
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
    "SIGINFO",
    "SIGUSR1",
    "SIGUSR2",
];

pub fn execute(args: &[String]) -> io::Result<i32> {
    let mut env_vars = HashMap::new();
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args, &mut env_vars, &mut stdout, &mut stderr)
}

/// Build the shell diagnostic prefix (`<script>: line N: `) from the shell
/// environment, falling back to `rubash: ` without script context. GNU
/// trap.def errors go through builtin_error -> error_prolog, which prefixes
/// `./script: line N:` when running a script file (trap.tests line 118
/// baseline: `./trap.tests: line 118: trap: cannot specify both -p and -P`).
fn diagnostic_prefix(env_vars: &HashMap<String, String>) -> String {
    if let (Some(script), Some(line)) = (
        env_vars.get("__RUBASH_SCRIPT_NAME"),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }
    "rubash: ".to_string()
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

    let mut list_signals = false;
    let mut print_trap_commands = false;
    let mut print_actions = false;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            index += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        for option in arg[1..].chars() {
            match option {
                'l' => list_signals = true,
                'p' => print_trap_commands = true,
                'P' => print_actions = true,
                _ => {
                    let prefix = diagnostic_prefix(env_vars);
                    writeln!(stderr, "{prefix}trap: {arg}: invalid option")?;
                    print_usage(stderr)?;
                    return Ok(EX_USAGE);
                }
            }
        }
        index += 1;
    }

    if list_signals {
        print_signal_list(stdout)?;
        return Ok(0);
    }

    if print_trap_commands && print_actions {
        let prefix = diagnostic_prefix(env_vars);
        writeln!(stderr, "{prefix}trap: cannot specify both -p and -P")?;
        return Ok(EX_USAGE);
    }

    if print_actions && index >= args.len() {
        let prefix = diagnostic_prefix(env_vars);
        writeln!(stderr, "{prefix}trap: -P requires at least one signal name")?;
        return Ok(EX_USAGE);
    }

    if index >= args.len() || print_trap_commands || print_actions {
        let signals = normalized_signals(&args[index..], env_vars, stderr)?;
        if signals.invalid {
            return Ok(1);
        }
        let selected = if args[index..].is_empty() {
            None
        } else {
            Some(signals.signals.as_slice())
        };
        print_traps(env_vars, selected, stdout, print_actions)?;
        return Ok(i32::from(signals.invalid));
    }

    let action = args[index].as_str();
    index += 1;
    if index >= args.len() {
        // GNU builtins/trap.def lines 170-199: a single argument is either
        // a valid signal (REVERT), "-" (REVERT via else branch, but list->next
        // is NULL so builtin_usage + EX_USAGE), an all-digit string naming a
        // valid signal (first_signal REVERT, e.g. "trap 0" reverts EXIT), or
        // anything else. For anything else (empty string, non-signal name
        // like "zzz", all-digit invalid like "512"), the else branch advances
        // list to NULL, calls builtin_usage() and returns EX_USAGE.
        //   first_signal = *first_arg && all_digits && signal_object_p
        //   REVERT (non-posix) = *first_arg && (*first_arg != '-' || first_arg[1])
        //                         && signal_object_p && list->next == 0
        //   else: list = list->next; if list == 0: builtin_usage; return EX_USAGE
        if let Some(signal) = normalize_signal(action) {
            remove_trap(env_vars, signal);
            return Ok(0);
        }
        print_usage(stderr)?;
        return Ok(EX_USAGE);
    }

    let signals = normalized_signals(&args[index..], env_vars, stderr)?;
    if action == "-" || action == "0" {
        for signal in signals.signals {
            remove_trap(env_vars, &signal);
        }
        // Bash treats the numeric zero action as clearing the EXIT trap too.
        if action == "0" {
            remove_trap(env_vars, "EXIT");
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
    if reset_trap_signals(env_vars).contains("EXIT") {
        return None;
    }
    let action = env_vars.remove(&trap_key("EXIT"));
    let mut signals = trap_list(env_vars);
    signals.remove("EXIT");
    store_trap_list(env_vars, signals);
    action
}

pub(crate) fn get_trap_action(env_vars: &HashMap<String, String>, signal: &str) -> Option<String> {
    if reset_trap_signals(env_vars).contains(signal) {
        return None;
    }
    env_vars.get(&trap_key(signal)).cloned()
}

struct NormalizedSignals {
    signals: Vec<String>,
    invalid: bool,
}

fn normalized_signals<E>(
    args: &[String],
    env_vars: &HashMap<String, String>,
    stderr: &mut E,
) -> io::Result<NormalizedSignals>
where
    E: Write,
{
    let prefix = diagnostic_prefix(env_vars);
    let mut signals = Vec::new();
    let mut invalid = false;
    for arg in args {
        match normalize_signal(arg) {
            Some(signal) => signals.push(signal.to_string()),
            None => {
                invalid = true;
                writeln!(stderr, "{prefix}trap: {arg}: invalid signal specification")?;
            }
        }
    }
    Ok(NormalizedSignals { signals, invalid })
}

fn print_usage<E>(stderr: &mut E) -> io::Result<()>
where
    E: Write,
{
    // GNU builtins/trap.def builtin_usage: "trap: usage: trap [-Plp] [[action] signal_spec ...]"
    writeln!(
        stderr,
        "trap: usage: trap [-Plp] [[action] signal_spec ...]"
    )
}

fn print_signal_list<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    // GNU trap -l output is byte-identical to `kill -l` (Linux numbering,
    // 5 entries per line, tab-separated, trailing tab on the final partial
    // line). Slots 32/33 do not exist on Linux and are skipped.
    let numbered: Vec<(usize, &str)> = SIGNALS
        .iter()
        .enumerate()
        .filter(|(_, name)| !name.is_empty())
        .map(|(index, name)| (index + 1, *name))
        .collect();
    let total = numbered.len();
    for (position, (number, signal)) in numbered.iter().enumerate() {
        let position = position + 1;
        if position > 1 && (position - 1) % 5 == 0 {
            writeln!(stdout)?;
        } else if position > 1 {
            write!(stdout, "\t")?;
        }
        write!(stdout, "{number:>2}) {signal}")?;
        if position == total && position % 5 != 0 {
            write!(stdout, "\t")?;
        }
    }
    writeln!(stdout)?;
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
    // GNU trap.c set_signal: "A signal ignored on entry to the shell cannot
    // be trapped or reset, but no error is reported" (SIG_HARD_IGNORE).
    // Only real signals are hard-ignored; the EXIT/DEBUG/ERROR/RETURN
    // special traps are always settable (trap.c SPECIAL_TRAP branch runs
    // before the hard-ignore check).
    if is_hard_ignored(env_vars, signal) {
        return;
    }
    env_vars.insert(trap_key(signal), action.to_string());
    let mut signals = trap_list(env_vars);
    signals.insert(signal.to_string());
    store_trap_list(env_vars, signals);
    // GNU set_signal re-arms a subshell-reset disposition: the new handler
    // is trapped, not reset, so it fires and stays listed.
    let mut reset = reset_trap_signals(env_vars);
    if reset.remove(signal) {
        store_reset_traps(env_vars, reset);
    }
}

fn remove_trap(env_vars: &mut HashMap<String, String>, signal: &str) {
    if is_hard_ignored(env_vars, signal) {
        return;
    }
    env_vars.remove(&trap_key(signal));
    let mut signals = trap_list(env_vars);
    signals.remove(signal);
    store_trap_list(env_vars, signals);
    let mut reset = reset_trap_signals(env_vars);
    if reset.remove(signal) {
        store_reset_traps(env_vars, reset);
    }
}

fn is_hard_ignored(env_vars: &HashMap<String, String>, signal: &str) -> bool {
    if matches!(signal, "EXIT" | "DEBUG" | "ERROR" | "RETURN") {
        return false;
    }
    orig_ignored_signals(env_vars)
        .iter()
        .any(|ignored| ignored == signal)
}

fn orig_ignored_signals(env_vars: &HashMap<String, String>) -> Vec<String> {
    env_vars
        .get(TRAP_ORIG_IGNORES)
        .map(|value| {
            value
                .split(':')
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Value for the child process's TRAP_ORIG_IGNORES at a spawn boundary.
/// GNU's fork passes the parent's SIG_IGN dispositions through the kernel:
/// signals the parent ignores at runtime (empty trap actions) become
/// SIG_HARD_IGNORE in the child, alongside the parent's own inherited
/// ignores. Windows environment blocks cannot carry empty values (the
/// per-signal trap keys with "" actions are dropped), so the merged set is
/// transported in the single non-empty ORIG_IGN variable instead.
pub(crate) fn transport_inherited_ignores(env_vars: &HashMap<String, String>) -> String {
    let mut ignored: Vec<String> = trap_list(env_vars)
        .into_iter()
        .filter(|signal| {
            !matches!(signal.as_str(), "EXIT" | "DEBUG" | "ERROR" | "RETURN")
                && env_vars
                    .get(&trap_key(signal))
                    .is_some_and(String::is_empty)
        })
        .collect();
    ignored.extend(orig_ignored_signals(env_vars));
    ignored.sort();
    ignored.dedup();
    ignored.join(":")
}

fn store_orig_ignored_signals(env_vars: &mut HashMap<String, String>, mut signals: Vec<String>) {
    signals.sort();
    signals.dedup();
    if signals.is_empty() {
        env_vars.remove(TRAP_ORIG_IGNORES);
    } else {
        env_vars.insert(TRAP_ORIG_IGNORES.to_string(), signals.join(":"));
    }
}

fn add_orig_ignored_signal(env_vars: &mut HashMap<String, String>, signal: &str) {
    let mut signals = orig_ignored_signals(env_vars);
    if signals.iter().any(|ignored| ignored == signal) {
        return;
    }
    signals.push(signal.to_string());
    store_orig_ignored_signals(env_vars, signals);
}

/// Seed the traps a shell inherits from its environment at startup. WSL's
/// init leaves SIGRTMIN ignored for every child process, so the GNU 5.3.0
/// contract baseline lists "trap -- '' SIGRTMIN" in every shell; model that
/// inherited ignore here. Ignored-disposition traps carried in through the
/// environment (a parent shell's "trap '' SIG" reaching a fresh rubash
/// child) become hard-ignored for this shell, matching trap.c's
/// original_signals == SIG_IGN startup handling.
pub(crate) fn seed_startup_traps(env_vars: &mut HashMap<String, String>) {
    // Rebuild the empty trap-table entries for ignores transported across a
    // process boundary (transport_inherited_ignores): the per-signal "" keys
    // cannot survive a Windows environment block, so the ORIG_IGN list is
    // the only record the child has. GNU's child displays these as
    // `trap -- '' SIG` and refuses to trap or reset them.
    for signal in orig_ignored_signals(env_vars) {
        if env_vars.get(&trap_key(&signal)).is_none() {
            env_vars.insert(trap_key(&signal), String::new());
            let mut signals = trap_list(env_vars);
            signals.insert(signal);
            store_trap_list(env_vars, signals);
        }
    }
    for signal in SIGNALS {
        let key = trap_key(signal);
        if env_vars.get(&key).is_some_and(String::is_empty) {
            let mut signals = trap_list(env_vars);
            signals.insert(signal.to_string());
            store_trap_list(env_vars, signals);
            add_orig_ignored_signal(env_vars, signal);
        }
    }
    // WSL's init leaves SIGRTMIN ignored for every child process, so the
    // GNU 5.3.0 contract baseline lists "trap -- '' SIGRTMIN" in every
    // fresh shell (both 5.2.21 and 5.3.0 under WSL show it, including
    // `bash -c 'trap'`). Seed that inherited ignore when no trap table
    // entry arrived. SIGRTMIN exists only where the SIGNALS table has it
    // (signames.c:97 `#if defined (SIGRTMIN)`): Windows keeps the WSL
    // wire-format contract and Linux has RT signals; Darwin/BSD compile
    // the block out so no phantom SIGRTMIN trap appears in `trap` output
    // there (GNU bash on Darwin has no SIGRTMIN).
    #[cfg(any(not(unix), target_os = "linux"))]
    if env_vars.get(&trap_key("SIGRTMIN")).is_none() {
        env_vars.insert(trap_key("SIGRTMIN"), String::new());
        let mut signals = trap_list(env_vars);
        signals.insert("SIGRTMIN".to_string());
        store_trap_list(env_vars, signals);
        add_orig_ignored_signal(env_vars, "SIGRTMIN");
    }
}

/// A script executed through the same-shell ENOEXEC path runs in a fresh
/// shell context: every real signal the parent currently ignores becomes an
/// original ignore for the script (the fork child inherits the SIG_IGN
/// disposition, and trap.c re-derives SIG_HARD_IGNORE from it). Runtime
/// ignores of plain subshells stay mutable; only this shell-entry boundary
/// freezes them.
pub(crate) fn mark_startup_ignores(env_vars: &mut HashMap<String, String>) {
    let ignored: Vec<String> = trap_list(env_vars)
        .into_iter()
        .filter(|signal| {
            !matches!(signal.as_str(), "EXIT" | "DEBUG" | "ERROR" | "RETURN")
                && env_vars
                    .get(&trap_key(signal))
                    .is_some_and(String::is_empty)
        })
        .collect();
    for signal in ignored {
        add_orig_ignored_signal(env_vars, &signal);
    }
}

/// GNU execute_cmd.c uw_maybe_set_debug_trap: restore a saved DEBUG trap at
/// function exit only when the body did not set a new one.
pub(crate) fn maybe_restore_debug_trap(env_vars: &mut HashMap<String, String>, action: String) {
    if env_vars.contains_key(&trap_key("DEBUG")) {
        return;
    }
    set_trap(env_vars, "DEBUG", &action);
}

/// GNU execute_cmd.c restore_default_signal(DEBUG_TRAP) at function entry:
/// remove the inherited DEBUG trap for the function body.
pub(crate) fn clear_debug_trap(env_vars: &mut HashMap<String, String>) {
    env_vars.remove(&trap_key("DEBUG"));
    let mut signals = trap_list(env_vars);
    signals.remove("DEBUG");
    store_trap_list(env_vars, signals);
}

fn print_traps<W>(
    env_vars: &HashMap<String, String>,
    selected: Option<&[String]>,
    stdout: &mut W,
    actions_only: bool,
) -> io::Result<()>
where
    W: Write,
{
    // GNU builtins/trap.def display_traps walks the trap table in signal
    // number order (trap.h: EXIT_TRAP == 0, real signals 1..NSIG-1, then
    // DEBUG_TRAP == NSIG, ERROR_TRAP == NSIG+1, RETURN_TRAP == NSIG+2), so
    // an unselected listing prints EXIT first, then every set signal trap in
    // numeric order, then DEBUG/ERROR/RETURN.
    let signals: Vec<String> = match selected {
        Some(signals) => signals.to_vec(),
        None => canonical_trap_order()
            .into_iter()
            .filter(|signal| trap_list(env_vars).contains(signal))
            .collect(),
    };

    let orig_ignores = orig_ignored_signals(env_vars);
    for signal in signals {
        if let Some(action) = env_vars.get(&trap_key(&signal)) {
            if actions_only {
                // GNU display_traps DISP_ACTIONONLY: a signal that was
                // ignored when the shell started is skipped entirely; a
                // runtime ignore ("trap '' SIG") prints an empty line.
                if orig_ignores.iter().any(|ignored| ignored == &signal) {
                    continue;
                }
                writeln!(stdout, "{action}")?;
            } else {
                writeln!(stdout, "trap -- {} {}", shell_quote(action), signal)?;
            }
        }
    }
    Ok(())
}

/// Trap-table iteration order matching GNU trap.c display_traps: EXIT
/// (index 0), real signals in numeric order, then DEBUG, ERROR, RETURN
/// (rubash keys the ERR trap as "ERR").
fn canonical_trap_order() -> Vec<String> {
    std::iter::once("EXIT".to_string())
        .chain(
            SIGNALS
                .iter()
                .filter(|signal| !signal.is_empty())
                .map(|signal| signal.to_string()),
        )
        .chain(["DEBUG", "ERR", "RETURN"].iter().map(|s| s.to_string()))
        .collect()
}

fn trap_key(signal: &str) -> String {
    format!("{TRAP_PREFIX}{signal}")
}

/// Reset caught signal traps when entering a subshell. GNU
/// reset_signal_handlers (trap.c:1480) restores the original handler for
/// trapped signals but does not free trap_list strings: `trap` in a subshell
/// still lists the parent's traps (`trap 'x' TERM; (trap)` prints the TERM
/// entry). Ignored dispositions (`trap ''`) stay ignored. Keep every table
/// entry and record the reset dispositions so get_trap_action/take_exit_trap
/// skip firing them while the listing still shows them.
pub(crate) fn reset_for_subshell(env_vars: &mut HashMap<String, String>) {
    let mut reset = reset_trap_signals(env_vars);
    for signal in trap_list(env_vars) {
        // DEBUG and RETURN are tracing hooks, not OS signal dispositions;
        // functrace/extdebug controls their inheritance separately.
        if matches!(signal.as_str(), "DEBUG" | "RETURN") {
            continue;
        }
        let key = trap_key(&signal);
        if env_vars.get(&key).is_some_and(|action| !action.is_empty()) {
            reset.insert(signal);
        }
    }
    store_reset_traps(env_vars, reset);
}

fn reset_trap_signals(env_vars: &HashMap<String, String>) -> BTreeSet<String> {
    env_vars
        .get(TRAP_RESET)
        .map(|value| value.split(':').map(str::to_string).collect())
        .unwrap_or_default()
}

fn store_reset_traps(env_vars: &mut HashMap<String, String>, signals: BTreeSet<String>) {
    if signals.is_empty() {
        env_vars.remove(TRAP_RESET);
    } else {
        env_vars.insert(
            TRAP_RESET.to_string(),
            signals.into_iter().collect::<Vec<_>>().join(":"),
        );
    }
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

#[cfg(test)]
mod tests {
    /// The trap table and the kill builtin's table must name the same signal
    /// for the same number on every platform (GNU has ONE signal_names[]
    /// serving trap.c:236 decode_signal and builtins/kill.def). Any cfg
    /// drift between trap.rs and kill.rs breaks here first.
    #[test]
    fn trap_table_agrees_with_kill_table() {
        for (index, slot) in super::SIGNALS.iter().enumerate() {
            let number = (index + 1).to_string();
            let kill_name = crate::builtins::kill::translate_signal(&number);
            match (slot.is_empty(), kill_name) {
                (true, None) => {}
                (false, Some(name)) => {
                    assert_eq!(*slot, format!("SIG{name}"), "signal {}", index + 1);
                }
                (true, Some(name)) => panic!(
                    "kill names signal {} ({name}) but trap slot {} is empty",
                    index + 1,
                    index + 1
                ),
                (false, None) => panic!(
                    "trap names signal {} ({slot}) but kill has no name for it",
                    index + 1
                ),
            }
        }
        // Reverse direction: every kill-table row lands on the same trap slot.
        for number in 1..=super::SIGNALS.len() {
            if let Some(name) = crate::builtins::kill::translate_signal(&number.to_string()) {
                let expected = format!("SIG{name}");
                assert_eq!(
                    super::normalize_signal(&number.to_string()),
                    Some(expected.as_str()),
                    "trap slot for signal {number}"
                );
            }
        }
    }

    /// Positional slots must carry the libc name of THIS platform's signal
    /// number (the mksignames.c contract; CI runs this on ubuntu and macos,
    /// where it is the Darwin-drift trip wire: CHLD=20, CONT=19, USR1=30).
    #[cfg(unix)]
    #[test]
    fn positional_slots_match_libc() {
        for (constant, name) in [
            (libc::SIGHUP, "SIGHUP"),
            (libc::SIGINT, "SIGINT"),
            (libc::SIGQUIT, "SIGQUIT"),
            (libc::SIGILL, "SIGILL"),
            (libc::SIGTRAP, "SIGTRAP"),
            (libc::SIGABRT, "SIGABRT"),
            (libc::SIGFPE, "SIGFPE"),
            (libc::SIGKILL, "SIGKILL"),
            (libc::SIGSEGV, "SIGSEGV"),
            (libc::SIGPIPE, "SIGPIPE"),
            (libc::SIGALRM, "SIGALRM"),
            (libc::SIGTERM, "SIGTERM"),
            (libc::SIGCHLD, "SIGCHLD"),
            (libc::SIGCONT, "SIGCONT"),
            (libc::SIGSTOP, "SIGSTOP"),
            (libc::SIGTSTP, "SIGTSTP"),
            (libc::SIGTTIN, "SIGTTIN"),
            (libc::SIGTTOU, "SIGTTOU"),
            (libc::SIGXCPU, "SIGXCPU"),
            (libc::SIGXFSZ, "SIGXFSZ"),
            (libc::SIGVTALRM, "SIGVTALRM"),
            (libc::SIGPROF, "SIGPROF"),
            (libc::SIGWINCH, "SIGWINCH"),
            (libc::SIGUSR1, "SIGUSR1"),
            (libc::SIGUSR2, "SIGUSR2"),
        ] {
            let slot = super::SIGNALS
                .get(constant as usize - 1)
                .copied()
                .unwrap_or("<past end>");
            assert_eq!(slot, name, "slot for libc signal {name}");
            assert_eq!(super::normalize_signal(name), Some(name), "{name}");
            let bare = name.trim_start_matches("SIG");
            assert_eq!(super::normalize_signal(bare), Some(name), "{bare}");
            assert_eq!(
                super::normalize_signal(&constant.to_string()),
                Some(name),
                "numeric {name}"
            );
        }
    }

    /// Linux keeps the RT arithmetic names (signames.c:92-139) and the
    /// 32/33 holes GNU's trap -l skips.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_realtime_and_holes() {
        assert_eq!(super::normalize_signal("RTMIN"), Some("SIGRTMIN"));
        assert_eq!(super::normalize_signal("RTMIN+1"), Some("SIGRTMIN+1"));
        assert_eq!(super::normalize_signal("34"), Some("SIGRTMIN"));
        assert_eq!(super::normalize_signal("64"), Some("SIGRTMAX"));
        assert_eq!(super::normalize_signal("STKFLT"), Some("SIGSTKFLT"));
        assert_eq!(super::normalize_signal("17"), Some("SIGCHLD"));
        assert_eq!(super::normalize_signal("32"), Some(""));
        assert_eq!(super::normalize_signal("33"), Some(""));
        assert_eq!(super::normalize_signal("65"), None);
    }

    /// Darwin/BSD: reordered slots follow libc, Linux-only names are gone.
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    #[test]
    fn darwin_family_slots() {
        assert_eq!(super::normalize_signal("EMT"), Some("SIGEMT"));
        assert_eq!(super::normalize_signal("INFO"), Some("SIGINFO"));
        assert_eq!(super::normalize_signal("17"), Some("SIGSTOP"));
        assert_eq!(super::normalize_signal("18"), Some("SIGTSTP"));
        assert_eq!(super::normalize_signal("19"), Some("SIGCONT"));
        assert_eq!(super::normalize_signal("20"), Some("SIGCHLD"));
        assert_eq!(super::normalize_signal("30"), Some("SIGUSR1"));
        assert_eq!(super::normalize_signal("31"), Some("SIGUSR2"));
        assert_eq!(super::normalize_signal("STKFLT"), None);
        assert_eq!(super::normalize_signal("PWR"), None);
        assert_eq!(super::normalize_signal("RTMIN"), None);
        assert_eq!(super::normalize_signal("RTMAX"), None);
        assert_eq!(super::normalize_signal("32"), None);
        assert_eq!(super::normalize_signal("64"), None);
    }

    /// Non-unix builds pin the Windows mailbox wire format (Linux
    /// numbering) — the GNU 5.3.0 WSL baseline contract.
    #[cfg(not(unix))]
    #[test]
    fn wire_format_slots_are_linux_numbering() {
        assert_eq!(super::SIGNALS.len(), 64);
        assert_eq!(super::SIGNALS[16], "SIGCHLD");
        assert_eq!(super::SIGNALS[17], "SIGCONT");
        assert_eq!(super::SIGNALS[18], "SIGSTOP");
        assert_eq!(super::SIGNALS[19], "SIGTSTP");
        assert_eq!(super::SIGNALS[9], "SIGUSR1");
        assert_eq!(super::SIGNALS[28], "SIGIO");
        assert_eq!(super::SIGNALS[29], "SIGPWR");
        assert_eq!(super::SIGNALS[33], "SIGRTMIN");
        assert_eq!(super::SIGNALS[63], "SIGRTMAX");
    }
}
