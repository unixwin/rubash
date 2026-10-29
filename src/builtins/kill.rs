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
///   numbers into `%LOCALAPPDATA%\rubash-signals`) and of the suspend/resume state
///   backend — Linux numbering is the design contract. DO NOT renumber.
/// * `cfg(unix)`: numbers follow the target platform via libc (Darwin:
///   7=EMT, 10=BUS, 12=SYS, 17=STOP, 18=TSTP, 19=CONT, 20=CHLD, 29=INFO,
///   30=USR1, 31=USR2; no STKFLT/PWR/RT signals). Where Linux and the BSD
///   family disagree, both alternatives occupy the same numeric slot
///   (Linux BUS=7 / Darwin EMT=7, Linux USR1=10 / Darwin BUS=10, ...), so
///   the interleaved table stays in numeric order on every platform —
///   `kill -l` walks it in order and GNU's `signal_names[]` is indexed by
///   signal number, so ordering is part of the GNU-compatible surface.
///   The Linux legs cover android too: bionic resolves the Linux kernel
///   numbering (libc's android module pins SIGBUS=7, SIGUSR1=10,
///   SIGSTKFLT=16, SIGPWR=30, SIGSYS=31, ...), so the 1..31 slots compile
///   in there unchanged.
/// * The Linux RT block pins the glibc contract 34..64 as literals:
///   `libc::SIGRTMIN` is a FUNCTION on glibc (linux_like libc, forwarding
///   to `__libc_current_sigrtmin()`), not a const, so it cannot populate a
///   const table. Android carries its own literal block for bionic's RT
///   base: `__libc_current_sigrtmin()` is 41 and `__libc_current_sigrtmax()`
///   is 64 on the Termux oracle (GNU bash 5.3.20 aarch64-linux-android
///   maps `kill -l 41` to RTMIN), so 41..64 carry the signames.c:92-139
///   arithmetic names and 32..40 stay holes.
///   `signal_table_tests::linux_realtime_block_matches_libc` and its
///   android sibling guard the equality at runtime on every CI run.
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
    #[cfg(any(target_os = "linux", target_os = "android"))]
    (libc::SIGBUS as i32, "7", "BUS"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGEMT as i32, "7", "EMT"),
    (libc::SIGFPE as i32, "8", "FPE"),
    (libc::SIGKILL as i32, "9", "KILL"),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    (libc::SIGUSR1 as i32, "10", "USR1"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGBUS as i32, "10", "BUS"),
    (libc::SIGSEGV as i32, "11", "SEGV"),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    (libc::SIGUSR2 as i32, "12", "USR2"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGSYS as i32, "12", "SYS"),
    (libc::SIGPIPE as i32, "13", "PIPE"),
    (libc::SIGALRM as i32, "14", "ALRM"),
    (libc::SIGTERM as i32, "15", "TERM"),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    (libc::SIGSTKFLT as i32, "16", "STKFLT"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGURG as i32, "16", "URG"),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    (libc::SIGCHLD as i32, "17", "CHLD"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGSTOP as i32, "17", "STOP"),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    (libc::SIGCONT as i32, "18", "CONT"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGTSTP as i32, "18", "TSTP"),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    (libc::SIGSTOP as i32, "19", "STOP"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGCONT as i32, "19", "CONT"),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    (libc::SIGTSTP as i32, "20", "TSTP"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGCHLD as i32, "20", "CHLD"),
    (libc::SIGTTIN as i32, "21", "TTIN"),
    (libc::SIGTTOU as i32, "22", "TTOU"),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    (libc::SIGURG as i32, "23", "URG"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGIO as i32, "23", "IO"),
    (libc::SIGXCPU as i32, "24", "XCPU"),
    (libc::SIGXFSZ as i32, "25", "XFSZ"),
    (libc::SIGVTALRM as i32, "26", "VTALRM"),
    (libc::SIGPROF as i32, "27", "PROF"),
    (libc::SIGWINCH as i32, "28", "WINCH"),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    (libc::SIGIO as i32, "29", "IO"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGINFO as i32, "29", "INFO"),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    (libc::SIGPWR as i32, "30", "PWR"),
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    (libc::SIGUSR1 as i32, "30", "USR1"),
    #[cfg(any(target_os = "linux", target_os = "android"))]
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
    // Android-only POSIX realtime block, numbered 41..64 per the bionic
    // contract (see the table doc above; 32..40 are holes, exactly like
    // GNU's `#if defined (SIGRTMIN)` at signames.c:97-139 generates).
    #[cfg(target_os = "android")]
    (41, "41", "RTMIN"),
    #[cfg(target_os = "android")]
    (42, "42", "RTMIN+1"),
    #[cfg(target_os = "android")]
    (43, "43", "RTMIN+2"),
    #[cfg(target_os = "android")]
    (44, "44", "RTMIN+3"),
    #[cfg(target_os = "android")]
    (45, "45", "RTMIN+4"),
    #[cfg(target_os = "android")]
    (46, "46", "RTMIN+5"),
    #[cfg(target_os = "android")]
    (47, "47", "RTMIN+6"),
    #[cfg(target_os = "android")]
    (48, "48", "RTMIN+7"),
    #[cfg(target_os = "android")]
    (49, "49", "RTMIN+8"),
    #[cfg(target_os = "android")]
    (50, "50", "RTMIN+9"),
    #[cfg(target_os = "android")]
    (51, "51", "RTMIN+10"),
    #[cfg(target_os = "android")]
    (52, "52", "RTMIN+11"),
    #[cfg(target_os = "android")]
    (53, "53", "RTMAX-11"),
    #[cfg(target_os = "android")]
    (54, "54", "RTMAX-10"),
    #[cfg(target_os = "android")]
    (55, "55", "RTMAX-9"),
    #[cfg(target_os = "android")]
    (56, "56", "RTMAX-8"),
    #[cfg(target_os = "android")]
    (57, "57", "RTMAX-7"),
    #[cfg(target_os = "android")]
    (58, "58", "RTMAX-6"),
    #[cfg(target_os = "android")]
    (59, "59", "RTMAX-5"),
    #[cfg(target_os = "android")]
    (60, "60", "RTMAX-4"),
    #[cfg(target_os = "android")]
    (61, "61", "RTMAX-3"),
    #[cfg(target_os = "android")]
    (62, "62", "RTMAX-2"),
    #[cfg(target_os = "android")]
    (63, "63", "RTMAX-1"),
    #[cfg(target_os = "android")]
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

/// The platform's SIGQUIT number for dispatch paths that special-case the
/// shell-policy ignore (sig.c:333 sets SIGQUIT to SIG_IGN in every shell,
/// before the interactive-only SIGINT/SIGTERM installs). Linux: 3; Darwin/
/// BSD also number it 3. The non-unix value keeps the Windows file-mailbox
/// wire format's Linux numbering by contract (see SIGNALS above).
#[cfg(unix)]
pub const SIGQUIT_NUMBER: i32 = libc::SIGQUIT;
/// Non-unix (Windows mailbox wire format, Linux numbering by contract).
#[cfg(not(unix))]
pub const SIGQUIT_NUMBER: i32 = 3;

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
    execute_with_io_posix(args, stdout, stderr, false)
}

/// [`execute_with_io`] with the POSIX-mode flag. POSIX mode rejects signal
/// specifications bearing the `SIG` prefix (bash posix-mode notes, item 58:
/// `kill -s SIGKILL` and `kill -SIGKILL` report "invalid signal
/// specification" with status 1); the default mode keeps the historical
/// acceptance of both spellings.
pub fn execute_with_io_posix<W, E>(
    args: &[String],
    stdout: &mut W,
    stderr: &mut E,
    posix: bool,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    if args.is_empty() {
        write_kill_usage(stderr)?;
        return Ok(2);
    }

    // GNU builtins/kill.def:104-168 kill_builtin: ONE option loop feeds
    // both the listing dispatch and the delivery loop. `-l`/`-L` only set
    // the listing flag, `-s`/`-n` (separate or attached) and a leading
    // `-sigspec` word decode a signal (an invalid spec is remembered, not
    // reported yet — with listing set it is never reported at all), `--`
    // ends the loop, and the first non-option word ends it too. After
    // the loop: listing -> display_signal_list(rest); else a NO_SIG spec
    // -> sh_invalidsig; else no operands -> builtin_usage (EX_USAGE).
    // The previous two-branch shape (a `-l`-first branch plus a delivery
    // loop) diverged on the whole option-soup class: `kill -l -s TERM`
    // listed nothing instead of the full list, `kill -s TERM -l` read
    // `-l` as a pid operand, and `kill -l --` reported `--` as a bad
    // sigspec (rubash#1399 family).
    let mut listing = false;
    let mut signal: Option<i32> = Some(15);
    let mut sigspec: &str = "TERM";
    // kill.def saw_signal: once any signal spec is seen (separate or
    // attached), a later `-word` is an operand (process group) rather
    // than another signal specification.
    let mut saw_signal = false;
    let mut index = 0;
    while let Some(value) = args.get(index).map(String::as_str) {
        if value == "-l" || value == "-L" {
            listing = true;
            index += 1;
            continue;
        }
        // kill.def:114-142: `-s`/`-n` take the next word; the spec may
        // also be attached (`-sNAME` with an alphabetic next char,
        // `-nNUM` with a digit). A trailing separate form with no next
        // word is sh_needarg ("option requires an argument", kill.def:
        // 130-131) and returns EXECUTION_FAILURE immediately — even in
        // listing mode (rubash#1399: `kill -l -s`).
        let spec_form: Option<(&str, usize)> = if value == "-s" || value == "-n" {
            match args.get(index + 1).map(String::as_str) {
                Some(spec) => Some((spec, 2)),
                None => {
                    writeln!(
                        stderr,
                        "{}kill: {value}: option requires an argument",
                        diagnostic_prefix()
                    )?;
                    return Ok(1);
                }
            }
        } else if let Some(spec) = value
            .strip_prefix("-s")
            .filter(|s| s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
        {
            Some((spec, 1))
        } else if let Some(spec) = value
            .strip_prefix("-n")
            .filter(|s| s.chars().next().is_some_and(|c| c.is_ascii_digit()))
        {
            Some((spec, 1))
        } else {
            None
        };
        if let Some((spec, advance)) = spec_form {
            sigspec = spec;
            signal = decode_signal_spec(spec, posix);
            saw_signal = true;
            index += advance;
            continue;
        }
        if value == "--" {
            index += 1;
            break;
        }
        if value == "-?" {
            // kill.def ISOPTION(word, '?'): builtin_usage, EX_USAGE.
            write_kill_usage(stderr)?;
            return Ok(2);
        }
        if value.starts_with('-') && !saw_signal {
            // kill.def takes the signal spec as the word after exactly
            // one leading dash, so `--badflag` reports `-badflag` in the
            // diagnostic (rubash#1259) and a bare `-` decodes the empty
            // spec (invalid, per the GNU oracle).
            sigspec = value.strip_prefix('-').unwrap_or(value);
            signal = decode_signal_spec(sigspec, posix);
            saw_signal = true;
            index += 1;
            continue;
        }
        break;
    }
    let operands_start = index;

    if listing {
        return display_kill_list(&args[operands_start..], posix, stdout, stderr);
    }

    let Some(signal) = signal else {
        // kill.def:171-175: sig == NO_SIG -> sh_invalidsig(sigspec).
        writeln!(
            stderr,
            "{}kill: {sigspec}: invalid signal specification",
            diagnostic_prefix()
        )?;
        return Ok(1);
    };

    if operands_start >= args.len() {
        write_kill_usage(stderr)?;
        return Ok(2);
    }

    // GNU kill.def:87-268: only real kill_pid attempts move the counters
    // — any_succeeded for a delivered signal, any_failed for a failed
    // delivery. sh_badpid/DUP_JOB diagnostics touch neither counter, so
    // `kill -0 pid badspec` still returns 0. The final status is
    // `any_succeeded ? 0 : 1` (posix: `any_failed == 0 ? 0 : 1`).
    let mut any_succeeded = false;
    let mut any_failed = false;
    for operand in &args[operands_start..] {
        if signal == 0 && matches!(operand.as_str(), "-1" | "0") {
            // Windows cannot address a Unix process group through
            // OpenProcess, but signal 0 is only an existence probe. These
            // group targets are valid Bash operands and do not require a
            // native termination call — GNU's kill(0|-1, 0) probe itself
            // succeeds, so they count as delivered.
            any_succeeded = true;
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
            continue;
        };

        // Negative pid → Unix process group (GNU kill.def:194-216
        // kill_pid: a valid negative number is delivered through
        // kill(2) as-is — the negative value IS the group addressing —
        // and a failure reports `({pid}) - {strerror}` and moves
        // any_failed, exactly like a positive pid). Windows has no such
        // target, so the silent skip stays there (signal 0 probes of
        // the current group are handled above). rubash#1631.
        if pid < 0 {
            #[cfg(unix)]
            match signal_process_group(pid, signal) {
                Ok(()) => any_succeeded = true,
                Err(message) => {
                    writeln!(stderr, "{}kill: ({pid}) - {message}", diagnostic_prefix())?;
                    any_failed = true;
                }
            }
            continue;
        }

        let pid = pid as u32;

        // Bash treats pid 0 as the current process group. Windows does not
        // expose that target through OpenProcess, but signal 0 is only an
        // existence probe, so the shell can answer it from its own group
        // context without attempting to terminate a process.
        if pid == 0 && signal == 0 {
            any_succeeded = true;
            continue;
        }

        // rubash#1654: a delivery addressed to the shell's own pid
        // from inside a flat subshell layer (ancestor route armed) is
        // judged by the top-level layer that OWNS the pid, not by the
        // subshell layer's just-reprogrammed kernel disposition —
        // queue it for the depth-0 boundary dispatch instead of
        // raising it into the child's SIG_IGN / handler. Signal 0
        // probes and non-adjudicable signals (SIGKILL, stop class)
        // keep the raw path.
        #[cfg(unix)]
        if pid == std::process::id()
            && signal != 0
            && ancestor_route_active()
            && boundary_adjudicable_signal(signal)
        {
            queue_ancestor_signal(signal);
            any_succeeded = true;
            continue;
        }

        match deliver_rubash_signal(pid, signal) {
            Ok(true) => {
                any_succeeded = true;
                continue;
            }
            Ok(false) => {}
            Err(error) => {
                writeln!(stderr, "{}kill: ({pid}) - {error}", diagnostic_prefix())?;
                any_failed = true;
                continue;
            }
        }

        if let Err(message) = signal_process(pid, signal) {
            writeln!(stderr, "{}kill: ({pid}) - {message}", diagnostic_prefix())?;
            any_failed = true;
        } else {
            any_succeeded = true;
        }
    }

    Ok(if posix {
        i32::from(any_failed)
    } else {
        i32::from(!any_succeeded)
    })
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
/// GNU jobs.c:1955 j_strsignal prints strsignal(signal) in the job-death
/// notice (`Killed` for SIGKILL, jobs.c:1993 WTERMSIG). The invariant
/// Linux/MSVC signal numbers map to the glibc strsignal wording the WSL
/// GNU 5.3.0 oracle prints (probe 2026-10-12: `kill -9 $p` notices show
/// `Killed`, `-1` `Hangup`, `-6` `Aborted`, `-8` `Floating point
/// exception`, `-10`/`-12` `User defined signal N`, `-14` `Alarm clock`).
/// The numeric fallback mirrors j_strsignal's own `_("Signal %d")` tail.
fn signal_death_description_fallback(signal: i32) -> String {
    let text = match signal {
        1 => "Hangup",
        2 => "Interrupt",
        3 => "Quit",
        4 => "Illegal instruction",
        5 => "Trace/breakpoint trap",
        6 => "Aborted",
        7 => "Bus error",
        8 => "Floating point exception",
        9 => "Killed",
        10 => "User defined signal 1",
        11 => "Segmentation fault",
        12 => "User defined signal 2",
        13 => "Broken pipe",
        14 => "Alarm clock",
        15 => "Terminated",
        16 => "Stack fault",
        17 => "Child exited",
        18 => "Continued",
        19 => "Stopped (signal)",
        20 => "Stopped",
        21 => "Stopped (tty input)",
        22 => "Stopped (tty output)",
        23 => "Urgent I/O condition",
        24 => "CPU time limit exceeded",
        25 => "File size limit exceeded",
        26 => "Virtual timer expired",
        27 => "Profiling timer expired",
        28 => "Window changed",
        29 => "I/O possible",
        30 => "Power failure",
        31 => "Bad system call",
        _ => return format!("Signal {signal}"),
    };
    text.to_string()
}

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
    signal_death_description_fallback(signal)
}

pub fn translate_signal(value: &str) -> Option<&'static str> {
    if value == "0" {
        return Some("EXIT");
    }

    if let Ok(mut number) = value.parse::<i32>() {
        if number > 128 {
            number -= 128;
        }
        return signal_name(number);
    }

    decode_signal_name(value, true)
}

