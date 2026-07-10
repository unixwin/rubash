pub(in crate::executor) fn collect_braced_parameter_name(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> String {
    let mut name = String::new();
    let mut nested = 0usize;
    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek().copied() == Some('{') {
            chars.next();
            nested += 1;
            name.push('$');
            name.push('{');
            continue;
        }
        if ch == '}' {
            if nested == 0 {
                break;
            }
            nested -= 1;
            name.push(ch);
            continue;
        }
        name.push(ch);
    }
    name
}

pub(super) fn unescape_remaining_shell_escapes(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let mut lookahead = chars.clone();
            if lookahead.next() == Some('\\') && lookahead.next() == Some('\'') {
                chars.next();
                chars.next();
                output.push('\'');
                continue;
            }
            if let Some(
                next @ ('\'' | '"' | '\\' | '$' | '`' | '(' | ')' | '{' | '}' | ';' | '&' | '|'
                | '<' | '>' | '!' | '*' | '?' | '#'),
            ) = chars.peek().copied()
            {
                chars.next();
                output.push(next);
                continue;
            }
        }
        output.push(ch);
    }
    output
}

pub(in crate::executor) fn echo_command_substitution_output(args: &[String]) -> String {
    let mut newline = true;
    let mut escapes = false;
    let mut index = 0;

    while let Some(option) = args.get(index).map(String::as_str) {
        if !option.starts_with('-') || option == "-" {
            break;
        }
        if option[1..].chars().all(|ch| matches!(ch, 'n' | 'e' | 'E')) {
            for ch in option[1..].chars() {
                match ch {
                    'n' => newline = false,
                    'e' => escapes = true,
                    'E' => escapes = false,
                    _ => {}
                }
            }
            index += 1;
        } else {
            break;
        }
    }

    let mut output = args[index..].join(" ");
    if escapes {
        output = expand_echo_command_substitution_escapes(&output);
    }
    if newline {
        output.push('\n');
    }
    output.trim_end_matches('\n').to_string()
}

pub(in crate::executor) fn expand_echo_command_substitution_escapes(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => output.push('\n'),
            Some('t') => output.push('\t'),
            Some('\\') => output.push('\\'),
            Some(other) => {
                output.push('\\');
                output.push(other);
            }
            None => output.push('\\'),
        }
    }
    output
}

pub(in crate::executor) fn split_pipeline_words(words: &[String]) -> Option<Vec<&[String]>> {
    let mut stages = Vec::new();
    let mut start = 0usize;
    for (index, word) in words.iter().enumerate() {
        if word == "|" {
            if start == index {
                return None;
            }
            stages.push(&words[start..index]);
            start = index + 1;
        }
    }
    if start >= words.len() {
        return None;
    }
    stages.push(&words[start..]);
    (stages.len() > 1).then_some(stages)
}
