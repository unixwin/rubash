use super::*;
use crate::executor::markers::DATA_DOLLAR;

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
    // Keep variable values in their shell-visible form. Windows-native paths
    // are converted only at filesystem and process boundaries; converting
    // them during parameter expansion changes observable Bash string values
    // (for example, `${LOCALAPPDATA}`) and breaks backslash pattern matches.
    value.to_string()
}

pub(in crate::executor) fn decode_ansi_c_quoted_word(word: &str) -> Option<String> {
    let value = word.strip_prefix("$'")?.strip_suffix('\'')?;
    Some(decode_ansi_c_escapes(value))
}

pub(in crate::executor) fn decode_ansi_c_escapes(value: &str) -> String {
    // Same decoder as the lexer's ANSI-C path (GNU strtrans.c ansicstr):
    // carrier bytes get raw-byte marker tags, \x/octal bytes >= 0x80 stay
    // raw bytes, and NUL truncates. A second untagged decode here left
    // carrier bytes (0x15/0x19/0x1e) claimable by carrier restores and
    // mangled high bytes into Latin-1 chars (unicode1.sub).
    crate::lexer::decode_ansi_c_quoted(value)
}

pub(in crate::executor) fn push_ansi_c_codepoint(output: &mut String, value: Option<u32>) {
    let Some(value) = value else {
        return;
    };
    if let Some(ch) = char::from_u32(value) {
        output.push(ch);
    }
}

pub(in crate::executor) fn is_marked_var(
    env_vars: &HashMap<String, String>,
    key: &str,
    name: &str,
) -> bool {
    env_vars
        .get(key)
        .map(|value| value.split(DATA_DOLLAR).any(|marked| marked == name))
        .unwrap_or(false)
}
