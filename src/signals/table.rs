//! Signal name/number tables and signal-spec decoding.

use crate::platform::Platform;

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

/// The platform's SIGTSTP number. GNU's stop class is stable across
/// the platforms bash models (20 on Linux and Darwin/BSD); the
/// non-unix value keeps the Windows file-mailbox wire format's Linux
/// numbering by contract (see SIGNALS above). Used by the Windows
/// foreground-stop arm (rubash#3467), which matches the 128+20
/// verdict by number.
#[cfg(unix)]
pub const SIGTSTP_NUMBER: i32 = libc::SIGTSTP;
/// Non-unix (Windows mailbox wire format, Linux numbering by contract).
#[cfg(not(unix))]
pub const SIGTSTP_NUMBER: i32 = 20;

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

/// The platform's SIGINT number for dispatch paths that poll the pending
/// queue (the winuxcmd fifo rendezvous wait, rubash#1903). Linux: 2; Darwin/
/// BSD also number it 2. The non-unix value keeps the Windows file-mailbox
/// wire format's Linux numbering by contract (see SIGNALS above).
#[cfg(unix)]
pub const SIGINT_NUMBER: i32 = libc::SIGINT;
/// Non-unix (Windows mailbox wire format, Linux numbering by contract).
#[cfg(not(unix))]
pub const SIGINT_NUMBER: i32 = 2;

pub fn list_first_signal_for_sed() -> &'static str {
    "SIGHUP"
}

pub fn signal_number_for_spec(value: &str) -> Option<i32> {
    signal_number_from_spec(value)
}

pub fn signal_number_for_spec_on_platform(platform: Platform, value: &str) -> Option<i32> {
    decode_signal_spec_for_platform(value, false, platform)
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
    translate_signal_for_platform(Platform::host(), value)
}

pub fn translate_signal_for_platform(platform: Platform, value: &str) -> Option<&'static str> {
    if value == "0" {
        return Some("EXIT");
    }

    if let Ok(mut number) = value.parse::<i32>() {
        if number > 128 {
            number -= 128;
        }
        return signal_name_for_platform(platform, number);
    }

    decode_signal_name_for_platform(value, true, platform)
}

/// GNU general.c valid_number: leading whitespace is skipped, an
/// optional sign follows, then digits, then trailing whitespace, and
/// the whole word must be consumed. Returns the parsed value.
pub(crate) fn valid_number(value: &str) -> Option<i64> {
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
pub(crate) fn decode_signal_name_for_platform(
    value: &str,
    allow_sig_prefix: bool,
    platform: Platform,
) -> Option<&'static str> {
    if value.eq_ignore_ascii_case("EXIT") {
        return Some("0");
    }
    // rubash#2926: the trap pseudo-signals (GNU trap.h:40-46 —
    // DEBUG_TRAP = NSIG, ERROR_TRAP = NSIG+1, RETURN_TRAP = NSIG+2;
    // support/signames.c:466-468 puts the BARE names in signal_names[],
    // so trap.c:236 decode_signal resolves them to 65/66/67). The
    // match is on the value as written: the table entries carry no
    // `SIG` prefix, so `SIGERR`/`SIGDEBUG`/`SIGRETURN` do NOT match —
    // they fall through to the prefix strip below and stay NO_SIG.
    // They are deliberately NOT added to the SIGNALS table: the full
    // `kill -l` list (common.c:747) iterates 1..NSIG and must keep
    // excluding them, and `kill -l 65/66/67` stays invalid.
    if value.eq_ignore_ascii_case("DEBUG") {
        return Some("65");
    }
    if value.eq_ignore_ascii_case("ERR") {
        return Some("66");
    }
    if value.eq_ignore_ascii_case("RETURN") {
        return Some("67");
    }
    let bare = if allow_sig_prefix {
        strip_sig_prefix(value).unwrap_or(value)
    } else {
        value
    };
    if bare.eq_ignore_ascii_case("EXIT") {
        return None;
    }
    signal_number_for_platform(platform, bare)
}

