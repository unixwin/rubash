//! kill module.
//!
//! GNU Bash source ownership:
// - builtins/kill.def

use std::io::{self, Write};

/// Signal table: `(signal number, number as string, name without `SIG`)`.
///
/// GNU bash builds this table per-target at configure time: Makefile.in
/// compiles support/mksignames.c against the TARGET machine's `<signal.h>`
/// (Makefile.in:599-617 CREATED_SUPPORT/signames.h, rules at :775-804) and
/// support/signames.c:69 `initialize_signames()` fills `signal_names[]`
/// indexed by signal number, guarded per signal — SIGPWR at signames.c:278,
/// SIGSTKFLT at :292, SIGEMT at :353, SIGINFO at :442 — with the
/// SIGRTMIN..SIGRTMAX arithmetic names generated at :92-139
/// (`signal_names[rtmin+i] = "SIGRTMIN+%d"`). This is the Rust compile-time
/// equivalent of that generator: numbers come from the libc crate, which
/// resolves them per target.
///
/// * `cfg(not(unix))`: the literal Linux x86 table is the wire format of
///   the Windows file-mailbox protocol (other rubash processes write these
///   numbers into `%TEMP%\rubash-signals`) and of the suspend/resume state
///   backend — Linux numbering is the design contract. DO NOT renumber.
/// * `cfg(unix)`: numbers follow the target platform via libc (Darwin:
///   7=EMT, 10=BUS, 12=SYS, 17=STOP, 18=TSTP, 19=CONT, 20=CHLD, 29=INFO,
///   30=USR1, 31=USR2; no STKFLT/PWR/RT signals). Where Linux and the BSD
///   family disagree, both alternatives occupy the same numeric slot
///   (Linux BUS=7 / Darwin EMT=7, Linux USR1=10 / Darwin BUS=10, ...), so
///   the interleaved table stays in numeric order on every platform —
///   `kill -l` walks it in order and GNU's `signal_names[]` is indexed by
///   signal number, so ordering is part of the GNU-compatible surface.
/// * The Linux RT block pins the glibc contract 34..64 as literals:
///   `libc::SIGRTMIN` is a FUNCTION on glibc (linux_like libc, forwarding
///   to `__libc_current_sigrtmin()`), not a const, so it cannot populate a
///   const table. `signal_table_tests::linux_realtime_block_matches_libc`
///   guards the equality at runtime on every Linux CI run.
#[cfg(not(unix))]
const SIGNALS: &[(i32, &str, &str)] = &[
    (1, "1", "HUP"),
    (2, "2", "INT"),
    (3, "3", "QUIT"),
    (4, "4", "ILL"),
    (5, "5", "TRAP"),
    (6, "6", "ABRT"),
    (7, "7", "BUS"),
    (8, "8", "FPE"),
    (9, "9", "KILL"),
    (10, "10", "USR1"),
    (11, "11", "SEGV"),
    (12, "12", "USR2"),
    (13, "13", "PIPE"),
    (14, "14", "ALRM"),
    (15, "15", "TERM"),
    (16, "16", "STKFLT"),
    (17, "17", "CHLD"),
    (18, "18", "CONT"),
    (19, "19", "STOP"),
    (20, "20", "TSTP"),
    (21, "21", "TTIN"),
    (22, "22", "TTOU"),
    (23, "23", "URG"),
    (24, "24", "XCPU"),
    (25, "25", "XFSZ"),
    (26, "26", "VTALRM"),
    (27, "27", "PROF"),
    (28, "28", "WINCH"),
    (29, "29", "IO"),
    (30, "30", "PWR"),
    (31, "31", "SYS"),
    (34, "34", "RTMIN"),
    (35, "35", "RTMIN+1"),
    (36, "36", "RTMIN+2"),
    (37, "37", "RTMIN+3"),
    (38, "38", "RTMIN+4"),
    (39, "39", "RTMIN+5"),
    (40, "40", "RTMIN+6"),
    (41, "41", "RTMIN+7"),
    (42, "42", "RTMIN+8"),
    (43, "43", "RTMIN+9"),
    (44, "44", "RTMIN+10"),
    (45, "45", "RTMIN+11"),
    (46, "46", "RTMIN+12"),
    (47, "47", "RTMIN+13"),
    (48, "48", "RTMIN+14"),
    (49, "49", "RTMIN+15"),
    (50, "50", "RTMAX-14"),
    (51, "51", "RTMAX-13"),
    (52, "52", "RTMAX-12"),
    (53, "53", "RTMAX-11"),
    (54, "54", "RTMAX-10"),
    (55, "55", "RTMAX-9"),
    (56, "56", "RTMAX-8"),
    (57, "57", "RTMAX-7"),
    (58, "58", "RTMAX-6"),
    (59, "59", "RTMAX-5"),
    (60, "60", "RTMAX-4"),
    (61, "61", "RTMAX-3"),
    (62, "62", "RTMAX-2"),
    (63, "63", "RTMAX-1"),
    (64, "64", "RTMAX"),
];

/// Unix half of the signal seam: same triple format and per-platform numeric
/// order as the mailbox table above, but every number is resolved from libc
/// for the compilation target (the mksignames.c contract — see the table
/// doc above for the GNU anchors).
#[cfg(unix)]
const SIGNALS: &[(i32, &str, &str)] = &[
    (libc::SIGHUP as i32, "1", "HUP"),
    (libc::SIGINT as i32, "2", "INT"),
    (libc::SIGQUIT as i32, "3", "QUIT"),
    (libc::SIGILL as i32, "4", "ILL"),
    (libc::SIGTRAP as i32, "5", "TRAP"),
    (libc::SIGABRT as i32, "6", "ABRT"),
    #[cfg(target_os = "linux")]
    (libc::SIGBUS as i32, "7", "BUS"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGEMT as i32, "7", "EMT"),
    (libc::SIGFPE as i32, "8", "FPE"),
    (libc::SIGKILL as i32, "9", "KILL"),
    #[cfg(target_os = "linux")]
    (libc::SIGUSR1 as i32, "10", "USR1"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGBUS as i32, "10", "BUS"),
    (libc::SIGSEGV as i32, "11", "SEGV"),
    #[cfg(target_os = "linux")]
    (libc::SIGUSR2 as i32, "12", "USR2"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGSYS as i32, "12", "SYS"),
    (libc::SIGPIPE as i32, "13", "PIPE"),
    (libc::SIGALRM as i32, "14", "ALRM"),
    (libc::SIGTERM as i32, "15", "TERM"),
    #[cfg(target_os = "linux")]
    (libc::SIGSTKFLT as i32, "16", "STKFLT"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGURG as i32, "16", "URG"),
    #[cfg(target_os = "linux")]
    (libc::SIGCHLD as i32, "17", "CHLD"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGSTOP as i32, "17", "STOP"),
    #[cfg(target_os = "linux")]
    (libc::SIGCONT as i32, "18", "CONT"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGTSTP as i32, "18", "TSTP"),
    #[cfg(target_os = "linux")]
    (libc::SIGSTOP as i32, "19", "STOP"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGCONT as i32, "19", "CONT"),
    #[cfg(target_os = "linux")]
    (libc::SIGTSTP as i32, "20", "TSTP"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGCHLD as i32, "20", "CHLD"),
    (libc::SIGTTIN as i32, "21", "TTIN"),
    (libc::SIGTTOU as i32, "22", "TTOU"),
    #[cfg(target_os = "linux")]
    (libc::SIGURG as i32, "23", "URG"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGIO as i32, "23", "IO"),
    (libc::SIGXCPU as i32, "24", "XCPU"),
    (libc::SIGXFSZ as i32, "25", "XFSZ"),
    (libc::SIGVTALRM as i32, "26", "VTALRM"),
    (libc::SIGPROF as i32, "27", "PROF"),
    (libc::SIGWINCH as i32, "28", "WINCH"),
    #[cfg(target_os = "linux")]
    (libc::SIGIO as i32, "29", "IO"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGINFO as i32, "29", "INFO"),
    #[cfg(target_os = "linux")]
    (libc::SIGPWR as i32, "30", "PWR"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGUSR1 as i32, "30", "USR1"),
    #[cfg(target_os = "linux")]
    (libc::SIGSYS as i32, "31", "SYS"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGUSR2 as i32, "31", "USR2"),
    // Linux-only POSIX realtime block, numbered 34..64 per the glibc
    // contract (see the table doc above). Darwin/BSD targets have no RT
    // signals and compile the whole block out, exactly like GNU's
    // `#if defined (SIGRTMIN)` at signames.c:97-139.
    #[cfg(target_os = "linux")]
    (34, "34", "RTMIN"),
    #[cfg(target_os = "linux")]
    (35, "35", "RTMIN+1"),
    #[cfg(target_os = "linux")]
    (36, "36", "RTMIN+2"),
    #[cfg(target_os = "linux")]
    (37, "37", "RTMIN+3"),
    #[cfg(target_os = "linux")]
    (38, "38", "RTMIN+4"),
    #[cfg(target_os = "linux")]
    (39, "39", "RTMIN+5"),
    #[cfg(target_os = "linux")]
    (40, "40", "RTMIN+6"),
    #[cfg(target_os = "linux")]
    (41, "41", "RTMIN+7"),
    #[cfg(target_os = "linux")]
    (42, "42", "RTMIN+8"),
    #[cfg(target_os = "linux")]
    (43, "43", "RTMIN+9"),
    #[cfg(target_os = "linux")]
    (44, "44", "RTMIN+10"),
    #[cfg(target_os = "linux")]
    (45, "45", "RTMIN+11"),
    #[cfg(target_os = "linux")]
    (46, "46", "RTMIN+12"),
    #[cfg(target_os = "linux")]
    (47, "47", "RTMIN+13"),
    #[cfg(target_os = "linux")]
    (48, "48", "RTMIN+14"),
    #[cfg(target_os = "linux")]
    (49, "49", "RTMIN+15"),
    #[cfg(target_os = "linux")]
    (50, "50", "RTMAX-14"),
    #[cfg(target_os = "linux")]
    (51, "51", "RTMAX-13"),
    #[cfg(target_os = "linux")]
    (52, "52", "RTMAX-12"),
    #[cfg(target_os = "linux")]
    (53, "53", "RTMAX-11"),
    #[cfg(target_os = "linux")]
    (54, "54", "RTMAX-10"),
    #[cfg(target_os = "linux")]
    (55, "55", "RTMAX-9"),
    #[cfg(target_os = "linux")]
    (56, "56", "RTMAX-8"),
    #[cfg(target_os = "linux")]
    (57, "57", "RTMAX-7"),
    #[cfg(target_os = "linux")]
    (58, "58", "RTMAX-6"),
    #[cfg(target_os = "linux")]
    (59, "59", "RTMAX-5"),
    #[cfg(target_os = "linux")]
    (60, "60", "RTMAX-4"),
    #[cfg(target_os = "linux")]
    (61, "61", "RTMAX-3"),
    #[cfg(target_os = "linux")]
    (62, "62", "RTMAX-2"),
    #[cfg(target_os = "linux")]
    (63, "63", "RTMAX-1"),
    #[cfg(target_os = "linux")]
    (64, "64", "RTMAX"),
];

