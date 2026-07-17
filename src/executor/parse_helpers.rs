use super::*;

pub(in crate::executor) fn command_node_source_line(command: &CommandNode) -> String {
    command.words.join(" ")
}

pub(in crate::executor) fn pending_heredoc_delimiter(source: &str) -> Option<String> {
    let mut pending: Option<(String, bool)> = None;
    for line in source.lines() {
        if let Some((delimiter, strip_tabs)) = &pending {
            if heredoc_delimiter_line_matches(line, delimiter, *strip_tabs) {
                pending = None;
            }
            continue;
        }
        pending = heredoc_delimiter_from_line(line);
    }

    pending.map(|(delimiter, _)| delimiter)
}

pub(in crate::executor) fn heredoc_delimiter_from_line(line: &str) -> Option<(String, bool)> {
    let words = split_shell_words(line);
    let mut index = 0;
    while index < words.len() {
        let word = &words[index];
        if word == "<<" || word == "<<-" {
            let delimiter = words.get(index + 1)?;
            return Some((normalize_heredoc_delimiter(delimiter), word == "<<-"));
        }
        if let Some(delimiter) = word.strip_prefix("<<-") {
            return Some((normalize_heredoc_delimiter(delimiter), true));
        }
        if let Some(delimiter) = word.strip_prefix("<<") {
            return Some((normalize_heredoc_delimiter(delimiter), false));
        }
        index += 1;
    }
    None
}

pub(in crate::executor) fn normalize_heredoc_delimiter(delimiter: &str) -> String {
    delimiter
        .trim_matches('\'')
        .trim_matches('"')
        .trim_start_matches('\\')
        .to_string()
}

pub(in crate::executor) fn heredoc_delimiter_line_matches(
    line: &str,
    delimiter: &str,
    strip_tabs: bool,
) -> bool {
    let line = if strip_tabs {
        line.trim_start_matches('\t')
    } else {
        line
    };
    line == delimiter
}

pub(in crate::executor) fn case_command_from_words(words: &[String]) -> Option<CaseCommand> {
    // TODO(parse.y): This recovers from the current parser losing `)` tokens
    // when a case command is exposed only after alias expansion. Replace this
    // with real parser input-stack alias expansion.
    if words.first().map(String::as_str) != Some("case") || words.len() < 5 {
        return None;
    }

    let word = words.get(1)?.clone();
    let mut index = 2;
    while index < words.len() && words[index] != "in" {
        index += 1;
    }
    if index >= words.len() {
        return None;
    }
    index += 1;

    let mut clauses = Vec::new();
    while index < words.len() && words[index] != "esac" {
        let pattern = words.get(index)?.clone();
        index += 1;

        let body_start = index;
        while index < words.len() && words[index] != ";;" && words[index] != "esac" {
            index += 1;
        }
        let body_source = words[body_start..index].join(" ");
        let body = if body_source.is_empty() {
            Vec::new()
        } else {
            let tokens = crate::lexer::tokenize(&body_source);
            crate::parser::parse(&tokens).commands
        };
        let clause_index = clauses.len();
        let pattern_nodes = vec![crate::parser::CasePattern::new(
            pattern.clone(),
            clause_index,
            0,
        )];
        let terminator_text = (index < words.len() && words[index] == ";;").then(|| ";;".into());
        clauses.push(CaseClause {
            pattern_open_delimiter: None,
            pattern_open_delimiter_metadata: None,
            patterns: vec![pattern],
            pattern_separators: Vec::new(),
            pattern_separator_metadata: Vec::new(),
            pattern_close_delimiter: ")".to_string(),
            pattern_close_delimiter_metadata: synthetic_keyword_metadata(")"),
            pattern_nodes,
            body,
            terminator: CaseTerminator::Break,
            terminator_metadata: terminator_text.as_deref().map(synthetic_keyword_metadata),
            terminator_text,
        });

        if index < words.len() && words[index] == ";;" {
            index += 1;
        }
    }

    Some(CaseCommand {
        keyword: "case".to_string(),
        keyword_metadata: synthetic_keyword_metadata("case"),
        word_metadata: crate::parser::WordMetadata::new(0, word.clone(), word.clone()),
        word,
        in_keyword: "in".to_string(),
        in_keyword_metadata: synthetic_keyword_metadata("in"),
        clauses,
        end_keyword: "esac".to_string(),
        end_keyword_metadata: synthetic_keyword_metadata("esac"),
    })
}

fn synthetic_keyword_metadata(keyword: &str) -> Box<crate::parser::WordMetadata> {
    Box::new(crate::parser::WordMetadata::new(
        0,
        keyword.to_string(),
        keyword.to_string(),
    ))
}

