//! trap module.
//!
//! GNU Bash source ownership:
//! - builtins/trap.def

use crate::shell::var_table::VarTable;
use std::collections::{BTreeSet, HashMap};
use std::io::{self, Write};

const TRAP_LIST: &str = "__RUBASH_TRAPS";
/// Action-key prefix for installed traps (`__RUBASH_TRAP_<SIGNAL>`).
/// `pub(crate)` for the arith-for counter fast driver's whitelist gate
/// (`compound_exec.rs`): the fast loop runs no commands, so any installed
/// action — including a reset-suppressed one, which the gate conservatively
/// treats as doubt — disqualifies and the slow loop runs instead.
pub(crate) const TRAP_PREFIX: &str = "__RUBASH_TRAP_";
/// Signals whose disposition was SIG_IGN when this shell started (trap.c
/// sigmodes SIG_HARD_IGNORE). They cannot be trapped or reset, and POSIX
/// reports no error when attempted (trap.c set_signal/ignore_signal).
pub(crate) const TRAP_ORIG_IGNORES: &str = "__RUBASH_TRAP_ORIG_IGN";
/// Signals whose dispositions were reset on subshell entry. GNU
/// reset_signal_handlers (trap.c:1480 -> reset_or_restore_signal_handlers:1433)
/// restores the original handler for trapped signals but deliberately does
/// not free trap_list strings, so `trap` in a subshell still lists the
/// parent's traps; the reset set records which entries may not fire.
/// `pub(crate)` alongside [`TRAP_PREFIX`] for the same fast-driver gate,
/// which skips exactly this metadata key (and [`TRAP_ORIG_IGNORES`]).
pub(crate) const TRAP_RESET: &str = "__RUBASH_TRAP_RESET";
pub(crate) const EX_USAGE: i32 = 2;
/// GNU's internal EX_USAGE (builtins/common.h): what trap.def returns before
/// execute_cmd.c:4886-4888 converts it for `$?`. It exceeds EX_SHERRBASE,
/// which is exactly what the execute_trap wrapper's special_builtin_failed
/// gate keys on, so the POSIX reset-by-name usage failure surfaces THIS raw
/// status and the wrapper converts it back to the user-visible 2 in every
/// non-exiting mode.
const EX_USAGE_INTERNAL: i32 = 258;
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
/// * `target_os = "android"`: bionic resolves the Linux kernel numbering
///   for 1..31 (libc's android module pins SIGBUS=7, SIGUSR1=10,
///   SIGSTKFLT=16, SIGPWR=30, SIGSYS=31, ...), so those slots match the
///   Linux table. The realtime block follows bionic's own RT base:
///   `__libc_current_sigrtmin()` is 41 and `__libc_current_sigrtmax()`
///   is 64 on the Termux oracle (GNU bash 5.3.20 aarch64-linux-android
///   lists `41) SIGRTMIN` through `64) SIGRTMAX`), so signals 32..40 are
///   holes — GNU's `trap -l` skips them and accepts the numeric
///   specifiers silently, exactly like the Linux 32/33 holes — and
///   41..64 carry the signames.c:92-139 arithmetic names
///   (SIGRTMIN..SIGRTMIN+11, SIGRTMAX-11..SIGRTMAX). The unix tests pin
///   these slots against `__libc_current_sigrtmin/max` so a bionic ABI
///   drift fails loudly instead of shipping a lying table.
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

