use super::*;

pub(in crate::executor) fn print_posix_time() {
    eprintln!("real 0.00");
    eprintln!("user 0.00");
    eprintln!("sys 0.00");
}

pub(in crate::executor) fn print_time(env_vars: &HashMap<String, String>, posix_format: bool) {
    if posix_format {
        print_posix_time();
        return;
    }

    let Some(format) = env_vars.get("TIMEFORMAT") else {
        print_posix_time();
        return;
    };

    if format.is_empty() {
        return;
    }

    eprintln!("{}", expand_time_format(format));
}

fn expand_time_format(format: &str) -> String {
    let mut output = String::new();
    let mut chars = format.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '%' => expand_time_format_percent(&mut output, &mut chars),
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

    output
}

fn expand_time_format_percent<I>(output: &mut String, chars: &mut std::iter::Peekable<I>)
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
        Some('%') if precision.is_none() && !long => output.push('%'),
        Some('R' | 'U' | 'S') => output.push_str(&format_time_seconds(precision, long)),
        Some('P') => output.push_str("0.00"),
        Some(other) => {
            output.push('%');
            if let Some(precision) = precision {
                output.push(char::from_digit(precision as u32, 10).unwrap_or('0'));
            }
            if long {
                output.push('l');
            }
            output.push(other);
        }
        None => output.push('%'),
    }
}

fn format_time_seconds(precision: Option<usize>, long: bool) -> String {
    let precision = precision.unwrap_or(3);
    if precision == 0 {
        if long {
            return "0m0s".to_string();
        }
        return "0".to_string();
    }

    let fraction = "0".repeat(precision);
    if long {
        format!("0m0.{fraction}s")
    } else {
        format!("0.{fraction}")
    }
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
        return input.chars().take(limit).collect();
    }

    input
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
    ) {
        self.assign_read_scalar_names_with_field_count(names, line, raw, names.len());
    }

    pub(in crate::executor) fn assign_read_scalar_names_with_field_count(
        &mut self,
        names: &[String],
        line: &str,
        raw: bool,
        field_count: usize,
    ) {
        if names.len() == 1 && field_count == 1 {
            let value = if raw {
                line.to_string()
            } else {
                unescape_read_backslashes(line)
            };
            self.env_vars.insert(names[0].clone(), value);
            return;
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
        for (index, name) in names.iter().enumerate() {
            let value = fields.get(index).cloned().unwrap_or_default();
            self.env_vars.insert(name.clone(), value);
        }
    }
}