pub(in crate::executor) fn needs_parser_level_alias_expansion(value: &str) -> bool {
    value
        .chars()
        .any(|ch| matches!(ch, ';' | '\n' | '<' | '>' | '|' | '&'))
        || has_unclosed_quote(value)
}

pub(in crate::executor) fn has_unclosed_quote(value: &str) -> bool {
    // TODO(parse.y/alias.c): Bash tracks parser quoting state while pushing
    // alias replacement text back onto the input stream. This detects the
    // simple alias4.sub case where alias text opens a quote completed by a
    // following command word.
    let mut single = false;
    let mut double = false;
    let mut escaped = false;

    for ch in value.chars() {
        if escaped {
            escaped = false;
            continue;
        }

        if ch == '\\' && !single {
            escaped = true;
            continue;
        }

        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            _ => {}
        }
    }

    single || double
}

pub(in crate::executor) fn shell_safe_value(value: &str) -> String {
    // TODO(subst.c/findcmd.c): On Windows, Git Bash passes many environment
    // paths to native executables as `C:\...`. If those values are substituted
    // back into shell input for alias reparsing, backslashes are treated as
    // shell escapes. Keep absolute drive paths in `/c/...` form until Rubash
    // has a dedicated shell path type.
    if cfg!(windows) {
        let bytes = value.as_bytes();
        if bytes.len() >= 3
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/')
            && bytes[0].is_ascii_alphabetic()
        {
            let drive = (bytes[0] as char).to_ascii_lowercase();
            let rest = value[3..].replace('\\', "/");
            return format!("/{drive}/{rest}");
        }
    }

    value.to_string()
}

pub(in crate::executor) fn decode_ansi_c_quoted_word(word: &str) -> Option<String> {
    let value = word.strip_prefix("$'")?.strip_suffix('\'')?;
    Some(decode_ansi_c_escapes(value))
}

pub(in crate::executor) fn decode_ansi_c_escapes(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }

        match chars.next() {
            Some('a') => output.push('\x07'),
            Some('b') => output.push('\x08'),
            Some('e') | Some('E') => output.push('\x1b'),
            Some('f') => output.push('\x0c'),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('t') => output.push('\t'),
            Some('v') => output.push('\x0b'),
            Some('\\') => output.push('\\'),
            Some('\'') => output.push('\''),
            Some('"') => output.push('"'),
            Some('?') => output.push('?'),
            Some('x') => push_ansi_c_escape_or_literal(
                &mut output,
                'x',
                read_ansi_c_digits(&mut chars, 16, 2),
            ),
            Some('u') => push_ansi_c_escape_or_literal(
                &mut output,
                'u',
                read_ansi_c_digits(&mut chars, 16, 4),
            ),
            Some('U') => push_ansi_c_escape_or_literal(
                &mut output,
                'U',
                read_ansi_c_digits(&mut chars, 16, 8),
            ),
            Some(octal @ '0'..='7') => {
                let mut value = octal.to_digit(8).unwrap_or(0);
                for _ in 0..2 {
                    let Some(next) = chars.peek().copied() else {
                        break;
                    };
                    let Some(digit) = next.to_digit(8) else {
                        break;
                    };
                    value = value * 8 + digit;
                    chars.next();
                }
                push_ansi_c_codepoint(&mut output, Some(value));
            }
            Some(other) => {
                output.push('\\');
                output.push(other);
            }
            None => output.push('\\'),
        }
    }
    output
}

pub(in crate::executor) fn read_ansi_c_digits<I>(
    chars: &mut std::iter::Peekable<I>,
    radix: u32,
    max: usize,
) -> Option<u32>
where
    I: Iterator<Item = char>,
{
    let mut value = String::new();
    while value.len() < max {
        let Some(next) = chars.peek().copied() else {
            break;
        };
        if next.to_digit(radix).is_none() {
            break;
        }
        value.push(next);
        chars.next();
    }

    if value.is_empty() {
        None
    } else {
        u32::from_str_radix(&value, radix).ok()
    }
}

pub(in crate::executor) fn push_ansi_c_codepoint(output: &mut String, value: Option<u32>) {
    let Some(value) = value else {
        return;
    };
    if let Some(ch) = char::from_u32(value) {
        output.push(ch);
    }
}

fn push_ansi_c_escape_or_literal(output: &mut String, escape: char, value: Option<u32>) {
    if let Some(value) = value {
        push_ansi_c_codepoint(output, Some(value));
    } else {
        output.push('\\');
        output.push(escape);
    }
}

pub(in crate::executor) fn is_marked_var(
    env_vars: &HashMap<String, String>,
    key: &str,
    name: &str,
) -> bool {
    env_vars
        .get(key)
        .map(|value| value.split('\x1f').any(|marked| marked == name))
        .unwrap_or(false)
}
