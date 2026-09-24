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
    // GNU read_token (parse.y:3630-3643): `#` introduces a comment only at
    // a token boundary — after whitespace, a separator (`;&|()<>`), or at
    // the start. `word_boundary`/`word.is_empty()` are wrong: `$`, quotes
    // and other non-alphanumeric word characters never reach `word`, so
    // `$(echo $#)` and `$(echo a # )` both misjudge `#`.
    let mut token_boundary = true;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            token_boundary = false;
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
            token_boundary = false;
            index += 1;
            continue;
        }
        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            token_boundary = false;
            index += 2;
            continue;
        }
        if ch == '#' && !single && !double && token_boundary {
            while index + 1 < chars.len() && chars[index + 1] != '\n' {
                index += 1;
            }
            word.clear();
            word_boundary = true;
            current_word_boundary = true;
            token_boundary = true;
            index += 1;
            continue;
        }
        if ch == '`' && !single {
            if let Some((_, next_index)) = backtick_command_substitution(chars, index) {
                index = next_index;
                token_boundary = false;
                continue;
            }
        }
        // Inside double quotes, `$(...)` is a nested command substitution
        // (GNU parse.y `xparse_dolparen` / subst.c `extract_command_substitution`):
        // a `"` inside the nested `$(...)` does NOT close the outer double
        // quote.  Recursively skip the nested `$(...)` as a unit so the
        // outer `double` state is preserved.  Without this,
        // `$(echo "foo$(echo ")")")` misparses the inner `"` as the close of
        // the outer quote, truncating the substitution body.
        if double
            && ch == '$'
            && chars.get(index + 1) == Some(&'(')
            && chars.get(index + 2) != Some(&'(')
        {
            if let Some((_, next_index)) = dollar_command_substitution(chars, index) {
                index = next_index;
                token_boundary = false;
                continue;
            }
        }
        // Heredoc bodies are opaque to command-substitution delimiter matching.
        if ch == '<' && !single && !double && chars.get(index + 1) == Some(&'<') {
            if let Some((next_index, header_close)) =
                skip_command_substitution_heredoc(chars, index)
            {
                if let Some((paren, header_end)) = header_close {
                    // GNU parse.y parse_comsub:4564 + print_comsub
                    // (parse.y:4632): the `)` on the heredoc header line
                    // closes the substitution while the pending body is
                    // gathered from the following input lines and reprinted
                    // inside the word (`echo $(cat << EOF)` + `foo`/`bar`/
                    // `EOF` lines yields `foo bar`).  Rebuild the inner
                    // source the same way: header text up to the `)`, then
                    // the gathered body lines.
                    let mut source: String = chars[start + 2..paren].iter().collect();
                    source.extend(chars[header_end..next_index].iter());
                    let mut text = String::from("$(");
                    text.push_str(&source);
                    text.push(')');
                    return Some((
                        command_substitution_node(text, source, false, false, false),
                        next_index,
                    ));
                }
                index = next_index;
                // A heredoc terminator ends on its own line, so the next
                // character begins a fresh token.
                token_boundary = true;
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
        // Quoted characters are word text handled by the quote state
        // arms; the boundary only tracks characters the live tokenizer
        // sees.
        if !single && !double {
            token_boundary = ch.is_whitespace()
                || matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
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

fn skip_command_substitution_heredoc(
    chars: &[char],
    start: usize,
) -> Option<(usize, Option<(usize, usize)>)> {
    let mut header_end = start + 2;
    while header_end < chars.len() && chars[header_end] != '\n' {
        header_end += 1;
    }
    if header_end >= chars.len() {
        return None;
    }

    // GNU parse.y (PST_EOFTOKEN): inside `$(...)` the `)` is the
    // substitution's eof token and a shell break character, so the heredoc
    // delimiter word ends at the first unquoted `)` and that `)` closes the
    // substitution on the header line (`cat << EOF)`).  parse_comsub then
    // gathers the still-pending heredoc body from the following input lines
    // (parse.y:4564 gather_here_documents) and print_comsub reprints the
    // command with the body inside the word.  Parse the delimiter word
    // quote/escape aware so `<<\)` keeps `)` as delimiter text.
    let mut index = start + 2;
    let strip_tabs = if chars.get(index) == Some(&'-') {
        index += 1;
        true
    } else {
        false
    };
    while index < header_end && matches!(chars[index], ' ' | '\t') {
        index += 1;
    }
    let mut delimiter = String::new();
    let mut single = false;
    let mut double = false;
    while index < header_end {
        let ch = chars[index];
        if single {
            if ch == '\'' {
                single = false;
            } else {
                delimiter.push(ch);
            }
            index += 1;
            continue;
        }
        if double {
            if ch == '"' {
                double = false;
            } else {
                delimiter.push(ch);
            }
            index += 1;
            continue;
        }
        match ch {
            '\'' => single = true,
            '"' => double = true,
            '\\' if index + 1 < header_end => {
                delimiter.push(chars[index + 1]);
                index += 1;
            }
            c if c.is_whitespace() || matches!(c, ';' | '|' | '&' | ')') => break,
            _ => delimiter.push(ch),
        }
        index += 1;
    }
    if strip_tabs {
        delimiter = delimiter.trim_start_matches('\t').to_string();
    }
    if delimiter.is_empty() {
        return None;
    }

    // An unquoted `)` anywhere after the delimiter word on the header line
    // closes the substitution (it is the eof token, not body text).
    let mut header_close_paren = None;
    {
        let mut rest = index;
        let mut rest_single = false;
        let mut rest_double = false;
        while rest < header_end {
            let ch = chars[rest];
            if rest_single {
                if ch == '\'' {
                    rest_single = false;
                }
            } else if rest_double {
                if ch == '"' {
                    rest_double = false;
                }
            } else {
                match ch {
                    '\'' => rest_single = true,
                    '"' => rest_double = true,
                    '\\' => rest += 1,
                    ')' => {
                        header_close_paren = Some(rest);
                        break;
                    }
                    _ => {}
                }
            }
            rest += 1;
        }
    }

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
                header_close_paren.map(|paren| (paren, header_end)),
            ));
        }
        // GNU parse.y gather_here_documents reads the whole heredoc body
        // before the parser looks at the next token, so a body line that
        // starts with the delimiter and contains `)` later on ends the
        // heredoc AND pushes the remainder back into the parser input
        // (make_cmd.c:602-611 PST_EOFTOKEN, shell_ungets(line+redir_len)).
        // Resume the caller's scan right after the delimiter prefix — the
        // pushed-back `)` (and anything before it, e.g. `x` in `EOFx)`)
        // re-enters the token stream, where the `)` closes the substitution.
        // Covers `EOF)`, `EOF )`, and `))` when the delimiter is `)` itself.
        if candidate.starts_with(delimiter.as_str())
            && candidate[delimiter.len()..].contains(')')
        {
            let resume = line_start
                + (line.chars().count() - candidate.chars().count())
                + delimiter.chars().count();
            return Some((resume, None));
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
            crate::executor::markers::DATA_BACKTICK => decoded.push('`'),
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
