use super::*;

#[derive(Clone, Copy, Debug)]
pub(in crate::executor) struct TimeCommandMetrics {
    real: std::time::Duration,
    user: std::time::Duration,
    sys: std::time::Duration,
}

/// Start snapshot for a `time` measurement: wall clock plus the process
/// CPU counters. GNU execute_cmd.c print_time_info reports the deltas of
/// getrusage(RUSAGE_SELF) + getrusage(RUSAGE_CHILDREN) between the start
/// and end of the timed command; rubash used to leave user/sys at zero
/// unconditionally (rubash#1212).
#[derive(Clone, Copy, Debug)]
pub(in crate::executor) struct TimeCommandStart {
    instant: std::time::Instant,
    rusage: RusageCounters,
}

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::executor) struct RusageCounters {
    user: std::time::Duration,
    sys: std::time::Duration,
}

#[cfg(unix)]
fn read_rusage(who: libc::c_int) -> (std::time::Duration, std::time::Duration) {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(who, &mut usage) } != 0 {
        return (std::time::Duration::ZERO, std::time::Duration::ZERO);
    }
    let to_duration =
        |tv: libc::timeval| std::time::Duration::new(tv.tv_sec as u64, (tv.tv_usec as u32) * 1_000);
    (to_duration(usage.ru_utime), to_duration(usage.ru_stime))
}

#[cfg(unix)]
fn rusage_counters() -> RusageCounters {
    let (self_user, self_sys) = read_rusage(libc::RUSAGE_SELF);
    let (child_user, child_sys) = read_rusage(libc::RUSAGE_CHILDREN);
    RusageCounters {
        user: self_user + child_user,
        sys: self_sys + child_sys,
    }
}

/// Accumulated CPU of reaped external children (rubash#1776): GNU
/// getrusage(RUSAGE_CHILDREN) sums every waited child, but Windows has
/// no such counter — each child's times must be sampled from its
/// process handle before the executor closes it, so every reap site
/// calls note_reaped_child_cpu while the handle is still live and
/// rusage_counters adds the sum to the SELF half. The dedup key is
/// (pid, process creation time): a child observed at two reap points
/// (poll + formal wait) must count once, and a recycled pid must not
/// be mistaken for the earlier child. Process-wide by necessity — a
/// Windows process has one handle table, so sessions sharing a
/// process share this sum (same caveat class as the executor's other
/// process-global state; a per-Session split rides the 1.5.0
/// state-view work, not this fix). Lock poisoning is tolerated: the
/// accumulator is a sum, not guarded data.
#[cfg(windows)]
static REAPED_CHILDREN_CPU: std::sync::LazyLock<
    std::sync::Mutex<(std::collections::HashSet<(u32, u64)>, RusageCounters)>,
> = std::sync::LazyLock::new(|| {
    std::sync::Mutex::new((std::collections::HashSet::new(), RusageCounters::default()))
});

/// A reaped child that can still yield its live process handle to the
/// CHILDREN sampler below. std::process::Child and fd::SpawnedChild
/// differ only in how the handle is reached: SpawnedChild's Whitelisted
/// form holds a raw OwnedHandle, not a std Child, so demanding a Child
/// here made the two job_builtins reap sites uncompilable (E0308).
pub(in crate::executor) trait ReapedChildCpu {
    fn reaped_pid(&self) -> u32;
    #[cfg(windows)]
    fn reaped_handle(&self) -> windows_sys::Win32::Foundation::HANDLE;
}

impl ReapedChildCpu for std::process::Child {
    fn reaped_pid(&self) -> u32 {
        self.id()
    }
    #[cfg(windows)]
    fn reaped_handle(&self) -> windows_sys::Win32::Foundation::HANDLE {
        use std::os::windows::io::AsRawHandle;
        self.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE
    }
}

impl ReapedChildCpu for crate::fd::SpawnedChild {
    fn reaped_pid(&self) -> u32 {
        self.id()
    }
    #[cfg(windows)]
    fn reaped_handle(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.process_handle() as windows_sys::Win32::Foundation::HANDLE
    }
}

/// Sample one reaped child's CPU into the CHILDREN accumulator. Call
/// only at a reap point, while the child's handle is still live (after
/// try_wait/wait returns a status, before the child is dropped). On
/// non-Windows this is a no-op: getrusage(RUSAGE_CHILDREN) already
/// covers waited children there.
pub(in crate::executor) fn note_reaped_child_cpu<C: ReapedChildCpu>(child: &C) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::GetProcessTimes;
        let handle = child.reaped_handle();
        let mut creation: FILETIME = unsafe { std::mem::zeroed() };
        let mut exit: FILETIME = unsafe { std::mem::zeroed() };
        let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
        let mut user: FILETIME = unsafe { std::mem::zeroed() };
        let ok =
            unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
        if ok == 0 {
            return;
        }
        let ticks = |ft: FILETIME| ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
        let key = (child.reaped_pid(), ticks(creation));
        let mut guard = REAPED_CHILDREN_CPU
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.0.insert(key) {
            guard.1.user += std::time::Duration::from_nanos(ticks(user).saturating_mul(100));
            guard.1.sys += std::time::Duration::from_nanos(ticks(kernel).saturating_mul(100));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = child;
    }
}

#[cfg(windows)]
fn rusage_counters() -> RusageCounters {
    // Windows has no getrusage. The SELF half of GNU's SELF+CHILDREN
    // sum maps to GetProcessTimes on the current process — which is
    // also where rubash's no-fork subshell and builtin work lands, so
    // it is the half that moves for in-process timed commands (a pure
    // builtin loop under `time` used to report user 0m0.000s here).
    // The CHILDREN half is the reap-site accumulator above
    // (rubash#1776): external children sampled at wait time. One
    // known gap remains: a capture pipeline's last member is consumed
    // by wait_with_output, which closes its handle before any sample
    // point, so that member's CPU is still missing.
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    fn ticks(ft: FILETIME) -> std::time::Duration {
        // FILETIME counts 100-nanosecond intervals since 1601.
        let raw = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
        std::time::Duration::from_nanos(raw.saturating_mul(100))
    }

    let mut creation: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
    let mut user: FILETIME = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        return RusageCounters::default();
    }
    let mut counters = RusageCounters {
        user: ticks(user),
        sys: ticks(kernel),
    };
    let reaped = REAPED_CHILDREN_CPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    counters.user += reaped.1.user;
    counters.sys += reaped.1.sys;
    counters
}

