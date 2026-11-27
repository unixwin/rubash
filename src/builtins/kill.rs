//! kill module.
//!
//! GNU Bash source ownership:
// - builtins/kill.def

use crate::platform::Platform;
#[cfg(unix)]
use crate::signals::delivery::signal_process_group;
use crate::signals::delivery::{deliver_rubash_signal, signal_process};
use crate::signals::table::{
    decode_signal_name_for_platform, decode_signal_spec_for_platform, is_trap_pseudo_signal,
    signal_name_for_platform, valid_number,
};
use std::io::{self, Write};

pub use crate::signals::*;

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
    execute_with_io_posix_for_platform(args, stdout, stderr, posix, Platform::host())
}

/// [`execute_with_io_posix`] parameterized by runtime [`Platform`].
pub fn execute_with_io_posix_for_platform<W, E>(
    args: &[String],
    stdout: &mut W,
    stderr: &mut E,
    posix: bool,
    platform: Platform,
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
            signal = decode_signal_spec_for_platform(spec, posix, platform);
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
            signal = decode_signal_spec_for_platform(sigspec, posix, platform);
            saw_signal = true;
            index += 1;
            continue;
        }
        break;
    }
    let operands_start = index;

    if listing {
        return display_kill_list(&args[operands_start..], posix, platform, stdout, stderr);
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
            if is_trap_pseudo_signal(signal) {
                // rubash#2926: kill.def:194-216 — the kernel decides:
                // ESRCH (dead group) beats EINVAL (live group, bad
                // signal). Probe with signal 0 (which never EINVALs)
                // and report the probe's verdict, not a delivery.
                match signal_process_group(pid, 0) {
                    Ok(()) => {
                        writeln!(
                            stderr,
                            "{}kill: {sigspec}: invalid signal specification",
                            diagnostic_prefix()
                        )?;
                    }
                    Err(message) => {
                        writeln!(stderr, "{}kill: ({pid}) - {message}", diagnostic_prefix())?;
                    }
                }
                any_failed = true;
            } else {
                match signal_process_group(pid, signal) {
                    Ok(()) => any_succeeded = true,
                    Err(message) => {
                        writeln!(stderr, "{}kill: ({pid}) - {message}", diagnostic_prefix())?;
                        any_failed = true;
                    }
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

        // rubash#2236: a delivery addressed to the process's own pid
        // from inside an in-process child shell (direct-child route
        // armed) is addressed to the CHILD — GNU's child is a separate
        // process whose $$ is its own pid, so the kill can only kill
        // the child. It joins the ancestor lane (ANCESTOR_SIGNALS):
        // that lane's contract is exactly the child's — no kernel
        // disposition judges it (the stale-handler check in
        // run_pending_signal_traps does not apply), the boundary
        // dispatch adjudicates it against the then-current (child)
        // trap table, and an untrapped verdict ends only the child's
        // run. The plain self-signal queue would be wrong here: its
        // entries carry the stale check, and an untrapped signal has
        // no kernel handler installed by design (#226), so the
        // delivery would be dropped instead of adjudicated. Raising
        // it into the hosting process instead killed the outer shell
        // with the child's signal.
        #[cfg(unix)]
        if pid == std::process::id()
            && signal != 0
            && direct_child_route_active()
            && boundary_adjudicable_signal(signal)
        {
            queue_ancestor_signal(signal);
            any_succeeded = true;
            continue;
        }

        // rubash#2926: a trap pseudo-signal (65/66/67) must never be
        // delivered — kill(2) rejects it with EINVAL — and it must
        // never reach the mailbox path either. Adjudicate by the
        // kernel's order (kill.def:194-216): a dead pid reports
        // "No such process" (ESRCH first); a live pid reports the
        // invalid signal specification. process_exists probes with
        // signal 0, which never EINVALs.
        if is_trap_pseudo_signal(signal) {
            if process_exists(pid) {
                writeln!(
                    stderr,
                    "{}kill: {sigspec}: invalid signal specification",
                    diagnostic_prefix()
                )?;
            } else {
                writeln!(
                    stderr,
                    "{}kill: ({pid}) - No such process",
                    diagnostic_prefix()
                )?;
            }
            any_failed = true;
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
    platform: Platform,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    if list.is_empty() {
        if posix {
            write_signal_list_posix(platform, stdout)?;
        } else {
            write_signal_list(platform, stdout)?;
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
            } else if let Some(translation) = signal_name_for_platform(platform, number as i32) {
                writeln!(stdout, "{translation}")?;
            }
            continue;
        }
        if let Some(translation) = decode_signal_name_for_platform(value, !posix, platform) {
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

/// rubash#2927: GNU kill.def:194 routes pid operands through
/// general.c valid_number (strtoimax, base 10 — leading whitespace,
/// optional sign, digits, trailing whitespace, whole word consumed)
/// and then requires the value to fit pid_t. `value.parse::<i32>()`
/// rejected every whitespace-padded operand; route through the
/// file's existing valid_number port (i64) plus the pid_t range
/// check instead.
fn parse_pid(value: &str) -> Option<i32> {
    valid_number(value).and_then(|number| i32::try_from(number).ok())
}

fn write_signal_list<W>(platform: Platform, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    // GNU kill.def list_signals: 5 entries per line, tab-separated; the
    // final partial line still carries the tab separator before the closing
    // newline (captured from GNU 5.3.0 `kill -l` on the WSL baseline).
    let entries: Vec<_> = platform
        .signal_table()
        .iter()
        .filter(|entry| !entry.bare_name.is_empty())
        .collect();
    let total = entries.len();
    for (position, entry) in entries.iter().enumerate() {
        let position = position + 1;
        if position > 1 && (position - 1) % 5 == 0 {
            writeln!(stdout)?;
        } else if position > 1 {
            write!(stdout, "\t")?;
        }
        write!(stdout, "{:>2}) {}", entry.number, entry.sig_name)?;
        if position == total && position % 5 != 0 {
            write!(stdout, "\t")?;
        }
    }
    writeln!(stdout)?;
    Ok(())
}

/// POSIX-mode `kill -l` with no operand (rubash#1054): one line, bare
/// names in signal-number order, single-space separated.
fn write_signal_list_posix<W>(platform: Platform, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    let names: Vec<&str> = platform
        .signal_table()
        .iter()
        .filter(|entry| !entry.bare_name.is_empty())
        .map(|entry| entry.bare_name)
        .collect();
    writeln!(stdout, "{}", names.join(" "))?;
    Ok(())
}

fn diagnostic_prefix() -> String {
    // The single argv[0] carrier chain (builtins::prefix,
    // rubash#2791): the old fallback's "rubash: " literal named
    // every renamed shell "rubash" whenever one arm was missing.
    crate::builtins::prefix::diagnostic_prefix(&|key: &str| std::env::var(key).ok())
}

#[cfg(test)]
mod posix_sigspec_tests {
    use super::*;
    use crate::signals::table::{signal_number_from_spec, sigspec_accepted};

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
