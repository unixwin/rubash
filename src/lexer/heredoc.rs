use super::token::{Token, TokenKind};

pub(super) struct HereDocDelimiter {
    pub(super) value: String,
    pub(super) quoted: bool,
    pub(super) strip_tabs: bool,
    pub(super) allow_closing_paren: bool,
}

pub(super) fn heredoc_delimiters(
    tokens: &[Token],
    source: &str,
    in_comsub: bool,
) -> Vec<HereDocDelimiter> {
    tokens
        .windows(2)
        .filter(|pair| pair[0].kind == TokenKind::HereDoc)
        .map(|pair| {
            // Anchor the scan on the delimiter word inside this logical line
            // instead of searching for the first `<<` in the source:
            // arithmetic shifts like `$((x << 2))` or here-strings (`<<<`)
            // would otherwise be mistaken for heredoc operators.
            // `value` has already undergone quote removal.  Use the token's
            // raw spelling for the quoted-delimiter decision: with a
            // partially quoted delimiter such as `E"OF"`, the normalized
            // value is just `EOF`, so searching the source for that value can
            // accidentally find the closing delimiter in the body instead.
            let context = heredoc_operator_context(source, &pair[1].raw);
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
                allow_closing_paren: context.in_command_substitution || in_comsub,
            }
        })
        .collect()
}

struct HereDocOperatorContext {
    quoted: bool,
    in_command_substitution: bool,
}

fn heredoc_operator_context(source: &str, delimiter_raw: &str) -> HereDocOperatorContext {
    // Find the delimiter word, then the `<<` operator immediately before it.
    let Some(delimiter_index) = source.rfind(delimiter_raw) else {
        return HereDocOperatorContext {
            quoted: false,
            in_command_substitution: false,
        };
    };
    let Some(operator_index) = source[..delimiter_index].rfind("<<") else {
        return HereDocOperatorContext {
            quoted: false,
            in_command_substitution: false,
        };
    };
    let index = operator_index;
    let mut chars = source[index + 2..].chars().peekable();
    if chars.peek() == Some(&'-') {
        chars.next();
    }
    while chars.peek().is_some_and(|ch| ch.is_ascii_whitespace()) {
        chars.next();
    }
    // GNU make_cmd.c: heredoc inside command substitution needs PST_EOFTOKEN
    // handling for `EOF)` closing. The original depth check via
    // command_substitution_depth_before could miss `$(` when source is a
    // truncated logical_line slice (e.g., `cat <<EOF` without the `$(` prefix).
    // Fall back to a simple `$(` scan when depth is 0.
    let depth = command_substitution_depth_before(source, index);
    let in_sub = if depth > 0 {
        true
    } else {
        // Fallback: check if `$(` appears before `<<` in the source, even if
        // the depth counter missed it due to truncated source or quote handling.
        let prefix = &source[..index];
        prefix.contains("$(") || prefix.contains("`")
    };
    HereDocOperatorContext {
        quoted: delimiter_raw
            .chars()
            .any(|ch| matches!(ch, '\'' | '"' | '\\'))
            || heredoc_delimiter_word_is_quoted(chars),
        in_command_substitution: in_sub,
    }
}

fn heredoc_delimiter_word_is_quoted<I>(chars: I) -> bool
where
    I: Iterator<Item = char>,
{
    let mut chars = chars.peekable();
    while let Some(ch) = chars.next() {
        if ch.is_ascii_whitespace() {
            break;
        }
        if matches!(ch, '\'' | '"') {
            return true;
        }
        if ch == '\\' {
            if chars.peek() == Some(&'\r') {
                chars.next();
                if chars.peek() == Some(&'\n') {
                    chars.next();
                    continue;
                }
                return true;
            }
            if chars.peek() == Some(&'\n') {
                chars.next();
                continue;
            }
            return true;
        }
    }
    false
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
