//! Runtime platform semantic axis.
//!
//! First slice (`unixwin/rubash#2829`): unified, runtime-parameterized
//! signal name/number table replacing the duplicated per-`#[cfg]` `SIGNALS`
//! tables in `builtins::kill` and `builtins::trap`.
//!
//! GNU Bash source ownership:
//! - `support/mksignames.c` / `support/signames.c:69` (`initialize_signames`)
//! - `builtins/kill.def` / `builtins/trap.def` / `trap.c:236` (`decode_signal`)
//!
//! All real OS syscalls (`libc::kill`, `sigaction`, `strsignal`, etc.) remain
//! behind `#[cfg(...)]` for the compiled host target; only the data-shape
//! signal table is keyed by [`Platform`] at runtime so any host can test
//! Linux, macOS/BSD, Android, and Windows signal semantics directly.

/// Target platform whose shell-level data semantics (signal numbering, etc.)
/// are active for an [`crate::Executor`] / [`crate::shell::ShellState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    Linux,
    Android,
    MacOs,
    FreeBsd,
    Windows,
}

impl Default for Platform {
    fn default() -> Self {
        Self::host()
    }
}

/// One positional slot in a platform's signal table (1-based `number`).
///
/// Holes (numbers with no signal name on that platform, e.g. Linux 32/33 or
/// Android 32..=40) have empty `sig_name` and `bare_name` strings, matching
/// GNU `signal_names[]` in `support/signames.c`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalEntry {
    pub number: i32,
    pub number_str: &'static str,
    pub sig_name: &'static str,
    pub bare_name: &'static str,
}

const fn entry(
    number: i32,
    number_str: &'static str,
    sig_name: &'static str,
    bare_name: &'static str,
) -> SignalEntry {
    SignalEntry {
        number,
        number_str,
        sig_name,
        bare_name,
    }
}

const fn hole(number: i32, number_str: &'static str) -> SignalEntry {
    SignalEntry {
        number,
        number_str,
        sig_name: "",
        bare_name: "",
    }
}

/// Linux (glibc) and Windows wire-format 64-slot signal table.
///
/// Slots 32 and 33 are holes (`""`); 34..=64 carry the GNU `signames.c:92-139`
/// POSIX realtime arithmetic names. On Windows (`not(unix)`), the
/// file-mailbox protocol (`%LOCALAPPDATA%\rubash-signals`) and suspend/resume
/// state backend speak Linux numbering by design.
const LINUX_SIGNALS: &[SignalEntry] = &[
    entry(1, "1", "SIGHUP", "HUP"),
    entry(2, "2", "SIGINT", "INT"),
    entry(3, "3", "SIGQUIT", "QUIT"),
    entry(4, "4", "SIGILL", "ILL"),
    entry(5, "5", "SIGTRAP", "TRAP"),
    entry(6, "6", "SIGABRT", "ABRT"),
    entry(7, "7", "SIGBUS", "BUS"),
    entry(8, "8", "SIGFPE", "FPE"),
    entry(9, "9", "SIGKILL", "KILL"),
    entry(10, "10", "SIGUSR1", "USR1"),
    entry(11, "11", "SIGSEGV", "SEGV"),
    entry(12, "12", "SIGUSR2", "USR2"),
    entry(13, "13", "SIGPIPE", "PIPE"),
    entry(14, "14", "SIGALRM", "ALRM"),
    entry(15, "15", "SIGTERM", "TERM"),
    entry(16, "16", "SIGSTKFLT", "STKFLT"),
    entry(17, "17", "SIGCHLD", "CHLD"),
    entry(18, "18", "SIGCONT", "CONT"),
    entry(19, "19", "SIGSTOP", "STOP"),
    entry(20, "20", "SIGTSTP", "TSTP"),
    entry(21, "21", "SIGTTIN", "TTIN"),
    entry(22, "22", "SIGTTOU", "TTOU"),
    entry(23, "23", "SIGURG", "URG"),
    entry(24, "24", "SIGXCPU", "XCPU"),
    entry(25, "25", "SIGXFSZ", "XFSZ"),
    entry(26, "26", "SIGVTALRM", "VTALRM"),
    entry(27, "27", "SIGPROF", "PROF"),
    entry(28, "28", "SIGWINCH", "WINCH"),
    entry(29, "29", "SIGIO", "IO"),
    entry(30, "30", "SIGPWR", "PWR"),
    entry(31, "31", "SIGSYS", "SYS"),
    hole(32, "32"),
    hole(33, "33"),
    entry(34, "34", "SIGRTMIN", "RTMIN"),
    entry(35, "35", "SIGRTMIN+1", "RTMIN+1"),
    entry(36, "36", "SIGRTMIN+2", "RTMIN+2"),
    entry(37, "37", "SIGRTMIN+3", "RTMIN+3"),
    entry(38, "38", "SIGRTMIN+4", "RTMIN+4"),
    entry(39, "39", "SIGRTMIN+5", "RTMIN+5"),
    entry(40, "40", "SIGRTMIN+6", "RTMIN+6"),
    entry(41, "41", "SIGRTMIN+7", "RTMIN+7"),
    entry(42, "42", "SIGRTMIN+8", "RTMIN+8"),
    entry(43, "43", "SIGRTMIN+9", "RTMIN+9"),
    entry(44, "44", "SIGRTMIN+10", "RTMIN+10"),
    entry(45, "45", "SIGRTMIN+11", "RTMIN+11"),
    entry(46, "46", "SIGRTMIN+12", "RTMIN+12"),
    entry(47, "47", "SIGRTMIN+13", "RTMIN+13"),
    entry(48, "48", "SIGRTMIN+14", "RTMIN+14"),
    entry(49, "49", "SIGRTMIN+15", "RTMIN+15"),
    entry(50, "50", "SIGRTMAX-14", "RTMAX-14"),
    entry(51, "51", "SIGRTMAX-13", "RTMAX-13"),
    entry(52, "52", "SIGRTMAX-12", "RTMAX-12"),
    entry(53, "53", "SIGRTMAX-11", "RTMAX-11"),
    entry(54, "54", "SIGRTMAX-10", "RTMAX-10"),
    entry(55, "55", "SIGRTMAX-9", "RTMAX-9"),
    entry(56, "56", "SIGRTMAX-8", "RTMAX-8"),
    entry(57, "57", "SIGRTMAX-7", "RTMAX-7"),
    entry(58, "58", "SIGRTMAX-6", "RTMAX-6"),
    entry(59, "59", "SIGRTMAX-5", "RTMAX-5"),
    entry(60, "60", "SIGRTMAX-4", "RTMAX-4"),
    entry(61, "61", "SIGRTMAX-3", "RTMAX-3"),
    entry(62, "62", "SIGRTMAX-2", "RTMAX-2"),
    entry(63, "63", "SIGRTMAX-1", "RTMAX-1"),
    entry(64, "64", "SIGRTMAX", "RTMAX"),
];