#[cfg(not(any(unix, windows)))]
fn rusage_counters() -> RusageCounters {
    RusageCounters::default()
}

impl TimeCommandMetrics {
    pub(in crate::executor) fn from_start(started: TimeCommandStart) -> Self {
        Self::from_start_until(started, std::time::Instant::now())
    }

    /// Metrics with the real interval ending at a CAPTURED instant
    /// instead of now (rubash#1528): GNU's waitchld stamps a job's
    /// timeval when the child is reaped, so a timed coproc's report
    /// shows the true lifetime even when the completion is only
    /// APPLIED at a later command boundary (a long foreground command
    /// in between must not inflate `real`). user/sys stay delta-based:
    /// RUSAGE_CHILDREN cannot be backdated, and the intervening
    /// commands' own children are the only over-attribution (their
    /// CPU is ~0 for the sleep-class commands this affects).
    pub(in crate::executor) fn from_start_until(
        started: TimeCommandStart,
        end: std::time::Instant,
    ) -> Self {
        let now = rusage_counters();
        Self {
            real: end.saturating_duration_since(started.instant),
            user: now.user.saturating_sub(started.rusage.user),
            sys: now.sys.saturating_sub(started.rusage.sys),
        }
    }
}

pub(in crate::executor) fn time_command_started() -> TimeCommandStart {
    TimeCommandStart {
        instant: std::time::Instant::now(),
        rusage: rusage_counters(),
    }
}

pub(in crate::executor) fn print_time(
    env_vars: &HashMap<String, String>,
    posix_format: bool,
    started: TimeCommandStart,
) {
    let metrics = TimeCommandMetrics::from_start(started);
    if let Some(report) = format_time_report(env_vars, posix_format, metrics) {
        eprintln!("{report}");
    }
}

/// The composed `time` report (no trailing newline), or None when the active
/// format produces nothing. An invalid TIMEFORMAT still reports its
/// diagnostic on the raw stderr — the caller never sees it.
pub(in crate::executor) fn format_time_report(
    env_vars: &HashMap<String, String>,
    posix_format: bool,
    metrics: TimeCommandMetrics,
) -> Option<String> {
    if posix_format {
        return Some(format!(
            "real {}\nuser {}\nsys {}",
            format_time_seconds(metrics.real, Some(2), false),
            format_time_seconds(metrics.user, Some(2), false),
            format_time_seconds(metrics.sys, Some(2), false),
        ));
    }

    // GNU execute_cmd.c:1245: without `time -p` the default format is
    // BASH_TIMEFORMAT — a leading blank line, tab separators and the
    // minute form (`\nreal\t%3lR\nuser\t%3lU\nsys\t%3lS`, with REAL
    // control characters in the value); only a set TIMEFORMAT
    // overrides it (print_time_info, execute_cmd.c:1552-1557).
    // POSIX_TIMEFORMAT (:1244) is exclusively the `time -p` format above.
    let format = env_vars
        .get("TIMEFORMAT")
        .map(String::as_str)
        .unwrap_or("\nreal\t%3lR\nuser\t%3lU\nsys\t%3lS");

    if format.is_empty() {
        return None;
    }

    match expand_time_format(format, metrics) {
        Ok(output) => Some(output),
        Err(invalid) => {
            eprintln!(
                "{}TIMEFORMAT: `{invalid}': invalid format character",
                crate::builtins::set::builtin_error_prefix(env_vars)
            );
            None
        }
    }
}

fn expand_time_format(format: &str, metrics: TimeCommandMetrics) -> Result<String, char> {
    let mut output = String::new();
    let mut chars = format.chars().peekable();

    // GNU print_formatted_time (execute_cmd.c:1346): only `%`
    // directives are special — every other byte, backslash included,
    // is copied verbatim (rubash#1939: `TIMEFORMAT='A\nB'` prints
    // the two characters `\` and `n`, not a newline).
    while let Some(ch) = chars.next() {
        match ch {
            '%' => expand_time_format_percent(&mut output, &mut chars, metrics)?,
            other => output.push(other),
        }
    }

    Ok(output)
}

fn expand_time_format_percent<I>(
    output: &mut String,
    chars: &mut std::iter::Peekable<I>,
    metrics: TimeCommandMetrics,
) -> Result<(), char>
where
    I: Iterator<Item = char>,
{
    let precision = if chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
        chars
            .next()
            .and_then(|ch| ch.to_digit(10))
            .map(|value| value as usize)
    } else {
        None
    };
    let long = chars.peek().is_some_and(|ch| *ch == 'l');
    if long {
        chars.next();
    }

    match chars.next() {
        Some('%') if precision.is_none() && !long => {
            output.push('%');
        }
        Some('%') => return Err('%'),
        Some('R') => output.push_str(&format_time_seconds(metrics.real, precision, long)),
        Some('U') => output.push_str(&format_time_seconds(metrics.user, precision, long)),
        Some('S') => output.push_str(&format_time_seconds(metrics.sys, precision, long)),
        Some('P') if precision.is_none() && !long => output.push_str(&format_cpu_percent(metrics)),
        Some('P') => return Err('P'),
        Some(other) => return Err(other),
        None => {
            output.push('%');
            return Ok(());
        }
    }

    Ok(())
}

fn format_time_seconds(
    duration: std::time::Duration,
    precision: Option<usize>,
    long: bool,
) -> String {
    let precision = precision.unwrap_or(3);
    let seconds = duration.as_secs_f64();
    if precision == 0 {
        if long {
            let minutes = (seconds / 60.0).floor() as u64;
            let remainder = (seconds - (minutes as f64 * 60.0)).round() as u64;
            return format!("{minutes}m{remainder}s");
        }
        return format!("{seconds:.0}");
    }

    if long {
        let minutes = (seconds / 60.0).floor() as u64;
        let remainder = seconds - (minutes as f64 * 60.0);
        format!("{minutes}m{remainder:.precision$}s")
    } else {
        format!("{seconds:.precision$}")
    }
}

fn format_cpu_percent(metrics: TimeCommandMetrics) -> String {
    let real = metrics.real.as_secs_f64();
    if real <= f64::EPSILON {
        return "0.00".to_string();
    }
    let cpu = metrics.user.as_secs_f64() + metrics.sys.as_secs_f64();
    format!("{:.2}", (cpu / real) * 100.0)
}