/// The platform's SIGCHLD number for dispatch paths that special-case child
/// notifications (GNU keys them on SIGCHLD via the target's signal.h —
/// signames.c initialize_signames). Linux: 17; Darwin/BSD: 20. The
/// non-unix value stays 17 because the Windows file-mailbox wire format
/// speaks Linux numbering by design (see SIGNALS above).
#[cfg(unix)]
pub const SIGCHLD_NUMBER: i32 = libc::SIGCHLD;
/// Non-unix (Windows mailbox wire format, Linux numbering by contract).
#[cfg(not(unix))]
pub const SIGCHLD_NUMBER: i32 = 17;

/// The platform's SIGCONT number. GNU jobs.c:3928 `killpg (jobs[job]->pgrp,
/// SIGCONT)` (start_job/continue_job; also jobs.c:4010) resumes a stopped
/// job with the TARGET's SIGCONT: 18 on Linux, 19 on Darwin/BSD. Non-unix
/// keeps the wire-format 18.
#[cfg(unix)]
pub const SIGCONT_NUMBER: i32 = libc::SIGCONT;
/// Non-unix (Windows mailbox wire format, Linux numbering by contract).
#[cfg(not(unix))]
pub const SIGCONT_NUMBER: i32 = 18;

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
    let Some(first) = args.first().map(String::as_str) else {
        write_kill_usage(stderr)?;
        return Ok(2);
    };

    if matches!(first, "-l" | "-L") {
        if args.len() == 1 || args.get(1).is_some_and(|value| value == "-1") {
            write_signal_list(stdout)?;
            return Ok(0);
        }
        let mut status = 0;
        for value in &args[1..] {
            // GNU builtins/common.c:795-810 display_signal_list: a numeric
            // argument is range-checked against NSIG; an IN-RANGE number
            // whose slot has no name (glibc's reserved RT slots 32/33 —
            // bash's signames array holds SIGJUNK there) prints NOTHING and
            // keeps rc 0. Only an out-of-range number is an error
            // (rubash#230: `kill -l 32` must be silent rc=0).
            if let Ok(mut number) = value.parse::<i32>() {
                if number > 128 {
                    number -= 128;
                }
                if number == 0 {
                    // signal_name slot 0 is bash's EXIT (signames.c), and
                    // common.c prints it for `kill -l 0`.
                    writeln!(stdout, "EXIT")?;
                } else if !(0..=64).contains(&number) {
                    writeln!(
                        stderr,
                        "{}kill: {value}: invalid signal specification",
                        diagnostic_prefix()
                    )?;
                    status = 1;
                } else if let Some(translation) = signal_name(number) {
                    writeln!(stdout, "{translation}")?;
                }
                continue;
            }
            if let Some(translation) = translate_signal(value) {
                writeln!(stdout, "{translation}")?;
            } else {
                writeln!(
                    stderr,
                    "{}kill: {value}: invalid signal specification",
                    diagnostic_prefix()
                )?;
                status = 1;
            }
        }
        return Ok(status);
    }

    let mut signal = 15;
    let mut index = 0;
    let mut operands_start = 0;
    // GNU builtins/kill.def:99,124 saw_signal: once any signal spec is seen
    // (separate or attached), a later `-word` is an operand (process group)
    // rather than another signal specification.
    let mut saw_signal = false;
    while let Some(value) = args.get(index).map(String::as_str) {
        if value == "--" {
            operands_start = index + 1;
            break;
        }
        if value == "-s" || value == "-n" {
            let Some(sigspec) = args.get(index + 1).map(String::as_str) else {
                // GNU builtins/kill.def:130-131: sh_needarg(word) prints
                // "kill: -s: option requires an argument" with the full
                // diagnostic prolog, then returns EXECUTION_FAILURE.
                writeln!(
                    stderr,
                    "{}kill: {value}: option requires an argument",
                    diagnostic_prefix()
                )?;
                return Ok(1);
            };
            if translate_signal(sigspec).is_none() {
                writeln!(
                    stderr,
                    "{}kill: {sigspec}: invalid signal specification",
                    diagnostic_prefix()
                )?;
                return Ok(1);
            }
            signal = signal_number_from_spec(sigspec).unwrap_or(15);
            index += 2;
            operands_start = index;
            saw_signal = true;
            continue;
        }
        // GNU builtins/kill.def:134-142: the signal spec may be attached to
        // the option letter — `-sNAME` when the letter is followed by an
        // alphabetic char, `-nNUM` when followed by a digit.
        let attached = value
            .strip_prefix("-s")
            .filter(|s| s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
            .or_else(|| {
                value
                    .strip_prefix("-n")
                    .filter(|s| s.chars().next().is_some_and(|c| c.is_ascii_digit()))
            });
        if let Some(sigspec) = attached {
            if translate_signal(sigspec).is_none() {
                writeln!(
                    stderr,
                    "{}kill: {sigspec}: invalid signal specification",
                    diagnostic_prefix()
                )?;
                return Ok(1);
            }
            signal = signal_number_from_spec(sigspec).unwrap_or(15);
            index += 1;
            operands_start = index;
            saw_signal = true;
            continue;
        }
        if value.starts_with('-') && value != "-" && !saw_signal {
            let sigspec = value.trim_start_matches('-');
            if translate_signal(sigspec).is_none() {
                writeln!(
                    stderr,
                    "{}kill: {sigspec}: invalid signal specification",
                    diagnostic_prefix()
                )?;
                return Ok(1);
            }
            signal = signal_number_from_spec(sigspec).unwrap_or(15);
            index += 1;
            operands_start = index;
            saw_signal = true;
            continue;
        }
        operands_start = index;
        break;
    }

    if operands_start >= args.len() {
        write_kill_usage(stderr)?;
        return Ok(2);
    }

    let mut status = 0;
    for operand in &args[operands_start..] {
        if signal == 0 && matches!(operand.as_str(), "-1" | "0") {
            // Windows cannot address a Unix process group through
            // OpenProcess, but signal 0 is only an existence probe. These
            // group targets are valid Bash operands and do not require a
            // native termination call.
            continue;
        }

        let Some(pid) = parse_pid(operand) else {
            // GNU builtins/common.c:236 sh_badpid: "`%s': not a pid or
            // valid job spec"
            writeln!(
                stderr,
                "{}kill: `{operand}': not a pid or valid job spec",
                diagnostic_prefix()
            )?;
            status = 1;
            continue;
        };

        // Negative pid → Unix process group.  Windows has no such target,
        // so silently skip delivery (signal 0 already handled above for
        // existence probes of the current group).
        if pid < 0 {
            continue;
        }

        let pid = pid as u32;

        // Bash treats pid 0 as the current process group. Windows does not
        // expose that target through OpenProcess, but signal 0 is only an
        // existence probe, so the shell can answer it from its own group
        // context without attempting to terminate a process.
        if pid == 0 && signal == 0 {
            continue;
        }

        match deliver_rubash_signal(pid, signal) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                writeln!(stderr, "{}kill: ({pid}) - {error}", diagnostic_prefix())?;
                status = 1;
                continue;
            }
        }

        if let Err(message) = signal_process(pid, signal) {
            writeln!(stderr, "{}kill: ({pid}) - {message}", diagnostic_prefix())?;
            status = 1;
        }
    }

    Ok(status)
}