/// GNU general.c valid_number: leading whitespace is skipped, an
/// optional sign follows, then digits, then trailing whitespace, and
/// the whole word must be consumed. Returns the parsed value.
fn valid_number(value: &str) -> Option<i64> {
    let trimmed = value.trim();
    let digits = trimmed
        .strip_prefix('+')
        .or_else(|| trimmed.strip_prefix('-'))
        .unwrap_or(trimmed);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    trimmed.parse::<i64>().ok()
}

/// The `SIG` prefix of a signal spec, folded case-insensitively (GNU
/// trap.c:236 decode_signal compares with strcasecmp under DSIG_NOCASE,
/// which kill.def:103 sets unconditionally).
fn strip_sig_prefix(value: &str) -> Option<&str> {
    (value.len() >= 3 && value[..3].eq_ignore_ascii_case("SIG")).then(|| &value[3..])
}

/// GNU trap.c:236 decode_signal, name half, with the kill.def:103 /
/// common.c:823 dflags (DSIG_NOCASE always; DSIG_SIGPREFIX unless POSIX
/// mode): signal_names[] entries are `SIG<name>` — the bare name compares
/// first and the prefixed spelling second — except slot 0, whose entry
/// is `EXIT` with NO `SIG` in it, so `EXIT` (any case) decodes to 0 but
/// a prefixed `SIGEXIT` (any case) is NO_SIG in every mode. Returns the
/// signal number as a string. The previous implementation stripped only
/// an exact-uppercase `SIG` and special-cased `SIGEXIT` as valid, so
/// mixed/lowercase prefixed spellings (`SigTerm`, `sigterm`, `sighup`)
/// were rejected by `kill -l` (rubash#1386) and the `-s` validator.
fn decode_signal_name(value: &str, allow_sig_prefix: bool) -> Option<&'static str> {
    if value.eq_ignore_ascii_case("EXIT") {
        return Some("0");
    }
    let bare = if allow_sig_prefix {
        strip_sig_prefix(value).unwrap_or(value)
    } else {
        value
    };
    if bare.eq_ignore_ascii_case("EXIT") {
        return None;
    }
    signal_number(bare)
}

/// GNU trap.c:236 decode_signal, whole spec: a valid_number is the
/// signal number itself (no 128 adjustment — that mapping exists only
/// in display_signal_list's exit-status translation), anything else is
/// a name. A number naming a hole in the table (glibc's reserved 32/33)
/// stays NO_SIG here: the SIGNALS table is the acceptance contract the
/// rest of the kill machinery (mailbox wire numbers, `kill -l` holes)
/// is built on.
fn decode_signal_spec(value: &str, posix: bool) -> Option<i32> {
    if let Some(number) = valid_number(value) {
        if number == 0 {
            return Some(0);
        }
        return (number >= 1 && number <= 64 && signal_name(number as i32).is_some())
            .then_some(number as i32);
    }
    decode_signal_name(value, !posix)?.parse::<i32>().ok()
}

fn signal_number_from_spec(value: &str) -> Option<i32> {
    decode_signal_spec(value, false)
}

/// The kill signal-spec validator for the `-s`/`-n`/`-SPEC` forms:
/// exactly decode_signal_spec (kept as a named entry for the POSIX
/// sigspec unit tests). The previous validator accepted a spec through
/// translate_signal but the caller then re-decoded it with an
/// exact-uppercase-only prefix strip and an exact `EXIT` compare, so
/// `kill -s SigTerm` was rejected and `kill -s Exit` fell through to
/// the TERM default and delivered the wrong signal (rubash#1385).
fn sigspec_accepted(value: &str, posix: bool) -> bool {
    decode_signal_spec(value, posix).is_some()
}