pub(in crate::executor) fn read_char_limit_argument<S>(
    word: Option<&S>,
) -> Result<Option<usize>, String>
where
    S: AsRef<str> + ?Sized,
{
    let Some(word) = word else {
        return Ok(None);
    };
    let value = word.as_ref();
    value
        .parse::<usize>()
        .map(Some)
        .map_err(|_| value.to_string())
}

/// One unit produced by [`StdinCharDecoder`]: either a decoded UTF-8
/// character or a single input byte that is not valid UTF-8, carried as its
/// raw-byte marker text (the `substitution_metadata::bytes_to_shell_text`
/// contract, so executor output paths re-emit the original byte).
pub(in crate::executor) enum StdinUnit {
    Char(char),
    RawByte { text: String },
}

/// Incremental UTF-8 decoder for byte-wise process-stdin reads (the `read`
/// builtin's unbuffered path and the inherited-stdin fallback). Widening
/// each byte with `byte as char` Latin-1-encodes multibyte input and lets a
/// `-n`/`char_limit` break land mid-character; this decoder instead reads a
/// lead byte, waits for its continuation bytes, and surfaces undecodable
/// bytes as raw-byte markers. `lookahead` holds at most a few bytes that
/// could not join the pending sequence, so at most the bytes of one
/// character are ever consumed ahead of the current unit.
pub(in crate::executor) struct StdinCharDecoder {
    /// Bytes of the in-progress sequence; never holds a completed unit.
    pending: Vec<u8>,
    /// Read-ahead bytes that could not continue `pending`.
    lookahead: std::collections::VecDeque<u8>,
}

impl StdinCharDecoder {
    pub(in crate::executor) fn new() -> Self {
        Self {
            pending: Vec::new(),
            lookahead: std::collections::VecDeque::new(),
        }
    }

    pub(in crate::executor) fn queue_byte(&mut self, byte: u8) {
        self.lookahead.push_back(byte);
    }

    /// True while a queued byte still waits for a sequence decision. Callers
    /// must not `read` more stdin until this returns false, so a
    /// delimiter/limit break leaves at most the in-flight character's bytes
    /// consumed.
    pub(in crate::executor) fn has_queued(&self) -> bool {
        !self.lookahead.is_empty()
    }

    /// Pop the next completed unit, or `None` while `pending` is a
    /// valid-but-incomplete UTF-8 prefix awaiting more bytes.
    pub(in crate::executor) fn next_unit(&mut self) -> Option<StdinUnit> {
        if !self.pending.is_empty() {
            match self.lookahead.front() {
                Some(&next) if (next & 0xC0) == 0x80 => {
                    self.pending.push(next);
                    self.lookahead.pop_front();
                }
                Some(_) => {
                    let head = self.pending.remove(0);
                    return Some(Self::raw_byte_unit(head));
                }
                None => {}
            }
        }
        if self.pending.is_empty() {
            let byte = self.lookahead.pop_front()?;
            self.pending.push(byte);
        }
        match std::str::from_utf8(&self.pending) {
            Ok(text) => {
                let ch = text.chars().next().expect("pending is non-empty");
                self.pending.drain(..ch.len_utf8());
                Some(StdinUnit::Char(ch))
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    let ch = std::str::from_utf8(&self.pending[..valid])
                        .expect("valid prefix")
                        .chars()
                        .next()
                        .expect("non-empty valid prefix");
                    self.pending.drain(..ch.len_utf8());
                    return Some(StdinUnit::Char(ch));
                }
                if error.error_len().is_some() {
                    let head = self.pending.remove(0);
                    return Some(Self::raw_byte_unit(head));
                }
                None
            }
        }
    }

    /// Drain any bytes still pending at end of input (a truncated UTF-8
    /// sequence) into `output` as raw-byte markers.
    pub(in crate::executor) fn flush(&mut self, output: &mut String) {
        while let Some(unit) = self.next_unit() {
            match unit {
                StdinUnit::Char(ch) => output.push(ch),
                StdinUnit::RawByte { text } => output.push_str(&text),
            }
        }
        for byte in self.pending.drain(..).chain(self.lookahead.drain(..)) {
            output.push_str(&substitution_metadata::bytes_to_shell_text(&[byte]));
        }
    }

    fn raw_byte_unit(byte: u8) -> StdinUnit {
        StdinUnit::RawByte {
            text: substitution_metadata::bytes_to_shell_text(&[byte]),
        }
    }
}

/// GNU read.def takes `-d` as the first *byte* of the option word
/// (`delim = (unsigned char)*optarg`). Decode raw-byte markers so
/// `$'\200'` yields byte 0x80; `char::from` keeps `delimiter as u8`
/// byte-compare sites working unchanged.
pub(in crate::executor) fn read_delimiter_char(word: &str) -> char {
    let bytes = substitution_metadata::shell_text_to_raw_bytes(word);
    // GNU builtins/read.def: the -d delimiter is the first CHARACTER of
    // the argument (read_builtin decodes it with the locale's multibyte
    // rules, and zread compares decoded characters via read_mbchar). In a
    // multibyte locale `-d é` is the character é, not its first byte 0xC3:
    // taking byte 0 made the delimiter U+00C3, which neither the literal
    // text nor the marker-pair needle in the buffered FUNCTION_STDIN
    // mirror ever matches — a builtin producer's `read -d é` read to EOF
    // (rubash#1062). Single-byte locales (and words that are not valid
    // UTF-8, e.g. -d $'\xff') keep the first-byte rule.
    if crate::locale::is_multi_byte() {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if let Some(ch) = text.chars().next() {
                return ch;
            }
        }
    }
    bytes.first().copied().map(char::from).unwrap_or('\0')
}

/// The delimiter needle inside buffered shell text: raw bytes live in text
/// as marker pairs, so a >=0x80 delimiter matches its encoded pair. Carrier
/// bytes (0x0c, 0x11, 0x13, 0x14..=0x1f) are stored the same way once the
/// record has been normalized by `tag_read_carrier_text`, so `-d $'\x1b'`
/// matches the tagged record too.
pub(in crate::executor) fn read_delimiter_needle(delimiter: char) -> String {
    let code = delimiter as u32;
    if code >= 0x80 || crate::lexer::ansi::is_assignment_carrier_byte(code) {
        // The needle must be the marker form of the delimiter's bytes AS
        // THEY APPEAR IN THE STREAM: a multibyte delimiter travels as its
        // UTF-8 byte sequence, not as its codepoint truncated to one byte
        // (the old `[code as u8]` form made `read -d é` over a pipe never
        // register the delimiter, so the read reported EOF failure even
        // though the value was split correctly — rubash#1062).
        let mut buf = [0u8; 4];
        substitution_metadata::bytes_to_shell_text(delimiter.encode_utf8(&mut buf).as_bytes())
    } else {
        delimiter.to_string()
    }
}