fn write_kill_usage<E>(stderr: &mut E) -> io::Result<()>
where
    E: Write,
{
    // GNU builtins/common.c:128-135 builtin_usage: prints only
    // "this_command_name: usage: " + short_doc, WITHOUT the script/line
    // prolog from builtin_error_prolog.
    writeln!(
        stderr,
        "kill: usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]"
    )
}

pub fn list_first_signal_for_sed() -> &'static str {
    "SIGHUP"
}

pub fn signal_number_for_spec(value: &str) -> Option<i32> {
    signal_number_from_spec(value)
}

/// strsignal(3) text for a signal-death notice (rubash#229). GNU jobs.c
/// printable_job_status renders a dead pipeline with j_strsignal, which
/// resolves to strsignal(3) ("Terminated", "Killed", "User defined signal
/// 1", ...). glibc's strsignal uses per-thread storage, so calling it from
/// the shell's main thread is safe.
pub fn signal_death_description(signal: i32) -> String {
    #[cfg(unix)]
    {
        let text = unsafe { libc::strsignal(signal) };
        if !text.is_null() {
            return unsafe { std::ffi::CStr::from_ptr(text) }
                .to_string_lossy()
                .into_owned();
        }
    }
    format!("Signal {signal}")
}

pub fn translate_signal(value: &str) -> Option<&'static str> {
    if value == "0" {
        return Some("EXIT");
    }

    if value == "EXIT" || value == "SIGEXIT" {
        return Some("0");
    }

    if let Ok(mut number) = value.parse::<i32>() {
        if number > 128 {
            number -= 128;
        }
        return signal_name(number);
    }

    let name = value.strip_prefix("SIG").unwrap_or(value);
    signal_number(name)
}

fn signal_number_from_spec(value: &str) -> Option<i32> {
    if value == "0" {
        return Some(0);
    }

    let name = value.strip_prefix("SIG").unwrap_or(value);
    if name == "EXIT" {
        return Some(0);
    }

    if let Ok(mut number) = value.parse::<i32>() {
        if number > 128 {
            number -= 128;
        }
        return (number == 0 || signal_name(number).is_some()).then_some(number);
    }

    signal_number(name)?.parse::<i32>().ok()
}

fn parse_pid(value: &str) -> Option<i32> {
    value.parse::<i32>().ok()
}

pub fn process_exists(pid: u32) -> bool {
    pid == std::process::id() || signal_process(pid, 0).is_ok_or_permission_denied()
}

/// jobs.c:3926-3928 continue_job: fg/bg resume a stopped job by signalling
/// SIGCONT to the process group; per-pid delivery is the rubash equivalent
/// of killpg over the job's process list.
pub fn send_signal(pid: u32, signal: i32) -> Result<(), &'static str> {
    signal_process(pid, signal)
}

/// Install this process's signal-receiving backend.
///
/// Windows: a per-process `{pid}.alive` marker in the shared
/// `%TEMP%\rubash-signals` directory; other rubash processes deliver
/// cross-process signals as atomic `.part`-rename entries beside it
/// (deliver_rubash_signal).
///
/// Unix: real kernel signal delivery -- GNU sig.c:102 initialize_signals
/// installs handlers for the terminating-signal set (the table at
/// sig.c:133). signal_hook owns the async-signal-safe self-pipe; arrivals
/// are drained by take_pending_signals and dispatched by the shared
/// trap_exec::run_pending_signal_traps boundary poll. No marker file is
/// written, so deliver_rubash_signal finds no mailbox and routes rubash
/// targets through the real kill(2) instead (signal_process).
#[cfg(unix)]
pub fn register_signal_mailbox(_pid: u32) -> io::Result<()> {
    kernel_signals::install()
}

#[cfg(not(unix))]
pub fn register_signal_mailbox(pid: u32) -> io::Result<()> {
    let dir = signal_mailbox_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::write(signal_marker_path(pid), std::process::id().to_string())
}

/// Unix keeps the kernel dispositions for the whole process lifetime:
/// nothing to tear down, and caught handlers reset to default in respawned
/// children on exec anyway (bash subshell semantics for trapped signals).
#[cfg(unix)]
pub fn unregister_signal_mailbox(_pid: u32) {}

#[cfg(not(unix))]
pub fn unregister_signal_mailbox(pid: u32) {
    let _ = std::fs::remove_file(signal_marker_path(pid));
    let _ = std::fs::remove_file(signal_queue_path(pid));
    // startup21: enumerate with the pattern-limited FindFirstFileW scan the
    // poller already uses (pending_signal_entries) instead of read_dir over
    // the whole shared directory. The directory accumulates one marker or
    // queue file per process that ever ran (213 stale markers at lane
    // measurement time) and the full scan cost ~0.5ms of every shell's exit
    // path; the pattern query removes exactly the same set of files.
    let prefix = format!("{pid}.q.");
    if let Ok(entries) = pending_signal_entries(&signal_mailbox_dir(), &prefix) {
        for entry in entries {
            let _ = std::fs::remove_file(entry);
        }
    }
}

/// Startup21 (non-unix): register the mailbox OFF the Executor::new
/// critical path. The synchronous registration (create_dir_all + marker
/// write) costs ~0.5ms of Windows filesystem time per spawn, and the
/// marker is only ever consulted by OTHER processes' kill builtin
/// (deliver_rubash_signal) — this process's own execution never reads it.
/// A sub-millisecond delay before the marker appears only widens the
/// pre-mailbox window in which a cross-process kill falls back to the
/// signal_process route, which is the behavior shells had before the
/// mailbox existed. Ordering stays eager in the GNU sense: the
/// registration is initiated at Executor::new (cf. trap.c:102
/// initialize_signals at shell startup), and
/// join_signal_mailbox_registration() lets Drop wait for the thread before
/// unregistering, so a normally exiting shell never leaks its marker.
#[cfg(not(unix))]
static MAILBOX_REGISTRATION: std::sync::Mutex<Option<std::thread::JoinHandle<()>>> =
    std::sync::Mutex::new(None);

#[cfg(not(unix))]
pub fn register_signal_mailbox_async(pid: u32) {
    let mut slot = MAILBOX_REGISTRATION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // A previous top-level executor in this process (unit tests) may still
    // hold the slot; joining first keeps registrations strictly ordered so
    // a late write can never resurrect a marker an executor just removed.
    if let Some(handle) = slot.take() {
        let _ = handle.join();
    }
    match std::thread::Builder::new()
        .name("rubash-sig-mailbox".to_string())
        .spawn(move || {
            let _ = register_signal_mailbox(pid);
            prune_stale_markers(pid);
        }) {
        Ok(handle) => *slot = Some(handle),
        // Thread spawn failure is not a shell error: fall back to the
        // synchronous registration the executor previously paid.
        Err(_) => {
            let _ = register_signal_mailbox(pid);
        }
    }
}

