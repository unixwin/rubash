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

pub(in crate::executor) fn read_stdin_until(
    delimiter: char,
    char_limit: Option<usize>,
    exact_char_limit: bool,
) -> std::io::Result<(usize, String)> {
    if char_limit == Some(0) {
        return Ok((0, String::new()));
    }

    // TODO(builtins/read.def/input.c): Avoid buffered prefetching so callers
    // that read commands from stdin can let child scripts consume the next
    // physical line, as Bash does for tests/input-line.sh.
    let mut stdin = std::io::stdin().lock();
    let mut bytes = [0_u8; 1];
    let mut output = String::new();
    let mut read = 0;
    loop {
        match stdin.read(&mut bytes)? {
            0 => break,
            count => {
                read += count;
                let ch = bytes[0] as char;
                if !exact_char_limit && ch == delimiter {
                    break;
                }
                output.push(ch);
                if char_limit.is_some_and(|limit| output.chars().count() >= limit) {
                    break;
                }
                if delimiter == '\n' && ch == '\r' {
                    continue;
                }
            }
        }
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
        if let Some((before, _)) = input.split_once(delimiter) {
            input = before.trim_end_matches('\r').to_string();
        } else if delimiter == '\n' {
            while input.ends_with('\n') || input.ends_with('\r') {
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
        Some(ifs) if !ifs.is_empty() => line
            .split(|ch| ifs.contains(ch))
            .filter(|word| !word.is_empty())
            .map(str::to_string)
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
        Some(ifs) if !ifs.is_empty() => split_escaped_words_on_set(line, ifs),
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
    ) -> bool {
        self.assign_read_scalar_names_with_field_count(names, line, raw, names.len())
    }

    pub(in crate::executor) fn assign_read_scalar_names_with_field_count(
        &mut self,
        names: &[String],
        line: &str,
        raw: bool,
        field_count: usize,
    ) -> bool {
        if names.len() == 1 && field_count == 0 {
            let value = if raw {
                line.to_string()
            } else {
                unescape_read_backslashes(line)
            };
            return self.apply_shell_assignment(&names[0], value);
        }

        let ifs = self
            .env_vars
            .get("IFS")
            .map(String::as_str)
            .unwrap_or(" \t\n");
        let fields = if raw {
            read_scalar_fields(line, field_count, ifs)
        } else {
            read_scalar_fields_with_backslashes(line, field_count, ifs)
        };
        // GNU read binds every name (bind_read_variable), clearing surplus
        // names to the empty string even when the line has fewer fields;
        // apply_shell_assignment keeps nameref/readonly/array semantics and
        // the same variable store as regular assignments.
        // GNU variables.c: bind_read_variable stops on readonly failure and
        // does not bind subsequent names (read.tests: `readonly b; read a b c`
        // leaves `c` unset, stat >1).  Propagate failure and stop the loop.
        let mut ok = true;
        for (index, name) in names.iter().enumerate() {
            let value = fields.get(index).cloned().unwrap_or_default();
            if !self.apply_shell_assignment(name, value) {
                ok = false;
                break;
            }
        }
        ok
    }
}