/// True when `text` holds the delimiter in any transport form: the tagged
/// marker pair (normalized records), the single-byte marker form a
/// byte-locale delimiter takes (the delimiter char then stands for one
/// raw byte), or the literal char (raw mirror texts such as
/// FUNCTION_STDIN, which stores raw pipe bytes). With `escape_aware`
/// (a non-raw read with a non-newline delimiter), an occurrence whose
/// character is backslash-escaped does not count (rubash#1300).
pub(in crate::executor) fn record_contains_delimiter(
    text: &str,
    delimiter: char,
    escape_aware: bool,
) -> bool {
    if escape_aware {
        return unescaped_delimiter_index(text, delimiter).is_some();
    }
    if text.contains(delimiter) || text.contains(&read_delimiter_needle(delimiter)) {
        return true;
    }
    let code = delimiter as u32;
    if (0x80..=0xFF).contains(&code) {
        let single = substitution_metadata::bytes_to_shell_text(&[code as u8]);
        return text.contains(&single);
    }
    false
}

/// EOF accounting for the slurp transports — the serves that hold the
/// whole remaining input as one text (the fd-0 regular-file and procsub
/// serves, here-strings, heredocs, and the `-u N` same-command redirect
/// serve in read_redirected_fd.rs). GNU builtins/read.def consumes one
/// character at a time: its loop breaks on the delimiter (read.def:834),
/// breaks when the -n/-N budget is spent (read.def:880-881: `nr++; if
/// (nchars > 0 && nr >= nchars) break;`), and sets `eof` only when the
/// descriptor itself reports EOF (read.def:767); the builtin then
/// returns `eof ? EXECUTION_FAILURE : EXECUTION_SUCCESS` (read.def:949).
/// Replaying that walk over the slurped text decides whether THIS record
/// ended at the delimiter or at the budget (not EOF) or by exhaustion
/// (EOF, no delimiter). Testing the whole text for the delimiter alone
/// (the old `!record_contains_delimiter` formula) misjudges both ways:
/// a satisfied -n budget over delimiter-less input reported EOF, and a
/// serve that never set the flag reported success at EOF. The unit model
/// is the one read_function_stdin and truncate_read_post_escape use:
/// post-escape characters, a raw-byte marker pair is one unit, and a
/// backslash-newline continuation costs zero.
pub(in crate::executor) fn slurped_record_eof_no_delimiter(
    input: &str,
    delimiter: char,
    char_limit: Option<usize>,
    exact_char_limit: bool,
    escape_aware: bool,
    raw: bool,
) -> bool {
    let needle = read_delimiter_needle(delimiter);
    let chars: Vec<(usize, char)> = input.char_indices().collect();
    let mut cursor = 0usize;
    let mut units = 0usize;
    let mut escaped = false;
    while cursor < chars.len() {
        let (index, ch) = chars[cursor];
        if !exact_char_limit && !escaped && !(escape_aware && ch == '\\') {
            if input[index..].starts_with(&needle) || input[index..].starts_with(delimiter) {
                return false;
            }
        }
        let pair = ch as u32 == RAW_BYTE_MARKER_ESCAPE
            && chars.get(cursor + 1).is_some_and(|&(_, payload)| {
                let code = payload as u32;
                (RAW_BYTE_MARKER_FIRST..=RAW_BYTE_MARKER_LAST).contains(&code)
            });
        if pair {
            // A marker pair is one raw byte: consumed as a whole, it is
            // the escaped character when an escape was pending and never
            // an escape introducer itself (as in read_function_stdin).
            cursor += 2;
            escaped = false;
            units += 1;
        } else {
            cursor += 1;
            if escaped {
                if ch != '\n' {
                    units += 1;
                }
                escaped = false;
            } else if !raw && ch == '\\' {
                escaped = true;
            } else {
                units += 1;
            }
        }
        if char_limit.is_some_and(|limit| units >= limit) {
            return false;
        }
    }
    true
}

/// Byte index of the first UNESCAPED delimiter occurrence in `text`,
/// for non-raw reads (GNU builtins/read.def: a backslash consumes the
/// following character as an escaped literal, and an escaped character
/// can never be the delimiter — rubash#1300). Matches the same forms
/// as record_contains_delimiter: the marker needle, the literal char,
/// and the single-byte marker form of a U+0080..=U+00FF delimiter.
pub(in crate::executor) fn unescaped_delimiter_index(text: &str, delimiter: char) -> Option<usize> {
    let needle = read_delimiter_needle(delimiter);
    let code = delimiter as u32;
    let single = (0x80..=0xFF)
        .contains(&code)
        .then(|| substitution_metadata::bytes_to_shell_text(&[code as u8]));
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        let rest = &text[index..];
        if rest.starts_with(&needle)
            || rest.starts_with(delimiter)
            || single.as_deref().is_some_and(|form| rest.starts_with(form))
        {
            return Some(index);
        }
    }
    None
}

/// Normalize a served `read` record into the shell's storage domain: every
/// ASCII byte the assignment walkers use as an internal carrier
/// (lexer/ansi.rs is_assignment_carrier_byte) travels as the E000 raw-byte
/// marker pair, exactly like `bytes_to_shell_text` (file/coproc inputs) and
/// `push_ansi_c_byte` ($'...' words) already do. Without this, a record
/// served from the byte transports (process stdin, the FUNCTION_STDIN
/// mirror) carries 0x1b/0x0c as literal chars that collide with the lexer's
/// marker system (QUOTED_WORD_PREFIX is 0x1b): declare -p drops them as
/// markers and pattern comparison against $'\x1b' never matches. Idempotent:
/// pair chars are >= 0xE000 and pass through untouched.
pub(in crate::executor) fn tag_read_carrier_text(input: &str) -> String {
    let needs_tagging = input.chars().any(|ch| {
        let code = ch as u32;
        code < 0x80 && crate::lexer::ansi::is_assignment_carrier_byte(code)
    });
    if !needs_tagging {
        return input.to_string();
    }
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        let code = ch as u32;
        if code < 0x80 && crate::lexer::ansi::is_assignment_carrier_byte(code) {
            output.push_str(&substitution_metadata::encode_raw_byte_marker(code as u8));
        } else {
            output.push(ch);
        }
    }
    output
}