/// Android (bionic) 64-slot signal table.
///
/// Signals 1..=31 match Linux kernel numbering; 32..=40 are holes; 41..=64
/// carry bionic's realtime block (`__libc_current_sigrtmin()` = 41,
/// `__libc_current_sigrtmax()` = 64).
const ANDROID_SIGNALS: &[SignalEntry] = &[
    entry(1, "1", "SIGHUP", "HUP"),
    entry(2, "2", "SIGINT", "INT"),
    entry(3, "3", "SIGQUIT", "QUIT"),
    entry(4, "4", "SIGILL", "ILL"),
    entry(5, "5", "SIGTRAP", "TRAP"),
    entry(6, "6", "SIGABRT", "ABRT"),
    entry(7, "7", "SIGBUS", "BUS"),
    entry(8, "8", "SIGFPE", "FPE"),
    entry(9, "9", "SIGKILL", "KILL"),
    entry(10, "10", "SIGUSR1", "USR1"),
    entry(11, "11", "SIGSEGV", "SEGV"),
    entry(12, "12", "SIGUSR2", "USR2"),
    entry(13, "13", "SIGPIPE", "PIPE"),
    entry(14, "14", "SIGALRM", "ALRM"),
    entry(15, "15", "SIGTERM", "TERM"),
    entry(16, "16", "SIGSTKFLT", "STKFLT"),
    entry(17, "17", "SIGCHLD", "CHLD"),
    entry(18, "18", "SIGCONT", "CONT"),
    entry(19, "19", "SIGSTOP", "STOP"),
    entry(20, "20", "SIGTSTP", "TSTP"),
    entry(21, "21", "SIGTTIN", "TTIN"),
    entry(22, "22", "SIGTTOU", "TTOU"),
    entry(23, "23", "SIGURG", "URG"),
    entry(24, "24", "SIGXCPU", "XCPU"),
    entry(25, "25", "SIGXFSZ", "XFSZ"),
    entry(26, "26", "SIGVTALRM", "VTALRM"),
    entry(27, "27", "SIGPROF", "PROF"),
    entry(28, "28", "SIGWINCH", "WINCH"),
    entry(29, "29", "SIGIO", "IO"),
    entry(30, "30", "SIGPWR", "PWR"),
    entry(31, "31", "SIGSYS", "SYS"),
    hole(32, "32"),
    hole(33, "33"),
    hole(34, "34"),
    hole(35, "35"),
    hole(36, "36"),
    hole(37, "37"),
    hole(38, "38"),
    hole(39, "39"),
    hole(40, "40"),
    entry(41, "41", "SIGRTMIN", "RTMIN"),
    entry(42, "42", "SIGRTMIN+1", "RTMIN+1"),
    entry(43, "43", "SIGRTMIN+2", "RTMIN+2"),
    entry(44, "44", "SIGRTMIN+3", "RTMIN+3"),
    entry(45, "45", "SIGRTMIN+4", "RTMIN+4"),
    entry(46, "46", "SIGRTMIN+5", "RTMIN+5"),
    entry(47, "47", "SIGRTMIN+6", "RTMIN+6"),
    entry(48, "48", "SIGRTMIN+7", "RTMIN+7"),
    entry(49, "49", "SIGRTMIN+8", "RTMIN+8"),
    entry(50, "50", "SIGRTMIN+9", "RTMIN+9"),
    entry(51, "51", "SIGRTMIN+10", "RTMIN+10"),
    entry(52, "52", "SIGRTMIN+11", "RTMIN+11"),
    entry(53, "53", "SIGRTMAX-11", "RTMAX-11"),
    entry(54, "54", "SIGRTMAX-10", "RTMAX-10"),
    entry(55, "55", "SIGRTMAX-9", "RTMAX-9"),
    entry(56, "56", "SIGRTMAX-8", "RTMAX-8"),
    entry(57, "57", "SIGRTMAX-7", "RTMAX-7"),
    entry(58, "58", "SIGRTMAX-6", "RTMAX-6"),
    entry(59, "59", "SIGRTMAX-5", "RTMAX-5"),
    entry(60, "60", "SIGRTMAX-4", "RTMAX-4"),
    entry(61, "61", "SIGRTMAX-3", "RTMAX-3"),
    entry(62, "62", "SIGRTMAX-2", "RTMAX-2"),
    entry(63, "63", "SIGRTMAX-1", "RTMAX-1"),
    entry(64, "64", "SIGRTMAX", "RTMAX"),
];

