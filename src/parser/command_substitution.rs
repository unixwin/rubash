use super::{parse, CommandNode, CommandSubstitutionNode};

pub(super) fn record_command_substitutions_for_word(
    command: &mut CommandNode,
    word_index: usize,
    word: &str,
) {
    let substitutions = command_substitutions_in_word(word)
        .into_iter()
        .map(|mut substitution| {
            substitution.word_index = Some(word_index);
            substitution
        });
    command.command_substitutions.extend(substitutions);
}

pub(super) fn record_command_substitutions_for_assignment(
    command: &mut CommandNode,
    assignment_name: &str,
    value: &str,
    word_index: Option<usize>,
) {
    let substitutions = command_substitutions_in_word(value)
        .into_iter()
        .map(|mut substitution| {
            substitution.assignment_name = Some(assignment_name.to_string());
            substitution.word_index = word_index;
            substitution
        });
    command.command_substitutions.extend(substitutions);
}

pub(super) fn command_substitutions_in_word(word: &str) -> Vec<CommandSubstitutionNode> {
    let chars = word.chars().collect::<Vec<_>>();
    let mut substitutions = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '$'
            && chars.get(index + 1) == Some(&'(')
            && chars.get(index + 2) != Some(&'(')
        {
            if let Some((substitution, next_index)) = dollar_command_substitution(&chars, index) {
                substitutions.push(substitution);
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
            if let Some((substitution, next_index)) = braced_command_substitution(&chars, index) {
                substitutions.push(substitution);
                index = next_index;
                continue;
            }
        }

        if chars[index] == '`' {
            if let Some((substitution, next_index)) = backtick_command_substitution(&chars, index) {
                substitutions.push(substitution);
                index = next_index;
                continue;
            }
        }

        index += 1;
    }
    substitutions
}