/// True when a raw stream byte can terminate a record for `delimiter`.
/// Byte-wise readers stop at the delimiter's first byte: for a delimiter
/// that arrived as a raw byte (read_delimiter_char's first-byte form,
/// U+0000..=U+00FF) that is the byte itself; for a real multibyte
/// character (rubash#1062: -d é is U+00E9) it is the first byte of its
/// UTF-8 encoding — the byte GNU's readchar comparison keys on before
/// the buffered readers hand the record to the character-level trim.
pub(in crate::executor) fn delimiter_byte_matches(byte: u8, delimiter: char) -> bool {
    if byte == delimiter as u8 {
        return true;
    }
    let mut encoded = [0u8; 4];
    byte == delimiter.encode_utf8(&mut encoded).as_bytes()[0]
}

/// True when a decoded stdin unit is the delimiter byte (GNU read.def
/// compares `c == delim` on input bytes). Raw bytes surface as RawByte
/// marker units; a multibyte char is compared by its first byte, matching
/// GNU's byte-level `readchar` semantics.
pub(in crate::executor) fn stdin_unit_is_delimiter(unit: &StdinUnit, delimiter: char) -> bool {
    let byte = match unit {
        StdinUnit::Char(ch) => {
            let mut encoded = [0u8; 4];
            ch.encode_utf8(&mut encoded).as_bytes()[0]
        }
        StdinUnit::RawByte { text } => {
            match substitution_metadata::shell_text_to_raw_bytes(text).first() {
                Some(byte) => *byte,
                None => return false,
            }
        }
    };
    delimiter_byte_matches(byte, delimiter)
}

/// rubash#1250: poll the pending-signal mailbox for an untrapped
/// SIGINT held by the foreground guard (the caller's `poll_sigint_abort`
/// is the guard's armed state). Consumes the SIGINT, re-queues any
/// other signal for its own boundary handling. Mirrors
/// ForegroundBuiltinSigintGuard::take_sigint_abort for this free
/// function's loops.
fn take_untrapped_sigint() -> bool {
    #[cfg(unix)]
    {
        let pending =
            crate::builtins::kill::take_pending_signals_now(std::process::id()).unwrap_or_default();
        if pending.is_empty() {
            return false;
        }
        let mut rest = Vec::new();
        let mut interrupted = false;
        for signal in pending {
            if signal == libc::SIGINT && crate::builtins::kill::kernel_handler_installed(signal) {
                interrupted = true;
            } else {
                rest.push(signal);
            }
        }
        crate::builtins::kill::requeue_pending_signals(rest);
        return interrupted;
    }
    #[cfg(not(unix))]
    false
}