/// True for the trap pseudo-signals (65/66/67): they decode to real
/// numbers, but kill(2) rejects them with EINVAL, so the delivery
/// loop must adjudicate them by the kernel's ESRCH-before-EINVAL
/// order (kill.def:194-216) instead of delivering them.
pub(crate) fn is_trap_pseudo_signal(signal: i32) -> bool {
    matches!(signal, 65 | 66 | 67)
}

/// GNU trap.c:236 decode_signal, whole spec: a valid_number is the
/// signal number itself (no 128 adjustment — that mapping exists only
/// in display_signal_list's exit-status translation), anything else is
/// a name. A number naming a hole in the table (glibc's reserved 32/33)
/// stays NO_SIG here: the SIGNALS table is the acceptance contract the
/// rest of the kill machinery (mailbox wire numbers, `kill -l` holes)
/// is built on.
fn decode_signal_spec(value: &str, posix: bool) -> Option<i32> {
    decode_signal_spec_for_platform(value, posix, Platform::host())
}

pub(crate) fn decode_signal_spec_for_platform(
    value: &str,
    posix: bool,
    platform: Platform,
) -> Option<i32> {
    if let Some(number) = valid_number(value) {
        if number == 0 {
            return Some(0);
        }
        return (number >= 1
            && number <= 64
            && signal_name_for_platform(platform, number as i32).is_some())
        .then_some(number as i32);
    }
    decode_signal_name_for_platform(value, !posix, platform)?
        .parse::<i32>()
        .ok()
}

pub(crate) fn signal_number_from_spec(value: &str) -> Option<i32> {
    decode_signal_spec(value, false)
}

/// The kill signal-spec validator for the `-s`/`-n`/`-SPEC` forms:
/// exactly decode_signal_spec (kept as a named entry for the POSIX
/// sigspec unit tests). The previous validator accepted a spec through
/// translate_signal but the caller then re-decoded it with an
/// exact-uppercase-only prefix strip and an exact `EXIT` compare, so
/// `kill -s SigTerm` was rejected and `kill -s Exit` fell through to
/// the TERM default and delivered the wrong signal (rubash#1385).
#[cfg(test)]
pub(crate) fn sigspec_accepted(value: &str, posix: bool) -> bool {
    decode_signal_spec(value, posix).is_some()
}

#[cfg(all(test, unix))]
fn signal_name(number: i32) -> Option<&'static str> {
    signal_name_for_platform(Platform::host(), number)
}

pub(crate) fn signal_name_for_platform(platform: Platform, number: i32) -> Option<&'static str> {
    platform.signal_name_for_number(number)
}

#[cfg(all(test, any(target_os = "macos", target_os = "freebsd")))]
fn signal_number(name: &str) -> Option<&'static str> {
    signal_number_for_platform(Platform::host(), name)
}

fn signal_number_for_platform(platform: Platform, name: &str) -> Option<&'static str> {
    // GNU trap.c decode_signal with DSIG_NOCASE (kill.def:103 sets it
    // unconditionally): signal names fold case (rubash#1367 #1385 #1386).
    platform.signal_number_str_for_bare_name(name)
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
        for entry in crate::platform::Platform::host().signal_table() {
            assert_eq!(
                entry.number_str.parse::<i32>().unwrap(),
                entry.number,
                "digit field {:?} does not match number {}",
                entry.number_str,
                entry.number
            );
        }
    }

    /// `kill -l` output order is a GNU-compatible surface; the table must
    /// stay in strictly ascending signal-number order on every platform.
    #[test]
    fn table_is_in_ascending_signal_number_order() {
        for pair in crate::platform::Platform::host().signal_table().windows(2) {
            assert!(
                pair[0].number < pair[1].number,
                "signal table out of numeric order at {} then {}",
                pair[0].number,
                pair[1].number
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