fn dollar_command_substitution(
    chars: &[char],
    start: usize,
) -> Option<(CommandSubstitutionNode, usize)> {
    let mut index = start + 2;
    let mut depth = 1usize;
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    let mut case_depth = 0usize;
    let mut word = String::new();
    let mut word_boundary = true;
    let mut current_word_boundary = true;
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
        if ch == '#' && !single && !double && word_boundary {
            while index + 1 < chars.len() && chars[index + 1] != '\n' {
                index += 1;
            }
            word.clear();
            word_boundary = true;
            current_word_boundary = true;
            index += 1;
            continue;
        }
        if ch == '`' && !single {
            if let Some((_, next_index)) = backtick_command_substitution(chars, index) {
                index = next_index;
                continue;
            }
        }
        // Heredoc bodies are opaque to command-substitution delimiter matching.
        if ch == '<' && !single && !double && chars.get(index + 1) == Some(&'<') {
            if let Some((next_index, header_closes)) =
                skip_command_substitution_heredoc(chars, index)
            {
                if header_closes {
                    let text = chars[start..next_index].iter().collect();
                    let source = chars[start + 2..next_index].iter().collect();
                    return Some((
                        command_substitution_node(text, source, false, false, false),
                        next_index,
                    ));
                }
                index = next_index;
                continue;
            }
        }
        update_command_substitution_case_depth(
            &chars,
            index,
            ch,
            single,
            double,
            &mut word,
            &mut case_depth,
            &mut word_boundary,
            &mut current_word_boundary,
        );
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '(' if !single && !double && case_depth == 0 => depth += 1,
            ')' if !single && !double && case_depth == 0 => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let text = chars[start..=index].iter().collect();
                    let source = chars[start + 2..index].iter().collect();
                    return Some((
                        command_substitution_node(text, source, false, false, false),
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

fn braced_command_substitution(
    chars: &[char],
    start: usize,
) -> Option<(CommandSubstitutionNode, usize)> {
    let body_start = start + 2;
    let pipe_output = chars.get(body_start) == Some(&'|');
    if !pipe_output && !chars.get(body_start).is_some_and(|ch| ch.is_whitespace()) {
        return None;
    }

    let source_start = if pipe_output {
        body_start + 1
    } else {
        body_start
    };
    let mut index = source_start;
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
                    let text = chars[start..=index].iter().collect();
                    let source = chars[source_start..index].iter().collect();
                    return Some((
                        command_substitution_node(text, source, false, true, pipe_output),
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

fn skip_command_substitution_heredoc(chars: &[char], start: usize) -> Option<(usize, bool)> {
    let mut header_end = start + 2;
    while header_end < chars.len() && chars[header_end] != '\n' {
        header_end += 1;
    }
    if header_end >= chars.len() {
        return None;
    }

    let mut header = chars[start + 2..header_end].iter().collect::<String>();
    let strip_tabs = header.trim_start().starts_with('-');
    if strip_tabs {
        header = header.trim_start_matches([' ', '\t', '-']).to_string();
    }
    let raw_delimiter = header.split_whitespace().next()?;
    let delimiter = raw_delimiter
        .trim_matches(['\'', '"'])
        .trim_start_matches('\\')
        .to_string();
    if delimiter.is_empty() {
        return None;
    }

    let header_closes_command_substitution = header.contains(')');
    let mut line_start = header_end + 1;
    while line_start <= chars.len() {
        let mut line_end = line_start;
        while line_end < chars.len() && chars[line_end] != '\n' {
            line_end += 1;
        }
        let line = chars[line_start..line_end].iter().collect::<String>();
        let candidate = if strip_tabs {
            line.trim_start_matches('\t')
        } else {
            line.as_str()
        };
        if candidate == delimiter {
            return Some((
                (line_end + (line_end < chars.len()) as usize).min(chars.len()),
                header_closes_command_substitution,
            ));
        }
        // GNU parse.y gather_here_documents reads the whole heredoc body
        // before the parser looks at the next token, so a body line that is
        // the delimiter followed by `)` ends the heredoc AND supplies the
        // command substitution's closing paren (make_cmd.c:585-640
        // PST_EOFTOKEN). Consume only up to the `)` and report that the
        // header did not close the substitution: the caller's paren scan
        // then closes it at the unconsumed `)`. Mirror of the lexer-side
        // scanner (lexer/skip.rs strip_suffix form).
        if let Some(body) = candidate.strip_suffix(')') {
            if body == delimiter {
                let paren_index = line_start
                    + (line.chars().count() - candidate.chars().count())
                    + delimiter.chars().count();
                return Some((paren_index, false));
            }
        }
        if line_end >= chars.len() {
            break;
        }
        line_start = line_end + 1;
    }
    None
}

fn update_command_substitution_case_depth(
    chars: &[char],
    index: usize,
    ch: char,
    single: bool,
    double: bool,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
) {
    if single || double {
        word.clear();
        *word_boundary = false;
        return;
    }

    if ch == '_' || ch.is_ascii_alphanumeric() {
        if word.is_empty() {
            *current_word_boundary = *word_boundary;
        }
        word.push(ch);
        return;
    }

    if word.is_empty() {
        if command_substitution_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    let reserved_word_allows_next = update_command_substitution_reserved_word_depth(
        chars,
        index,
        word,
        *current_word_boundary,
        case_depth,
    );
    word.clear();
    *word_boundary =
        reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
}

fn update_command_substitution_reserved_word_depth(
    chars: &[char],
    index: usize,
    word: &str,
    word_boundary: bool,
    case_depth: &mut usize,
) -> bool {
    if !word_boundary {
        return false;
    }

    match word {
        "case" => {
            *case_depth += 1;
            false
        }
        "esac" if !case_pattern_starts_with_esac_chars(chars, index) => {
            *case_depth = case_depth.saturating_sub(1);
            true
        }
        "esac" => false,
        "for" | "select" | "while" | "until" | "then" | "do" | "else" | "elif" | "in" | "fi"
        | "done" => true,
        _ => false,
    }
}

fn case_pattern_starts_with_esac_chars(chars: &[char], delimiter_index: usize) -> bool {
    if !matches!(chars.get(delimiter_index), Some(')' | '|')) {
        return false;
    }

    let mut close = delimiter_index;
    while close < chars.len() {
        match chars[close] {
            ')' => break,
            ';' | '\n' => return false,
            _ => close += 1,
        }
    }
    if chars.get(close) != Some(&')') {
        return false;
    }

    let mut scan = close + 1;
    let mut word = String::new();
    let mut word_boundary = true;
    while scan < chars.len() {
        let ch = chars[scan];
        if ch == ';' && chars.get(scan + 1) == Some(&';') {
            return true;
        }
        if ch == '_' || ch.is_ascii_alphanumeric() {
            word.push(ch);
            scan += 1;
            continue;
        }
        if word == "esac" && word_boundary {
            return true;
        }
        if ch == ')' {
            return false;
        }
        if word.is_empty() {
            if command_substitution_separator_allows_reserved_word(ch) {
                word_boundary = true;
            } else if !ch.is_whitespace() {
                word_boundary = false;
            }
            scan += 1;
            continue;
        }
        let reserved_word_allows_next =
            word_boundary && command_substitution_reserved_word_allows_next(&word);
        word.clear();
        word_boundary =
            reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
        scan += 1;
    }

    word == "esac" && word_boundary
}

fn command_substitution_reserved_word_allows_next(word: &str) -> bool {
    matches!(
        word,
        "for"
            | "select"
            | "while"
            | "until"
            | "then"
            | "do"
            | "else"
            | "elif"
            | "in"
            | "fi"
            | "done"
            | "esac"
    )
}

fn command_substitution_separator_allows_reserved_word(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | ')' | '\n')
}

fn backtick_command_substitution(
    chars: &[char],
    start: usize,
) -> Option<(CommandSubstitutionNode, usize)> {
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
            let text = chars[start..=index].iter().collect();
            let source =
                decode_backtick_substitution_source(chars[start + 1..index].iter().copied());
            return Some((
                command_substitution_node(text, source, true, false, false),
                index + 1,
            ));
        }
        index += 1;
    }
    None
}

fn decode_backtick_substitution_source<I>(chars: I) -> String
where
    I: IntoIterator<Item = char>,
{
    let mut decoded = String::new();
    let mut chars = chars.into_iter().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\x1a' => decoded.push('`'),
            '\\' if chars.peek().copied() == Some('`') => {
                chars.next();
                decoded.push('`');
            }
            _ => decoded.push(ch),
        }
    }

    decoded
}

fn command_substitution_node(
    text: String,
    source: String,
    backtick: bool,
    current_shell: bool,
    pipe_output: bool,
) -> CommandSubstitutionNode {
    let tokens = crate::lexer::tokenize(&source);
    let commands = parse(&tokens).commands;
    let (open_delimiter, operator, close_delimiter) = if backtick {
        ("`".to_string(), "`".to_string(), "`".to_string())
    } else if current_shell {
        (
            "${".to_string(),
            if pipe_output { "${|" } else { "${" }.to_string(),
            "}".to_string(),
        )
    } else {
        ("$(".to_string(), "$".to_string(), ")".to_string())
    };
    CommandSubstitutionNode {
        text,
        open_delimiter_metadata: delimiter_metadata(&open_delimiter),
        open_delimiter,
        operator_metadata: delimiter_metadata(&operator),
        operator,
        source,
        close_delimiter_metadata: delimiter_metadata(&close_delimiter),
        close_delimiter,
        commands,
        backtick,
        current_shell,
        pipe_output,
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