pub(in crate::executor) fn read_stdin_until(
    deadline: Option<std::time::Instant>,
    silent: bool,
    delimiter: char,
    char_limit: Option<usize>,
    exact_char_limit: bool,
    poll_sigint_abort: bool,
    escape_aware: bool,
    raw: bool,
) -> std::io::Result<(usize, String, bool, bool, bool)> {
    if char_limit == Some(0) {
        return Ok((0, String::new(), false, false, false));
    }

    // GNU builtins/read.def reads fd 0 through zread (lib/sh/zread.c) — one
    // raw read syscall per byte, no userspace buffer. std::io::stdin()
    // instead owns a process-wide BufReader whose fill_buf prefetches the
    // whole pipe/file: a child sharing the parent's stdin offset (async
    // `{ read; } &` inside a `done <file` loop — redir.tests) would find the
    // descriptor already at EOF. Read the raw OS handle directly so the
    // shared file offset advances only by what was actually consumed.
    let stdin_handle = crate::fd::process_std_handle(0);
    // read.def:570-590: a -n/-d read from a tty runs in one-char mode and
    // restores the terminal when the builtin exits — a console in cooked
    // line mode would hold every keystroke until Enter, breaking both the
    // -n record shape and the -t deadline. The mode gate:
    // read.def:570-590 switches the tty to one-char mode only for -n/-N
    // counts or a non-newline delimiter; a plain newline-terminated read
    // stays in canonical mode, and -s alone only silences the echo
    // (read.def:603-621 ttfd_noecho).
    //
    // rubash#806: a deadline on a CONSOLE also takes the one-char path.
    // The cooked line discipline and the raw input queue are two different
    // buffers: the first cooked ReadFile(1) pulls the WHOLE typed line into
    // conhost's pending read state (returning only its first byte), so the
    // raw-queue readiness gate then sees an empty queue and burns the whole
    // deadline (`read -t 3 a` + typed `hello` + Enter returned 142 with
    // a=`h`, and the orphaned `ello\r` leaked into the NEXT read). One-char
    // mode keeps every keystroke a raw record the gate can see, so the
    // deadline cuts where GNU's select() would and the line stays atomic.
    // GNU never buffers ahead here either: read.def:736 reads a -n/-d tty
    // byte-wise through zread, and a deadline read gates per byte
    // (read.def:659 check_read_timeout at the top of every iteration).
    let onechar = if char_limit.is_some()
        || delimiter != '\n'
        || (deadline.is_some() && crate::fd::is_console_handle(stdin_handle))
    {
        crate::fd::enter_console_onechar_mode(stdin_handle)
    } else if silent {
        crate::fd::enter_console_noecho_mode(stdin_handle)
    } else {
        None
    };
    // Echo ownership follows the mode guard: on a console the onechar mode
    // silenced the driver's echo, so a non-silent read re-emits each
    // consumed unit itself (read.def ttfd_onechar keeps ECHO on).
    let manual_echo = onechar.is_some() && !silent;
    let echo_unit = |unit: &StdinUnit| {
        if let Some(guard) = onechar.as_ref() {
            match unit {
                StdinUnit::Char(ch) => {
                    let mut buf = [0u8; 4];
                    for byte in ch.encode_utf8(&mut buf).as_bytes() {
                        guard.echo_byte(*byte);
                    }
                }
                StdinUnit::RawByte { text } => {
                    for byte in substitution_metadata::shell_text_to_raw_bytes(text) {
                        guard.echo_byte(byte);
                    }
                }
            }
        }
    };
    let mut output = String::new();
    let mut read = 0;
    let mut decoder = StdinCharDecoder::new();
    let mut units = 0usize;
    // GNU read.def escape state for the -n/-N budget (rubash#1328).
    let mut escaped = false;
    let mut eof = false;
    let mut timed_out = false;
    let mut interrupted = false;
    // rubash#1300: in a non-raw read an escaped unit is literal data,
    // never the delimiter (read.def zread escape consumption).
    let mut escaped = false;
    loop {
        if !decoder.has_queued() {
            // read.def check_read_timeout (read.def:659) runs at the top of
            // every read-loop iteration: wait out the remaining deadline for
            // the handle to become readable before the blocking read.
            // rubash#1250: the wait runs in slices polling for an
            // untrapped SIGINT, and with no deadline an armed guard
            // gates the blocking read on sliced readability (a canonical
            // tty reports readable only with a complete line buffered,
            // so the record shape is unchanged).
            if let Some(deadline) = deadline {
                let mut ready = false;
                loop {
                    let now = std::time::Instant::now();
                    if now >= deadline {
                        timed_out = true;
                        break;
                    }
                    if poll_sigint_abort && take_untrapped_sigint() {
                        interrupted = true;
                        break;
                    }
                    let slice = (deadline - now).min(std::time::Duration::from_millis(50));
                    if crate::fd::wait_readable(stdin_handle, slice) != crate::fd::ReadWait::Timeout
                    {
                        ready = true;
                        break;
                    }
                }
                if timed_out || interrupted {
                    break;
                }
                let _ = ready;
            } else if poll_sigint_abort && crate::fd::is_console_handle(stdin_handle) {
                loop {
                    if take_untrapped_sigint() {
                        interrupted = true;
                        break;
                    }
                    if crate::fd::wait_readable(stdin_handle, std::time::Duration::from_millis(50))
                        != crate::fd::ReadWait::Timeout
                    {
                        break;
                    }
                }
                if interrupted {
                    break;
                }
            }
            match crate::fd::read_some(stdin_handle, 1)? {
                buf if buf.is_empty() => {
                    eof = true;
                    break;
                }
                buf => {
                    read += buf.len();
                    decoder.queue_byte(buf[0]);
                }
            }
        }
        let Some(mut unit) = decoder.next_unit() else {
            continue;
        };
        // shtty.c:115 tt_setonechar: `c_iflag |= ICRNL` — one-char tty reads
        // get CR->NL on input (read.def:570-590 installs this for every -n,
        // -N and -d tty read; the deadline console path above joins it).
        // ConPTY delivers VK_RETURN as a CR record in every console mode, so
        // the translation is what keeps `read -n1` + Enter at the delimiter
        // (zero chars) and a `read -t` line completing on the typed CR
        // instead of hanging for an LF that never comes. Pipe/file reads
        // keep raw CRs HERE (GNU reads them untranslated: input.c never
        // touches a non-tty) — but only at this translation gate: on
        // Windows `trim_read_input_literal`'s wave-2 pop still strips a
        // record's trailing CR afterwards, so a CRLF pipe is NOT byte-GNU
        // end to end (`printf 'x\r\n' | read x; ${#x}` = 5 vs GNU 6) — see
        // the trim layer's NOTE below; pre-existing, cr-816 NIT-2.
        if onechar.is_some() && delimiter == '\n' {
            if matches!(unit, StdinUnit::Char('\r')) {
                unit = StdinUnit::Char('\n');
            }
        }
        if manual_echo {
            echo_unit(&unit);
        }
        // rubash#1300: the escape introducer itself is consumed as an
        // escape, never as the delimiter — with `-d '\\'` a non-raw read
        // can therefore never match (GNU read.def/zread consumes the
        // backslash first), it reads to EOF and fails. rubash#1328: an
        // escaped unit is likewise data, never the delimiter — the check
        // uses the un-gated escape state, so a `\\<newline>` continuation
        // does not end a newline-delimited record either.
        let is_introducer = escape_aware && matches!(unit, StdinUnit::Char('\\'));
        if !exact_char_limit
            && !escaped
            && !is_introducer
            && stdin_unit_is_delimiter(&unit, delimiter)
        {
            break;
        }
        let unit_char = match &unit {
            StdinUnit::Char(ch) => Some(*ch),
            StdinUnit::RawByte { .. } => None,
        };
        match unit {
            StdinUnit::Char(ch) => {
                output.push(ch);
            }
            StdinUnit::RawByte { text } => {
                output.push_str(&text);
            }
        }
        // Post-escape counting, as in read_inherited_process_stdin
        // (rubash#1328).
        if escaped {
            if unit_char != Some('\n') {
                units += 1;
            }
            escaped = false;
        } else if !raw && unit_char == Some('\\') {
            escaped = true;
        } else {
            units += 1;
        }
        if char_limit.is_some_and(|limit| units >= limit) {
            break;
        }
    }
    if eof {
        decoder.flush(&mut output);
    }
    // Third element: read.def:949 eof — the handle closed before the
    // delimiter (a char-limit break or timeout is not EOF). Fourth element:
    // read.def:553 — the deadline expired; the caller maps it to
    // 128+SIGALRM while still assigning the partial record. Fifth element
    // (rubash#1250): an untrapped SIGINT aborted the read; the caller
    // assigns NOTHING and returns 128+SIGINT.
    Ok((
        read,
        trim_read_input(
            output,
            delimiter,
            char_limit,
            exact_char_limit,
            escape_aware,
            raw,
        ),
        eof,
        timed_out,
        interrupted,
    ))
}

/// First UNESCAPED occurrence of the read delimiter: the needle form
/// wins over the literal char (the priority the split_once chain gave
/// it), and a position is unescaped when it is not quoted by an odd run
/// of backslashes — a `\` quotes the next character, `\\` is itself
/// quoted. Returns the record text before the delimiter (rubash#1328).
fn find_unescaped_read_delimiter<'a>(
    input: &'a str,
    needle: &str,
    delimiter: char,
) -> Option<&'a str> {
    let scan = |needle_mode: bool| -> Option<usize> {
        let mut escaped = false;
        for (index, ch) in input.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if needle_mode {
                if input[index..].starts_with(needle) {
                    return Some(index);
                }
            } else if ch == delimiter {
                return Some(index);
            }
        }
        None
    };
    scan(true)
        .or_else(|| scan(false))
        .map(|index| &input[..index])
}

pub(in crate::executor) fn trim_read_input(
    mut input: String,
    delimiter: char,
    char_limit: Option<usize>,
    exact_char_limit: bool,
    escape_aware: bool,
    raw: bool,
) -> String {
    // Trimming and limit math below run on the literal-char form (the
    // delimiter needle handles both transports); the storage-domain tagging
    // is the last step so every served record leaves this fn normalized.
    let trimmed = trim_read_input_literal(
        input,
        delimiter,
        char_limit,
        exact_char_limit,
        escape_aware,
        raw,
    );
    tag_read_carrier_text(&trimmed)
}