/// startup21 (non-unix): opportunistic mailbox-directory hygiene, run on
/// the background registration thread (never the shell's critical path —
/// process-exit executors no longer remove their own marker, so something
/// must keep the shared directory from growing without bound). Removes the
/// mailbox of every DEAD pid found among the first 128 marker entries:
/// exactly the cleanup deliver()'s dead-pid path performs on demand, done
/// here proactively. A live pid's marker is left untouched; removals are
/// idempotent, so racing an owner's own unregister is harmless.
#[cfg(not(unix))]
fn prune_stale_markers(own_pid: u32) {
    const PRUNE_LIMIT: usize = 128;
    let dir = signal_mailbox_dir();
    let Ok(entries) = pending_signal_entries(&dir, "") else {
        return;
    };
    let mut pruned = 0usize;
    for path in entries {
        if pruned >= PRUNE_LIMIT {
            break;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };
        let Some(pid) = name
            .strip_suffix(".alive")
            .and_then(|stem| stem.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == own_pid || process_exists(pid) {
            continue;
        }
        unregister_signal_mailbox(pid);
        pruned += 1;
    }
}

/// Wait for an in-flight async mailbox registration to finish. Called by
/// the top-level Executor's Drop BEFORE unregister_signal_mailbox, so the
/// unregister cannot race a registration that has not written its marker
/// yet (that ordering would leak the marker file).
#[cfg(not(unix))]
pub fn join_signal_mailbox_registration() {
    if let Some(handle) = MAILBOX_REGISTRATION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
    {
        let _ = handle.join();
    }
}

#[cfg(unix)]
pub fn join_signal_mailbox_registration() {}

#[cfg(not(unix))]
fn parse_signal_lines(content: &str) -> Vec<i32> {
    content
        .lines()
        .filter_map(|line| line.trim().parse::<i32>().ok())
        .collect()
}

/// Signals delivered by this very process (`kill` to `$$`): drained by the
/// executor's signal poll without any filesystem traffic.
static SELF_SIGNALS: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());

/// Counts polls so the mailbox-file scan (for signals sent by OTHER
/// processes) runs at a reduced interval instead of twice per command.
/// Unix has no file mailbox: kernel deliveries drain through
/// take_all_signals and this throttle does not exist.
#[cfg(not(unix))]
static FILE_POLL_TICK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Unix drain: in-process queue (a `kill` to `$$` via queue_self_signal)
/// plus real kernel deliveries from the signal_hook backend.
#[cfg(unix)]
fn take_all_signals() -> Vec<i32> {
    let mut signals = take_self_signals();
    signals.extend(kernel_signals::drain());
    signals
}

fn queue_self_signal(signal: i32) {
    SELF_SIGNALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(signal);
}

fn take_self_signals() -> Vec<i32> {
    let mut queue = SELF_SIGNALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::mem::take(&mut *queue)
}

pub fn take_pending_signals(pid: u32) -> io::Result<Vec<i32>> {
    // Unix: real kernel deliveries drain through the in-process queue; no
    // filesystem traffic, and no cross-process file mailbox exists.
    #[cfg(unix)]
    {
        let _ = pid;
        return Ok(take_all_signals());
    }
    // In-process deliveries are drained unconditionally (a Mutex op, no
    // filesystem traffic). Signals from OTHER processes arrive via mailbox
    // files; with Windows Defender-style real-time scanning each
    // intercepted file operation costs ~0.25-0.5ms, so the file poll runs
    // every 64th poll (~every 32 commands) instead of twice per command.
    // Delivery latency for external kills stays in the millisecond range.
    // NOTE: a directory-mtime fast path was tried here and removed: NTFS
    // does not update a directory's LastWriteTime synchronously on entry
    // create/delete (verified 2026-09-12: mtime unchanged 0.5s after a
    // create), so it silently swallowed real deliveries.
    #[cfg(not(unix))]
    {
        let mut signals = take_self_signals();
        let tick = FILE_POLL_TICK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if signals.is_empty() && tick % 64 == 0 {
            signals = take_file_signals(pid)?;
        }
        Ok(signals)
    }
}

/// Unthrottled variant of take_pending_signals: always scans the
/// per-delivery file mailbox. Used where delivery latency is observable —
/// `wait`'s interruptible poll (wait.def:170-206) and the in-process
/// ${THIS_SH} child boundary that must drain the emulated child's queue.
pub fn take_pending_signals_now(pid: u32) -> io::Result<Vec<i32>> {
    #[cfg(unix)]
    {
        let _ = pid;
        return Ok(take_all_signals());
    }
    #[cfg(not(unix))]
    {
        let mut signals = take_self_signals();
        signals.extend(take_file_signals(pid)?);
        Ok(signals)
    }
}

/// Push drained signals back to the front of this process's pending queue
/// (preserving order). `wait` re-queues what it peeked so the command
/// boundary's run_pending_signal_traps still dispatches the trap actions
/// (wait.def:174 — "the trap associated with that signal shall be taken"
/// AFTER wait returns >128), and the in-process ${THIS_SH} child boundary
/// restores signals that were pending for the parent before it started.
pub fn requeue_pending_signals(signals: Vec<i32>) {
    if signals.is_empty() {
        return;
    }
    let mut queue = SELF_SIGNALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    queue.splice(..0, signals);
}

#[cfg(not(unix))]
fn take_file_signals(pid: u32) -> io::Result<Vec<i32>> {
    let dir = signal_mailbox_dir();
    let mut signals = Vec::new();
    // Legacy single-file queue: take it atomically via rename so a signal
    // appended after the read is not dropped by the remove below. Check
    // existence first: an unconditional rename is a failing syscall on
    // every poll, and this poll runs on the executor's command boundaries.
    let legacy_queue = signal_queue_path(pid);
    if legacy_queue.try_exists()? {
        let legacy_taken = dir.join(format!("{pid}.queue.taking"));
        if std::fs::rename(&legacy_queue, &legacy_taken).is_ok() {
            if let Ok(content) = std::fs::read_to_string(&legacy_taken) {
                signals.extend(parse_signal_lines(&content));
            }
            let _ = std::fs::remove_file(&legacy_taken);
        }
    }
    // Per-delivery entries: senders write a unique .part file and rename it
    // into place, so the reader only ever sees fully written entries and a
    // concurrent delivery can never be lost. The old read-then-remove
    // window dropped a signal whenever a delivery landed between the two
    // steps, which made the trap9 kill-to-self from a busy loop
    // unreliable.
    let prefix = format!("{pid}.q.");
    let entries = pending_signal_entries(&dir, &prefix)?;
    for path in entries {
        if let Ok(content) = std::fs::read_to_string(&path) {
            signals.extend(parse_signal_lines(&content));
        }
        let _ = std::fs::remove_file(&path);
    }
    Ok(signals)
}

/// Enumerate pending-signal queue entries whose file name starts with
/// `prefix`. On Windows this uses FindFirstFileW with a `{prefix}*` pattern
/// so a real scan touches only matching names: the shared mailbox
/// directory accumulates one marker or queue file per process that ever
/// ran, and a full read_dir scan costs ~1ms per poll on busy systems
/// (measured 2026-09-12: 592 files -> 1.05ms per call). The pattern-limited
/// query halves that; the caller gates the scan itself to every 64th poll
/// so the per-command amortized cost is negligible.
fn pending_signal_entries(
    dir: &std::path::Path,
    prefix: &str,
) -> io::Result<Vec<std::path::PathBuf>> {
    scan_pending_signal_entries(dir, prefix)
}