/// GNU builtins/common.c:747 display_signal_list over the words left
/// after kill's option loop: no words -> the full list (POSIX mode: the
/// one-line bare-names form, rubash#1054); a numeric word prints the
/// signal name (with the >128 exit-status adjustment; an in-range hole
/// prints nothing and keeps rc 0, rubash#230; 0 prints EXIT); a name
/// word prints its number via decode_signal; an undecodable word is
/// sh_invalidsig and the status becomes EXECUTION_FAILURE.
fn display_kill_list<W, E>(
    list: &[String],
    posix: bool,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    if list.is_empty() {
        if posix {
            write_signal_list_posix(stdout)?;
        } else {
            write_signal_list(stdout)?;
        }
        return Ok(0);
    }
    let mut status = 0;
    for value in list {
        let value = value.as_str();
        if let Some(mut number) = valid_number(value) {
            if number > 128 {
                number -= 128;
            }
            if number == 0 {
                writeln!(stdout, "EXIT")?;
            } else if !(0..=64).contains(&number) {
                writeln!(
                    stderr,
                    "{}kill: {value}: invalid signal specification",
                    diagnostic_prefix()
                )?;
                status = 1;
            } else if let Some(translation) = signal_name(number as i32) {
                writeln!(stdout, "{translation}")?;
            }
            continue;
        }
        if let Some(translation) = decode_signal_name(value, !posix) {
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
    Ok(status)
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

/// Record a console `CTRL_C_EVENT` into this process's self-signal queue.
///
/// The niubash host owns `SetConsoleCtrlHandler` on Windows and must not
/// terminate the shell; it calls this so the engine's wait-slice and
/// command-boundary polls (`observe_signals_while_blocked` /
/// `run_pending_signal_traps`) observe Ctrl+C the same way they would a
/// mailbox SIGINT from another process. Without it, an in-process wait
/// such as `sleep_interruptibly` never sees the console event and the
/// command runs to completion (niubash D8).
///
/// Unix: no-op — the kernel `signal_hook` backend already receives SIGINT
/// for the process; a second injection would double-dispatch.
pub(crate) fn note_console_ctrl_c() {
    // PLAN row 11 (M-3): this hook is engine plumbing now. The
    // consumed faces are `script_driver::inject_console_ctrl_c`
    // (host handlers) and `Executor::inject_signal` /
    // `Executor::interrupt` (Tier A). Demoted to `pub(crate)`
    // in the Session-migration follow-up recorded in the API
    // ledger: niubash's migration consumes the faces above, so
    // nothing outside this crate names the hook any more.
    #[cfg(not(unix))]
    {
        // Linux mailbox wire format: SIGINT = 2.
        queue_self_signal(2);
    }
}

/// Install this process's signal-receiving backend.
///
/// Windows: a per-process `{pid}.alive` marker in the shared
/// `%LOCALAPPDATA%\rubash-signals` directory; other rubash processes deliver
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

/// Signal numbers (1..=64) whose disposition was SIG_IGN when this
/// process started — GNU trap.c's original_signals snapshot, feeding
/// the trap table's hard-ignore seed (trap.rs seed_startup_traps,
/// rubash#1124). Probed once per process; empty on non-unix, where the
/// trap table is transported through the environment instead.
#[cfg(unix)]
pub(crate) fn entry_ignored_signal_numbers() -> Vec<i32> {
    let mask = kernel_signals::entry_ignored_mask_all();
    (1..=64)
        .filter(|sig| mask & (1u64 << (sig - 1)) != 0)
        .collect()
}

#[cfg(not(unix))]
pub(crate) fn entry_ignored_signal_numbers() -> Vec<i32> {
    Vec::new()
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

#[cfg(unix)]
pub fn register_signal_mailbox_async(_pid: u32) {
    // Unix keeps kernel dispositions (see register_signal_mailbox): there
    // is no marker file, so the async Windows path has no work — a no-op
    // keeps the init.rs call site platform-uniform.
    // rubash#1027: startup does only the fd-free capture/policy step;
    // the signal_hook backend (self-pipe socketpair) installs lazily on
    // the first trapped signal (kernel_signals::reconcile).
    kernel_signals::capture_entry_and_policy();
}

/// rubash#1882: the fd-free startup policy capture WITHOUT the mailbox
/// registration, for executor children that must not own a TERM mailbox
/// (coproc children — a blocked coprocess reader cannot consume a queued
/// TERM) but must still run under the shell's signal policy: entry
/// dispositions captured, SIGPIPE reset to SIG_DFL (#1333), SIGQUIT
/// policy applied. Off Unix there is no kernel-disposition capture to
/// run (the mailbox marker is the only registration artifact), so this
/// is a no-op there.
pub fn capture_entry_policy_without_mailbox() {
    #[cfg(unix)]
    kernel_signals::capture_entry_and_policy();
}

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

/// Lock-free "at least one signal may be waiting" flag (rubash#437). The
/// per-command boundary poll (`run_pending_signal_traps`) runs once per
/// loop iteration; before this flag, `take_all_signals` took the
/// SELF_SIGNALS mutex AND the kernel backend mutex (plus a pending()
/// collect) on every poll even when nothing was pending — profiled as the
/// single hottest self-time frame in a `while` arithmetic loop. Producers
/// (queue_self_signal, and the kernel handler via kernel_signals::note
/// hook) set it with Release; the drain reads it with Acquire and clears
/// it only after both queues are empty, so a delivery that races the clear
/// either lands before the clear (drained next poll) or re-sets the flag.
#[cfg(unix)]
static PENDING_ANY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
    // Lock-free fast path (rubash#437). Two independent producers feed the
    // two queues, and either one being provably empty lets us skip its
    // mutex entirely:
    //   * SELF_SIGNALS  -> guarded by PENDING_ANY (set in queue_self_signal).
    //   * kernel queue  -> a signal_hook handler is only installed for a
    //     signal that is actually trapped, so when HANDLER_SET == 0 no
    //     kernel delivery can be pending and the backend mutex + pending()
    //     collect can be skipped. This is the steady state of a script
    //     with no traps (the common loop case).
    let self_pending = PENDING_ANY.load(std::sync::atomic::Ordering::Acquire);
    let kernel_pending = kernel_signals::handler_any_installed();
    // rubash#1475: the fatal pair's handler is always installed, so its
    // queue is a third independent producer for this drain.
    let fatal_pending = kernel_signals::fatal_pending_any();
    if !self_pending && !kernel_pending && !fatal_pending {
        return Vec::new();
    }
    let mut signals = if self_pending {
        take_self_signals()
    } else {
        Vec::new()
    };
    if kernel_pending {
        signals.extend(kernel_signals::drain());
    }
    if fatal_pending {
        signals.extend(kernel_signals::drain_fatal());
    }
    // Clear only when both sources are provably empty again. A delivery
    // that raced this drain either set PENDING_ANY or installed a handler
    // (both Release stores), so re-reading here and clearing with Release
    // cannot lose a wakeup: the racing store is either visible now (flag
    // stays / self queue non-empty) or lands strictly after this clear.
    if take_self_signals().is_empty() && !kernel_signals::handler_any_installed() {
        PENDING_ANY.store(false, std::sync::atomic::Ordering::Release);
    }
    signals
}

pub(crate) fn queue_self_signal(signal: i32) {
    SELF_SIGNALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(signal);
    #[cfg(unix)]
    {
        PENDING_ANY.store(true, std::sync::atomic::Ordering::Release);
        kernel_signals::note_self_delivery(signal);
    }
}

fn take_self_signals() -> Vec<i32> {
    let mut queue = SELF_SIGNALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::mem::take(&mut *queue)
}

/// rubash#1654: signals a flat subshell layer addressed to the shell's
/// own pid (`kill -s TERM $$` inside `( ... )`). In GNU the subshell
/// is a separate process and `$$` names the PARENT shell, whose own
/// dispositions judge the delivery; here the layers share one process
/// and one kernel disposition table, which the subshell body's
/// `trap ''` / `trap '...'` may just have reprogrammed — a raw
/// raise(2) is swallowed by the child's SIG_IGN, or queued under the
/// child's handler and then dropped as stale once the parent boundary
/// reconciles back. So while the ancestor route is armed (see below),
/// the kill builtin queues such deliveries here instead, and the
/// first depth-0 boundary poll dispatches them against the restored
/// top-level table (trap_exec::run_pending_signal_traps): trapped ->
/// run the action, ignored -> drop, untrapped -> the 128+sig death —
/// exactly the verdicts the addressed process would have produced.
static ANCESTOR_SIGNALS: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn queue_ancestor_signal(signal: i32) {
    ANCESTOR_SIGNALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(signal);
    #[cfg(unix)]
    {
        PENDING_ANY.store(true, std::sync::atomic::Ordering::Release);
        // An in-process routed delivery is by construction not a
        // kernel/group delivery (same record queue_self_signal makes).
        kernel_signals::note_self_delivery(signal);
    }
}

pub(crate) fn take_ancestor_signals() -> Vec<i32> {
    let mut queue = ANCESTOR_SIGNALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::mem::take(&mut *queue)
}

thread_local! {
    /// Armed by Executor::execute_kill while the current executor is
    /// a flat subshell layer sharing this process (rubash#1654): kill
    /// deliveries addressed to the shell's own pid then route to
    /// ANCESTOR_SIGNALS instead of raise(2). A Session is confined to
    /// one thread and the builtin's delivery is synchronous, so a
    /// thread-local flag is exact; the guard restores the prior value
    /// so nested kill invocations cannot leak the routing.
    static ANCESTOR_ROUTE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn ancestor_route_active() -> bool {
    ANCESTOR_ROUTE.with(|flag| flag.get())
}

/// RAII arm for the ancestor route (rubash#1654).
pub(crate) struct AncestorRouteGuard {
    previous: bool,
}

pub(crate) fn arm_ancestor_route() -> AncestorRouteGuard {
    let previous = ANCESTOR_ROUTE.with(|flag| flag.replace(true));
    AncestorRouteGuard { previous }
}

impl Drop for AncestorRouteGuard {
    fn drop(&mut self) {
        ANCESTOR_ROUTE.with(|flag| flag.set(self.previous));
    }
}

// rubash#375/perf3: the per-command signal-poll's Windows-port catch_flag.
// GNU run_pending_traps (trap.c:336) is `if (catch_flag == 0) return;` —
// one int test, because the KERNEL handler (sig.c trap_handler) sets
// catch_flag asynchronously the instant a signal arrives. Windows has no
// kernel signal delivery into the shell process, so the file mailbox is
// the transport and the empty check used to be a directory pattern query
// every 64th command (~100us per query under real-time scanning,
// 1.7us/command amortized — 15% of the 20000-iteration null-command
// loop's wall). A named auto-reset kernel Event is the faithful
// catch_flag port: cross-process senders SetEvent after their mailbox
// rename; the per-command check is WaitForSingleObject(, 0) on a handle
// opened once (~sub-us); the filesystem query runs only when the event
// fired OR on the retained every-64th safety tick (senders that cannot
// set the event — a different binary version, a restricted session —
// keep today's delivery cadence). Auto-reset coalescing is exactly the
// "at least one delivery is pending" contract: the drain reads ALL
// pending entries whenever it runs, and a delivery landing between the
// wait and the drain leaves the event set for the next command.
#[cfg(windows)]
mod signal_hint {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::HANDLE;

    extern "system" {
        fn CreateEventW(
            attributes: *const core::ffi::c_void,
            manual_reset: i32,
            initial_state: i32,
            name: *const u16,
        ) -> HANDLE;
        fn WaitForSingleObject(handle: HANDLE, milliseconds: u32) -> u32;
        fn CloseHandle(handle: HANDLE) -> i32;
    }

    const EVENT_NAME_PREFIX: &str = "Local
ubash-signal-hint-";

    fn event_name(pid: u32) -> Vec<u16> {
        let wide: Vec<u16> = format!("{EVENT_NAME_PREFIX}{pid}").encode_utf16().collect();
        let mut name = wide;
        name.push(0);
        name
    }

    /// A kernel HANDLE is an opaque integer the kernel interprets; waiting
    /// on it from any thread is safe (object-level synchronization), so the
    /// OnceLock cell can carry it across threads.
    struct HintHandle(HANDLE);
    unsafe impl Send for HintHandle {}
    unsafe impl Sync for HintHandle {}

    fn open_or_create(pid: u32) -> Option<HintHandle> {
        // CreateEventW on an existing name returns the existing event with
        // ERROR_ALREADY_EXISTS — both directions are fine (the handle is
        // ours to wait on; the kernel refcounts the object).
        let handle = unsafe {
            CreateEventW(
                core::ptr::null(),
                0, // auto-reset
                0, // initially nonsignaled
                event_name(pid).as_ptr(),
            )
        };
        (!handle.is_null()).then(|| HintHandle(handle))
    }

    fn hint_handle() -> Option<HANDLE> {
        static HANDLE_CELL: OnceLock<Option<HintHandle>> = OnceLock::new();
        // The handle lives for the whole process: the OS closes it at exit
        // (same lifecycle as the {pid}.alive marker) — one handle per
        // process is the documented trade.
        HANDLE_CELL
            .get_or_init(|| open_or_create(std::process::id()))
            .as_ref()
            .map(|holder| holder.0)
    }

    /// True when a cross-process sender signaled a pending mailbox entry.
    /// Any failure (restricted session, exhausted handles) reads as false —
    /// the periodic fallback tick keeps polling the filesystem, so delivery
    /// semantics never depend on the event.
    pub fn signaled() -> bool {
        let Some(handle) = hint_handle() else {
            return false;
        };
        const WAIT_OBJECT_0: u32 = 0;
        unsafe { WaitForSingleObject(handle, 0) == WAIT_OBJECT_0 }
    }

    /// Whether this process's hint event exists. When it does, every
    /// same-version sender pulses it at delivery time, so the filesystem
    /// fallback cadence stretches (eventless senders — an older binary, a
    /// foreign tool writing the mailbox format — still get delivered, just
    /// at the stretched cadence instead of the tight one).
    pub fn available() -> bool {
        hint_handle().is_some()
    }

    /// Best-effort signal from the SENDER side (deliver_rubash_signal):
    /// open the target's event and pulse it. Failures are ignored — the
    /// mailbox entry itself is the source of truth.
    pub fn signal_target(pid: u32) {
        extern "system" {
            fn OpenEventW(desired_access: u32, inherit: i32, name: *const u16) -> HANDLE;
            fn SetEvent(handle: HANDLE) -> i32;
        }
        const EVENT_MODIFY_STATE: u32 = 0x0002;
        let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, event_name(pid).as_ptr()) };
        if handle.is_null() {
            return;
        }
        unsafe {
            SetEvent(handle);
            CloseHandle(handle);
        }
    }
}

#[cfg(not(windows))]
mod signal_hint {
    pub fn signaled() -> bool {
        false
    }
    #[allow(unused_variables)]
    pub fn signal_target(pid: u32) {}
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
        // catch_flag shape (see mod signal_hint): the kernel event fires the
        // instant a same-version cross-process delivery lands, so the
        // filesystem query runs on the event; the periodic fallback poll
        // covers senders that cannot set the event (tight cadence when the
        // event infrastructure is unavailable, stretched otherwise).
        let fallback_period: u64 = if signal_hint::available() { 1024 } else { 64 };
        // unixwin/rubash#682: WaitForSingleObject(,0) is a real syscall
        // (~0.3-0.5us) charged to EVERY command boundary — a measurable slice
        // of the shape-independent per-command floor. The auto-reset event
        // latches a sender's pulse until a wait consumes it, so sampling it
        // every HINT_POLL_PERIOD-th poll loses no delivery: a pulse landing
        // between samples keeps the event set until the next sampled
        // boundary drains the mailbox. Worst-case added latency is
        // HINT_POLL_PERIOD-1 boundaries (~25us at the ~3.5us floor), far
        // inside the 1024-tick filesystem cadence eventless senders already
        // tolerate.
        const HINT_POLL_PERIOD: u64 = 8;
        if signals.is_empty()
            && (tick % fallback_period == 0
                || (tick % HINT_POLL_PERIOD == 0 && signal_hint::signaled()))
        {
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
    // This is a SECOND producer of SELF_SIGNALS (the other is
    // queue_self_signal), so it must set the same lock-free wakeup flag:
    // the boundary poll's fast path (take_all_signals) skips the self queue
    // when PENDING_ANY is clear, and a requeue that did not raise it would
    // strand the re-queued signal forever. `wait`/observe_signals_while_blocked
    // peek a delivery, put it back here, and rely on the NEXT command
    // boundary to dispatch the trap (wait.def:174).
    #[cfg(unix)]
    PENDING_ANY.store(true, std::sync::atomic::Ordering::Release);
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
    // GNU trap.c decode_signal with DSIG_NOCASE (kill.def:103 sets it
    // unconditionally): signal names fold case (rubash#1367 #1385 #1386).
    SIGNALS.iter().find_map(|(_, number, signal_name)| {
        signal_name.eq_ignore_ascii_case(name).then_some(*number)
    })
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

/// POSIX-mode `kill -l` with no operand (rubash#1054): one line, bare
/// names in signal-number order, single-space separated.
fn write_signal_list_posix<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    let names: Vec<&str> = SIGNALS.iter().map(|(_, _, name)| *name).collect();
    writeln!(stdout, "{}", names.join(" "))?;
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
    signal_hint::signal_target(pid);
    Ok(true)
}

/// The shared signal-mailbox directory.
///
/// Windows (rubash#357): `%LOCALAPPDATA%\rubash-signals`, resolved ONCE per
/// process and cached. TMP/TEMP are deliberately NOT consulted:
///
/// - An MSYS parent forwards raw POSIX values (`TEMP=/tmp`), which Win32 file
///   APIs resolve against each process's CURRENT DRIVE — a process on `D:`
///   gets `D:\tmp\rubash-signals`, one on `C:` gets `C:\tmp\...`, while a
///   parent with the native `%TEMP%` reads `C:\Users\...\Temp\rubash-signals`.
///   Two such processes split into different boxes, the killer misses the
///   victim's `{pid}.alive` marker, and the kill falls back to
///   TerminateProcess — the trap never runs (143/NOTRAPPED, rubash#357).
/// - The process environment is also MUTABLE mid-run (a rubash child's env
///   restore can re-apply the raw spawn value), so re-reading temp_dir() per
///   call let the same process flip boxes between registration and polling
///   (observed: marker written to the native box, later polls scanning
///   `D:\tmp\rubash-signals` forever). The OnceLock pins one box for the
///   process lifetime.
///
/// `%LOCALAPPDATA%` is user-scoped, always an absolute Windows path, and
/// never MSYS-mangled, so every rubash process of the same user converges on
/// one box regardless of TMP/TEMP form or cwd drive. Fallbacks mirror the
/// user-profile chain (USERPROFILE, then the OS temp as a last resort) and
/// keep the same first-use caching.
///
/// Unix: there is no file mailbox (kernel dispositions own delivery — see
/// `register_signal_mailbox`); the plain temp join stays for the dead-marker
/// helpers that never execute there.
#[cfg(not(unix))]
fn signal_mailbox_dir() -> std::path::PathBuf {
    static MAILBOX_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    MAILBOX_DIR
        .get_or_init(|| {
            for key in ["LOCALAPPDATA", "USERPROFILE"] {
                if let Ok(value) = std::env::var(key) {
                    let path = std::path::PathBuf::from(&value);
                    if path.is_absolute() {
                        return path.join("rubash-signals");
                    }
                }
            }
            std::env::temp_dir().join("rubash-signals")
        })
        .clone()
}

#[cfg(unix)]
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

/// kill(2) with a signed pid: a negative value addresses a process
/// group (GNU kill.def kill_pid passes the operand through verbatim).
/// rubash#1631.
#[cfg(unix)]
fn signal_process_group(pid: i32, signal: i32) -> Result<(), &'static str> {
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
    use signal_hook::iterator::exfiltrator::WithOrigin;
    use signal_hook::iterator::SignalsInfo;
    use signal_hook::low_level::siginfo::Cause;
    use std::io;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};

    static KERNEL_SIGNALS: OnceLock<Mutex<SignalsInfo<WithOrigin>>> = OnceLock::new();

    /// Bitmask (same bit layout as HANDLER_SET) of managed signals whose
    /// most recently drained kernel delivery was KERNEL-caused
    /// (siginfo si_code SI_KERNEL: generated by the terminal driver for
    /// ^C/^\\, not by a kill(2) from any process). The distinction is
    /// observable only here, at the drain — the pending queues carry
    /// bare signal numbers — and the one consumer is the builtin-sleep
    /// fast path's virtual-child model (trap_exec sleep_interruptibly,
    /// rubash#1034): a real foreground child dies only from the
    /// group-addressed terminal delivery, so only a kernel-caused
    /// arrival may end the virtual sleep early; a kill(2) delivery
    /// addressed to this shell alone leaves GNU's real child running
    /// and the trap deferred until the command completes.
    static KERNEL_CAUSED: AtomicU64 = AtomicU64::new(0);

    /// Whether the most recently drained delivery of `sig` was
    /// kernel-caused (see KERNEL_CAUSED).
    pub(super) fn last_delivery_kernel_caused(sig: i32) -> bool {
        if !is_managed(sig) {
            return false;
        }
        KERNEL_CAUSED.load(Ordering::Relaxed) & bit(sig) != 0
    }

    /// An in-process self delivery (the kill builtin addressing this
    /// shell alone) supersedes the origin record for `sig`: it is by
    /// construction NOT a kernel/group delivery.
    pub(super) fn note_self_delivery(sig: i32) {
        if is_managed(sig) {
            KERNEL_CAUSED.fetch_and(!bit(sig), Ordering::Relaxed);
        }
    }

    /// Monotonic count of kernel-caused SIGINT deliveries drained so
    /// far (rubash#1990). Unlike KERNEL_CAUSED — a last-wins origin bit
    /// a later kill(2) delivery of the same signal can flip back — the
    /// epoch never decreases, so a consumer that snapshots it at the
    /// start of a wait can tell "a terminal ^C arrived DURING this
    /// wait" even if other deliveries interleave. The foreground wait
    /// (executor external_inner) uses it to forward the interrupt to a
    /// child the terminal delivery never reached.
    static SIGINT_KERNEL_EPOCH: AtomicU64 = AtomicU64::new(0);

    /// Current value of the kernel-SIGINT epoch (see SIGINT_KERNEL_EPOCH).
    pub(super) fn sigint_kernel_epoch() -> u64 {
        SIGINT_KERNEL_EPOCH.load(Ordering::Relaxed)
    }

    /// The `trap ''` ignore set, recorded from the trap TABLE by the
    /// boundary reconcile (reconcile_kernel_trap_dispositions) — never
    /// from the live kernel state word (rubash#1991). The state word
    /// also carries shell-policy ignores that exist only for this
    /// process: an interactive shell ignores SIGTERM for itself
    /// (sig.c:338-339, where get_original_signal snapshots the entry
    /// disposition BEFORE the ignore is installed). GNU's fork-child
    /// restore (trap.c:1489 restore_original_signals) restores from
    /// that entry snapshot plus the trap '' set, so a policy ignore
    /// must not leak into children — reading the live state word in
    /// pre_exec did exactly that, and every child of an interactive
    /// shell inherited SIG_IGN for SIGTERM (kill %1 then "succeeded"
    /// while the job ran on, immune to TERM).
    static TRAP_IGNORED: AtomicU64 = AtomicU64::new(0);

    /// Record whether `sig` is currently ignored via `trap ''`.
    pub(super) fn set_trap_ignored(sig: i32, ignored: bool) {
        if !is_managed(sig) {
            return;
        }
        if ignored {
            TRAP_IGNORED.fetch_or(bit(sig), Ordering::Relaxed);
        } else {
            TRAP_IGNORED.fetch_and(!bit(sig), Ordering::Relaxed);
        }
    }

    /// True when `sig` is ignored via `trap ''` (see TRAP_IGNORED).
    pub(super) fn trap_ignored(sig: i32) -> bool {
        is_managed(sig) && TRAP_IGNORED.load(Ordering::Relaxed) & bit(sig) != 0
    }

    /// Clear the whole trap '' record (the reconcile fast path, where
    /// no trap-table keys exist at all).
    pub(super) fn clear_trap_ignored_all() {
        TRAP_IGNORED.store(0, Ordering::Relaxed);
    }

    /// The named signals the backend manages dispositions for (the
    /// signal_hook-catchable set a trap can name and the executor can
    /// dispatch). SIGPIPE joined the set with rubash#1333: it used to be
    /// deliberately excluded because the Rust runtime pre-ignores it
    /// (see capture_entry_and_policy, which resets it to SIG_DFL), but
    /// GNU leaves SIGPIPE at SIG_DFL in a script shell, so an untrapped
    /// delivery must kill the shell (exit 141) and a PIPE trap must
    /// fire.
    ///
    /// rubash#1511 extends the original seven (HUP/INT/QUIT/TERM/USR1/
    /// USR2/PIPE) to every remaining trappable signal: until then a trap
    /// (or `trap ''`) on TRAP/ABRT/ALRM/STKFLT/XCPU/XFSZ/VTALRM/PROF/IO/
    /// PWR/SYS/RT* changed nothing at the kernel, so a trapped
    /// terminating signal still killed the shell by its default action
    /// and a trapped URG/WINCH (default-ignore) was never delivered —
    /// GNU installs one handler per trapped signal (sig.c:830), so all
    /// of them run their trap. SIGSEGV/SIGBUS are NOT here: the fatal
    /// set below routes them (rubash#1475), and rubash#1511 routes
    /// SIGILL/SIGFPE there too (signal-hook refuses to register them).
    /// SIGCHLD and the stop class stay unregistered for the reasons in
    /// the module doc.
    const MANAGED_CORE: &[i32] = &[
        libc::SIGHUP as i32,
        libc::SIGINT as i32,
        libc::SIGQUIT as i32,
        libc::SIGTERM as i32,
        libc::SIGUSR1 as i32,
        libc::SIGUSR2 as i32,
        libc::SIGPIPE as i32,
        libc::SIGTRAP as i32,
        libc::SIGABRT as i32,
        libc::SIGALRM as i32,
        libc::SIGXCPU as i32,
        libc::SIGXFSZ as i32,
        libc::SIGVTALRM as i32,
        libc::SIGPROF as i32,
        libc::SIGIO as i32,
        libc::SIGSYS as i32,
        libc::SIGURG as i32,
        libc::SIGWINCH as i32,
    ];

    /// Linux/Android-only managed additions: SIGSTKFLT and SIGPWR
    /// (no Darwin/BSD counterpart in GNU's table either) plus the
    /// POSIX realtime block, whose bounds are runtime values (glibc
    /// SIGRTMIN()=34, SIGRTMAX()=64; bionic 41..64; Darwin/BSD have
    /// none, where GNU's signames.c compiles the block out too).
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn extend_managed(signals: &mut Vec<i32>) {
        signals.push(libc::SIGSTKFLT as i32);
        signals.push(libc::SIGPWR as i32);
        let lo = libc::SIGRTMIN();
        let hi = libc::SIGRTMAX();
        if lo > 0 && hi >= lo {
            signals.extend(lo..=hi);
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn extend_managed(_signals: &mut Vec<i32>) {}

    /// The full managed list: MANAGED_CORE plus the platform extras.
    fn managed_signals() -> &'static [i32] {
        static MANAGED: OnceLock<Vec<i32>> = OnceLock::new();
        MANAGED.get_or_init(|| {
            let mut signals = MANAGED_CORE.to_vec();
            extend_managed(&mut signals);
            signals
        })
    }

    fn is_managed(sig: i32) -> bool {
        managed_signals().contains(&sig)
    }

    /// Bitmask (1 << index into MANAGED) of signals currently routed to the
    /// signal_hook handler.
    static HANDLER_SET: AtomicU64 = AtomicU64::new(0);

    /// Packed per-signal disposition state (2 bits each: 0=Default,
    /// 1=Ignore, 2=Handler) so the per-boundary reconcile skips
    /// unchanged signals — no sigaction traffic on the steady-state
    /// command path. Two words: rubash#1511's full managed list runs
    /// past 32 signals (102 state bits on Linux).
    static DISPOSITION_STATE: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

    /// (word, shift) of `sig`'s 2-bit field in DISPOSITION_STATE (two
    /// bits per signal, in managed_signals() order).
    fn state_slot(sig: i32) -> (usize, u32) {
        let bits = 2 * managed_signals()
            .iter()
            .position(|&m| m == sig)
            .expect("managed signal");
        (bits / 64, (bits % 64) as u32)
    }

    fn current_state(sig: i32) -> u8 {
        let (word, shift) = state_slot(sig);
        ((DISPOSITION_STATE[word].load(Ordering::Relaxed) >> shift) & 0b11) as u8
    }

    fn store_state(sig: i32, state: u8) {
        let (word, shift) = state_slot(sig);
        let mask = 0b11u64 << shift;
        let previous = DISPOSITION_STATE[word].fetch_and(!mask, Ordering::SeqCst);
        DISPOSITION_STATE[word].fetch_or((state as u64) << shift, Ordering::SeqCst);
        let _ = previous;
    }

    /// Bitmask of signals that were SIG_IGN when this process was exec'd.
    /// POSIX 2.11: a non-interactive shell keeps entry-ignored signals
    /// ignored (GNU sig.c:273-277 marks them hard-ignored).
    static ENTRY_IGNORED: AtomicU64 = AtomicU64::new(0);

    /// Process-wide probe of EVERY signal's entry disposition (bit
    /// sig-1), captured once on first use. GNU trap.c initialize_traps
    /// snapshots original_signals[] for all NSIG signals at startup;
    /// trap -p lists the SIG_IGN ones as `trap -- '' SIG` and the trap
    /// builtin refuses to trap or reset them (SIG_HARD_IGNORE,
    /// trap.c:850). The MANAGED-only ENTRY_IGNORED above cannot serve
    /// that display: entry-ignored SIGPIPE/SIGXFSZ/realtime signals are
    /// outside the managed set (rubash#1124). The probe runs before any
    /// rubash code changes a disposition (seed_startup_traps in
    /// Executor::new precedes install()'s SIGQUIT policy ignore), and
    /// the OnceLock keeps later executors (subshell clones) from
    /// re-probing a live, already-mutated disposition table.
    static ENTRY_IGNORED_ALL: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

    pub(super) fn entry_ignored_mask_all() -> u64 {
        *ENTRY_IGNORED_ALL.get_or_init(|| {
            let mut mask = 0u64;
            for sig in 1..=64 {
                if current_disposition_is_ignored(sig) {
                    mask |= 1u64 << (sig - 1);
                }
            }
            // SIGPIPE is excluded: the Rust runtime sets it to SIG_IGN
            // before main() runs, so its observed disposition is the
            // runtime's, never the invoking environment's — attributing
            // it to the parent would list `trap -- '' SIGPIPE` in shells
            // whose parent left it at SIG_DFL (GNU lists nothing there).
            // The DISPOSITION itself is separately reset to SIG_DFL in
            // capture_entry_and_policy (rubash#1333); only the entry
            // attribution stays unreconstructed, since a parent that
            // genuinely ignored SIGPIPE is indistinguishable here.
            mask &= !(1u64 << (libc::SIGPIPE as u64 - 1));
            mask
        })
    }

    fn bit(sig: i32) -> u64 {
        1 << managed_signals()
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
    fn ensure_handler(signals: &SignalsInfo<WithOrigin>, sig: i32) -> io::Result<()> {
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
            // glibc/musl/OH/bionic all expose sa_restorer as Option; None
            // lets the kernel supply the default restorer.
            #[cfg(any(target_os = "linux", target_os = "android"))]
            sa_restorer: None,
        };
        unsafe { libc::sigaction(sig, &action, std::ptr::null_mut()) };
    }

    /// Effective disposition for one managed signal, derived from the trap
    /// table: a real action -> handler, `trap ''` -> SIG_IGN, no trap ->
    /// entry disposition (SIG_DFL, or SIG_IGN for entry-ignored signals and
    /// SIGQUIT -- sig.c:333 sets SIGQUIT to SIG_IGN for every shell).
    #[derive(Clone, Copy)]
    pub(super) enum Disposition {
        Handler,
        Ignore,
        Default,
    }

    /// True when every managed signal currently sits at `Default` in the
    /// recorded disposition state, so a reconcile whose wish-list is also
    /// all-Default has nothing to do. Used by the boundary-poll fast path
    /// (rubash#437) to skip the mutex + per-signal loop on the steady-state
    /// command path. Reads the packed state words atomically; no lock.
    pub(super) fn at_default_state() -> bool {
        let total_bits = 2 * managed_signals().len() as u32;
        let mask = |word: u32| -> u64 {
            let used = total_bits.saturating_sub(64 * word);
            if used >= 64 {
                u64::MAX
            } else if used == 0 {
                0
            } else {
                (1u64 << used) - 1
            }
        };
        DISPOSITION_STATE[0].load(Ordering::Relaxed) & mask(0) == 0
            && DISPOSITION_STATE[1].load(Ordering::Relaxed) & mask(1) == 0
    }

    pub(super) fn reconcile(dispositions: &[(i32, Disposition)]) {
        // rubash#1027: the signal_hook backend is installed lazily, on
        // the first Handler wish — before that, Ignore/Default wishes
        // are plain sigaction calls that need no backend (and no
        // self-pipe socketpair on fds 3/4). Startup ran only
        // capture_entry_and_policy (fd-free).
        let mut signals_guard = None;
        for &(sig, ref want) in dispositions {
            // rubash#1250: a foreground builtin wait (sleep fast path)
            // holds untrapped SIGINT at the mailbox handler so the
            // delivery becomes an observable abort instead of a DFL
            // death. The hold outranks the trap table's Default wish
            // until released — without this, every per-slice
            // reconcile from observe_signals_while_blocked would flip
            // the routing back to SIG_DFL mid-sleep.
            if FG_INT_HOLD.load(Ordering::Relaxed)
                && sig == libc::SIGINT as i32
                && matches!(want, Disposition::Default)
            {
                continue;
            }
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
                    if signals_guard.is_none() {
                        if install().is_err() {
                            continue;
                        }
                        let Some(mutex) = KERNEL_SIGNALS.get() else {
                            continue;
                        };
                        let Ok(guard) = mutex.lock() else {
                            continue;
                        };
                        signals_guard = Some(guard);
                    }
                    let signals = signals_guard.as_ref().expect("installed above");
                    if ensure_handler(signals, sig).is_err() {
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
        if fatal_index(sig).is_some() {
            // The fatal-set handler is installed unconditionally at
            // startup (rubash#1475); a queued entry for the set is
            // legitimate by construction, never a stale registry flag.
            return true;
        }
        is_managed(sig) && HANDLER_SET.load(Ordering::Relaxed) & bit(sig) != 0
    }

    /// True when `sig` was SIG_IGN at process entry (captured by
    /// capture_entry_and_policy). Entry-ignored dispositions are sticky
    /// (trap.c:850): neither traps nor the interactive SIGINT pin
    /// (rubash#1662) may route them to the handler.
    pub(super) fn entry_ignored(sig: i32) -> bool {
        is_managed(sig) && ENTRY_IGNORED.load(Ordering::Relaxed) & bit(sig) != 0
    }

    /// Whether the boundary dispatcher can adjudicate `sig` at all
    /// (the managed set or the fatal set). The rubash#1654 ancestor
    /// route queues only signals whose top-level verdict the
    /// dispatcher owns; SIGKILL and the stop class are not adjudicable
    /// and keep raw raise(2) semantics.
    pub(super) fn adjudicable(sig: i32) -> bool {
        is_managed(sig) || fatal_index(sig).is_some()
    }

    /// rubash#1475: the fatal set, SIGSEGV and SIGBUS. Both are caught
    /// by the Rust runtime before main() — its stack-overflow guard
    /// installs a handler for them (visible in /proc/self/status SigCgt
    /// on every rubash process) — and that handler RETURNS for a signal
    /// that was SENT rather than faulted, so an untrapped `kill -SEGV
    /// $$`, whether from this shell's own kill builtin (a real raise(2)
    /// on unix) or from any other process, was silently swallowed and
    /// the shell ran on. GNU runs both at SIG_DFL in a script shell
    /// (sig.c initialize_signals never catches them), so an untrapped
    /// delivery kills the shell and a trapped one runs its trap.
    /// signal_hook cannot serve the pair: its handler is a signo queue
    /// writer with no siginfo, and returning from a handler for a
    /// synchronous fault re-executes the faulting instruction forever.
    ///
    /// rubash#1511 adds SIGILL and SIGFPE to the set: signal-hook's
    /// registry flatly refuses to register them (it panics with
    /// "Attempted to register forbidden signal"), so the managed
    /// backend cannot route them either. They share the synchronous-
    /// fault nature that demands the si_code split below — a SENT
    /// delivery gets shell semantics, a genuine fault chains — so they
    /// ride this backend rather than the signo queue.
    /// The custom handler below splits arrivals on siginfo.si_code:
    ///   - si_code <= 0 (SI_USER / SI_TKILL / SI_QUEUE / ...): the
    ///     signal was SENT. With a trap armed, count it for the boundary
    ///     dispatcher (drained through drain_fatal into the same
    ///     Vec<i32> as the managed set); with `trap ''`, drop it;
    ///     untrapped, restore SIG_DFL and re-raise so the process dies
    ///     BY the signal on the spot, exactly as a default disposition.
    ///   - si_code > 0: a hardware fault (SEGV_MAPERR, BUS_ADRALN...).
    ///     Chain to the disposition saved at install time — in practice
    ///     the runtime's overflow handler — so stack-overflow detection
    ///     and genuine-fault death keep working byte for byte.
    const FATAL: [i32; 4] = [
        libc::SIGSEGV as i32,
        libc::SIGBUS as i32,
        libc::SIGILL as i32,
        libc::SIGFPE as i32,
    ];

    /// Queued SENT deliveries per fatal signal (handler increments,
    /// drain_fatal swaps out; a plain atomic add is async-signal-safe).
    static FATAL_PENDING: [AtomicU64; 4] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];

    /// Trap routing for SENT deliveries, per fatal signal:
    /// 0 = Default (die), 1 = Ignore (drop), 2 = Handler (queue).
    /// Mirrored from the trap table by reconcile_fatal.
    static FATAL_STATE: [AtomicU64; 4] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];

    /// Dispositions saved when the fatal handler was installed — the
    /// hardware-fault chain targets. Written once inside
    /// capture_entry_and_policy's Once before the handler is installed;
    /// the Once's happens-before edge covers the handler's later reads.
    static FATAL_PREV_HANDLER: [AtomicU64; 4] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];
    static FATAL_PREV_FLAGS: [AtomicU64; 4] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];

    fn fatal_index(sig: i32) -> Option<usize> {
        FATAL.iter().position(|&f| f == sig)
    }

    extern "C" fn fatal_signal_handler(
        sig: i32,
        info: *mut libc::siginfo_t,
        context: *mut libc::c_void,
    ) {
        let Some(index) = fatal_index(sig) else {
            return;
        };
        let si_code = if info.is_null() {
            0
        } else {
            unsafe { (*info).si_code }
        };
        if si_code > 0 {
            // Hardware fault: chain to the saved disposition.
            let prev = FATAL_PREV_HANDLER[index].load(Ordering::Relaxed) as usize;
            match prev {
                libc::SIG_IGN => return,
                libc::SIG_DFL | 0 => {
                    set_kernel_handler(sig, libc::SIG_DFL);
                    unsafe { libc::raise(sig) };
                    return;
                }
                _ => {}
            }
            let prev_flags = FATAL_PREV_FLAGS[index].load(Ordering::Relaxed) as i32;
            // SAFETY: prev is the handler address the process had
            // installed for this signal at startup (the Rust runtime's
            // overflow guard); invoking it with the delivery's own
            // arguments is exactly what the kernel would have done.
            unsafe {
                if prev_flags & libc::SA_SIGINFO != 0 {
                    let handler: extern "C" fn(i32, *mut libc::siginfo_t, *mut libc::c_void) =
                        std::mem::transmute(prev);
                    handler(sig, info, context);
                } else {
                    let handler: extern "C" fn(i32) = std::mem::transmute(prev);
                    handler(sig);
                }
            }
            return;
        }
        // Sent by kill/tgkill/raise/sigqueue: shell semantics apply.
        match FATAL_STATE[index].load(Ordering::Relaxed) {
            2 => {
                FATAL_PENDING[index].fetch_add(1, Ordering::Relaxed);
            }
            1 => {}
            _ => {
                set_kernel_handler(sig, libc::SIG_DFL);
                unsafe { libc::raise(sig) };
            }
        }
    }

    /// Install the fatal-set handler, saving the current dispositions
    /// as the hardware-fault chain targets. Called from
    /// capture_entry_and_policy's Once, before any trap can exist, so
    /// both signals start at Default routing (an untrapped sent delivery
    /// dies). A signal that is SIG_IGN at this point (entry-ignored, or
    /// on a platform whose runtime ignores it) starts at Ignore routing
    /// and reconcile_fatal never arms it — POSIX hard-ignore.
    fn install_fatal_handlers() {
        for (index, &sig) in FATAL.iter().enumerate() {
            let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
            unsafe { libc::sigaction(sig, std::ptr::null(), &mut current) };
            FATAL_PREV_HANDLER[index].store(current.sa_sigaction as u64, Ordering::Relaxed);
            FATAL_PREV_FLAGS[index].store(current.sa_flags as i64 as u64, Ordering::Relaxed);
            if current.sa_sigaction == libc::SIG_IGN {
                FATAL_STATE[index].store(1, Ordering::Relaxed);
            }
            let action = libc::sigaction {
                sa_sigaction: fatal_signal_handler as *const () as usize,
                sa_flags: libc::SA_SIGINFO,
                sa_mask: unsafe { std::mem::zeroed() },
                #[cfg(any(target_os = "linux", target_os = "android"))]
                sa_restorer: None,
            };
            unsafe { libc::sigaction(sig, &action, std::ptr::null_mut()) };
        }
    }

    /// Drain the fatal set's queued SENT deliveries (take_all_signals
    /// merges them with the managed drain).
    pub(super) fn drain_fatal() -> Vec<i32> {
        let mut signals = Vec::new();
        for (index, &sig) in FATAL.iter().enumerate() {
            let count = FATAL_PENDING[index].swap(0, Ordering::Relaxed);
            for _ in 0..count {
                signals.push(sig);
            }
        }
        signals
    }

    pub(super) fn fatal_pending_any() -> bool {
        FATAL_PENDING
            .iter()
            .any(|count| count.load(Ordering::Relaxed) > 0)
    }

    /// True when every fatal signal sits at Default routing, so the
    /// boundary reconcile's fast path may skip the trap-table walk.
    pub(super) fn fatal_at_default() -> bool {
        FATAL_STATE
            .iter()
            .all(|state| state.load(Ordering::Relaxed) == 0)
    }

    /// Mirror the trap table's wishes for the fatal set (the managed
    /// reconcile's Disposition mapping): Handler queues sent deliveries
    /// for the boundary dispatcher, Ignore drops them, Default restores
    /// die-on-delivery. Entry-ignored signals stay pinned at Ignore.
    pub(super) fn reconcile_fatal(dispositions: &[(i32, Disposition)]) {
        for &(sig, want) in dispositions {
            let Some(index) = fatal_index(sig) else {
                continue;
            };
            if FATAL_PREV_HANDLER[index].load(Ordering::Relaxed) as usize == libc::SIG_IGN {
                continue;
            }
            let target = match want {
                Disposition::Handler => 2u64,
                Disposition::Ignore => 1u64,
                Disposition::Default => 0u64,
            };
            FATAL_STATE[index].store(target, Ordering::Relaxed);
        }
    }

    /// rubash#1250: foreground-builtin SIGINT hold. While set, the
    /// reconcile above refuses to route SIGINT back to Default, so a
    /// builtin wait's temporary mailbox routing survives the per-slice
    /// reconciles from observe_signals_while_blocked. Single-threaded
    /// executor: no concurrent trap-table mutation can interleave, and
    /// the hold only ever arms when SIGINT is untrapped (a trapped
    /// signal keeps its handler and never reaches the hold).
    static FG_INT_HOLD: AtomicBool = AtomicBool::new(false);

    /// rubash#1824: the steady state the live hold found on entry,
    /// recorded by the FIRST holder only (a nested hold finds the
    /// outer hold's Handler routing and must not overwrite the
    /// record). The release restores this state instead of always
    /// routing back to Default: since the rubash#1662 interactive
    /// pin, an interactive shell's untrapped SIGINT already sits at
    /// the mailbox Handler when a foreground wait begins, and
    /// flipping it to Default on release would reopen the unpinned
    /// window the pin closed until the next boundary reconcile.
    static FG_INT_WAS_DEFAULT: AtomicBool = AtomicBool::new(true);

    fn sigint_at_default() -> bool {
        // sigaction(null) query, same probe as the entry-ignored
        // snapshot: only SIG_DFL counts as the Default steady state.
        let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
        if unsafe { libc::sigaction(libc::SIGINT, std::ptr::null(), &mut current) } != 0 {
            return false;
        }
        current.sa_sigaction == libc::SIG_DFL
    }

    /// Route untrapped SIGINT to the mailbox handler and hold it there.
    /// Deliveries become observable pending entries (aborted as 128+SIGINT
    /// by the waiter's slice loop) instead of DFL deaths.
    pub(super) fn hold_fg_sigint() {
        if !FG_INT_HOLD.swap(true, Ordering::SeqCst) {
            FG_INT_WAS_DEFAULT.store(sigint_at_default(), Ordering::SeqCst);
        }
        reconcile(&[(libc::SIGINT as i32, Disposition::Handler)]);
    }

    /// Release the hold: clear the flag first so the recorded wish
    /// applies, then restore the steady state the hold found
    /// (rubash#1824) — Default when the wait started from SIG_DFL,
    /// Handler when the rubash#1662 pin had already routed SIGINT
    /// to the mailbox.
    pub(super) fn release_fg_sigint() {
        FG_INT_HOLD.store(false, Ordering::SeqCst);
        let target = if FG_INT_WAS_DEFAULT.load(Ordering::SeqCst) {
            Disposition::Default
        } else {
            Disposition::Handler
        };
        reconcile(&[(libc::SIGINT as i32, target)]);
    }

    static ENTRY_CAPTURED: std::sync::Once = std::sync::Once::new();

    /// The fd-free part of startup (rubash#1027): capture entry
    /// dispositions and apply the SIGQUIT policy ignore. Neither needs
    /// the signal_hook self-pipe, so this runs at shell startup without
    /// allocating the socketpair that used to sit on fds 3/4 of every
    /// shell (and of every process listing /proc/$$/fd) for its whole
    /// life. GNU has no self-pipe at all (sig.c handlers set flags), so
    /// the pair must not exist before a trap actually needs the backend.
    pub(super) fn capture_entry_and_policy() {
        ENTRY_CAPTURED.call_once(|| {
            // Capture entry dispositions BEFORE changing anything:
            // signals ignored on entry must stay ignored (POSIX 2.11,
            // GNU sig.c:273).
            for &sig in managed_signals() {
                if sig == libc::SIGPIPE as i32 {
                    // rubash#1333: SIGPIPE's entry disposition is
                    // unobservable — the Rust runtime sets SIG_IGN before
                    // main() on every process, whatever the parent did.
                    // GNU's script shell runs with SIGPIPE at SIG_DFL
                    // (sig.c initialize_signals never ignores it), so
                    // reset it: an untrapped delivery (self `kill -PIPE`,
                    // EPIPE on a closed pipe) must kill the shell with
                    // 141 and a PIPE trap must be installable. This
                    // supersedes the rubash#440 rationale for keeping the
                    // runtime ignore: write errors now surface as the
                    // signal, exactly as under GNU.
                    set_kernel_handler(sig, libc::SIG_DFL);
                    store_state(sig, 0);
                    continue;
                }
                if current_disposition_is_ignored(sig) {
                    ENTRY_IGNORED.fetch_or(bit(sig), Ordering::SeqCst);
                }
            }
            // sig.c:102 initialize_signals -> sig.c:333: every shell
            // ignores SIGQUIT (script shells do not die on ^\).
            if ENTRY_IGNORED.load(Ordering::Relaxed) & bit(libc::SIGQUIT as i32) == 0
                && !current_disposition_is_ignored(libc::SIGQUIT as i32)
            {
                set_kernel_handler(libc::SIGQUIT as i32, libc::SIG_IGN);
            }
            store_state(libc::SIGQUIT as i32, 1);
            // rubash#1475: replace the runtime's SIGSEGV/SIGBUS handlers
            // with the fatal-pair handler (see install_fatal_handlers).
            install_fatal_handlers();
        });
    }

    /// RAII guard: SIGPIPE blocked on the current thread while alive.
    /// The startup reset (rubash#1333) puts SIGPIPE at SIG_DFL for the
    /// shell process itself — GNU parity for the shell's own writes and
    /// for signal delivery. But an in-process pipeline member SHARES
    /// this process, while GNU forks every member
    /// (execute_cmd.c:2620): a member's write to a closed pipe kills
    /// only the member there. Blocking SIGPIPE for the duration of a
    /// member's execution (and on short-lived payload-writer threads)
    /// makes those writes fail with EPIPE instead, and the stage
    /// machinery converts the failure into the member's 141 status —
    /// the model the pre-#1333 process-wide SIG_IGN provided
    /// implicitly (niubash#345 producers must not take the shell down).
    pub struct SigpipeBlockGuard {
        previous: libc::sigset_t,
    }

    impl SigpipeBlockGuard {
        fn new() -> Self {
            let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
            let mut previous: libc::sigset_t = unsafe { std::mem::zeroed() };
            unsafe {
                libc::sigemptyset(&mut set);
                libc::sigaddset(&mut set, libc::SIGPIPE);
                libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut previous);
            }
            Self { previous }
        }
    }

    impl Drop for SigpipeBlockGuard {
        fn drop(&mut self) {
            unsafe {
                // A write to a broken pipe with SIGPIPE blocked returns
                // EPIPE but the kernel still leaves the signal PENDING
                // for this thread; it would deliver the instant the mask
                // clears and kill the shell for what was only a member's
                // broken pipe. When the restored mask leaves SIGPIPE
                // unblocked (outermost guard), consume a pending
                // SIGPIPE first. Nested guards restore a still-blocked
                // mask and skip the drain — the outer guard owns it.
                if libc::sigismember(&self.previous, libc::SIGPIPE) == 0 {
                    let mut set: libc::sigset_t = std::mem::zeroed();
                    libc::sigemptyset(&mut set);
                    libc::sigaddset(&mut set, libc::SIGPIPE);
                    dequeue_pending_sigpipe(&set);
                }
                libc::pthread_sigmask(libc::SIG_SETMASK, &self.previous, std::ptr::null_mut());
            }
        }
    }

    /// Dequeue a pending SIGPIPE for this thread without ever blocking
    /// (SIGPIPE is blocked on the thread at this point, so a pending one
    /// just sits in the thread's pending set). Linux/Android spell this
    /// `sigtimedwait` with a zero timeout; that call does not exist in
    /// Darwin's libc (E0425 there), where the portable equivalent is
    /// `sigpending` to test + `sigwait` to consume — sigwait returns
    /// immediately because the signal is already pending, and no other
    /// thread in this process ever waits on SIGPIPE.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    unsafe fn dequeue_pending_sigpipe(set: &libc::sigset_t) {
        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let _ = libc::sigtimedwait(set, std::ptr::null_mut(), &timeout);
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    unsafe fn dequeue_pending_sigpipe(set: &libc::sigset_t) {
        let mut pending: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut pending);
        if libc::sigpending(&mut pending) == 0 && libc::sigismember(&pending, libc::SIGPIPE) == 1 {
            let mut received: libc::c_int = 0;
            let _ = libc::sigwait(set, &mut received);
        }
    }

    pub(super) fn sigpipe_block_guard() -> SigpipeBlockGuard {
        SigpipeBlockGuard::new()
    }

    /// Block SIGPIPE on the calling thread with no restore (for
    /// short-lived writer threads that exit after one payload).
    pub(super) fn block_sigpipe_on_this_thread() {
        let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
        unsafe {
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGPIPE);
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        }
    }

    /// Unblock SIGPIPE on the calling thread (child pre-exec: the
    /// pipeline guard's block must not cross an exec).
    fn block_sigpipe_on_this_thread_unblock() {
        let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
        unsafe {
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGPIPE);
            libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
        }
    }

    pub(super) fn install() -> io::Result<()> {
        capture_entry_and_policy();
        if KERNEL_SIGNALS.get().is_some() {
            return Ok(());
        }
        // Empty initial set: no kernel handler exists until a trap names a
        // signal (GNU leaves non-interactive terminating signals SIG_DFL,
        // sig.c:315-316).
        let signals = SignalsInfo::new([] as [i32; 0])?;
        let _ = KERNEL_SIGNALS.set(Mutex::new(signals));
        // Entry-ignored signals stay ignored (captured above); record their
        // state so the first reconcile does not "restore" them to SIG_DFL.
        for &sig in managed_signals() {
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
        signals
            .pending()
            .map(|origin| {
                // Record the delivery's origin for the virtual-child
                // model (see KERNEL_CAUSED) before the queues flatten
                // it to a bare signal number. The WithOrigin slot keeps
                // the latest origin per signal, which is exactly the
                // granularity of this record.
                if is_managed(origin.signal) {
                    if matches!(origin.cause, Cause::Kernel) {
                        KERNEL_CAUSED.fetch_or(bit(origin.signal), Ordering::Relaxed);
                        if origin.signal == libc::SIGINT {
                            SIGINT_KERNEL_EPOCH.fetch_add(1, Ordering::Relaxed);
                        }
                    } else {
                        KERNEL_CAUSED.fetch_and(!bit(origin.signal), Ordering::Relaxed);
                    }
                }
                origin.signal
            })
            .collect()
    }

    /// True when at least one managed signal is routed to the signal_hook
    /// handler, i.e. the kernel backend can have a pending delivery. A
    /// lock-free atomic read: with no handler installed no kernel delivery
    /// is possible, so the boundary poll (rubash#437) can skip the backend
    /// mutex and `pending()` collect entirely on the untrapped path.
    pub(super) fn handler_any_installed() -> bool {
        HANDLER_SET.load(Ordering::Relaxed) != 0
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
        // rubash#1333: the pipeline guard blocks SIGPIPE on the
        // EXECUTING thread; a signal mask survives fork+exec, so a
        // child spawned mid-pipeline would inherit the block and its
        // writes would fail EPIPE (a coreutils producer then prints
        // "standard output: Broken pipe" where GNU's child dies
        // silently by the signal). Children always start unblocked;
        // the disposition loop below gives SIGPIPE its entry default.
        block_sigpipe_on_this_thread_unblock();
        for &sig in managed_signals() {
            let action = if ENTRY_IGNORED.load(Ordering::Relaxed) & bit(sig) != 0 {
                // Ignored at shell entry: POSIX keeps it ignored for children.
                libc::SIG_IGN
            } else if sig == libc::SIGQUIT as i32 {
                // The sig.c:333 policy ignore is shell-only; the entry
                // disposition (SIG_DFL here, entry-ignored handled above)
                // is what children exec with.
                libc::SIG_DFL
            } else if trap_ignored(sig) {
                // trap '' ignore (trap.c:1451-1455): survives into
                // children. Read from the trap-table record, NOT the
                // live state word: the word also holds shell-policy
                // ignores (interactive SIGTERM, sig.c:339) that GNU
                // never folds into original_signals, so they must not
                // cross into children (rubash#1991).
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

/// The signals `kernel_signals` manages, paired with their trap-table
/// short name and the literal `__RUBASH_TRAP_SIG*` env key. Built once
/// (the realtime block's numbers and names are runtime values, so the
/// table cannot be a const) and reused by every per-command-boundary
/// reconcile, which never allocates a key string after init
/// (rubash#437: the boundary poll runs once or more per loop iteration).
/// Short names match both signal_trap_name's SIG{name} form and the
/// SIGNALS table's spellings. rubash#1511 extends the original seven to
/// the full managed set (see MANAGED_CORE).
#[cfg(unix)]
fn managed_trap_keys() -> &'static [(i32, String, String)] {
    static KEYS: std::sync::OnceLock<Vec<(i32, String, String)>> = std::sync::OnceLock::new();
    KEYS.get_or_init(|| {
        let mut keys: Vec<(i32, String, String)> = Vec::new();
        let mut push = |keys: &mut Vec<(i32, String, String)>, sig: i32, short: &str| {
            keys.push((sig, short.to_string(), format!("__RUBASH_TRAP_SIG{short}")));
        };
        push(&mut keys, libc::SIGHUP as i32, "HUP");
        push(&mut keys, libc::SIGINT as i32, "INT");
        push(&mut keys, libc::SIGQUIT as i32, "QUIT");
        push(&mut keys, libc::SIGTERM as i32, "TERM");
        push(&mut keys, libc::SIGUSR1 as i32, "USR1");
        push(&mut keys, libc::SIGUSR2 as i32, "USR2");
        push(&mut keys, libc::SIGPIPE as i32, "PIPE");
        push(&mut keys, libc::SIGTRAP as i32, "TRAP");
        push(&mut keys, libc::SIGABRT as i32, "ABRT");
        push(&mut keys, libc::SIGALRM as i32, "ALRM");
        push(&mut keys, libc::SIGXCPU as i32, "XCPU");
        push(&mut keys, libc::SIGXFSZ as i32, "XFSZ");
        push(&mut keys, libc::SIGVTALRM as i32, "VTALRM");
        push(&mut keys, libc::SIGPROF as i32, "PROF");
        push(&mut keys, libc::SIGIO as i32, "IO");
        push(&mut keys, libc::SIGSYS as i32, "SYS");
        push(&mut keys, libc::SIGURG as i32, "URG");
        push(&mut keys, libc::SIGWINCH as i32, "WINCH");
        extend_trap_keys(&mut keys);
        keys
    })
}

/// Linux/Android-only trap-key additions: SIGSTKFLT and SIGPWR plus
/// the realtime block, spelled with the nearer-anchor form the SIGNALS
/// table (and GNU signames.c) uses — RTMIN+k up to the midpoint,
/// RTMAX-m above it.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn extend_trap_keys(keys: &mut Vec<(i32, String, String)>) {
    keys.push((
        libc::SIGSTKFLT as i32,
        "STKFLT".to_string(),
        "__RUBASH_TRAP_SIGSTKFLT".to_string(),
    ));
    keys.push((
        libc::SIGPWR as i32,
        "PWR".to_string(),
        "__RUBASH_TRAP_SIGPWR".to_string(),
    ));
    let lo = libc::SIGRTMIN();
    let hi = libc::SIGRTMAX();
    if lo > 0 && hi >= lo {
        for n in lo..=hi {
            let k = n - lo;
            let m = hi - n;
            let short = if k == 0 {
                "RTMIN".to_string()
            } else if m == 0 {
                "RTMAX".to_string()
            } else if k <= m {
                format!("RTMIN+{k}")
            } else {
                format!("RTMAX-{m}")
            };
            keys.push((n, short.clone(), format!("__RUBASH_TRAP_SIG{short}")));
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn extend_trap_keys(_keys: &mut Vec<(i32, String, String)>) {}

/// Map the executor's trap table onto the kernel dispositions (unix arm of
/// rubash#226). Mirrors GNU's per-signal rules: `trap 'action' SIG` installs
/// the handler (sig.c:830, sa_flags=0), `trap '' SIG` sets SIG_IGN, no trap
/// restores the entry disposition (SIG_DFL for a plain script). Called from
/// the `trap` builtin's mutation paths and the command-boundary poll so the
/// kernel state can never lag the trap table by more than one command.
#[cfg(unix)]
pub fn reconcile_kernel_trap_dispositions(env_vars: &std::collections::HashMap<String, String>) {
    use kernel_signals::Disposition;
    // Steady-state fast path (rubash#437). The overwhelming majority of
    // boundary polls run with no managed signal trapped at all, so every
    // disposition is Default and the kernel already holds Default. Detect
    // that with six allocation-free `get`s — using the static key table so
    // no `format!` runs — and skip the disposition array + mutex entirely.
    // A non-empty reset list forces the slow path even when no key is
    // present, because a reset entry can only exist while a key does; the
    // reverse (key present, no reset) is the common trapped case and falls
    // through anyway.
    let no_reset = !env_vars.contains_key("__RUBASH_TRAP_RESET");
    // Any armed signal trap forces the slow path. Every trap-table key
    // for a signal shares the __RUBASH_TRAP_SIG prefix — the managed
    // set's and the fatal pair's alike (rubash#1475: the pair's routing
    // bits live outside the managed state, but their keys must still
    // defeat this fast path). One prefix pass over the var keys, no
    // allocation, no per-signal probing (the managed table outgrew a
    // fixed probe list with rubash#1511).
    let any_armed = env_vars
        .keys()
        .any(|key| key.starts_with("__RUBASH_TRAP_SIG"));
    // An armed EXIT trap also forces the slow path: it routes every
    // otherwise-untrapped managed signal through the handler
    // (sig.c:226-229, rubash#1332 — see the disposition loop below).
    let exit_armed = env_vars
        .get("__RUBASH_TRAP_EXIT")
        .is_some_and(|action| !action.is_empty());
    // rubash#1662: an interactive shell's untrapped SIGINT is pinned to
    // the mailbox handler (see the disposition loop below), so the
    // all-Default steady state is never the interactive steady state --
    // the fast path must not skip the pin's first application.
    let interactive = env_vars.get("__RUBASH_INTERACTIVE").map(String::as_str) == Some("1");
    if !any_armed
        && !exit_armed
        && !interactive
        && no_reset
        && kernel_signals::at_default_state()
        && kernel_signals::fatal_at_default()
    {
        // No trap keys exist on this path, so the trap '' record is
        // empty by construction (rubash#1991).
        kernel_signals::clear_trap_ignored_all();
        return;
    }
    // G-01: the slow path mutates process-wide kernel dispositions —
    // serialize it against sibling sessions' reconciles and env
    // mirror writes. The fast path above reads only atomics and
    // stays lock-free.
    let _process_state = crate::executor::local_helpers::acquire_process_state();
    let reset = env_vars
        .get("__RUBASH_TRAP_RESET")
        .map(|value| value.as_str())
        .unwrap_or("");
    // GNU sig.c:226-229: an EXIT trap forces
    // initialize_terminating_signals — `if (signal_is_trapped (EXIT))
    // set_signal_handler for every terminating signal` — even in a
    // non-interactive shell. Without it an untrapped SIGTERM kills the
    // process at the kernel and the EXIT trap never runs; with it the
    // delivery queues here and the boundary dispatch turns it into the
    // 128+signal exit, whose top-level path runs the EXIT trap first
    // (rubash#1332).
    let exit_armed = env_vars
        .get("__RUBASH_TRAP_EXIT")
        .is_some_and(|action| !action.is_empty())
        && !reset_contains(reset, "EXIT");
    let mut dispositions: Vec<(i32, Disposition)> = Vec::new();
    for (sig, short, key) in managed_trap_keys() {
        // rubash#1991: keep the trap '' record in step with the table
        // this reconcile is applying — it is the ONLY ignore source the
        // fork-child restore may consult besides the entry snapshot
        // (see kernel_signals::TRAP_IGNORED).
        kernel_signals::set_trap_ignored(
            *sig,
            matches!(env_vars.get(key.as_str()), Some(action) if action.is_empty())
                && !reset_contains(reset, short),
        );
        let want = match env_vars.get(key.as_str()) {
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
            // No trap of its own: an armed EXIT trap still routes the
            // signal through the handler (sig.c:226-229 above) — but
            // only for GNU's terminating set (see exit_reroutes).
            None if exit_armed && exit_reroutes(*sig) => Disposition::Handler,
            // rubash#1662: GNU sig.c:315-316 -- initialize_terminating_
            // signals is interactive-only: an interactive shell catches
            // SIGINT itself from startup, independent of the trap table.
            // Without the pin, an untrapped interactive SIGINT sits at
            // SIG_DFL in every window the foreground guards do not cover
            // (the prompt reader, in-process builtin execution), and the
            // kernel default action kills the shell on the first ^C.
            // The mailbox delivery becomes the boundary dispatch's
            // 128+SIGINT abort instead. Entry-ignored SIGINT stays
            // ignored (the pin must not un-ignore it).
            None if interactive
                && *sig == libc::SIGINT as i32
                && !kernel_signals::entry_ignored(*sig) =>
            {
                Disposition::Handler
            }
            // The same sig.c block: an interactive shell also ignores
            // SIGTERM (initialize_shell_signals: `if (interactive) ...
            // set_signal_handler (SIGTERM, SIG_IGN)`), so a directed
            // TERM cannot kill it either. An armed EXIT trap still wins
            // via the reroute arm above (Handler), and a user trap on
            // TERM is unaffected (the Some arms).
            None if interactive
                && *sig == libc::SIGTERM as i32
                && !kernel_signals::entry_ignored(*sig) =>
            {
                Disposition::Ignore
            }
            None => Disposition::Default,
        };
        dispositions.push((*sig, want));
    }
    kernel_signals::reconcile(&dispositions);
    // rubash#1475: mirror the fatal set's trap state into the fatal
    // handler's routing bits, with the same mapping as the managed set
    // (a reset-listed fatal trap behaves like no trap; an armed EXIT
    // trap does NOT reroute SIGSEGV/SIGBUS — GNU's sig.c:226-229
    // applies to the terminating-signal handler set, and a fatal
    // signal's default death already runs the EXIT trap through the
    // shell's exit path. SIGILL/SIGFPE below are the exception: GNU's
    // terminating set names them, so they use the rerouting mapping).
    let fatal_want = |short: &str, key: &str| match env_vars.get(key) {
        Some(action) if !action.is_empty() && !reset_contains(reset, short) => Disposition::Handler,
        Some(_) if reset_contains(reset, short) => Disposition::Default,
        Some(_) => Disposition::Ignore,
        None => Disposition::Default,
    };
    // rubash#1511: SIGILL/SIGFPE ride the fatal backend (signal-hook
    // refuses them), but unlike the pair they DO reroute under an
    // armed EXIT trap — GNU's terminating set names them (oracle:
    // EXIT trap + untrapped kill -ILL runs the EXIT trap, dies 132).
    let fatal_want_rerouted = |short: &str, key: &str| match env_vars.get(key) {
        Some(action) if !action.is_empty() && !reset_contains(reset, short) => Disposition::Handler,
        Some(_) if reset_contains(reset, short) => Disposition::Default,
        Some(_) => Disposition::Ignore,
        None if exit_armed => Disposition::Handler,
        None => Disposition::Default,
    };
    kernel_signals::reconcile_fatal(&[
        (
            libc::SIGSEGV as i32,
            fatal_want("SEGV", "__RUBASH_TRAP_SIGSEGV"),
        ),
        (
            libc::SIGBUS as i32,
            fatal_want("BUS", "__RUBASH_TRAP_SIGBUS"),
        ),
        (
            libc::SIGILL as i32,
            fatal_want_rerouted("ILL", "__RUBASH_TRAP_SIGILL"),
        ),
        (
            libc::SIGFPE as i32,
            fatal_want_rerouted("FPE", "__RUBASH_TRAP_SIGFPE"),
        ),
    ]);
}

/// Short signal name (TERM) for the __RUBASH_TRAP_SIG* key family, matching
/// signal_trap_name's SIG{name} form minus the SIG prefix.
#[cfg(unix)]
fn signal_short_name(signal: i32) -> &'static str {
    managed_trap_keys()
        .iter()
        .find(|(sig, _, _)| *sig == signal)
        .map(|(_, short, _)| short.as_str())
        .unwrap_or("")
}

/// Whether an armed EXIT trap reroutes `sig` through the handler
/// (sig.c:226-229 initialize_terminating_signals). GNU's terminating
/// set is a compile-time C table, so it cannot name the realtime
/// signals, and it excludes the default-ignore pair — oracle-verified
/// against GNU 5.3.20 (EXIT trap armed, untrapped self-kill): ILL, FPE,
/// TRAP, ABRT, ALRM, XCPU, PROF and SYS run the EXIT trap and die at
/// 128+n, while URG and WINCH are ignored outright and RTMIN dies
/// without the EXIT trap running. rubash#1511.
#[cfg(unix)]
fn exit_reroutes(sig: i32) -> bool {
    if sig == libc::SIGURG as i32 || sig == libc::SIGWINCH as i32 {
        return false;
    }
    exit_reroutes_platform(sig)
}

/// The realtime half of exit_reroutes: on Linux/Android the RT block
/// sits at SIGRTMIN() and above and never reroutes; elsewhere there
/// is no RT block to exclude.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn exit_reroutes_platform(sig: i32) -> bool {
    sig < libc::SIGRTMIN()
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn exit_reroutes_platform(_sig: i32) -> bool {
    true
}

#[cfg(unix)]
fn reset_contains(reset: &str, short: &str) -> bool {
    // Reset entries carry the table's SIG-prefixed name (trap.rs
    // normalize_signal keeps the SIG prefix, e.g. "SIGTERM"). Compare
    // without allocating: iterate the colon-separated entries and match
    // "SIG" + short by parts.
    reset.split(':').any(|entry| {
        entry.len() == short.len() + 3 && entry.starts_with("SIG") && entry[3..] == *short
    })
}

/// Whether `sig` is currently caught by the kernel backend (unix arm). A
/// pending entry whose signal is not caught is stale (see
/// kernel_signals::handler_installed) and must be dropped, not dispatched.
#[cfg(unix)]
pub fn kernel_handler_installed(signal: i32) -> bool {
    kernel_signals::handler_installed(signal)
}

/// Whether the most recently drained kernel delivery of `signal` was
/// kernel-caused (terminal-generated, si_code SI_KERNEL) rather than
/// sent by a kill(2) from some process (rubash#1034; see
/// kernel_signals::KERNEL_CAUSED).
#[cfg(unix)]
pub fn last_delivery_kernel_caused(signal: i32) -> bool {
    kernel_signals::last_delivery_kernel_caused(signal)
}

/// Monotonic count of kernel-caused SIGINT deliveries drained so far
/// (rubash#1990) — see kernel_signals::SIGINT_KERNEL_EPOCH.
#[cfg(unix)]
pub fn sigint_kernel_epoch() -> u64 {
    kernel_signals::sigint_kernel_epoch()
}

/// Whether the boundary dispatcher can adjudicate `signal` (managed or
/// fatal set) — see kernel_signals::adjudicable (rubash#1654).
#[cfg(unix)]
pub fn boundary_adjudicable_signal(signal: i32) -> bool {
    kernel_signals::adjudicable(signal)
}

/// rubash#1250: hold untrapped SIGINT at the mailbox handler across a
/// foreground builtin wait (the `sleep` fast path). While held, the
/// reconcile above will not route SIGINT back to Default.
#[cfg(unix)]
pub fn hold_fg_sigint() {
    kernel_signals::hold_fg_sigint()
}

/// Block SIGPIPE on the current thread until the returned guard drops
/// (see kernel_signals::SigpipeBlockGuard for the model).
#[cfg(unix)]
pub fn sigpipe_block_guard() -> kernel_signals::SigpipeBlockGuard {
    kernel_signals::sigpipe_block_guard()
}

/// Block SIGPIPE on the calling thread permanently (writer threads).
#[cfg(unix)]
pub fn block_sigpipe_on_this_thread() {
    kernel_signals::block_sigpipe_on_this_thread()
}

/// rubash#1250: release the foreground SIGINT hold (restores the
/// steady state the hold found — Default, or the rubash#1662
/// interactive pin's Handler routing; rubash#1824).
#[cfg(unix)]
pub fn release_fg_sigint() {
    kernel_signals::release_fg_sigint()
}

/// rubash#866: the reserved `__RUBASH_*` namespace key that used to carry
/// the forgeable constant "1" into the wrapper's environment. It now carries
/// an unguessable spawn-time nonce (see `new_background_wrapper_nonce`) and
/// is only ever honored in combination with the matching argv flag
/// (`BACKGROUND_WRAPPER_FLAG`) at handshake time; a user
/// `export __RUBASH_BACKGROUND_WRAPPER=<anything>` arms nothing and is
/// scrubbed from the process environment at startup.
pub const BACKGROUND_WRAPPER_ENV_KEY: &str = "__RUBASH_BACKGROUND_WRAPPER";

/// rubash#866: the internal argv flag carrying the handshake nonce. Like the
/// other `--internal-*` faces it is consumed by the host CLI's startup
/// consume call before any option parser ever sees it; the wrapper role arms
/// only when the flag's nonce and the environment nonce agree (both
/// constructed by the wrapper spawn site in compound_exec), so guessing or
/// exporting the environment name alone is not enough.
///
/// The flag is niu-CLI-specific: a host that never calls
/// `consume_background_wrapper_handshake` (an embedder's own launcher, or a
/// foreign bash on PATH) would reject it as an invalid option and kill the
/// background command. The wrapper spawn site therefore attaches it only
/// when `background_wrapper_flag_consumer_present()` is set — rubash#873
/// CI: the niubash launcher respawned its host CLI with the flag and
/// the launcher's GNU-aligned parser replied
/// `bash: --internal-bg-wrapper=<nonce>: invalid option` + usage, killing
/// every wrapper respawn.
pub const BACKGROUND_WRAPPER_FLAG: &str = "--internal-bg-wrapper=";

/// rubash#873: marks that THIS process's command-line face understands the
/// background-wrapper handshake — set unconditionally at the top of
/// `consume_background_wrapper_handshake`, which rubash's own main() calls
/// at every startup through the Tier B face
/// `script_driver::consume_background_wrapper_handshake` (embedders adopt
/// the same one call through that face). The wrapper spawn
/// site consults it before splicing `BACKGROUND_WRAPPER_FLAG` into a
/// respawned child's argv: a host that never consumes must never RECEIVE
/// the flag, or the child's option parser rejects it and the background
/// command dies (the niubash launcher CI failure). The env nonce channel
/// stays unconditionally: for a non-consuming child it is an inert reserved
/// key (a consumer scrub it at startup; a non-consumer never reads it), and
/// a single channel can never arm — that is the rubash#866 contract.
static BACKGROUND_WRAPPER_FLAG_CONSUMER: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// rubash#873: whether this process's CLI consumes the background-wrapper
/// handshake (see `BACKGROUND_WRAPPER_FLAG_CONSUMER`). Gates the argv side
/// of the handshake at the wrapper spawn site.
pub fn background_wrapper_flag_consumer_present() -> bool {
    BACKGROUND_WRAPPER_FLAG_CONSUMER.load(std::sync::atomic::Ordering::Acquire)
}

/// rubash#866: in-process wrapper role. Decided ONCE per process at startup
/// by `consume_background_wrapper_handshake` (nonce handshake) and consulted
/// by `child_pre_exec_reset` between fork and exec — never by reading the
/// ambient environment, which a user script can forge. Atomic because the
/// reader runs in the post-fork child (async-signal-safe: no locks, no
/// allocation — the old `std::env::var` probe allocated).
#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
static BACKGROUND_WRAPPER_ROLE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// rubash#867: the parent pid an armed child must still see from getppid().
/// Captured at handshake time (this process's own pid: the wrapper is the
/// parent of every child it spawns) instead of comparing against the
/// constant 1 — inside a PID namespace a healthy child of the namespace's
/// PID 1 legitimately observes getppid() == 1 before any fork race, and the
/// old constant comparison made it SIGKILL itself at spawn.
#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
static BACKGROUND_WRAPPER_EXPECTED_PARENT: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

/// rubash#866: per-process counter folded into the nonce so two wrappers
/// spawned within the same clock tick never share a handshake value.
static WRAPPER_NONCE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// rubash#866: 128 bits of spawn-time handshake material as 32 hex chars.
/// Entropy: wall-clock nanoseconds, pid, a process-local counter and an
/// ASLR-derived stack address, run through SplitMix64. No dependency on a
/// RNG crate; a user script cannot observe or predict the value another
/// process drew at its spawn instant, which is the whole point — the
/// forgeable constant "1" let `export __RUBASH_BACKGROUND_WRAPPER=1` impose
/// PDEATHSIG semantics on children that never went through the wrapper
/// spawn path.
pub fn new_background_wrapper_nonce() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let counter = WRAPPER_NONCE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let anchor = &counter as *const u64 as u64;
    let mut z = nanos
        ^ (std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ counter.wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ anchor.rotate_left(17);
    // SplitMix64 finalizer (Steele et al.), two draws for 128 bits.
    let mut draw = |z: &mut u64| {
        *z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut v = *z;
        v = (v ^ (v >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        v = (v ^ (v >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        v ^ (v >> 31)
    };
    let hi = draw(&mut z);
    let lo = draw(&mut z);
    format!("{hi:016x}{lo:016x}")
}

/// rubash#866: consume the background-wrapper spawn handshake — called from
/// main() before any parser or executor touches argv (same consume-on-read
/// face as NIU_INVOKED_AS and `--internal-yes|head|wc`). The wrapper role
/// arms only when the argv flag nonce and the environment nonce AGREE (both
/// were constructed by our own spawn site); afterwards the environment key
/// is scrubbed unconditionally so a user-forged value can neither arm
/// anything nor leak into grandchildren. On mismatch (hand forgery, stale
/// spawn, tampering) the process simply runs without the wrapper role.
///
/// The call itself (regardless of whether a handshake is present) marks the
/// process as a flag consumer — see `BACKGROUND_WRAPPER_FLAG_CONSUMER`.
/// Hosts that adopt this call opt into receiving the argv channel on their
/// own wrapper respawns; hosts that never call it get env-only respawns and
/// the flag never reaches their CLI.
pub(crate) fn consume_background_wrapper_handshake(args: &mut Vec<String>) {
    BACKGROUND_WRAPPER_FLAG_CONSUMER.store(true, std::sync::atomic::Ordering::Release);
    let nonce = match args
        .get(1)
        .and_then(|arg| arg.strip_prefix(BACKGROUND_WRAPPER_FLAG))
    {
        Some(nonce) => nonce.to_owned(),
        None => {
            // No flag: a plain (user-invoked) rubash. Still scrub a forged
            // ambient value — nothing may read it anymore.
            std::env::remove_var(BACKGROUND_WRAPPER_ENV_KEY);
            return;
        }
    };
    args.remove(1);
    if std::env::var(BACKGROUND_WRAPPER_ENV_KEY).as_deref() != Ok(nonce.as_str()) {
        std::env::remove_var(BACKGROUND_WRAPPER_ENV_KEY);
        return;
    }
    std::env::remove_var(BACKGROUND_WRAPPER_ENV_KEY);
    #[cfg(unix)]
    {
        BACKGROUND_WRAPPER_ROLE.store(true, std::sync::atomic::Ordering::Release);
        BACKGROUND_WRAPPER_EXPECTED_PARENT
            .store(std::process::id(), std::sync::atomic::Ordering::Release);
    }
}

/// Test/inspection accessor for the in-process wrapper role.
#[cfg(unix)]
pub fn background_wrapper_role_armed() -> bool {
    BACKGROUND_WRAPPER_ROLE.load(std::sync::atomic::Ordering::Acquire)
}

/// rubash#1035: environment key marking a `rubash -c` respawn as an
/// asynchronous (`&`) job child while job_control is off (the `monitor`
/// option is unset). GNU execute_cmd.c setup_async_signals forces
/// SIGINT/SIGQUIT to SIG_IGN in every such forked child; a respawned
/// process cannot inherit that disposition through spawn (the parent holds
/// no ignore itself), so the background spawn site stamps this key into
/// the child's environment block and startup consumes it (see
/// [`apply_async_child_entry_ignores`]) before the trap table seeds and
/// the kernel backend captures entry dispositions.
///
/// Forgery note: a user `export` of this key only buys the spawner kernel
/// ignore for that process subtree — the same disposition `trap '' INT`
/// achieves deliberately — and never arms traps or PDEATHSIG (those read
/// the trap table / the nonce handshake, rubash#866). The spawn site
/// overwrites a forged ambient value on every background respawn and
/// removes it otherwise, so `set -m` (job control on) children never see
/// it.
#[cfg(unix)]
pub const ASYNC_CHILD_ENV_KEY: &str = "__RUBASH_ASYNC_CHILD";

/// Consume [`ASYNC_CHILD_ENV_KEY`] at process startup (rubash#1035): a
/// background-job respawn starts life with SIGINT/SIGQUIT ignored, exactly
/// as if forked by GNU's setup_async_signals path. Must run before the
/// executor seeds the trap table and before the kernel backend captures
/// entry dispositions, so the ignore surfaces both as `trap -- '' SIGINT`
/// (via the ORIG_IGN transport the spawn site adds for exec-like bodies)
/// and as the kernel entry disposition (ENTRY_IGNORED) for the rest.
/// Scrubs the key unconditionally afterwards so it never reaches
/// `export -p` or grandchildren environments (kernel inheritance carries
/// the disposition there, as fork does in GNU).
///
/// The async-child ignore is a setup_async_signals disposition, NOT a
/// hard ignore: GNU's forked async child lists `trap -- '' SIGINT` yet
/// still lets a later `trap` override it (trap.c's ASYNCSIG special-case
/// in set_signal — the golden trap-signal-matrix fires its INT handler).
/// The entry-disposition fold in seed_startup_traps must therefore tell
/// this ignore apart from a foreign parent's inherited ignore, which IS
/// hard (trap.c original_signals). This flag is the marker: it is set
/// here, before the trap table seeds, and read only by that fold.
#[cfg(unix)]
static ASYNC_CHILD_ENTRY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True when this process started as an asynchronous (`&`) job child
/// (entry SIGINT/SIGQUIT ignored by the spawn site, not by inheritance).
#[cfg(unix)]
pub(crate) fn async_child_entry() -> bool {
    ASYNC_CHILD_ENTRY.load(std::sync::atomic::Ordering::Acquire)
}

#[cfg(unix)]
pub fn apply_async_child_entry_ignores() {
    if std::env::var(ASYNC_CHILD_ENV_KEY).as_deref() != Ok("1") {
        std::env::remove_var(ASYNC_CHILD_ENV_KEY);
        return;
    }
    std::env::remove_var(ASYNC_CHILD_ENV_KEY);
    ASYNC_CHILD_ENTRY.store(true, std::sync::atomic::Ordering::Release);
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGQUIT, libc::SIG_IGN);
    }
}

/// PR_SET_PDEATHSIG + race check against the EXPECTED parent pid. Runs
/// between fork and exec: syscalls and atomic loads only, no allocation.
#[cfg(target_os = "linux")]
fn arm_pdeathsig(expected_parent: u32) -> std::io::Result<()> {
    // PR_SET_PDEATHSIG: send SIGKILL to this process when its parent dies.
    const PR_SET_PDEATHSIG: i32 = 1;
    let ret = unsafe { libc::prctl(PR_SET_PDEATHSIG, libc::SIGKILL as u64, 0, 0, 0) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // rubash#867: race — the parent may have died between fork and prctl,
    // in which case we were reparented (to init, a subreaper, or the
    // namespace's PID 1). Self-kill only when the observed parent DIVERGES
    // from the expected one; never against the constant 1, which a healthy
    // child of a namespace-PID-1 parent matches before any race is possible.
    if unsafe { libc::getppid() } != expected_parent as libc::pid_t {
        unsafe { libc::kill(libc::getpid(), libc::SIGKILL) };
    }
    Ok(())
}

/// Non-Linux unix: no PR_SET_PDEATHSIG; parent tracking covers the wrapper
/// contract there (rubash#737).
#[cfg(all(unix, not(target_os = "linux")))]
fn arm_pdeathsig(_expected_parent: u32) -> std::io::Result<()> {
    Ok(())
}

/// pre_exec hook for spawned child processes (unix arm of rubash#229):
/// restore GNU's child signal dispositions between fork and exec. A no-op
/// when the backend was never installed (the process then still holds its
/// entry dispositions everywhere).
///
/// rubash#737/#866: a process that consumed the background-wrapper handshake
/// additionally arms PDEATHSIG for the children IT spawns, decided from the
/// in-process role — the ambient environment is never consulted here (it is
/// user-forgeable), and the race check compares against the wrapper's own
/// pid instead of the constant 1 (rubash#867).
///
/// Behavior note (why this is an exact predicate swap): pre_exec runs in the
/// forked child BEFORE execve, so the old `env::var` probe here read the
/// FORKING PARENT's environ — the check was always a parent-state predicate,
/// never a property of the child's own environment block. Swapping "parent
/// env contains the marker" for "parent holds the in-process role" keeps
/// every spawn's arming decision byte-identical in clean environments (the
/// main shell never arms — so a wrapper itself survives its shell's exit, as
/// GNU's background jobs do — and a wrapper arms every child it spawns),
/// while a forged `export` arms nothing.
#[cfg(unix)]
pub fn child_pre_exec_reset() -> std::io::Result<()> {
    kernel_signals::pre_exec_child_dispositions()?;
    // rubash#737: if this process is a background-job wrapper, arrange for
    // this child to die when the wrapper dies. Without this, `kill $!` kills
    // the wrapper but leaves the actual work (grandchild) orphaned and
    // running.
    #[cfg(target_os = "linux")]
    if BACKGROUND_WRAPPER_ROLE.load(std::sync::atomic::Ordering::Acquire) {
        let expected =
            BACKGROUND_WRAPPER_EXPECTED_PARENT.load(std::sync::atomic::Ordering::Acquire);
        arm_pdeathsig(expected)?;
    }
    Ok(())
}

/// pre_exec hook for asynchronous (`&`) child spawns (unix arm of
/// rubash#1035): GNU execute_cmd.c setup_async_signals runs in every
/// forked async child while job_control == 0 and forces SIGINT/SIGQUIT to
/// SIG_IGN before exec — a background `sleep` survives `kill -INT $!`,
/// and an exec'd script inherits the ignore as SIG_HARD_IGNORE (trap.c
/// set_signal refuses to trap it, `trap -p` shows `trap -- '' SIGINT`).
/// Runs after the plain entry-disposition restore on purpose: a parent
/// `trap '...' INT` must not leak a catchable disposition into the async
/// child (GNU ignores unconditionally). The background spawn site selects
/// this hook instead of [`child_pre_exec_reset`]; the hook itself takes no
/// decision input beyond the call — pre_exec runs post-fork and must not
/// consult user-forgeable ambient state (rubash#866).
#[cfg(unix)]
pub fn child_pre_exec_reset_async() -> std::io::Result<()> {
    child_pre_exec_reset()?;
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGQUIT, libc::SIG_IGN);
    }
    Ok(())
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

/// rubash#866/#867: the wrapper handshake is unforgeable — the role arms
/// only when the argv nonce and the env nonce agree (both constructed by the
/// wrapper spawn site), both channels are consumed afterwards, and a forged
/// ambient value arms nothing and is scrubbed. These tests touch
/// process-global state (the role statics + the env key); a Drop guard
/// restores them so sibling tests in this binary are unaffected even on
/// assertion failure.
#[cfg(all(test, unix))]
mod background_wrapper_handshake_tests {
    use super::{
        background_wrapper_flag_consumer_present, background_wrapper_role_armed,
        consume_background_wrapper_handshake, new_background_wrapper_nonce,
        BACKGROUND_WRAPPER_ENV_KEY, BACKGROUND_WRAPPER_EXPECTED_PARENT, BACKGROUND_WRAPPER_FLAG,
        BACKGROUND_WRAPPER_ROLE,
    };
    use std::sync::atomic::Ordering;

    /// Serializes the env-key mutation across these tests (plain `cargo
    /// test` runs them as threads of one process; nextest isolates them).
    static HANDSHAKE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct RoleGuard;
    impl RoleGuard {
        fn new() -> Self {
            Self
        }
    }
    impl Drop for RoleGuard {
        fn drop(&mut self) {
            BACKGROUND_WRAPPER_ROLE.store(false, Ordering::Release);
            BACKGROUND_WRAPPER_EXPECTED_PARENT.store(0, Ordering::Release);
            std::env::remove_var(BACKGROUND_WRAPPER_ENV_KEY);
        }
    }

    fn args_with_flag(nonce: &str) -> Vec<String> {
        vec![
            "rubash".to_string(),
            format!("{BACKGROUND_WRAPPER_FLAG}{nonce}"),
            "-c".to_string(),
            "true".to_string(),
        ]
    }

    #[test]
    fn forged_env_value_arms_nothing_and_is_scrubbed() {
        let _lock = HANDSHAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = RoleGuard::new();
        // The #866 attack: plain `export __RUBASH_BACKGROUND_WRAPPER=1` in
        // the environment of a rubash that never went through the wrapper
        // spawn path.
        std::env::set_var(BACKGROUND_WRAPPER_ENV_KEY, "1");
        let mut args = vec!["rubash".to_string(), "-c".to_string(), "true".to_string()];
        consume_background_wrapper_handshake(&mut args);
        assert!(!background_wrapper_role_armed());
        // The forged value must not survive to be inherited by children.
        assert!(std::env::var(BACKGROUND_WRAPPER_ENV_KEY).is_err());
        // argv untouched (no flag to strip).
        assert_eq!(
            args,
            vec!["rubash".to_string(), "-c".to_string(), "true".to_string()]
        );
    }

    #[test]
    fn argv_flag_without_matching_env_nonce_arms_nothing() {
        let _lock = HANDSHAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = RoleGuard::new();
        std::env::remove_var(BACKGROUND_WRAPPER_ENV_KEY);
        let nonce = new_background_wrapper_nonce();
        let mut args = args_with_flag(&nonce);
        consume_background_wrapper_handshake(&mut args);
        assert!(!background_wrapper_role_armed());
        // The internal flag is still consumed: the GNU option parser must
        // never see it.
        assert_eq!(
            args,
            vec!["rubash".to_string(), "-c".to_string(), "true".to_string()]
        );
    }

    #[test]
    fn nonce_mismatch_between_channels_arms_nothing() {
        let _lock = HANDSHAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = RoleGuard::new();
        std::env::set_var(BACKGROUND_WRAPPER_ENV_KEY, new_background_wrapper_nonce());
        let forged = new_background_wrapper_nonce();
        let mut args = args_with_flag(&forged);
        consume_background_wrapper_handshake(&mut args);
        assert!(!background_wrapper_role_armed());
        assert!(std::env::var(BACKGROUND_WRAPPER_ENV_KEY).is_err());
    }

    #[test]
    fn agreeing_channels_arm_role_capture_expected_parent_and_consume_both() {
        let _lock = HANDSHAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = RoleGuard::new();
        let nonce = new_background_wrapper_nonce();
        std::env::set_var(BACKGROUND_WRAPPER_ENV_KEY, &nonce);
        let mut args = args_with_flag(&nonce);
        consume_background_wrapper_handshake(&mut args);
        assert!(background_wrapper_role_armed());
        // rubash#867: the armed children compare getppid() against THIS
        // process's pid, never the constant 1.
        assert_eq!(
            BACKGROUND_WRAPPER_EXPECTED_PARENT.load(Ordering::Acquire),
            std::process::id()
        );
        assert!(std::env::var(BACKGROUND_WRAPPER_ENV_KEY).is_err());
        assert_eq!(
            args,
            vec!["rubash".to_string(), "-c".to_string(), "true".to_string()]
        );
    }

    #[test]
    fn nonces_are_unique_and_well_formed() {
        let a = new_background_wrapper_nonce();
        let b = new_background_wrapper_nonce();
        assert_ne!(a, b, "successive spawn-time nonces must differ");
        assert_eq!(a.len(), 32);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    /// rubash#873: any consume call — even the plain-invocation scrub —
    /// marks the process as an argv-flag consumer, which is what licenses
    /// the wrapper spawn site to splice `--internal-bg-wrapper=` into a
    /// respawned child. The inverse (no consume call → no flag) cannot be
    /// asserted in this shared test process (sibling tests above already
    /// flipped the bit; only nextest's process-per-test lane could); it is
    /// pinned end-to-end by the niubash launcher suite, whose host CLI
    /// never consumed and must never RECEIVE the flag (rubash#873 CI).
    #[test]
    fn consume_marks_the_process_as_a_flag_consumer() {
        let _lock = HANDSHAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut args = vec!["rubash".to_string(), "-c".to_string(), "true".to_string()];
        consume_background_wrapper_handshake(&mut args);
        assert!(background_wrapper_flag_consumer_present());
    }
}

/// rubash#867: the armed child's race check compares getppid() against the
/// EXPECTED parent pid, never the constant 1. Driven here through the real
/// role statics and the real pre_exec hook; the spawned processes are this
/// test's own children. Mutates process-global state — the guard restores
/// it, and the linux-only lib lane runs under nextest (process-per-test) in
/// CI and --test-threads=1 locally, so no sibling test observes the window.
#[cfg(all(test, target_os = "linux"))]
mod pdeathsig_expected_parent_tests {
    use super::{
        child_pre_exec_reset, BACKGROUND_WRAPPER_EXPECTED_PARENT, BACKGROUND_WRAPPER_ROLE,
    };
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use std::sync::atomic::Ordering;

    struct RoleGuard;
    impl Drop for RoleGuard {
        fn drop(&mut self) {
            BACKGROUND_WRAPPER_ROLE.store(false, Ordering::Release);
            BACKGROUND_WRAPPER_EXPECTED_PARENT.store(0, Ordering::Release);
        }
    }

    fn spawn_armed(expected_parent: u32) -> std::process::ExitStatus {
        BACKGROUND_WRAPPER_ROLE.store(true, Ordering::Release);
        BACKGROUND_WRAPPER_EXPECTED_PARENT.store(expected_parent, Ordering::Release);
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo ARMED-ALIVE"]);
        unsafe {
            cmd.pre_exec(|| child_pre_exec_reset());
        }
        cmd.status().expect("spawn armed child")
    }

    #[test]
    fn healthy_child_of_expected_parent_survives_arming() {
        let _guard = RoleGuard;
        let status = spawn_armed(std::process::id());
        // A healthy child of a namespace-PID-1 parent lands here too: this
        // assert is exactly the constant-1 comparison's false positive,
        // which must never fire.
        assert!(
            status.success(),
            "healthy armed child was killed: {status:?}"
        );
    }

    #[test]
    fn child_whose_parent_diverges_from_expected_self_kills() {
        let _guard = RoleGuard;
        // Deliberately wrong expectation: the observed ppid (this test's
        // pid) diverges, so the race check must fire — proving the check
        // reads the expected pid rather than `== 1` (on this host the old
        // check's `getppid() == 1` was false for the same child).
        let wrong = std::process::id().wrapping_add(0x4000_0000);
        let status = spawn_armed(wrong);
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(libc::SIGKILL), "status: {status:?}");
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

    /// Android keeps its own POSIX realtime block at 41..64 with GNU's
    /// arithmetic names (signames.c:92-139). bionic's RT base comes from
    /// `__libc_current_sigrtmin/max()` (functions, like glibc), so the
    /// table pins the literals and asserts the equality here so a
    /// bionic ABI drift fails CI instead of shipping silently
    /// (rubash#1036).
    #[cfg(target_os = "android")]
    #[test]
    fn android_realtime_block_matches_bionic() {
        assert_eq!(unsafe { libc::__libc_current_sigrtmin() }, 41);
        assert_eq!(unsafe { libc::__libc_current_sigrtmax() }, 64);
        assert_eq!(super::signal_name(41), Some("RTMIN"));
        assert_eq!(super::signal_name(64), Some("RTMAX"));
        assert_eq!(super::signal_name(42), Some("RTMIN+1"));
        assert_eq!(super::signal_name(52), Some("RTMIN+11"));
        assert_eq!(super::signal_name(53), Some("RTMAX-11"));
        assert_eq!(super::signal_name(48), Some("RTMIN+7"));
        assert_eq!(super::signal_name(32), None);
        assert_eq!(super::signal_name(40), None);
        assert_eq!(super::signal_number_for_spec("RTMIN"), Some(41));
        assert_eq!(super::signal_number_for_spec("RTMIN+7"), Some(48));
        assert_eq!(super::signal_number_for_spec("RTMAX"), Some(64));
        assert_eq!(super::signal_number_for_spec("SIGRTMIN+3"), Some(44));
        assert_eq!(super::translate_signal("41"), Some("RTMIN"));
        assert_eq!(super::translate_signal("RTMIN"), Some("41"));
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

#[cfg(test)]
mod posix_sigspec_tests {
    use super::*;

    /// Bash posix-mode notes item 58: the SIG-prefixed spelling is an invalid
    /// signal specification in POSIX mode; the default mode keeps accepting
    /// both spellings. Probed on the validator itself - a live `kill` would
    /// either usage-out on the missing pid operand or signal the test process.
    #[test]
    fn posix_rejects_sig_prefixed_specifications() {
        assert!(!sigspec_accepted("SIGKILL", true));
        assert!(!sigspec_accepted("SIGUSR1", true));
        assert!(sigspec_accepted("SIGKILL", false));
        assert!(sigspec_accepted("SIGUSR1", false));
    }

    /// Numbers and bare names stay valid signal specifications in POSIX mode.
    #[test]
    fn posix_still_accepts_numbers_and_bare_names() {
        assert!(sigspec_accepted("9", true));
        assert!(sigspec_accepted("KILL", true));
        assert!(sigspec_accepted("15", true));
        assert!(sigspec_accepted("TERM", false));
    }

    /// rubash#1386: `kill -l` name lookup folds case on the whole spec —
    /// the SIG prefix included — so mixed/lowercase prefixed spellings
    /// print their number like the uppercase ones.
    #[test]
    fn mixed_case_prefixed_names_translate() {
        for spec in ["SigTerm", "sigterm", "SIGTERM", "sIgTeRm", "Term", "term"] {
            assert_eq!(translate_signal(spec), Some("15"), "{spec}");
        }
        assert_eq!(translate_signal("sighup"), Some("1"));
        assert_eq!(translate_signal("SigHup"), Some("1"));
        // The SIGNALS table is cfg-gated per platform (USR1 is 10 on
        // Linux/Android/Windows but 30 on macOS/FreeBSD); the test must
        // follow the platform number, not hardcode the Linux one, and
        // must not reach for libc: the crate is a unix-only dependency
        // while this test module also compiles on Windows (the goldens
        // job runs cargo test --lib there).
        #[cfg(any(target_os = "macos", target_os = "freebsd"))]
        let usr1 = "30";
        #[cfg(not(any(target_os = "macos", target_os = "freebsd")))]
        let usr1 = "10";
        assert_eq!(translate_signal("sigusr1"), Some(usr1));
    }

    /// rubash#1386: the EXIT pseudo-signal takes no SIG prefix in GNU
    /// (signal_names[0] is "EXIT", not "SIGEXIT") — bare EXIT folds case
    /// and translates to 0; every prefixed spelling is invalid.
    #[test]
    fn exit_takes_no_sig_prefix_in_translation() {
        for spec in ["EXIT", "Exit", "exit"] {
            assert_eq!(translate_signal(spec), Some("0"), "{spec}");
        }
        for spec in ["SIGEXIT", "SigExit", "sigexit"] {
            assert_eq!(translate_signal(spec), None, "{spec}");
        }
    }

    /// rubash#1385: the `-s`/`-n`/`-SPEC` decode folds case on the whole
    /// spec, so mixed/lowercase prefixed spellings resolve to the same
    /// signal number as the uppercase ones.
    #[test]
    fn mixed_case_prefixed_specs_decode() {
        for spec in ["SigTerm", "sigterm", "SIGTERM", "sIgTeRm", "Term", "term"] {
            assert!(sigspec_accepted(spec, false), "{spec}");
            assert_eq!(signal_number_from_spec(spec), Some(15), "{spec}");
        }
        assert_eq!(signal_number_from_spec("sighup"), Some(1));
        assert_eq!(signal_number_from_spec("SigHup"), Some(1));
        // Same platform-number note as mixed_case_prefixed_names_translate.
        #[cfg(any(target_os = "macos", target_os = "freebsd"))]
        assert_eq!(signal_number_from_spec("sigusr1"), Some(30));
        #[cfg(not(any(target_os = "macos", target_os = "freebsd")))]
        assert_eq!(signal_number_from_spec("sigusr1"), Some(10));
    }

    /// rubash#1385: bare EXIT (any case) decodes to signal 0 in both
    /// modes — the old exact-`EXIT` compare let `kill -s Exit` fall
    /// through to the TERM default — and every SIG-prefixed EXIT
    /// spelling is NO_SIG.
    #[test]
    fn exit_decodes_to_zero_without_prefix() {
        for spec in ["EXIT", "Exit", "exit"] {
            assert_eq!(signal_number_from_spec(spec), Some(0), "{spec}");
            assert!(sigspec_accepted(spec, true), "{spec}");
        }
        for spec in ["SIGEXIT", "SigExit", "sigexit"] {
            assert_eq!(signal_number_from_spec(spec), None, "{spec}");
            assert!(!sigspec_accepted(spec, false), "{spec}");
        }
    }

    /// POSIX mode drops DSIG_SIGPREFIX: prefixed spellings fold case for
    /// the rejection too, bare names still decode.
    #[test]
    fn posix_rejects_prefixed_specs_in_any_case() {
        assert!(!sigspec_accepted("sigterm", true));
        assert!(!sigspec_accepted("SigTerm", true));
        assert!(sigspec_accepted("term", true));
        assert!(sigspec_accepted("Term", true));
    }
}
