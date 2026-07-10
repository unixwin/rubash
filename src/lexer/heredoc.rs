use super::token::{Token, TokenKind};

pub(super) struct HereDocDelimiter {
    pub(super) value: String,
    pub(super) quoted: bool,
    pub(super) strip_tabs: bool,
    pub(super) allow_closing_paren: bool,
}

pub(super) fn heredoc_delimiters(tokens: &[Token], source: &str) -> Vec<HereDocDelimiter> {
    let mut source_offset = 0;
    tokens
        .windows(2)
        .filter(|pair| pair[0].kind == TokenKind::HereDoc)
        .map(|pair| {
            let context = heredoc_operator_context(source, &mut source_offset);
            let strip_tabs = pair[0].value == "<<-";
            let value = if strip_tabs {
                pair[1].value.trim_start_matches('\t').to_string()
            } else {
                pair[1].value.clone()
            };
            HereDocDelimiter {
                value,
                quoted: context.quoted,
                strip_tabs,
                allow_closing_paren: context.in_command_substitution,
            }
        })
        .collect()
}

struct HereDocOperatorContext {
    quoted: bool,
    in_command_substitution: bool,
}

fn heredoc_operator_context(source: &str, source_offset: &mut usize) -> HereDocOperatorContext {
    let Some(relative_index) = source[*source_offset..].find("<<") else {
        return HereDocOperatorContext {
            quoted: false,
            in_command_substitution: false,
        };
    };
    let index = *source_offset + relative_index;
    *source_offset = index + 2;
    let mut chars = source[index + 2..].chars().peekable();
    if chars.peek() == Some(&'-') {
        chars.next();
        *source_offset += 1;
    }
    while chars.peek().is_some_and(|ch| ch.is_ascii_whitespace()) {
        chars.next();
    }
    HereDocOperatorContext {
        quoted: matches!(chars.peek(), Some('\'' | '"' | '\\')),
        in_command_substitution: command_substitution_depth_before(source, index) > 0,
    }
}

fn command_substitution_depth_before(source: &str, end: usize) -> usize {
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let chars = source[..end].chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
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
        if ch == '\'' && !double {
            single = !single;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            index += 1;
            continue;
        }
        if single {
            index += 1;
            continue;
        }
        if ch == '$' && chars.get(index + 1) == Some(&'(') {
            depth += 1;
            index += 2;
            continue;
        }
        if depth > 0 && ch == '(' {
            depth += 1;
        } else if depth > 0 && ch == ')' {
            depth = depth.saturating_sub(1);
        }
        index += 1;
    }
    depth
}