fn scan_pending_signal_entries(
    dir: &std::path::Path,
    prefix: &str,
) -> io::Result<Vec<std::path::PathBuf>> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{
            GetLastError, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, INVALID_HANDLE_VALUE,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            FindClose, FindFirstFileW, FindNextFileW, WIN32_FIND_DATAW,
        };

        // The pattern is the directory plus a separator and "{prefix}*";
        // only fully written entries match (senders rename .part files into
        // place), and the .part exclusion below keeps that guarantee.
        let mut pattern: Vec<u16> = dir
            .join(format!("{prefix}*"))
            .as_os_str()
            .encode_wide()
            .collect();
        pattern.push(0);

        let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
        let handle = unsafe { FindFirstFileW(pattern.as_ptr(), &mut data) };
        if handle == INVALID_HANDLE_VALUE {
            let error = unsafe { GetLastError() };
            if error == ERROR_FILE_NOT_FOUND || error == ERROR_PATH_NOT_FOUND {
                // No matching entries (or the mailbox dir does not exist
                // yet): nothing pending, same as the NotFound branch of the
                // former read_dir-based scan.
                return Ok(Vec::new());
            }
            return Err(io::Error::from_raw_os_error(error as i32));
        }

        let mut names = Vec::new();
        loop {
            let name = utf16_until_nul(&data.cFileName);
            if name.starts_with(prefix) && !name.ends_with(".part") {
                names.push(dir.join(&name));
            }
            data = unsafe { std::mem::zeroed() };
            if unsafe { FindNextFileW(handle, &mut data) } == 0 {
                unsafe { FindClose(handle) };
                break;
            }
        }
        names.sort();
        Ok(names)
    }
    #[cfg(not(windows))]
    {
        let mut entries: Vec<std::path::PathBuf> = match std::fs::read_dir(dir) {
            Ok(entries) => entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| name.starts_with(prefix) && !name.ends_with(".part"))
                        .unwrap_or(false)
                })
                .collect(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        entries.sort();
        Ok(entries)
    }
}

#[cfg(windows)]
fn utf16_until_nul(chars: &[u16]) -> String {
    let len = chars.iter().position(|c| *c == 0).unwrap_or(chars.len());
    String::from_utf16_lossy(&chars[..len])
}

fn signal_name(number: i32) -> Option<&'static str> {
    SIGNALS
        .iter()
        .find_map(|(signal_number, _, name)| (*signal_number == number).then_some(*name))
}

fn signal_number(name: &str) -> Option<&'static str> {
    SIGNALS
        .iter()
        .find_map(|(_, number, signal_name)| (*signal_name == name).then_some(*number))
}

fn write_signal_list<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    // GNU kill.def list_signals: 5 entries per line, tab-separated; the
    // final partial line still carries the tab separator before the closing
    // newline (captured from GNU 5.3.0 `kill -l` on the WSL baseline).
    let total = SIGNALS.len();
    for (position, (number, _, name)) in SIGNALS.iter().enumerate() {
        let position = position + 1;
        if position > 1 && (position - 1) % 5 == 0 {
            writeln!(stdout)?;
        } else if position > 1 {
            write!(stdout, "\t")?;
        }
        write!(stdout, "{number:>2}) SIG{name}")?;
        if position == total && position % 5 != 0 {
            write!(stdout, "\t")?;
        }
    }
    writeln!(stdout)?;
    Ok(())
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

fn deliver_rubash_signal(pid: u32, signal: i32) -> io::Result<bool> {
    if !signal_marker_path(pid).is_file() {
        return Ok(false);
    }
    // A background Rubash process owns its own mailbox even though it
    // inherits the parent shell identity for expansion semantics. Do not
    // reject that child merely because the marker names the parent.
    // SIGKILL is not trappable. STOP/TSTP/CONT must use the native Windows
    // process-state backend rather than a mailbox event.
    //
    // NOTE: these are Linux wire-format numbers (9=KILL, 17=CHLD, 18=CONT,
    // 19=STOP), and this whole branch is reachable only on the non-unix
    // mailbox path: on unix nothing ever creates the `{pid}.alive` marker
    // (register_signal_mailbox installs the kernel backend instead), so the
    // `is_file()` check above returns first and delivery routes through the
    // real kill(2) in signal_process with platform numbering. Do NOT
    // constant-ize these against libc — that would renumber the Windows
    // mailbox wire format.
    if matches!(signal, 9 | 17 | 18 | 19) {
        return Ok(false);
    }
    if !process_exists(pid) {
        unregister_signal_mailbox(pid);
        return Ok(false);
    }
    if signal == 0 {
        return Ok(true);
    }

    // In-process delivery: a signal to the current shell never touches the
    // filesystem. The per-command signal poll drains this queue directly;
    // on Windows each intercepted filesystem operation costs ~0.25-0.5ms
    // under real-time scanning, so both this write and the reader's poll
    // must stay off the per-command hot path.
    if pid == std::process::id() {
        queue_self_signal(signal);
        return Ok(true);
    }

    std::fs::create_dir_all(signal_mailbox_dir())?;
    // Deliver as a unique per-signal entry: write to a .part file and
    // rename it into place. The rename is atomic, so the reading shell
    // either misses the entry entirely or observes it fully written; a
    // shared append-queue could lose the write to the reader's
    // read-then-remove window.
    static SIGNAL_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SIGNAL_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let base = format!("{pid}.q.{}.{}", std::process::id(), seq);
    let part = signal_mailbox_dir().join(format!("{base}.part"));
    let full = signal_mailbox_dir().join(&base);
    std::fs::write(
        &part,
        format!(
            "{signal}
"
        ),
    )?;
    std::fs::rename(&part, &full)?;
    Ok(true)
}

fn signal_mailbox_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("rubash-signals")
}

fn signal_marker_path(pid: u32) -> std::path::PathBuf {
    signal_mailbox_dir().join(format!("{pid}.alive"))
}

#[cfg(not(unix))]
fn signal_queue_path(pid: u32) -> std::path::PathBuf {
    signal_mailbox_dir().join(format!("{pid}.queue"))
}

#[cfg(windows)]
fn signal_process(pid: u32, signal: i32) -> Result<(), &'static str> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, OpenThread, ResumeThread, SuspendThread, TerminateProcess,
        PROCESS_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME,
    };

    // Linux numbering: CONT=18 resumes, STOP=19/TSTP=20 suspend. SIGCHLD
    // (17) is delivered-but-ignored by default, so kill -CHLD succeeds as a
    // no-op instead of terminating (GNU kill.def sends it through kill(2)
    // where the default disposition is SIG_DFL-ignored).
    if signal == 17 {
        return Ok(());
    }
    if signal == 18 || signal == 19 || signal == 20 {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == -1isize as _ {
            return Err("Cannot enumerate process threads");
        }
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..unsafe { std::mem::zeroed() }
        };
        let mut found = false;
        let mut result = unsafe { Thread32First(snapshot, &mut entry) } != 0;
        while result {
            if entry.th32OwnerProcessID == pid {
                found = true;
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    unsafe { CloseHandle(snapshot) };
                    return Err("Cannot open process thread");
                }
                let failed = if signal == 18 {
                    unsafe { ResumeThread(thread) == u32::MAX }
                } else {
                    unsafe { SuspendThread(thread) == u32::MAX }
                };
                unsafe { CloseHandle(thread) };
                if failed {
                    unsafe { CloseHandle(snapshot) };
                    return Err("Failed to change process state");
                }
            }
            result = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
        }
        unsafe { CloseHandle(snapshot) };
        return if found {
            Ok(())
        } else {
            Err("No such process")
        };
    }

    use windows_sys::Win32::System::Threading::PROCESS_TERMINATE;
    let access = if signal == 0 {
        PROCESS_QUERY_LIMITED_INFORMATION
    } else {
        PROCESS_TERMINATE
    };

    let handle = unsafe { OpenProcess(access, 0, pid) };
    if handle.is_null() {
        let error = unsafe { GetLastError() };
        return Err(match error {
            ERROR_ACCESS_DENIED => "Permission denied",
            ERROR_INVALID_PARAMETER => "No such process",
            _ => "Cannot open process",
        });
    }

    if signal == 0 {
        let mut exit_code = 0;
        let process_is_active = unsafe {
            GetExitCodeProcess(handle, &mut exit_code) != 0 && exit_code == STILL_ACTIVE as u32
        };
        if !process_is_active {
            unsafe { CloseHandle(handle) };
            return Err("No such process");
        }
    } else if unsafe { TerminateProcess(handle, (128 + signal) as u32) == 0 } {
        unsafe { CloseHandle(handle) };
        return Err("Failed to terminate process");
    }

    unsafe { CloseHandle(handle) };
    Ok(())
}