/// Android (bionic) table: Linux kernel numbering for 1..31 (see the table
/// doc above for the libc-crate anchors), holes for 32..40, and the
/// signames.c:92-139 arithmetic names for bionic's 41..64 realtime block.
/// The 64-slot length keeps the `kill -l` NSIG range check identical to
/// Linux. The unix tests (`positional_slots_match_libc`,
/// `android_realtime_and_holes`, `trap_table_agrees_with_kill_table`) pin
/// these slots whenever an android test runner exists.
#[cfg(all(unix, target_os = "android"))]
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
    "",
    "",
    "",
    "",
    "",
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
#[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
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
    let mut env_vars = VarTable::default();
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
    env_vars: &mut VarTable,
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
                    writeln!(stderr, "{prefix}trap: -{option}: invalid option")?;
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
        print_traps(
            env_vars,
            selected,
            stdout,
            print_actions,
            print_trap_commands,
        )?;
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
        // POSIX mode only takes the first_signal branch: the reset spec must
        // be all digits ("trap 15"), so `trap USR1` falls to the else branch
        // and returns EX_USAGE (bash posix-mode notes: use `trap -` to
        // reset). The EX_USAGE of a special builtin also exits the
        // non-interactive POSIX shell — the execute_trap wrapper marks
        // special_builtin_failed for that, so return GNU's RAW internal
        // status here (> EX_SHERRBASE is what execute_cmd.c:4886-4888 keys
        // the special-builtin failure on); the wrapper converts it back to
        // the user-visible 2 in every non-exiting mode.
        let digits_only = !action.is_empty() && action.bytes().all(|b| b.is_ascii_digit());
        if let Some(signal) = normalize_signal(action) {
            if !digits_only && crate::builtins::set::shell_option_enabled(env_vars, "posix") {
                print_usage(stderr)?;
                return Ok(EX_USAGE_INTERNAL);
            }
            remove_trap(env_vars, &signal);
            #[cfg(unix)]
            crate::builtins::kill::reconcile_kernel_trap_dispositions(env_vars);
            return Ok(0);
        }
        print_usage(stderr)?;
        return Ok(EX_USAGE);
    }

    let signals = normalized_signals(&args[index..], env_vars, stderr)?;
    if action == "-" || action == "0" {
        for signal in signals.signals {
            remove_trap(env_vars, &signal);
            // rubash#2347: GNU trap_builtin's REVERT branch reinstalls
            // the fatal SIGINT handler even when SIGINT was ignored on
            // entry (builtins/trap.def:228-251, citing POSIX interp
            // 751) — the single exception to the hard-ignore no-op
            // remove_trap just applied. SIGQUIT is the opposite
            // hard-code (forced SIG_IGN, already matched) and TERM /
            // USR1 reverts on entry-ignored signals stay no-ops in both
            // shells, so this keys on SIGINT alone, and only when it is
            // actually entry-ignored (an ordinary reset already landed
            // through the reconcile below).
            if action == "-" && signal == "SIGINT" && is_hard_ignored(env_vars, "SIGINT") {
                #[cfg(unix)]
                crate::builtins::kill::revert_entry_ignored_sigint(
                    env_vars.get("__RUBASH_INTERACTIVE").map(String::as_str) == Some("1"),
                );
            }
        }
        // Bash treats the numeric zero action as clearing the EXIT trap too.
        if action == "0" {
            remove_trap(env_vars, "EXIT");
        }
        #[cfg(unix)]
        crate::builtins::kill::reconcile_kernel_trap_dispositions(env_vars);
        return Ok(i32::from(signals.invalid));
    }

    for signal in signals.signals {
        set_trap(env_vars, &signal, action);
    }
    // GNU trap.c trap_builtin installs/removes the kernel disposition
    // synchronously inside the builtin (sig.c:830 set_signal_handler for a
    // new action; restore_original_signal on reset). Keep the unix kernel
    // state in lockstep with the trap table (rubash#226): the very next
    // blocking command must see the new disposition.
    #[cfg(unix)]
    crate::builtins::kill::reconcile_kernel_trap_dispositions(env_vars);
    Ok(i32::from(signals.invalid))
}

pub fn list_first_signal_for_sed() -> &'static str {
    "SIGHUP"
}

pub(crate) fn take_exit_trap(env_vars: &mut VarTable) -> Option<String> {
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

/// Allocation-light `get_trap_action` for per-call hot paths (function
/// entry consults DEBUG and ERR on every call, rubash#1121): the reset
/// membership test is a split-scan of the same `:`-joined TRAP_RESET
/// value `reset_trap_signals` collects into a BTreeSet — identical
/// semantics (a reset signal reads as unset), without building the set.
pub(crate) fn get_trap_action_fast(
    env_vars: &HashMap<String, String>,
    signal: &str,
) -> Option<String> {
    if env_vars
        .get(TRAP_RESET)
        .is_some_and(|value| value.split(':').any(|reset| reset == signal))
    {
        return None;
    }
    env_vars.get(&trap_key(signal)).cloned()
}

/// Borrowing, allocation-free `get_trap_action` for the per-firing trap
/// dispatches (rubash#375). GNU `_run_trap_internal` (trap.c:1154-1155)
/// reads `trap_list[sig]` — a pointer — and savestrings it once; it never
/// builds a container to decide whether the trap is live. `reset_trap_
/// signals` collects a BTreeSet per call and `trap_key` formats a String
/// per call; under a DEBUG trap that ran once per firing (~2300 firings
/// per bats gather, 70000+ in the probe loop). Semantics identical to
/// `get_trap_action`: reset-list signals read as unset, empty actions
/// read as unset.
pub(crate) fn active_trap_action_ref<'a>(
    env_vars: &'a HashMap<String, String>,
    signal: &str,
) -> Option<&'a str> {
    if env_vars
        .get(TRAP_RESET)
        .is_some_and(|value| value.split(':').any(|reset| reset == signal))
    {
        return None;
    }
    let mut key_buffer = [0u8; 32];
    let prefix = TRAP_PREFIX.as_bytes();
    if prefix.len() + signal.len() > key_buffer.len() {
        return env_vars
            .get(&trap_key(signal))
            .map(String::as_str)
            .filter(|action| !action.is_empty());
    }
    key_buffer[..prefix.len()].copy_from_slice(prefix);
    key_buffer[prefix.len()..prefix.len() + signal.len()].copy_from_slice(signal.as_bytes());
    let key = std::str::from_utf8(&key_buffer[..prefix.len() + signal.len()]).ok()?;
    env_vars
        .get(key)
        .map(String::as_str)
        .filter(|action| !action.is_empty())
}