/// Darwin / BSD-family 31-slot signal table (no realtime signals).
const BSD_SIGNALS: &[SignalEntry] = &[
    entry(1, "1", "SIGHUP", "HUP"),
    entry(2, "2", "SIGINT", "INT"),
    entry(3, "3", "SIGQUIT", "QUIT"),
    entry(4, "4", "SIGILL", "ILL"),
    entry(5, "5", "SIGTRAP", "TRAP"),
    entry(6, "6", "SIGABRT", "ABRT"),
    entry(7, "7", "SIGEMT", "EMT"),
    entry(8, "8", "SIGFPE", "FPE"),
    entry(9, "9", "SIGKILL", "KILL"),
    entry(10, "10", "SIGBUS", "BUS"),
    entry(11, "11", "SIGSEGV", "SEGV"),
    entry(12, "12", "SIGSYS", "SYS"),
    entry(13, "13", "SIGPIPE", "PIPE"),
    entry(14, "14", "SIGALRM", "ALRM"),
    entry(15, "15", "SIGTERM", "TERM"),
    entry(16, "16", "SIGURG", "URG"),
    entry(17, "17", "SIGSTOP", "STOP"),
    entry(18, "18", "SIGTSTP", "TSTP"),
    entry(19, "19", "SIGCONT", "CONT"),
    entry(20, "20", "SIGCHLD", "CHLD"),
    entry(21, "21", "SIGTTIN", "TTIN"),
    entry(22, "22", "SIGTTOU", "TTOU"),
    entry(23, "23", "SIGIO", "IO"),
    entry(24, "24", "SIGXCPU", "XCPU"),
    entry(25, "25", "SIGXFSZ", "XFSZ"),
    entry(26, "26", "SIGVTALRM", "VTALRM"),
    entry(27, "27", "SIGPROF", "PROF"),
    entry(28, "28", "SIGWINCH", "WINCH"),
    entry(29, "29", "SIGINFO", "INFO"),
    entry(30, "30", "SIGUSR1", "USR1"),
    entry(31, "31", "SIGUSR2", "USR2"),
];