#[cfg(unix)]
fn signal_process(pid: u32, signal: i32) -> Result<(), &'static str> {
    // A signal this shell sends to ITSELF must be delivered before the
    // kill builtin returns: GNU's single-threaded bash runs the SIGUSR1
    // handler during kill(2), so the pending-trap dispatch at the very
    // next command boundary (still inside the calling function) sees it —
    // trap9.sub's `func() { kill -USR1 $$; }` runs its trap inside func.
    // This process is multithreaded, and kill(2) hands the signal to any
    // thread that does not block it, so the handler's queue write may lag
    // past the function's last boundary (~35% of runs: the trap fired
    // after the function returned and its `return` errored at top level).
    // raise(3) targets the calling thread: the handler completes before
    // raise returns, restoring GNU's observable ordering.
    if pid == std::process::id() {
        let result = unsafe { libc::raise(signal) };
        return if result == 0 {
            Ok(())
        } else {
            Err("Failed to signal process")
        };
    }
    let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if result == 0 {
        return Ok(());
    }

    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Err("No such process"),
        Some(libc::EPERM) => Err("Permission denied"),
        _ => Err("Failed to signal process"),
    }
}

#[cfg(not(any(unix, windows)))]
fn signal_process(_pid: u32, signal: i32) -> Result<(), &'static str> {
    if signal == 0 {
        Err("No such process")
    } else {
        Err("Failed to signal process")
    }
}

trait KillResultExt {
    fn is_ok_or_permission_denied(&self) -> bool;
}

impl KillResultExt for Result<(), &'static str> {
    fn is_ok_or_permission_denied(&self) -> bool {
        self.is_ok() || matches!(self, Err(message) if *message == "Permission denied")
    }
}

