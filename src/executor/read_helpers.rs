use super::*;

pub(in crate::executor) fn print_posix_time() {
    eprintln!("real 0.00");
    eprintln!("user 0.00");
    eprintln!("sys 0.00");
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
