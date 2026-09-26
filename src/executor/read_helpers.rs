use super::*;

#[derive(Clone, Copy, Debug)]
pub(in crate::executor) struct TimeCommandMetrics {
    real: std::time::Duration,
    user: std::time::Duration,
    sys: std::time::Duration,
}

impl TimeCommandMetrics {
    fn from_start(started: std::time::Instant) -> Self {
        Self {
            real: started.elapsed(),
            user: std::time::Duration::ZERO,
            sys: std::time::Duration::ZERO,
        }
    }
}

pub(in crate::executor) fn time_command_started() -> std::time::Instant {
    std::time::Instant::now()
}

pub(in crate::executor) fn print_posix_time(metrics: TimeCommandMetrics) {
    eprintln!("real {}", format_time_seconds(metrics.real, Some(2), false));
    eprintln!("user {}", format_time_seconds(metrics.user, Some(2), false));
    eprintln!("sys {}", format_time_seconds(metrics.sys, Some(2), false));
}

pub(in crate::executor) fn print_time(
    env_vars: &HashMap<String, String>,
    posix_format: bool,
    started: std::time::Instant,
) {
    let metrics = TimeCommandMetrics::from_start(started);
    if posix_format {
        print_posix_time(metrics);
        return;
    }

    let Some(format) = env_vars.get("TIMEFORMAT") else {
        print_posix_time(metrics);
        return;
    };

    if format.is_empty() {
        return;
    }

    match expand_time_format(format, metrics) {
        Ok(output) => eprintln!("{output}"),
        Err(invalid) => eprintln!("rubash: TIMEFORMAT: `{invalid}': invalid format character"),
    }
}

fn expand_time_format(format: &str, metrics: TimeCommandMetrics) -> Result<String, char> {
    let mut output = String::new();
    let mut chars = format.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '%' => expand_time_format_percent(&mut output, &mut chars, metrics)?,
            '\\' => match chars.next() {
                Some('n') => output.push('\n'),
                Some('t') => output.push('\t'),
                Some('\\') => output.push('\\'),
                Some(other) => {
                    output.push('\\');
                    output.push(other);
                }
                None => output.push('\\'),
            },
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
    substitution_metadata::shell_text_to_raw_bytes(word)
        .first()
        .copied()
        .map(char::from)
        .unwrap_or('\0')
}

/// The delimiter needle inside buffered shell text: raw bytes live in text
/// as marker pairs, so a >=0x80 delimiter matches its encoded pair.
pub(in crate::executor) fn read_delimiter_needle(delimiter: char) -> String {
    if delimiter as u32 >= 0x80 {
        substitution_metadata::bytes_to_shell_text(&[delimiter as u8])
    } else {
        delimiter.to_string()
    }
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
    byte == delimiter as u8
}

pub(in crate::executor) fn read_stdin_until(
    delimiter: char,
    char_limit: Option<usize>,
    exact_char_limit: bool,
) -> std::io::Result<(usize, String)> {
    if char_limit == Some(0) {
        return Ok((0, String::new()));
    }

    // GNU builtins/read.def reads fd 0 through zread (lib/sh/zread.c) — one
    // raw read syscall per byte, no userspace buffer. std::io::stdin()
    // instead owns a process-wide BufReader whose fill_buf prefetches the
    // whole pipe/file: a child sharing the parent's stdin offset (async
    // `{ read; } &` inside a `done <file` loop — redir.tests) would find the
    // descriptor already at EOF. Read the raw OS handle directly so the
    // shared file offset advances only by what was actually consumed.
    let stdin_handle = crate::fd::process_std_handle(0);
    let mut output = String::new();
    let mut read = 0;
    let mut decoder = StdinCharDecoder::new();
    let mut units = 0usize;
    let mut eof = false;
    loop {
        if !decoder.has_queued() {
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
        let Some(unit) = decoder.next_unit() else {
            continue;
        };
        let is_cr = matches!(unit, StdinUnit::Char('\r'));
        match unit {
            unit => {
                if !exact_char_limit && stdin_unit_is_delimiter(&unit, delimiter) {
                    break;
                }
                match unit {
                    StdinUnit::Char(ch) => {
                        output.push(ch);
                        units += 1;
                    }
                    StdinUnit::RawByte { text } => {
                        output.push_str(&text);
                        units += 1;
                    }
                }
            }
        }
        if char_limit.is_some_and(|limit| units >= limit) {
            break;
        }
        if delimiter == '\n' && is_cr {
            continue;
        }
    }
    if eof {
        decoder.flush(&mut output);
    }
    Ok((
        read,
        trim_read_input(output, delimiter, char_limit, exact_char_limit),
    ))
}

pub(in crate::executor) fn trim_read_input(
    mut input: String,
    delimiter: char,
    char_limit: Option<usize>,
    exact_char_limit: bool,
) -> String {
    if !exact_char_limit {
        // GNU builtins/read.def read_builtin: only the delimiter itself is
        // removed from the line — WSL 5.3.0 baseline: `printf 'a\r\n' | read x`
        // yields "a\r" and `printf 'a\r' | read x` yields "a\r" too. The '\r'
        // stripping is the Windows CRT-text-mode compensation (native tools
        // emit CRLF); on unix it would eat legal filename bytes.
        if let Some((before, _)) = input.split_once(&read_delimiter_needle(delimiter)) {
            input = if cfg!(windows) {
                before.trim_end_matches('\r').to_string()
            } else {
                before.to_string()
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
        if crate::locale::is_multi_byte() {
            return input.chars().take(limit).collect();
        }
        return truncate_read_input_bytes(input, limit);
    }

    input
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
        // `:::` yields three empty elements).
        Some(ifs) if !ifs.is_empty() => split_read_field_ranges(line, ifs, false)
            .into_iter()
            .map(|(start, end)| line[start..end].to_string())
            .collect(),
        _ => line.split_whitespace().map(str::to_string).collect(),
    }
}

pub(in crate::executor) fn split_read_array_words_with_backslashes(
    line: &str,
    ifs: Option<&str>,
) -> Vec<String> {
    match ifs {
        Some("/") => split_escaped_words(line, '/'),
        Some(ifs) if !ifs.is_empty() => split_read_field_ranges(line, ifs, true)
            .into_iter()
            .map(|(start, end)| line[start..end].to_string())
            .collect(),
        _ => split_escaped_words_on_whitespace(line),
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
