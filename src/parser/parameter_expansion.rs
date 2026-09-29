use super::{CommandNode, ParameterExpansion};

pub(super) fn record_parameter_expansions_for_assignment(
    command: &mut CommandNode,
    assignment_name: &str,
    value: &str,
    word_index: Option<usize>,
) {
    let expansions = parameter_expansions_in_word(value)
        .into_iter()
        .map(|mut expansion| {
            expansion.assignment_name = Some(assignment_name.to_string());
            expansion.word_index = word_index;
            expansion
        });
    command.parameter_expansions.extend(expansions);
}

pub(super) fn parameter_expansions_in_word(word: &str) -> Vec<ParameterExpansion> {
    // Provably-empty admission (rubash#117 whitelist discipline): every
    // production this scan can find requires the trigger byte below, so its
    // absence proves the empty answer and skips the char collect + walk.
    // GNU anchor: parse.y:5305 read_token_word reads a word once; GNU runs
    // NO per-word expansion scans at parse time at all (subst.c analyzes at
    // execution) — this port's scans are the executor-facing metadata
    // source, and the gate only skips scans that cannot match.
    if !word.contains('$') {
        return Vec::new();
    }

    let chars = word.chars().collect::<Vec<_>>();
    let mut expansions = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '`' {
            if let Some(next_index) = skip_backtick(&chars, index) {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'(') {
            if chars.get(index + 2) == Some(&'(') {
                if let Some(next_index) = skip_arithmetic_expansion(&chars, index) {
                    index = next_index;
                    continue;
                }
            } else if let Some(next_index) = skip_command_substitution(&chars, index) {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'[') {
            if let Some(next_index) = skip_bracket_arithmetic_expansion(&chars, index) {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
            if let Some(next_index) = skip_braced_command_substitution(&chars, index) {
                index = next_index;
                continue;
            }
            if let Some((expansion, next_index)) = braced_parameter_expansion(&chars, index) {
                let nested = parameter_expansions_in_word(&expansion.parameter);
                expansions.push(expansion);
                expansions.extend(nested);
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' {
            if let Some((expansion, next_index)) = simple_parameter_expansion(&chars, index) {
                expansions.push(expansion);
                index = next_index;
                continue;
            }
        }

        index += 1;
    }
    expansions
}

fn quote_state_before(chars: &[char], end: usize) -> (bool, bool) {
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut index = 0usize;
    while index < end {
        let ch = chars[index];
        if escaped {
            escaped = false;
        } else if ch == '\\' && !single {
            escaped = true;
        } else if ch == '\'' && !double {
            single = !single;
        } else if ch == '"' && !single {
            double = !double;
        }
        index += 1;
    }
    (single, double)
}

fn braced_parameter_expansion(chars: &[char], start: usize) -> Option<(ParameterExpansion, usize)> {
    let mut index = start + 2;
    let mut depth = 1usize;
    // Preserve quote state from the word around `${...}`. An expansion inside
    // an outer double-quoted word must still accept its own closing brace.
    let (initial_single, initial_double) = quote_state_before(chars, start);
    let outer_double_quote = initial_double;
    let mut single = initial_single;
    let mut double = initial_double;
    let mut ansi_single = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ansi_single {
            if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                ansi_single = false;
            }
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            index += 2;
            continue;
        }
        if ch == '$' && chars.get(index + 1) == Some(&'{') && !single {
            depth += 1;
            index += 2;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            // A nested `${...}` closes while it is embedded in the outer
            // parameter's quoted pattern/word. The outer quote state must
            // not hide that nested delimiter from the depth counter.
            '}' if !single && (!double || depth > 1 || outer_double_quote) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let parameter = chars[start + 2..index].iter().collect::<String>();
                    let (name, operator, operator_prefix, word) = parameter_parts(&parameter);
                    return Some((
                        ParameterExpansion {
                            text: chars[start..=index].iter().collect(),
                            open_delimiter: "${".to_string(),
                            open_delimiter_metadata: delimiter_metadata("${"),
                            parameter,
                            close_delimiter: "}".to_string(),
                            close_delimiter_metadata: delimiter_metadata("}"),
                            name,
                            operator,
                            operator_prefix,
                            word,
                            braced: true,
                            word_index: None,
                            assignment_name: None,
                        },
                        index + 1,
                    ));
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn skip_braced_command_substitution(chars: &[char], start: usize) -> Option<usize> {
    let body_start = start + 2;
    let pipe_output = chars.get(body_start) == Some(&'|');
    if !pipe_output && !chars.get(body_start).is_some_and(|ch| ch.is_whitespace()) {
        return None;
    }

    let mut index = if pipe_output {
        body_start + 1
    } else {
        body_start
    };
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ansi_single {
            if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                ansi_single = false;
            }
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            index += 2;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '{' if !single && !double => depth += 1,
            '}' if !single && !double => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn simple_parameter_expansion(chars: &[char], start: usize) -> Option<(ParameterExpansion, usize)> {
    let next = *chars.get(start + 1)?;
    if next.is_ascii_alphabetic() || next == '_' {
        let mut end = start + 2;
        while chars
            .get(end)
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        {
            end += 1;
        }
        return Some((simple_expansion(chars, start, end), end));
    }

    if next.is_ascii_digit() || matches!(next, '*' | '@' | '#' | '?' | '-' | '$' | '!' | '_') {
        let end = start + 2;
        return Some((simple_expansion(chars, start, end), end));
    }

    None
}

fn simple_expansion(chars: &[char], start: usize, end: usize) -> ParameterExpansion {
    let parameter = chars[start + 1..end].iter().collect::<String>();
    ParameterExpansion {
        text: chars[start..end].iter().collect(),
        open_delimiter: "$".to_string(),
        open_delimiter_metadata: delimiter_metadata("$"),
        name: parameter.clone(),
        parameter,
        close_delimiter: String::new(),
        close_delimiter_metadata: delimiter_metadata(""),
        operator: None,
        operator_prefix: false,
        word: None,
        braced: false,
        word_index: None,
        assignment_name: None,
    }
}

fn delimiter_metadata(delimiter: &str) -> Box<crate::parser::WordMetadata> {
    Box::new(crate::parser::WordMetadata::new(
        0,
        delimiter.to_string(),
        delimiter.to_string(),
    ))
}

fn parameter_parts(parameter: &str) -> (String, Option<String>, bool, Option<String>) {
    if let Some(name) = parameter.strip_prefix('#').filter(|name| !name.is_empty()) {
        return (name.to_string(), Some("#".to_string()), true, None);
    }

    if let Some(name) = parameter.strip_prefix('!').filter(|name| !name.is_empty()) {
        if let Some((prefix, suffix)) = parameter_name_prefix_match(name) {
            return (
                prefix.to_string(),
                Some("!".to_string()),
                true,
                Some(suffix.to_string()),
            );
        }
        return (name.to_string(), Some("!".to_string()), true, None);
    }

    for operator in [
        ":-", ":=", ":?", ":+", "##", "%%", "//", "^^", ",,", "~~", ":", "-", "=", "?", "+", "#",
        "%", "/", "^", ",", "~", "@",
    ] {
        if let Some(index) = top_level_operator(parameter, operator) {
            return (
                parameter[..index].to_string(),
                Some(operator.to_string()),
                false,
                Some(parameter[index + operator.len()..].to_string()),
            );
        }
    }

    (parameter.to_string(), None, false, None)
}

fn parameter_name_prefix_match(name: &str) -> Option<(&str, &str)> {
    if name.len() <= 1 || name.ends_with(']') {
        return None;
    }

    name.strip_suffix('*')
        .map(|prefix| (prefix, "*"))
        .or_else(|| name.strip_suffix('@').map(|prefix| (prefix, "@")))
        .filter(|(prefix, _)| !prefix.is_empty())
}

fn top_level_operator(parameter: &str, operator: &str) -> Option<usize> {
    let chars = parameter.chars().collect::<Vec<_>>();
    let operator_chars = operator.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ansi_single {
            if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                ansi_single = false;
            }
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            index += 2;
            continue;
        }

        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '$' if !single && chars.get(index + 1) == Some(&'{') => {
                brace_depth += 1;
                index += 2;
                continue;
            }
            '}' if !single && !double && brace_depth > 0 => brace_depth -= 1,
            '(' if !single && !double => paren_depth += 1,
            ')' if !single && !double && paren_depth > 0 => paren_depth -= 1,
            '[' if !single && !double => bracket_depth += 1,
            ']' if !single && !double && bracket_depth > 0 => bracket_depth -= 1,
            _ => {}
        }

        if !single
            && !double
            && brace_depth == 0
            && paren_depth == 0
            && bracket_depth == 0
            && chars[index..].starts_with(&operator_chars)
            && operator_can_start(parameter, index, operator)
        {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn operator_can_start(_parameter: &str, index: usize, operator: &str) -> bool {
    if index == 0 {
        return false;
    }

    if operator == "/" || operator == "//" {
        return index > 0;
    }

    true
}

fn skip_command_substitution(chars: &[char], start: usize) -> Option<usize> {
    let mut index = start + 2;
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ansi_single {
            if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                ansi_single = false;
            }
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            index += 2;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '(' if !single && !double => depth += 1,
            ')' if !single && !double => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn skip_arithmetic_expansion(chars: &[char], start: usize) -> Option<usize> {
    let mut index = start + 3;
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index + 1 < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '(' if !single && !double => depth += 1,
            ')' if !single && !double && depth > 0 => depth -= 1,
            ')' if !single && !double && chars.get(index + 1) == Some(&')') => {
                return Some(index + 2);
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn skip_bracket_arithmetic_expansion(chars: &[char], start: usize) -> Option<usize> {
    let mut index = start + 2;
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '[' if !single && !double => depth += 1,
            ']' if !single && !double && depth > 0 => depth -= 1,
            ']' if !single && !double => return Some(index + 1),
            _ => {}
        }
        index += 1;
    }
    None
}

fn skip_backtick(chars: &[char], start: usize) -> Option<usize> {
    let mut index = start + 1;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '`' {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}