impl Platform {
    /// Compiled host platform.
    pub const fn host() -> Self {
        #[cfg(not(unix))]
        {
            Self::Windows
        }
        #[cfg(target_os = "linux")]
        {
            Self::Linux
        }
        #[cfg(target_os = "android")]
        {
            Self::Android
        }
        #[cfg(target_os = "macos")]
        {
            Self::MacOs
        }
        #[cfg(target_os = "freebsd")]
        {
            Self::FreeBsd
        }
        #[cfg(all(
            unix,
            not(any(
                target_os = "linux",
                target_os = "android",
                target_os = "macos",
                target_os = "freebsd"
            ))
        ))]
        {
            Self::MacOs
        }
    }

    /// Positional signal table for this platform (index `n - 1` is signal `n`).
    pub const fn signal_table(self) -> &'static [SignalEntry] {
        match self {
            Self::Linux | Self::Windows => LINUX_SIGNALS,
            Self::Android => ANDROID_SIGNALS,
            Self::MacOs | Self::FreeBsd => BSD_SIGNALS,
        }
    }

    /// Bare signal name (without `SIG`, e.g. `"HUP"`, `"RTMIN"`) for `number`,
    /// or `None` for out-of-range numbers and platform holes.
    pub fn signal_name_for_number(self, number: i32) -> Option<&'static str> {
        let index = usize::try_from(number.checked_sub(1)?).ok()?;
        let entry = self.signal_table().get(index)?;
        (!entry.bare_name.is_empty()).then_some(entry.bare_name)
    }

    /// Canonical `SIG`-prefixed trap name (e.g. `"SIGHUP"`, `"SIGRTMIN"`) for
    /// `number`, or `None` for out-of-range numbers and platform holes.
    pub fn signal_trap_name_for_number(self, number: i32) -> Option<&'static str> {
        let index = usize::try_from(number.checked_sub(1)?).ok()?;
        let entry = self.signal_table().get(index)?;
        (!entry.sig_name.is_empty()).then_some(entry.sig_name)
    }

    /// Look up signal number string (e.g. `"15"`) for a bare (non-`SIG`-prefixed)
    /// signal name, matched case-insensitively.
    pub fn signal_number_str_for_bare_name(self, bare_name: &str) -> Option<&'static str> {
        if bare_name.is_empty() {
            return None;
        }
        self.signal_table().iter().find_map(|entry| {
            (!entry.bare_name.is_empty() && entry.bare_name.eq_ignore_ascii_case(bare_name))
                .then_some(entry.number_str)
        })
    }

    /// Look up signal number for a signal name (bare or `SIG`-prefixed,
    /// case-insensitive).
    pub fn signal_number_for_name(self, name: &str) -> Option<i32> {
        let bare = if name.len() >= 3 && name[..3].eq_ignore_ascii_case("SIG") {
            &name[3..]
        } else {
            name
        };
        if bare.is_empty() {
            return None;
        }
        self.signal_table().iter().find_map(|entry| {
            (!entry.bare_name.is_empty() && entry.bare_name.eq_ignore_ascii_case(bare))
                .then_some(entry.number)
        })
    }

    /// Platform `SIGCHLD` signal number.
    pub const fn sigchld_number(self) -> i32 {
        match self {
            Self::Linux | Self::Android | Self::Windows => 17,
            Self::MacOs | Self::FreeBsd => 20,
        }
    }

    /// Platform `SIGCONT` signal number.
    pub const fn sigcont_number(self) -> i32 {
        match self {
            Self::Linux | Self::Android | Self::Windows => 18,
            Self::MacOs | Self::FreeBsd => 19,
        }
    }

    /// Platform `SIGTSTP` signal number.
    pub const fn sigtstp_number(self) -> i32 {
        match self {
            Self::Linux | Self::Android | Self::Windows => 20,
            Self::MacOs | Self::FreeBsd => 18,
        }
    }

    /// Platform `SIGQUIT` signal number.
    pub const fn sigquit_number(self) -> i32 {
        3
    }

    /// Platform `SIGINT` signal number.
    pub const fn sigint_number(self) -> i32 {
        2
    }
}

/// Look up signal number for `name` (bare or `SIG`-prefixed, case-insensitive)
/// on `platform`.
pub fn signal_number_for_name(platform: Platform, name: &str) -> Option<i32> {
    platform.signal_number_for_name(name)
}

/// Look up bare signal name (without `SIG`) for `num` on `platform`.
pub fn signal_name_for_number(platform: Platform, num: i32) -> Option<&'static str> {
    platform.signal_name_for_number(num)
}
