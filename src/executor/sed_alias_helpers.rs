pub(in crate::executor) fn sed_script_arg(args: &[String]) -> Option<&str> {
    match args {
        [option, script, ..] if option == "-e" => Some(script.as_str()),
        [script, ..] => Some(script.as_str()),
        _ => None,
    }
}

pub(in crate::executor) fn apply_simple_sed_substitution(
    input: &str,
    script: &str,
) -> Option<String> {
    let substitutions = parse_sed_substitutions(script)?;
    let mut output = input
        .lines()
        .map(|line| {
            substitutions
                .iter()
                .fold(line.to_string(), |line, (pattern, replacement)| {
                    apply_simple_sed_line(&line, pattern, replacement)
                })
        })
        .collect::<Vec<_>>()
        .join("\n");
    if input.ends_with('\n') {
        output.push('\n');
    }
    Some(output)
}

pub(in crate::executor) fn parse_sed_substitutions(script: &str) -> Option<Vec<(&str, &str)>> {
    let substitutions = script
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                None
            } else {
                parse_sed_substitution(line)
            }
        })
        .collect::<Vec<_>>();
    if substitutions.is_empty() {
        parse_sed_substitution(script).map(|substitution| vec![substitution])
    } else {
        Some(substitutions)
    }
}

pub(in crate::executor) fn parse_sed_substitution(script: &str) -> Option<(&str, &str)> {
    let rest = script.strip_prefix('s')?;
    let separator = rest.chars().next()?;
    let rest = &rest[separator.len_utf8()..];
    if separator == '#' && rest.starts_with("\\#") {
        let (replacement, _) = split_escaped_separator(&rest[2..], separator)?;
        return Some((&rest[..1], replacement));
    }
    let (pattern, rest) = split_escaped_separator(rest, separator)?;
    let (replacement, _) = split_escaped_separator(rest, separator)?;
    Some((pattern, replacement))
}

pub(in crate::executor) fn split_escaped_separator(
    value: &str,
    separator: char,
) -> Option<(&str, &str)> {
    let mut escaped = false;
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == separator {
            return Some((&value[..index], &value[index + ch.len_utf8()..]));
        }
    }
    None
}

pub(in crate::executor) fn apply_simple_sed_line(
    line: &str,
    pattern: &str,
    replacement: &str,
) -> String {
    match pattern {
        "\\" | r"\\" => line.replace('\\', &unescape_sed_replacement(replacement)),
        r"\!\*" => line.replace("!*", &unescape_sed_replacement(replacement)),
        r"\!:\([1-9]\)" => replace_aliasconv_positional_markers(line),
        "#" => line.replace('#', &unescape_sed_replacement(replacement)),
        r"\..*$" => line
            .split_once('.')
            .map(|(prefix, _)| format!("{prefix}{replacement}"))
            .unwrap_or_else(|| line.to_string()),
        r"^.*\." => line
            .rsplit_once('.')
            .map(|(_, suffix)| format!("{replacement}{suffix}"))
            .unwrap_or_else(|| line.to_string()),
        ".*/" => line
            .rsplit_once('/')
            .map(|(_, basename)| format!("{replacement}{basename}"))
            .unwrap_or_else(|| line.to_string()),
        _ => line.to_string(),
    }
}

pub(in crate::executor) fn unescape_sed_replacement(replacement: &str) -> String {
    let mut output = String::new();
    let mut chars = replacement.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                output.push(next);
            } else {
                output.push(ch);
            }
        } else {
            output.push(ch);
        }
    }
    output
}

pub(in crate::executor) fn replace_aliasconv_positional_markers(line: &str) -> String {
    let mut output = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '!' && chars.peek().copied() == Some(':') {
            chars.next();
            if let Some(digit @ '1'..='9') = chars.peek().copied() {
                chars.next();
                output.push('"');
                output.push('$');
                output.push(digit);
                output.push('"');
                continue;
            }
            output.push('!');
            output.push(':');
            continue;
        }
        output.push(ch);
    }
    output
}

pub(in crate::executor) fn split_shell_words(source: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for ch in source.chars() {
        match (ch, quote) {
            ('\'' | '"', None) => quote = Some(ch),
            (q, Some(active)) if q == active => quote = None,
            (' ' | '\t', None) => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

pub(in crate::executor) fn split_first_shell_word(source: &str) -> Option<(String, &str)> {
    let trimmed = source.trim_start();
    let offset = source.len() - trimmed.len();
    let mut quote = None;
    for (index, ch) in trimmed.char_indices() {
        match (ch, quote) {
            ('\'' | '"', None) => quote = Some(ch),
            (q, Some(active)) if q == active => quote = None,
            (' ' | '\t' | '\n' | '\r', None) => {
                let word = trimmed[..index].to_string();
                let remainder = &source[offset + index + ch.len_utf8()..];
                return Some((word, remainder));
            }
            _ => {}
        }
    }

    if trimmed.is_empty() {
        None
    } else {
        Some((trimmed.to_string(), ""))
    }
}

pub(in crate::executor) fn split_unquoted_and_and(source: &str) -> Option<(&str, &str)> {
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let chars = source.char_indices().collect::<Vec<_>>();
    let mut index = 0;

    while index < chars.len() {
        let (byte_index, ch) = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '&' if !single && !double && chars.get(index + 1).is_some_and(|(_, ch)| *ch == '&') => {
                return Some((&source[..byte_index], &source[byte_index + 2..]));
            }
            _ => {}
        }
        index += 1;
    }

    None
}