fn trim_read_input_literal(
    mut input: String,
    delimiter: char,
    char_limit: Option<usize>,
    exact_char_limit: bool,
    escape_aware: bool,
    raw: bool,
) -> String {
    if !exact_char_limit {
        // GNU builtins/read.def read_builtin: only the delimiter itself is
        // removed from the line — WSL 5.3.0 baseline: `printf 'a\r\n' | read x`
        // yields "a\r" and `printf 'a\r' | read x` yields "a\r" too. The '\r'
        // stripping is the Windows CRT-text-mode compensation (native tools
        // emit CRLF); on unix it would eat legal filename bytes.
        // The needle covers the raw-byte transport form; a multibyte
        // delimiter character (rubash#1062) travels as the literal char,
        // so fall back to it when the needle form is absent. In a
        // non-raw read the split point is the first UNESCAPED delimiter:
        // an escaped one is record data — for a custom delimiter
        // (rubash#1300) and for the newline continuation alike
        // (rubash#1328: the naive split cut `a\\<LF>zz<LF>` to `a\\`).
        let needle = read_delimiter_needle(delimiter);
        let split_at = if raw {
            input
                .split_once(&needle)
                .or_else(|| input.split_once(delimiter))
                .map(|(before, _)| before.len())
        } else {
            find_unescaped_read_delimiter(&input, &needle, delimiter).map(|before| before.len())
        };
        if let Some(at) = split_at {
            input = if cfg!(windows) {
                input[..at].trim_end_matches('\r').to_string()
            } else {
                input[..at].to_string()
            };
        } else if delimiter == '\n' {
            // NOTE (Windows): this also drops a lone trailing '\r' that is
            // real data. The permissive pop is load-bearing there: the
            // pty/pipe readers stop at the delimiter and hand the line over
            // without its '\n', so a CRLF terminator arrives here as a bare
            // trailing '\r' and can only be recognised by this pop.
            // Tightening it needs those readers to report whether the
            // delimiter they consumed was preceded by a '\r'; until then the
            // CRLF case wins over the lone-CR case (wave-2). Unix strips the
            // '\n' delimiters only, per the GNU baseline above.
            while input.ends_with('\n') || (cfg!(windows) && input.ends_with('\r')) {
                input.pop();
            }
        }
    }

    if let Some(limit) = char_limit {
        if !raw {
            // GNU builtins/read.def counts the -n/-N budget in
            // POST-ESCAPE characters: the read loop processes `\x` inline
            // and increments nr per character it APPENDS, so an escape
            // pair costs one unit and a backslash-newline continuation
            // costs zero (rubash#1328). Truncating the raw record by raw
            // units both stops early and cuts pairs in half.
            return truncate_read_post_escape(&input, limit);
        }
        if crate::locale::is_multi_byte() {
            return truncate_read_units(&input, limit);
        }
        return truncate_read_input_bytes(input, limit);
    }

    input
}

/// Truncate a record to `limit` POST-ESCAPE units (see the call site):
/// a `\x` pair is one unit and stays whole, a `\<newline>` continuation
/// is zero units, a dangling trailing `\` counts as one unit, and a
/// raw-byte marker pair is one unit as in truncate_read_units
/// (rubash#1328, rubash#714).
fn truncate_read_post_escape(input: &str, limit: usize) -> String {
    let mut units = 0usize;
    let mut chars = input.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if units == limit {
            return input[..index].to_string();
        }
        if ch == '\\' {
            match chars.next() {
                // Line continuation: appends nothing, costs nothing.
                Some((_, '\n')) => {}
                // Escaped character (or marker payload byte): the pair
                // is one stored character.
                Some(_) => units += 1,
                // Dangling backslash at end of record: stored as-is.
                None => units += 1,
            }
            continue;
        }
        if ch as u32 == RAW_BYTE_MARKER_ESCAPE {
            if let Some(&(_, payload)) = chars.peek() {
                let code = payload as u32;
                if (RAW_BYTE_MARKER_FIRST..=RAW_BYTE_MARKER_LAST).contains(&code) {
                    chars.next();
                }
            }
        }
        units += 1;
    }
    input.to_string()
}

/// Truncate transport text to `limit` decoded units. One raw-byte marker
/// pair (E000 + E001..=E100) is a single input byte and must consume one
/// unit and stay whole, so a `-n`/`-N` cut can never land inside the pair
/// (rubash#714). GNU builtins/read.def counts one per decoded character —
/// read.def:874 `read_mbchar` consumes a multibyte character whole,
/// read.def:884 `nr++`, read.def:886 `if (nchars > 0 && nr >= nchars)
/// break;` — so in a multibyte locale the tagged pair is one character.
/// This matches `read_split::logical_units`, the buffered splitter's walk.
fn truncate_read_units(input: &str, limit: usize) -> String {
    let mut units = 0usize;
    let mut chars = input.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if units == limit {
            return input[..index].to_string();
        }
        if ch as u32 == RAW_BYTE_MARKER_ESCAPE {
            if let Some(&(_, payload)) = chars.peek() {
                let code = payload as u32;
                if (RAW_BYTE_MARKER_FIRST..=RAW_BYTE_MARKER_LAST).contains(&code) {
                    chars.next();
                }
            }
        }
        units += 1;
    }
    input.to_string()
}

/// `read -n N` caps the read at N *bytes* in a single-byte locale (GNU
/// builtins/read.def + lib/sh/input.c: maxchars is a byte count while
/// MB_CUR_MAX is 1), and unlike the character walk it does not back off to a
/// character boundary, so the cut can leave a dangling multibyte lead byte
/// behind. Keep that byte as a raw byte instead of dropping it, which is what
/// makes intl1.sub print `-абв-(5)` rather than `-абвгд-(5)`.
fn truncate_read_input_bytes(input: String, limit: usize) -> String {
    let sentinel = char::from_u32(crate::executor::substitution_metadata::RAW_BYTE_MARKER_ESCAPE)
        .expect("raw-byte sentinel is a valid char");
    let raw = if input.contains(sentinel) {
        crate::executor::substitution_metadata::decode_raw_byte_markers(input.as_bytes())
    } else {
        input.as_bytes().to_vec()
    };
    if raw.len() <= limit {
        return input;
    }
    crate::executor::substitution_metadata::bytes_to_shell_text(&raw[..limit])
}