/// rubash#186: non-cloning `get_trap_action(...).is_some_and(|a| !a.is_empty())`
/// for the per-command hot path. get_trap_action builds a BTreeSet from the
/// reset list and clones the whole action string on every call; the ast_exec
/// preamble only needs the presence test, so share its exact semantics
/// (reset-list signals are absent; empty actions count as unset) without the
/// allocations.
pub(crate) fn has_active_trap_action(
    env_vars: &HashMap<String, String>,
    signal: &str,
    action_key: &str,
) -> bool {
    if env_vars
        .get(TRAP_RESET)
        .is_some_and(|value| value.split(':').any(|reset| reset == signal))
    {
        return false;
    }
    env_vars
        .get(action_key)
        .is_some_and(|action| !action.is_empty())
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
            Some(signal) => signals.push(signal),
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

fn normalize_signal(signal: &str) -> Option<String> {
    let signal = signal.to_ascii_uppercase();
    match signal.as_str() {
        "0" | "EXIT" => return Some("EXIT".to_string()),
        "DEBUG" => return Some("DEBUG".to_string()),
        "ERR" => return Some("ERR".to_string()),
        "RETURN" => return Some("RETURN".to_string()),
        _ => {}
    }

    if let Ok(number) = signal.parse::<usize>() {
        return number.checked_sub(1).and_then(|index| {
            SIGNALS.get(index).copied().map(|name| {
                // GNU trap.c keys a nameless (hole) slot by its number:
                // decode_signal has no name for Linux 32/33 (bionic
                // 32..40), yet the trap table holds those signals under
                // the bare number — `trap '' 32` lists back as
                // `trap -- '' 32`, and an entry-ignored hole (bionic
                // signal 38 in rubash#1124) lists the same way. Returning
                // the empty slot name here keyed every hole trap under
                // "", so listings dropped them and `-p 38` printed an
                // empty signal name.
                if name.is_empty() {
                    number.to_string()
                } else {
                    name.to_string()
                }
            })
        });
    }

    let name = signal.strip_prefix("SIG").unwrap_or(&signal);
    if name.is_empty() {
        return None;
    }
    SIGNALS
        .iter()
        .copied()
        .find(|candidate| !candidate.is_empty() && candidate.strip_prefix("SIG") == Some(name))
        .map(str::to_string)
}

/// GNU trap.c set_signal for the special traps. `pub(crate)` for the
/// source builtin's entry-trap restore (source.def:216's
/// uw_maybe_set_debug_trap -> trap.c:737-743 trap_if_untrapped ->
/// set_signal).
pub(crate) fn set_trap(env_vars: &mut VarTable, signal: &str, action: &str) {
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

/// GNU trap.c restore_default_signal for the special traps. `pub(crate)`
/// for the source builtin's entry-trap save (source.def:214-215).
pub(crate) fn remove_trap(env_vars: &mut VarTable, signal: &str) {
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

fn store_orig_ignored_signals(env_vars: &mut VarTable, mut signals: Vec<String>) {
    signals.sort();
    signals.dedup();
    if signals.is_empty() {
        env_vars.remove(TRAP_ORIG_IGNORES);
    } else {
        env_vars.insert(TRAP_ORIG_IGNORES.to_string(), signals.join(":"));
    }
}

/// Trap-table key for an entry-ignored signal number. Named slots key by
/// name; hole slots (no name in SIGNALS — Linux 32/33, bionic 32..40)
/// key by the bare number, exactly as normalize_signal keys a numeric
/// operand and canonical_trap_order lists them. Dropping holes here lost
/// the entry-ignored bionic signal 38 of rubash#1124: GNU lists it as
/// `trap -- '' 38`, rubash listed nothing.
fn entry_ignored_key(number: i32) -> Option<String> {
    let name = SIGNALS.get(number.checked_sub(1)? as usize).copied()?;
    if name.is_empty() {
        Some(number.to_string())
    } else {
        Some(name.to_string())
    }
}

fn add_orig_ignored_signal(env_vars: &mut VarTable, signal: &str) {
    let mut signals = orig_ignored_signals(env_vars);
    if signals.iter().any(|ignored| ignored == signal) {
        return;
    }
    signals.push(signal.to_string());
    store_orig_ignored_signals(env_vars, signals);
}

/// Canonicalize the transported entry-ignore set (TRAP_ORIG_IGNORES) to
/// the trap table's number-derived keys, in place. GNU keeps the entry
/// dispositions as original_signals[] indexed by signal NUMBER
/// (trap.c:136-150) and decode_signal (trap.c:236) resolves every
/// operand spelling to that number; names are a display spelling only.
/// The transport's own writer emits this table's canonical names, but a
/// foreign parent may spell an entry any way the signal can be written:
/// a bare number — the ONLY spelling for a bionic hole, whose table has
/// no name to give (rubash#1124/#2193) — or a short name (`USR1` for
/// `SIGUSR1`). Consumption keyed on the raw spelling dropped those
/// entries onto a phantom key no lookup ever uses: the rebuild below
/// inserted `__RUBASH_TRAP_38` / `__RUBASH_TRAP_USR1` while every reader
/// (canonical_trap_order, normalized operands, is_hard_ignored) keys by
/// the canonical slot key. Map each entry through normalize_signal —
/// numeric payloads land on their slot, name payloads resolve through
/// SIGNALS to the same slot — and store the result back, so the rebuild,
/// the hard-ignore checks, and the child-boundary re-export all read one
/// canonical set (store_orig_ignored_signals sorts/dedups, so dual
/// spellings of one signal collapse). Entries naming no signal in this
/// platform's table, and the EXIT/DEBUG/ERR/RETURN pseudo-signals (never
/// transportable dispositions — the writer excludes them), are dropped:
/// they key no trap here.
fn canonicalize_orig_ignored_signals(env_vars: &mut VarTable) {
    if !env_vars.contains_key(TRAP_ORIG_IGNORES) {
        return;
    }
    let canonical: Vec<String> = orig_ignored_signals(env_vars)
        .into_iter()
        .filter_map(|entry| normalize_signal(&entry))
        .filter(|key| !matches!(key.as_str(), "EXIT" | "DEBUG" | "ERR" | "RETURN"))
        .collect();
    store_orig_ignored_signals(env_vars, canonical);
}

/// Seed the traps a shell inherits from its environment at startup. WSL's
/// init leaves SIGRTMIN ignored for every child process, so the GNU 5.3.0
/// contract baseline lists "trap -- '' SIGRTMIN" in every shell; model that
/// inherited ignore here. Ignored-disposition traps carried in through the
/// environment (a parent shell's "trap '' SIG" reaching a fresh rubash
/// child) become hard-ignored for this shell, matching trap.c's
/// original_signals == SIG_IGN startup handling.
pub(crate) fn seed_startup_traps(env_vars: &mut VarTable) {
    // rubash#2193: the transported set is consumed here, at startup —
    // key it by signal number before the fold and the rebuild read it,
    // so a bare-number or short-name payload reconstructs the same
    // entry-ignore a full-name payload does.
    canonicalize_orig_ignored_signals(env_vars);
    // rubash#1124: fold this process's ENTRY dispositions into the
    // transported set before the rebuild below. GNU trap.c
    // initialize_traps snapshots original_signals[] at startup: a
    // signal ignored on entry (launcher/harness SIG_IGN, e.g. SIGUSR1,
    // SIGPIPE, SIGXFSZ, SIGRTMIN+4) is SIG_HARD_IGNORE — `trap -p`
    // lists it as `trap -- '' SIG` and set/reset are silent no-ops.
    // The env transports only cover rubash-to-rubash spawns; a foreign
    // parent's ignores are visible solely through the kernel probe.
    // Idempotent (orig list is sorted/deduped), so subshell executors
    // re-entering this constructor keep the same set.
    for number in crate::builtins::kill::entry_ignored_signal_numbers() {
        // rubash#1035 vs the golden trap-signal-matrix: an async child's
        // entry SIGINT/SIGQUIT ignore comes from the spawn site's
        // setup_async_signals, not from inheritance — it must NOT become
        // a hard ignore, or a compound background body's later `trap`
        // would be silently refused (the spawn site deliberately merges
        // ORIG_IGN only for exec-like bodies; compound bodies stay
        // trappable). Foreign entry ignores still fold to hard here, and
        // exec-like bodies still arrive hard via the ORIG_IGN transport.
        #[cfg(unix)]
        if crate::builtins::kill::async_child_entry()
            && (number == libc::SIGINT as i32 || number == libc::SIGQUIT as i32)
        {
            continue;
        }
        if let Some(key) = entry_ignored_key(number) {
            add_orig_ignored_signal(env_vars, &key);
        }
    }
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
    // rubash#546 (cell 1): a fresh shell lists NOTHING for signals the
    // SHELL ignores on its own behalf. The old hard-seed here planted
    // `trap -- '' SIGRTMIN` in every fresh shell on the premise that WSL's
    // init leaves SIGRTMIN ignored for every child — a premise today's
    // GNU 5.3.0 oracle falsifies (`bash -c 'trap'` prints an empty table
    // across -c/-i/--norc/POSIXLY_CORRECT probes, 2026-10-12). GNU lists a
    // signal only when it was ignored BEFORE startup — an inherited SIG_IGN
    // disposition (probe: a parent `trap '' RTMIN` makes the child list
    // `trap -- '' SIGRTMIN`) — and that path is served by the
    // TRAP_ORIG_IGNORES / per-signal-"" transports above, not by a seed.
    //
    // Async-child tail (see the entry-fold skip above): GNU's async child
    // still LISTS `trap -- '' SIGINT` — the ignore is a real trap-table
    // entry with an empty action, just not a hard one. Seed those empty
    // entries here, AFTER the loop above that promotes empty entries to
    // ORIG_IGN, so they stay listed-but-trappable. A body that already
    // set a real action (or an exec-like body that arrived hard via the
    // transport) keeps its entry untouched.
    #[cfg(unix)]
    if crate::builtins::kill::async_child_entry() {
        for signal in ["INT", "QUIT"] {
            let key = trap_key(signal);
            if env_vars.get(&key).is_none() {
                env_vars.insert(key, String::new());
                let mut signals = trap_list(env_vars);
                signals.insert(signal.to_string());
                store_trap_list(env_vars, signals);
            }
        }
    }
}

/// A script executed through the same-shell ENOEXEC path runs in a fresh
/// shell context: every real signal the parent currently ignores becomes an
/// original ignore for the script (the fork child inherits the SIG_IGN
/// disposition, and trap.c re-derives SIG_HARD_IGNORE from it). Runtime
/// ignores of plain subshells stay mutable; only this shell-entry boundary
/// freezes them.
pub(crate) fn mark_startup_ignores(env_vars: &mut VarTable) {
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
pub(crate) fn maybe_restore_debug_trap(env_vars: &mut VarTable, action: String) {
    if env_vars.contains_key(&trap_key("DEBUG")) {
        return;
    }
    set_trap(env_vars, "DEBUG", &action);
}

/// GNU execute_cmd.c uw_maybe_set_error_trap: at function exit the saved ERR
/// action is restored only when the body did not set a new one.
pub(crate) fn maybe_restore_error_trap(env_vars: &mut VarTable, action: String) {
    if env_vars.contains_key(&trap_key("ERR")) {
        return;
    }
    set_trap(env_vars, "ERR", &action);
}

/// GNU execute_cmd.c restore_default_signal(ERROR_TRAP) at function entry
/// (execute_cmd.c:5279-5286): the inherited ERR trap is removed for the
/// function body unless error_trace_mode (set -E) makes functions inherit
/// it — so `trap -p ERR` inside the body reports nothing and the trap table
/// matches the effective (non-inherited) state.
pub(crate) fn clear_error_trap(env_vars: &mut VarTable) {
    env_vars.remove(&trap_key("ERR"));
    let mut signals = trap_list(env_vars);
    signals.remove("ERR");
    store_trap_list(env_vars, signals);
}

/// GNU execute_cmd.c:5291-5303 (execute_function): shell functions inherit
/// the RETURN trap only when function tracing is on globally (functrace) or
/// for the function itself (`declare -ft`, trace_p) — and never while the
/// DEBUG trap is executing (signal_in_progress (DEBUG_TRAP)). Otherwise the
/// inherited action is removed for the body (restore_default_signal), so
/// `trap -p RETURN` inside an untraced body reports nothing. A trap the
/// body installs is global (set_signal) and survives the call.
pub(crate) fn clear_return_trap(env_vars: &mut VarTable) {
    env_vars.remove(&trap_key("RETURN"));
    let mut signals = trap_list(env_vars);
    signals.remove("RETURN");
    store_trap_list(env_vars, signals);
}

/// GNU execute_cmd.c uw_maybe_set_return_trap (trap.c:786-789,
/// trap_if_untrapped): at function exit the saved RETURN action is restored
/// only when the body left the trap untrapped — a body install (or a body
/// `trap - RETURN` after an inherited... no: entry removal means a body
/// `trap - RETURN` is a no-op and the saved action comes back, GNU probe
/// v4) persists. Restoring only when unset is exactly trap_if_untrapped.
pub(crate) fn maybe_restore_return_trap(env_vars: &mut VarTable, action: String) {
    if env_vars.contains_key(&trap_key("RETURN")) {
        return;
    }
    set_trap(env_vars, "RETURN", &action);
}

/// GNU execute_cmd.c restore_default_signal(DEBUG_TRAP) at function entry:
/// remove the inherited DEBUG trap for the function body.
pub(crate) fn clear_debug_trap(env_vars: &mut VarTable) {
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
    print_commands: bool,
) -> io::Result<()>
where
    W: Write,
{
    // GNU builtins/trap.def display_traps walks the trap table in signal
    // number order (trap.h: EXIT_TRAP == 0, real signals 1..NSIG-1, then
    // DEBUG_TRAP == NSIG, ERROR_TRAP == NSIG+1, RETURN_TRAP == NSIG+2), so
    // an unselected listing prints EXIT first, then every set signal trap in
    // numeric order, then DEBUG/ERROR/RETURN.
    let posix = crate::builtins::set::shell_option_enabled(env_vars, "posix");
    // POSIX mode `trap -p` with no signal operands lists EVERY signal
    // (trap.def display_traps under POSIX_MODE iterates the whole table,
    // printing `trap -- - NAME` for dispositions at SIG_DFL) — 68 lines
    // on this platform (rubash#1054). Bare `trap` keeps the trapped-only
    // listing in both modes.
    let list_all = posix && print_commands && selected.is_none();
    let signals: Vec<String> = match selected {
        Some(signals) => signals.to_vec(),
        None if list_all => canonical_trap_order(),
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
                writeln!(
                    stdout,
                    "trap -- {} {}",
                    shell_quote(action),
                    display_signal_name(&signal, posix)
                )?;
            }
        } else if posix && (selected.is_some() || list_all) {
            // GNU trap.def display_traps (POSIX_MODE): a signal with no
            // trap still lists as `trap -- - NAME` (bash posix-mode notes:
            // `trap -p` shows signals whose disposition is SIG_DFL, not
            // only trapped ones) — for explicitly selected signals and for
            // the whole-table `-p` listing (rubash#1054). The default
            // (non-posix) display never grows entries.
            writeln!(stdout, "trap -- - {}", display_signal_name(&signal, posix))?;
        }
    }
    Ok(())
}

/// The signal name `trap -p` prints for `signal`. POSIX mode strips the
/// `SIG` prefix (bash posix-mode notes: `trap` displays signal names
/// without the leading SIG); the default mode keeps the stored SIGx
/// spelling. EXIT/DEBUG/ERR/RETURN carry no prefix and pass through.
fn display_signal_name(signal: &str, posix: bool) -> &str {
    if posix {
        signal.strip_prefix("SIG").unwrap_or(signal)
    } else {
        signal
    }
}

/// Trap-table iteration order matching GNU trap.c display_traps: EXIT
/// (index 0), real signals in numeric order, then DEBUG, ERROR, RETURN
/// (rubash keys the ERR trap as "ERR").
fn canonical_trap_order() -> Vec<String> {
    std::iter::once("EXIT".to_string())
        .chain(SIGNALS.iter().enumerate().map(|(index, signal)| {
            // Slots 32/33 have no glibc name (SIGNALS holds ""); GNU's
            // whole-table listings print them as bare numbers
            // (`trap -- - 32`), so the order keeps them under their
            // signal number (rubash#1054).
            if signal.is_empty() {
                (index + 1).to_string()
            } else {
                signal.to_string()
            }
        }))
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
pub(crate) fn reset_for_subshell(env_vars: &mut VarTable) {
    reset_for_subshell_inner(env_vars, false);
}

/// Reset caught signal traps when entering a forked `( )` / `f() ( )`
/// subshell child. Identical to [`reset_for_subshell`] except that the ERR
/// trap follows GNU's error_trace_mode (`set -E`) inheritance, the way
/// DEBUG/RETURN follow functrace: a subshell child keeps the parent's ERR
/// trap when `-E` is on, so `set -E; trap 'echo ERR' ERR; ( false )` fires
/// ERR for the inner `false` and again for the failing subshell command
/// (rubash#702). Command substitutions use this variant too: bash's `-E`
/// contract names them explicitly (rubash#1133). The pipeline-element
/// and `read` builtin copy paths keep the unconditional
/// [`reset_for_subshell`]; their ERR inheritance is modeled at their own
/// call sites.
pub(crate) fn reset_for_subshell_child(env_vars: &mut VarTable) {
    reset_for_subshell_inner(env_vars, true);
}

fn reset_for_subshell_inner(env_vars: &mut VarTable, inherit_errtrace: bool) {
    let mut reset = reset_trap_signals(env_vars);
    for signal in trap_list(env_vars) {
        // DEBUG and RETURN are tracing hooks, not OS signal dispositions;
        // functrace/extdebug controls their inheritance separately
        // (subshell setup -> trap.c:1588 reset_or_restore_signal_handlers:
        // a subshell keeps the DEBUG trap only when function_trace_mode is
        // set). Without functrace the inherited hook is reset here —
        // get_trap_action then reports it unset, so nothing fires, while
        // the listing still shows the entry. A trap ARMED INSIDE the
        // subshell re-arms the disposition (set_trap removes the reset
        // marker, GNU set_signal) and fires for the remaining subshell
        // commands (rubash#345: `echo "$(trap 'echo DC' DEBUG; echo x)"`
        // prints DC then x).
        if matches!(signal.as_str(), "DEBUG" | "RETURN") {
            if crate::builtins::set::shell_option_enabled(env_vars, "functrace") {
                continue;
            }
            if env_vars
                .get(&trap_key(&signal))
                .is_some_and(|action| !action.is_empty())
            {
                reset.insert(signal);
            }
            continue;
        }
        // ERR inherits under `set -E` exactly as DEBUG/RETURN inherit under
        // `set -T`: the child keeps the parent's ERR action live, so its
        // failing commands fire the trap. An ERR trap armed inside the
        // subshell re-arms through set_trap regardless (rubash#702).
        if signal == "ERR" && inherit_errtrace {
            if crate::builtins::set::shell_option_enabled(env_vars, "errtrace") {
                continue;
            }
            let key = trap_key(&signal);
            if env_vars.get(&key).is_some_and(|action| !action.is_empty()) {
                reset.insert(signal);
            }
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

fn store_reset_traps(env_vars: &mut VarTable, signals: BTreeSet<String>) {
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

fn store_trap_list(env_vars: &mut VarTable, signals: BTreeSet<String>) {
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
    use super::*;

    /// Bash posix-mode notes item 68: the reset spec must be all digits, so
    /// `trap USR1` is a usage violation. The POSIX path returns GNU's RAW
    /// internal EX_USAGE (258) — the execute_trap wrapper keys
    /// special_builtin_failed on > EX_SHERRBASE and converts it to the
    /// user-visible 2; the default mode keeps the historical reset-by-name
    /// (it removes the USR1 trap and succeeds).
    #[test]
    fn posix_reset_by_name_returns_internal_usage() {
        let mut env_vars = VarTable::default();
        env_vars.insert("__RUBASH_SETOPT_posix".to_string(), "1".to_string());
        let mut out = Vec::new();
        let mut err = Vec::new();
        let status =
            execute_with_io(&["USR1".to_string()], &mut env_vars, &mut out, &mut err).unwrap();
        assert_eq!(status, EX_USAGE_INTERNAL);
        assert!(String::from_utf8(err).unwrap().contains("usage: trap"));

        let mut env_vars = VarTable::default();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let status =
            execute_with_io(&["USR1".to_string()], &mut env_vars, &mut out, &mut err).unwrap();
        assert_eq!(status, 0);

        // The all-digit reset form keeps working in POSIX mode.
        let mut env_vars = VarTable::default();
        env_vars.insert("__RUBASH_SETOPT_posix".to_string(), "1".to_string());
        let mut out = Vec::new();
        let mut err = Vec::new();
        let status =
            execute_with_io(&["15".to_string()], &mut env_vars, &mut out, &mut err).unwrap();
        assert_eq!(status, 0);
    }

    /// Bash posix-mode notes item 67: `trap` displays signal names without
    /// the leading SIG; the default mode keeps the stored SIGx spelling and
    /// pseudo-signals (EXIT/ERR) pass through either way.
    #[test]
    fn posix_trap_display_strips_the_sig_prefix() {
        assert_eq!(display_signal_name("SIGUSR1", true), "USR1");
        assert_eq!(display_signal_name("SIGUSR1", false), "SIGUSR1");
        assert_eq!(display_signal_name("EXIT", true), "EXIT");
        assert_eq!(display_signal_name("ERR", false), "ERR");
    }

    /// rubash#2193: startup consumption keys the transported entry-ignore
    /// set by signal number. A short-name payload and its full-name
    /// spelling collapse onto the one canonical key; entries naming no
    /// signal in this table and the pseudo-signals are dropped.
    #[test]
    fn orig_ignored_transport_canonicalizes_by_number() {
        let mut env_vars = VarTable::default();
        env_vars.insert(
            TRAP_ORIG_IGNORES.to_string(),
            "USR1:SIGUSR1:BOGUS:EXIT:0".to_string(),
        );
        super::canonicalize_orig_ignored_signals(&mut env_vars);
        assert_eq!(
            super::orig_ignored_signals(&env_vars),
            vec!["SIGUSR1".to_string()]
        );
    }

    /// rubash#2193 on the Linux-numbering tables (glibc and the Windows
    /// wire format): signal 38's slot is named, so a bare-number payload
    /// and the full-name spelling are the same entry.
    #[cfg(any(target_os = "linux", not(unix)))]
    #[test]
    fn orig_ignored_transport_number_keys_to_named_slot() {
        let mut env_vars = VarTable::default();
        env_vars.insert(TRAP_ORIG_IGNORES.to_string(), "38".to_string());
        super::canonicalize_orig_ignored_signals(&mut env_vars);
        assert_eq!(
            super::orig_ignored_signals(&env_vars),
            vec!["SIGRTMIN+4".to_string()]
        );

        let mut env_vars = VarTable::default();
        env_vars.insert(TRAP_ORIG_IGNORES.to_string(), "38:SIGRTMIN+4".to_string());
        super::canonicalize_orig_ignored_signals(&mut env_vars);
        assert_eq!(
            super::orig_ignored_signals(&env_vars),
            vec!["SIGRTMIN+4".to_string()]
        );
    }

    /// rubash#2193 on bionic: signal 38 is a hole (32..40 nameless), so
    /// the bare number IS the canonical key and survives unchanged.
    #[cfg(target_os = "android")]
    #[test]
    fn orig_ignored_transport_number_keys_to_hole_slot() {
        let mut env_vars = VarTable::default();
        env_vars.insert(TRAP_ORIG_IGNORES.to_string(), "38".to_string());
        super::canonicalize_orig_ignored_signals(&mut env_vars);
        assert_eq!(
            super::orig_ignored_signals(&env_vars),
            vec!["38".to_string()]
        );
    }

    /// rubash#2193: after seeding, a short-name transport payload has
    /// rebuilt the canonical trap-table entry and is hard-ignored under
    /// its canonical key (set/reset become silent no-ops).
    #[test]
    fn seed_startup_traps_rebuilds_short_name_transport() {
        let mut env_vars = VarTable::default();
        env_vars.insert(TRAP_ORIG_IGNORES.to_string(), "USR1".to_string());
        super::seed_startup_traps(&mut env_vars);
        assert_eq!(
            env_vars.get(&super::trap_key("SIGUSR1")),
            Some(&String::new())
        );
        assert!(super::trap_list(&env_vars).contains("SIGUSR1"));
        assert!(super::is_hard_ignored(&env_vars, "SIGUSR1"));
    }

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
                    Some(expected),
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
            assert_eq!(
                super::normalize_signal(name),
                Some(name.to_string()),
                "{name}"
            );
            let bare = name.trim_start_matches("SIG");
            assert_eq!(
                super::normalize_signal(bare),
                Some(name.to_string()),
                "{bare}"
            );
            assert_eq!(
                super::normalize_signal(&constant.to_string()),
                Some(name.to_string()),
                "numeric {name}"
            );
        }
    }

    /// Linux keeps the RT arithmetic names (signames.c:92-139) and the
    /// 32/33 holes GNU's trap -l skips.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_realtime_and_holes() {
        assert_eq!(super::normalize_signal("RTMIN"), Some("SIGRTMIN".into()));
        assert_eq!(
            super::normalize_signal("RTMIN+1"),
            Some("SIGRTMIN+1".into())
        );
        assert_eq!(super::normalize_signal("34"), Some("SIGRTMIN".into()));
        assert_eq!(super::normalize_signal("64"), Some("SIGRTMAX".into()));
        assert_eq!(super::normalize_signal("STKFLT"), Some("SIGSTKFLT".into()));
        assert_eq!(super::normalize_signal("17"), Some("SIGCHLD".into()));
        // Holes key by bare number (GNU: `trap '' 32` lists as
        // `trap -- '' 32`; rubash#1124).
        assert_eq!(super::normalize_signal("32"), Some("32".into()));
        assert_eq!(super::normalize_signal("33"), Some("33".into()));
        assert_eq!(super::normalize_signal("65"), None);
        assert_eq!(super::entry_ignored_key(32), Some("32".into()));
        assert_eq!(super::entry_ignored_key(38), Some("SIGRTMIN+4".into()));
        assert_eq!(super::entry_ignored_key(65), None);
    }

    /// Android (bionic): the realtime block sits at 41..64
    /// (`__libc_current_sigrtmin/max`), with 32..40 holes GNU's trap -l
    /// skips (rubash#1036).
    #[cfg(target_os = "android")]
    #[test]
    fn android_realtime_and_holes() {
        assert_eq!(super::normalize_signal("RTMIN"), Some("SIGRTMIN".into()));
        assert_eq!(
            super::normalize_signal("RTMIN+7"),
            Some("SIGRTMIN+7".into())
        );
        assert_eq!(super::normalize_signal("RTMAX"), Some("SIGRTMAX".into()));
        assert_eq!(
            super::normalize_signal("RTMAX-1"),
            Some("SIGRTMAX-1".into())
        );
        assert_eq!(super::normalize_signal("41"), Some("SIGRTMIN".into()));
        assert_eq!(super::normalize_signal("52"), Some("SIGRTMIN+11".into()));
        assert_eq!(super::normalize_signal("53"), Some("SIGRTMAX-11".into()));
        assert_eq!(super::normalize_signal("64"), Some("SIGRTMAX".into()));
        assert_eq!(super::normalize_signal("STKFLT"), Some("SIGSTKFLT".into()));
        assert_eq!(super::normalize_signal("17"), Some("SIGCHLD".into()));
        // Bionic holes 32..40 key by bare number; signal 38 is the
        // entry-ignored hole of rubash#1124 (GNU: `trap -- '' 38`).
        assert_eq!(super::normalize_signal("32"), Some("32".into()));
        assert_eq!(super::normalize_signal("40"), Some("40".into()));
        assert_eq!(super::normalize_signal("65"), None);
        assert_eq!(super::entry_ignored_key(38), Some("38".into()));
        assert_eq!(super::entry_ignored_key(41), Some("SIGRTMIN".into()));
    }

    /// Darwin/BSD: reordered slots follow libc, Linux-only names are gone.
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    #[test]
    fn darwin_family_slots() {
        assert_eq!(super::normalize_signal("EMT"), Some("SIGEMT".into()));
        assert_eq!(super::normalize_signal("INFO"), Some("SIGINFO".into()));
        assert_eq!(super::normalize_signal("17"), Some("SIGSTOP".into()));
        assert_eq!(super::normalize_signal("18"), Some("SIGTSTP".into()));
        assert_eq!(super::normalize_signal("19"), Some("SIGCONT".into()));
        assert_eq!(super::normalize_signal("20"), Some("SIGCHLD".into()));
        assert_eq!(super::normalize_signal("30"), Some("SIGUSR1".into()));
        assert_eq!(super::normalize_signal("31"), Some("SIGUSR2".into()));
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