/// Real kernel signal backend (the unix half of the signal-delivery seam).
///
/// GNU model (rubash#226): a NON-INTERACTIVE script shell installs no handler
/// for a terminating signal until a `trap` names it. sig.c:102
/// initialize_signals -> sig.c:313 initialize_shell_signals: every shell sets
/// `SIGQUIT -> SIG_IGN` (sig.c:333), but initialize_terminating_signals --
/// which installs termsig_sighandler for the whole terminating set
/// (sig.c:127-260) -- runs only when `interactive` (sig.c:315-316) or when an
/// EXIT trap / `trap` display forces it (sig.c:226-229). Everything else
/// stays SIG_DFL, so the KERNEL kills a blocked script shell on delivery --
/// there is no unkillable state. The `trap` builtin then installs one
/// handler per named signal (trap.c trap_builtin -> sig.c:830
/// set_signal_handler with `act.sa_flags = 0`, sig.c:835 -- NO SA_RESTART:
/// the handler may interrupt blocking syscalls exactly like GNU's), `trap ''`
/// sets SIG_IGN (trap.def: ignore), and `trap -` restores the entry
/// disposition. Arrivals under an installed handler are queued (the
/// record-while-blocked contract, trap.c:537-543 set_trap_state) and
/// dispatched at command boundaries by trap_exec::run_pending_signal_traps.
/// The dispatch side is platform-shared: this backend and the Windows file
/// mailbox both surface as the same `Vec<i32>` from take_pending_signals.
///
/// Deliberately NOT registered:
/// - SIGCHLD: the reap-point dispatcher
///   (Executor::run_sigchld_trap_for_reaped_child) already fires the CHLD
///   trap exactly once per reaped child; a kernel SIGCHLD here would
///   double-fire it.
/// - Stop-class signals (SIGTSTP/SIGTTIN/SIGTTOU): without a
///   stop-the-shell implementation, catching them would turn a stop into a
///   spurious 128+N exit at the next boundary. Kernel-default stop is the
///   closer bash behavior until the terminal/job-control layer exists.
/// - SIGKILL is uncatchable by design.
#[cfg(unix)]
mod kernel_signals {
    use signal_hook::iterator::Signals;
    use std::io;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};

    static KERNEL_SIGNALS: OnceLock<Mutex<Signals>> = OnceLock::new();

    /// The six signals the backend manages dispositions for (the
    /// signal_hook-catchable subset of GNU's terminating set that a trap can
    /// name and the executor can dispatch).
    const MANAGED: [i32; 6] = [
        libc::SIGHUP as i32,
        libc::SIGINT as i32,
        libc::SIGQUIT as i32,
        libc::SIGTERM as i32,
        libc::SIGUSR1 as i32,
        libc::SIGUSR2 as i32,
    ];

    /// Bitmask (1 << index into MANAGED) of signals currently routed to the
    /// signal_hook handler.
    static HANDLER_SET: AtomicU64 = AtomicU64::new(0);

    /// Packed per-signal disposition state (2 bits each: 0=Default, 1=Ignore,
    /// 2=Handler) so the per-boundary reconcile skips unchanged signals —
    /// no sigaction traffic on the steady-state command path.
    static DISPOSITION_STATE: AtomicU64 = AtomicU64::new(0);

    fn state_bits(sig: i32) -> u64 {
        let index = MANAGED
            .iter()
            .position(|&m| m == sig)
            .expect("managed signal");
        2 * index as u64
    }

    fn current_state(sig: i32) -> u8 {
        ((DISPOSITION_STATE.load(Ordering::Relaxed) >> state_bits(sig)) & 0b11) as u8
    }

    fn store_state(sig: i32, state: u8) {
        let bits = state_bits(sig);
        let mask = 0b11u64 << bits;
        let previous = DISPOSITION_STATE.fetch_and(!mask, Ordering::SeqCst);
        DISPOSITION_STATE.fetch_or((state as u64) << bits, Ordering::SeqCst);
        let _ = previous;
    }

    /// Bitmask of signals that were SIG_IGN when this process was exec'd.
    /// POSIX 2.11: a non-interactive shell keeps entry-ignored signals
    /// ignored (GNU sig.c:273-277 marks them hard-ignored).
    static ENTRY_IGNORED: AtomicU64 = AtomicU64::new(0);

    fn bit(sig: i32) -> u64 {
        1 << MANAGED
            .iter()
            .position(|&m| m == sig)
            .expect("managed signal")
    }

    fn current_disposition_is_ignored(sig: i32) -> bool {
        // sigaction(nullptr) queries without installing; the returned
        // sa_handler distinguishes SIG_IGN.
        let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
        unsafe { libc::sigaction(sig, std::ptr::null(), &mut current) };
        current.sa_sigaction == libc::SIG_IGN
    }

    /// Install the handler for `sig` and clear SA_RESTART from its flags.
    /// signal_hook's registry hardcodes SA_RESTART; GNU installs trap
    /// handlers with sa_flags = 0 (sig.c:835), so a caught signal interrupts
    /// a blocked read/wait exactly like GNU's does.
    fn ensure_handler(signals: &Signals, sig: i32) -> io::Result<()> {
        if HANDLER_SET.load(Ordering::Relaxed) & bit(sig) == 0 {
            signals.add_signal(sig)?;
            HANDLER_SET.fetch_or(bit(sig), Ordering::SeqCst);
        }
        let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
        if unsafe { libc::sigaction(sig, std::ptr::null(), &mut current) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if current.sa_flags & libc::SA_RESTART != 0 {
            current.sa_flags &= !libc::SA_RESTART;
            // The whole struct is copied (incl. glibc's sa_restorer), so
            // re-installing it only flips the flag.
            if unsafe { libc::sigaction(sig, &current, std::ptr::null_mut()) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    fn set_kernel_handler(sig: i32, handler: usize) {
        let action = libc::sigaction {
            sa_sigaction: handler,
            sa_flags: 0,
            sa_mask: unsafe { std::mem::zeroed() },
            #[cfg(target_os = "linux")]
            sa_restorer: None,
        };
        unsafe { libc::sigaction(sig, &action, std::ptr::null_mut()) };
    }

    /// Effective disposition for one managed signal, derived from the trap
    /// table: a real action -> handler, `trap ''` -> SIG_IGN, no trap ->
    /// entry disposition (SIG_DFL, or SIG_IGN for entry-ignored signals and
    /// SIGQUIT -- sig.c:333 sets SIGQUIT to SIG_IGN for every shell).
    pub(super) enum Disposition {
        Handler,
        Ignore,
        Default,
    }

    pub(super) fn reconcile(dispositions: &[(i32, Disposition)]) {
        let Some(mutex) = KERNEL_SIGNALS.get() else {
            return;
        };
        let Ok(signals) = mutex.lock() else {
            return;
        };
        for &(sig, ref want) in dispositions {
            let target = match want {
                Disposition::Handler => 2u8,
                Disposition::Ignore => 1u8,
                Disposition::Default => 0u8,
            };
            if current_state(sig) == target {
                continue; // steady state: no sigaction traffic
            }
            match want {
                Disposition::Handler => {
                    if ensure_handler(&signals, sig).is_err() {
                        continue;
                    }
                }
                Disposition::Ignore => {
                    set_kernel_handler(sig, libc::SIG_IGN);
                    HANDLER_SET.fetch_and(!bit(sig), Ordering::SeqCst);
                }
                Disposition::Default => {
                    let entry_ignored = ENTRY_IGNORED.load(Ordering::Relaxed) & bit(sig) != 0;
                    let base = if entry_ignored || sig == libc::SIGQUIT as i32 {
                        libc::SIG_IGN
                    } else {
                        libc::SIG_DFL
                    };
                    set_kernel_handler(sig, base);
                    HANDLER_SET.fetch_and(!bit(sig), Ordering::SeqCst);
                    // A delivery that raced the flip may have left the
                    // registry flag set; the boundary dispatch drops such
                    // entries via handler_installed() instead of acting on
                    // them (draining here would swallow OTHER signals'
                    // pending deliveries).
                }
            }
            store_state(sig, target);
        }
    }

    /// True when `sig` is currently routed to the signal_hook handler. A
    /// pending-queue entry for a signal that is NOT handler-installed is a
    /// stale registry flag from a delivery that raced a disposition flip
    /// (trap cleared): SIG_DFL would have killed the process and SIG_IGN
    /// swallows, so neither can produce a legitimate pending entry.
    pub(super) fn handler_installed(sig: i32) -> bool {
        MANAGED.contains(&sig) && HANDLER_SET.load(Ordering::Relaxed) & bit(sig) != 0
    }

    pub(super) fn install() -> io::Result<()> {
        if KERNEL_SIGNALS.get().is_some() {
            return Ok(());
        }
        // Capture entry dispositions BEFORE changing anything: signals
        // ignored on entry must stay ignored (POSIX 2.11, GNU sig.c:273).
        for sig in MANAGED {
            if current_disposition_is_ignored(sig) {
                ENTRY_IGNORED.fetch_or(bit(sig), Ordering::SeqCst);
            }
        }
        // Empty initial set: no kernel handler exists until a trap names a
        // signal (GNU leaves non-interactive terminating signals SIG_DFL,
        // sig.c:315-316).
        let signals = Signals::new([] as [i32; 0])?;
        let _ = KERNEL_SIGNALS.set(Mutex::new(signals));
        // sig.c:102 initialize_signals -> sig.c:333: every shell ignores
        // SIGQUIT (script shells do not die on ^\).
        set_kernel_handler(libc::SIGQUIT as i32, libc::SIG_IGN);
        store_state(libc::SIGQUIT as i32, 1);
        // Entry-ignored signals stay ignored (captured above); record their
        // state so the first reconcile does not "restore" them to SIG_DFL.
        for sig in MANAGED {
            if ENTRY_IGNORED.load(Ordering::Relaxed) & bit(sig) != 0 {
                store_state(sig, 1);
            }
        }
        Ok(())
    }

    pub(super) fn drain() -> Vec<i32> {
        let Some(mutex) = KERNEL_SIGNALS.get() else {
            return Vec::new();
        };
        let Ok(mut signals) = mutex.lock() else {
            return Vec::new();
        };
        signals.pending().collect()
    }

    /// Fork-child disposition restore (rubash#229). GNU's forked
    /// disk-command child runs trap.c:1489 restore_original_signals
    /// between fork and exec (execute_cmd.c:4224): every trapped signal
    /// returns to its ENTRY disposition (reset_signal -> original_signals,
    /// SIG_DFL unless it was SIG_IGN on entry), a `trap ''`-ignored signal
    /// STAYS ignored (trap.c:1451-1455 keeps IGNORE_SIG at SIG_IGN), and
    /// the shell-policy SIGQUIT ignore (sig.c:333) never crosses exec —
    /// without the restore, `sh -c 'kill -QUIT $$'` children inherit the
    /// SIG_IGN and exit 0 instead of dying with 131. Runs in pre_exec:
    /// sigaction and atomic loads are async-signal-safe.
    pub(super) fn pre_exec_child_dispositions() -> io::Result<()> {
        for &sig in MANAGED.iter() {
            let action = if ENTRY_IGNORED.load(Ordering::Relaxed) & bit(sig) != 0 {
                // Ignored at shell entry: POSIX keeps it ignored for children.
                libc::SIG_IGN
            } else if sig == libc::SIGQUIT as i32 {
                // The sig.c:333 policy ignore is shell-only; the entry
                // disposition (SIG_DFL here, entry-ignored handled above)
                // is what children exec with.
                libc::SIG_DFL
            } else if current_state(sig) == 1 {
                // trap '' ignore (trap.c:1451-1455): survives into children.
                libc::SIG_IGN
            } else {
                // Trapped (handler) or default: children get the entry
                // disposition, which is SIG_DFL for a plain script shell.
                libc::SIG_DFL
            };
            set_kernel_handler(sig, action);
        }
        Ok(())
    }
}

/// Map the executor's trap table onto the kernel dispositions (unix arm of
/// rubash#226). Mirrors GNU's per-signal rules: `trap 'action' SIG` installs
/// the handler (sig.c:830, sa_flags=0), `trap '' SIG` sets SIG_IGN, no trap
/// restores the entry disposition (SIG_DFL for a plain script). Called from
/// the `trap` builtin's mutation paths and the command-boundary poll so the
/// kernel state can never lag the trap table by more than one command.
#[cfg(unix)]
pub fn reconcile_kernel_trap_dispositions(env_vars: &std::collections::HashMap<String, String>) {
    use kernel_signals::Disposition;
    // Single pass over the six trap keys (runs on the per-command boundary
    // poll, so six HashMap gets + one reset-list scan is the whole cost —
    // get_trap_action's per-call BTreeSet build would multiply that by six).
    let reset = env_vars
        .get("__RUBASH_TRAP_RESET")
        .map(|value| value.as_str())
        .unwrap_or("");
    let dispositions: Vec<(i32, Disposition)> = [
        libc::SIGHUP,
        libc::SIGINT,
        libc::SIGQUIT,
        libc::SIGTERM,
        libc::SIGUSR1,
        libc::SIGUSR2,
    ]
    .iter()
    .map(|&raw| {
        let sig = raw as i32;
        let short = signal_short_name(sig);
        let want = match env_vars.get(&format!("__RUBASH_TRAP_SIG{short}")) {
            // A real action and not reset-listed -> handler (trap.c
            // trap_builtin -> sig.c:830 set_signal_handler).
            Some(action) if !action.is_empty() && !reset_contains(reset, short) => {
                Disposition::Handler
            }
            // Reset-listed (subshell reset, trap.c:1480 reset_signal_handlers
            // restores the original disposition) behaves like no trap.
            Some(_) if reset_contains(reset, short) => Disposition::Default,
            // trap '' SIG: ignored (trap.def ignore path, kernel SIG_IGN).
            Some(_) => Disposition::Ignore,
            None => Disposition::Default,
        };
        (sig, want)
    })
    .collect();
    kernel_signals::reconcile(&dispositions);
}

/// Short signal name (TERM) for the __RUBASH_TRAP_SIG* key family, matching
/// signal_trap_name's SIG{name} form minus the SIG prefix.
#[cfg(unix)]
fn signal_short_name(signal: i32) -> &'static str {
    const HUP: i32 = libc::SIGHUP as i32;
    const INT: i32 = libc::SIGINT as i32;
    const QUIT: i32 = libc::SIGQUIT as i32;
    const TERM: i32 = libc::SIGTERM as i32;
    const USR1: i32 = libc::SIGUSR1 as i32;
    const USR2: i32 = libc::SIGUSR2 as i32;
    match signal {
        HUP => "HUP",
        INT => "INT",
        QUIT => "QUIT",
        TERM => "TERM",
        USR1 => "USR1",
        USR2 => "USR2",
        _ => "",
    }
}

#[cfg(unix)]
fn reset_contains(reset: &str, short: &str) -> bool {
    // Reset entries carry the table's SIG-prefixed name (trap.rs
    // normalize_signal keeps the SIG prefix, e.g. "SIGTERM").
    let full = format!("SIG{short}");
    reset.split(':').any(|entry| entry == full)
}

/// Whether `sig` is currently caught by the kernel backend (unix arm). A
/// pending entry whose signal is not caught is stale (see
/// kernel_signals::handler_installed) and must be dropped, not dispatched.
#[cfg(unix)]
pub fn kernel_handler_installed(signal: i32) -> bool {
    kernel_signals::handler_installed(signal)
}

/// pre_exec hook for spawned child processes (unix arm of rubash#229):
/// restore GNU's child signal dispositions between fork and exec. A no-op
/// when the backend was never installed (the process then still holds its
/// entry dispositions everywhere).
#[cfg(unix)]
pub fn child_pre_exec_reset() -> std::io::Result<()> {
    kernel_signals::pre_exec_child_dispositions()
}

#[cfg(all(test, unix))]
mod kernel_signal_tests {
    #[test]
    fn kernel_signal_reaches_pending_queue() {
        super::register_signal_mailbox(std::process::id()).unwrap();
        // rubash#226 model: no handler exists until a trap names the signal
        // (GNU leaves non-interactive terminating signals SIG_DFL), so the
        // test must install one first or the raise below would kill the test
        // process through the kernel default action.
        let sig = libc::SIGUSR1 as i32;
        super::kernel_signals::reconcile(&[(sig, super::kernel_signals::Disposition::Handler)]);
        assert!(super::kernel_signals::handler_installed(sig));
        unsafe { libc::kill(std::process::id() as libc::pid_t, libc::SIGUSR1) };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if super::take_pending_signals(std::process::id())
                .unwrap()
                .contains(&libc::SIGUSR1)
            {
                // Restore the default disposition so later tests in this
                // process are not affected by the caught handler.
                super::kernel_signals::reconcile(&[(
                    sig,
                    super::kernel_signals::Disposition::Default,
                )]);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("SIGUSR1 did not reach the pending-signal queue");
    }
}

/// Compile-time-table guards: the mksignames.c contract (support/signames.c
/// initialize_signames) says every number must come from the target's
/// signal numbering, so each row is asserted against the libc constant of
/// the platform the tests run on (CI: ubuntu + macos — this is the trip
/// wire for Darwin drift, where CHLD=20/CONT=19/USR1=30/EMT=7).
#[cfg(all(test, unix))]
mod signal_table_tests {
    /// Every row's digit string must parse back to its own number: the
    /// second field feeds `kill -l USR1`-style name→number output.
    #[test]
    fn digit_field_matches_signal_number() {
        for (number, digits, _) in super::SIGNALS {
            assert_eq!(
                digits.parse::<i32>().unwrap(),
                *number,
                "digit field {digits:?} does not match number {number}"
            );
        }
    }

    /// `kill -l` output order is a GNU-compatible surface; the table must
    /// stay in strictly ascending signal-number order on every platform.
    #[test]
    fn table_is_in_ascending_signal_number_order() {
        for pair in super::SIGNALS.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "signal table out of numeric order at {} then {}",
                pair[0].0,
                pair[1].0
            );
        }
    }

    /// Number→name for the libc constants of THIS platform.
    #[test]
    fn libc_numbers_translate_to_names() {
        assert_eq!(super::signal_name(libc::SIGHUP as i32), Some("HUP"));
        assert_eq!(super::signal_name(libc::SIGINT as i32), Some("INT"));
        assert_eq!(super::signal_name(libc::SIGUSR1 as i32), Some("USR1"));
        assert_eq!(super::signal_name(libc::SIGUSR2 as i32), Some("USR2"));
        assert_eq!(super::signal_name(libc::SIGCHLD as i32), Some("CHLD"));
        assert_eq!(super::signal_name(libc::SIGCONT as i32), Some("CONT"));
        assert_eq!(super::signal_name(libc::SIGSTOP as i32), Some("STOP"));
        assert_eq!(super::signal_name(libc::SIGTSTP as i32), Some("TSTP"));
        assert_eq!(super::signal_name(libc::SIGTERM as i32), Some("TERM"));
    }

    /// Name→number round trip through the public spec parser.
    #[test]
    fn names_translate_to_libc_numbers() {
        for (name, constant) in [
            ("HUP", libc::SIGHUP as i32),
            ("USR1", libc::SIGUSR1 as i32),
            ("USR2", libc::SIGUSR2 as i32),
            ("CHLD", libc::SIGCHLD as i32),
            ("CONT", libc::SIGCONT as i32),
            ("STOP", libc::SIGSTOP as i32),
            ("TSTP", libc::SIGTSTP as i32),
        ] {
            assert_eq!(
                super::signal_number_for_spec(name),
                Some(constant),
                "{name}"
            );
            assert_eq!(
                super::translate_signal(name),
                Some(constant.to_string().as_str()),
                "{name}"
            );
        }
        // `kill -l <number>` prints the name; `kill -l SIG<name>` prints the
        // number (kill.def list_signals translation).
        assert_eq!(
            super::translate_signal(&(libc::SIGUSR1 as i32).to_string()),
            Some("USR1")
        );
    }

    #[test]
    fn sigchld_and_sigcont_constants_match_table() {
        assert_eq!(super::signal_name(super::SIGCHLD_NUMBER), Some("CHLD"));
        assert_eq!(super::signal_name(super::SIGCONT_NUMBER), Some("CONT"));
        assert_eq!(super::SIGCHLD_NUMBER, libc::SIGCHLD as i32);
        assert_eq!(super::SIGCONT_NUMBER, libc::SIGCONT as i32);
    }

    /// Linux keeps the POSIX realtime block with GNU's arithmetic names
    /// (signames.c:92-139). libc::SIGRTMIN is a function on glibc, so the
    /// table pins the 34..64 contract as literals — assert the equality
    /// here so a libc/ABI drift fails CI instead of shipping silently.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_realtime_block_matches_libc() {
        assert_eq!(libc::SIGRTMIN(), 34);
        assert_eq!(libc::SIGRTMAX(), 64);
        assert_eq!(super::signal_name(34), Some("RTMIN"));
        assert_eq!(super::signal_name(64), Some("RTMAX"));
        assert_eq!(super::signal_name(35), Some("RTMIN+1"));
        assert_eq!(super::signal_name(49), Some("RTMIN+15"));
        assert_eq!(super::signal_name(50), Some("RTMAX-14"));
        assert_eq!(super::signal_number_for_spec("RTMIN"), Some(34));
        assert_eq!(super::signal_number_for_spec("RTMIN+1"), Some(35));
        assert_eq!(super::signal_number_for_spec("RTMAX"), Some(64));
        assert_eq!(super::signal_number_for_spec("SIGRTMIN+3"), Some(37));
        assert_eq!(super::signal_name(libc::SIGSTKFLT as i32), Some("STKFLT"));
        assert_eq!(super::signal_name(libc::SIGPWR as i32), Some("PWR"));
        assert_eq!(super::signal_name(libc::SIGIO as i32), Some("IO"));
    }

    /// Darwin/BSD numbering: the slots that Linux fills differently
    /// (signames.c guards SIGEMT/SIGINFO at :353/:442) must carry the BSD
    /// names, and Linux-only signals must be absent.
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    #[test]
    fn darwin_family_slots_follow_libc() {
        assert_eq!(super::signal_name(libc::SIGEMT as i32), Some("EMT"));
        assert_eq!(super::signal_name(libc::SIGINFO as i32), Some("INFO"));
        assert_eq!(super::signal_name(libc::SIGBUS as i32), Some("BUS"));
        assert_eq!(super::signal_name(libc::SIGSYS as i32), Some("SYS"));
        // Stop-family reorder: STOP=17, TSTP=18, CONT=19, CHLD=20.
        assert_eq!(libc::SIGSTOP, 17);
        assert_eq!(libc::SIGTSTP, 18);
        assert_eq!(libc::SIGCONT, 19);
        assert_eq!(libc::SIGCHLD, 20);
        // USR1/USR2 move to 30/31; slot 16 is URG, not STKFLT.
        assert_eq!(libc::SIGUSR1, 30);
        assert_eq!(libc::SIGUSR2, 31);
        assert_eq!(super::signal_name(16), Some("URG"));
        assert_eq!(super::signal_name(29), Some("INFO"));
        // No realtime signals on this platform.
        assert!(super::signal_number("RTMIN").is_none());
        assert!(super::signal_number("RTMAX").is_none());
        assert!(super::signal_name(32).is_none());
    }
}