pub(in crate::executor) fn unescape_read_backslashes(input: &str) -> String {
    let mut output = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }

        match chars.next() {
            Some('\n') => {}
            Some('\r') if chars.peek() == Some(&'\n') => {
                chars.next();
            }
            Some(next) => output.push(next),
            None => {}
        }
    }
    output
}

pub(in crate::executor) fn split_read_array_words(line: &str, ifs: Option<&str>) -> Vec<String> {
    match ifs {
        Some("/") => line.split('/').map(str::to_string).collect(),
        // GNU read.def assigns -a words through the same field-splitting
        // rules as scalar names: interior empty fields bounded by
        // non-whitespace IFS delimiters are kept (`IFS=: read -a A` on
        // `:::` yields three empty elements). A set-but-empty IFS is not
        // the default: read.def:969 hands the line to list_string
        // (subst.c:3119) with the ambient IFS, and an empty IFS disables
        // splitting entirely — the whole line is one field, which
        // split_read_field_ranges already encodes for "" (rubash#1713).
        // Only an UNSET IFS takes the default whitespace split.
        Some(ifs) => split_read_field_ranges(line, ifs, false)
            .into_iter()
            .map(|(start, end)| line[start..end].to_string())
            .collect(),
        None => line.split_whitespace().map(str::to_string).collect(),
    }
}

pub(in crate::executor) fn split_read_array_words_with_backslashes(
    line: &str,
    ifs: Option<&str>,
) -> Vec<String> {
    match ifs {
        Some("/") => split_escaped_words(line, '/'),
        // GNU read.def:972-979: the -a path runs list_string over the
        // CTLESC-protected line and dequote_list afterwards — escaped IFS
        // characters stay inside the element and the backslash is dropped.
        // The raw slice here kept the backslash (rubash#397); the scalar
        // field_value helper performs the same escape-aware split + unescape.
        // A set-but-empty IFS disables splitting (one field, escapes still
        // processed) exactly as in the raw path above (rubash#1713); only
        // an unset IFS falls back to whitespace splitting.
        Some(ifs) => crate::executor::read_split::read_array_field_values(line, ifs),
        None => split_escaped_words_on_whitespace(line),
    }
}

pub(in crate::executor) fn split_escaped_words_on_whitespace(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('\n') => {}
                Some('\r') if chars.peek() == Some(&'\n') => {
                    chars.next();
                }
                Some(next) => current.push(next),
                None => {}
            }
            continue;
        }

        if ch.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }

        current.push(ch);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

pub(in crate::executor) fn split_escaped_words_on_set(line: &str, separators: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('\n') => {}
                Some('\r') if chars.peek() == Some(&'\n') => {
                    chars.next();
                }
                Some(next) => current.push(next),
                None => {}
            }
            continue;
        }

        if separators.contains(ch) {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }

        current.push(ch);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

pub(in crate::executor) fn split_escaped_words(line: &str, separator: char) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('\n') => {}
                Some('\r') if chars.peek() == Some(&'\n') => {
                    chars.next();
                }
                Some(next) => current.push(next),
                None => {}
            }
            continue;
        }

        if ch == separator {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }

        current.push(ch);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

impl Executor {
    pub(in crate::executor) fn assign_read_scalar_names(
        &mut self,
        names: &[String],
        line: &str,
        raw: bool,
    ) -> i32 {
        self.assign_read_scalar_names_with_field_count(names, line, raw, names.len())
    }

    /// rubash#776: GNU read.def under `-N` binds the record to the FIRST
    /// name verbatim — no IFS splitting and no leading/trailing whitespace
    /// strip — and leaves the remaining names empty. Probes against GNU
    /// 5.3.0 with `printf 'AB\nC'`:
    ///
    ///   read -N3 a   -> a=`AB\n`     (the delimiter is data here)
    ///   read -N3 a b -> a=`AB\n`, b=""
    ///   read -N5 a b on `AB CD` -> a=`AB CD`, b=""   (no splitting)
    ///   read -N6 a on `  AB  `  -> a=`  AB  `        (no strip)
    ///   read -n3 a   -> a=`AB`                       (-n splits/strips)
    ///
    /// Backslash handling still applies without -r (`A\\B` -> `A\B`).
    pub(in crate::executor) fn assign_read_exact_names(
        &mut self,
        names: &[String],
        line: &str,
        raw: bool,
    ) -> i32 {
        let value = if raw {
            line.to_string()
        } else {
            unescape_read_backslashes(line)
        };
        let mut status = 0i32;
        for (index, name) in names.iter().enumerate() {
            let value = if index == 0 {
                value.clone()
            } else {
                String::new()
            };
            if !self.apply_shell_assignment_command("read", name, value) {
                status = if index + 1 < names.len() { 2 } else { 1 };
                break;
            }
        }
        status
    }

    pub(in crate::executor) fn assign_read_scalar_names_with_field_count(
        &mut self,
        names: &[String],
        line: &str,
        raw: bool,
        field_count: usize,
    ) -> i32 {
        if names.len() == 1 && field_count == 0 {
            let value = if raw {
                line.to_string()
            } else {
                unescape_read_backslashes(line)
            };
            // GNU read.def bind_read_variable: success -> EXECUTION_SUCCESS;
            // failure of the last (only) name -> EXECUTION_FAILURE.
            return if self.apply_shell_assignment_command("read", &names[0], value) {
                0
            } else {
                1
            };
        }

        let ifs = self
            .shell_state
            .env_vars
            .get("IFS")
            .map(String::as_str)
            .unwrap_or(" \t\n");
        let fields = if raw {
            read_scalar_fields(line, field_count, ifs)
        } else {
            read_scalar_fields_with_backslashes(line, field_count, ifs)
        };
        // GNU read.def:1075-1081 + 1139-1140: bind_read_variable returns NULL
        // for readonly/disallowed vars.  A middle variable failure returns
        // EX_MISCERROR (2); the last variable failure returns
        // EXECUTION_FAILURE (1).  Stop the loop on first failure (GNU does
        // not bind subsequent names after a readonly failure).
        let mut status = 0i32;
        for (index, name) in names.iter().enumerate() {
            let value = fields.get(index).cloned().unwrap_or_default();
            if !self.apply_shell_assignment_command("read", name, value) {
                status = if index + 1 < names.len() { 2 } else { 1 };
                break;
            }
        }
        status
    }
}
