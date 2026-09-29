use super::dolbrace::{scan_braced_parameter_body_chars, BraceContext, DolbraceState};
use super::token::{Token, TokenKind};

/// Whether the just-tokenized logical line still awaits a `}' from a later
/// line. GNU parse.y has no logical lines: read_token reads a `{` group
/// across newlines, and only the RESERVED-WORD `{` (CHECK_FOR_RESERVED_WORD,
/// parse.y:3168-3175, gated by reserved_word_acceptable at parse.y:5899 and
/// suppressed in case patterns by PST_CASEPAT, parser.h:29) opens a group at
/// all. The Lexer encodes exactly that: an unclosed group fold emits a
/// standalone `Keyword "{"` token, while pattern/word-text braces come out as
/// Word/BraceExpand and never join. Text-level `contains("... {")` admission
/// here used to count `case x in {)`'s pattern brace as a group opener and
/// swallow the rest of the file into one logical line (rubash#117 family).
///
/// The join admission mirrors that pre-token-signal text check exactly — a
/// group `{` at the start of the line or right after `;' / `&&' / `||' — but
/// computed from tokens. A `{' after `)' (the `name() {' function-body shape)
/// stays OUT: the parser pairs it with a later `}' (matching_brace_group_end)
/// and that path keeps per-line LINENO attribution for the definition (the
/// dbg-support DEBUG-trap linenos regressed when this brace joined).
pub(super) fn tokens_open_unclosed_brace_group(line_tokens: &[Token]) -> bool {
    line_tokens.iter().enumerate().any(|(index, token)| {
        if token.kind != TokenKind::Keyword || token.value != "{" {
            return false;
        }
        match index {
            0 => true,
            _ => matches!(
                line_tokens[index - 1].kind,
                TokenKind::Semicolon | TokenKind::And | TokenKind::Or
            ),
        }
    })
}

pub(super) fn has_unclosed_brace_group(input: &str) -> bool {
    let trimmed = input.trim_start();
    let has_group = trimmed.starts_with('{')
        || input.contains("&& {")
        || input.contains("|| {")
        || input.contains("; {");

    (has_group && unquoted_brace_group_depth(input) > 0) || has_unclosed_parameter_expansion(input)
}

/// Checkpointable state of the `has_unclosed_parameter_expansion` scan.
/// Every field is finite-local scanner state, so a snapshot taken at a char
/// offset plus the chars from that offset on reproduce the remainder of a
/// fresh whole-buffer scan bit for bit — the same append-only contract as
/// the comsub/quotes/compound residual checkpoints (rubash#292, perf8).
/// GNU anchor: parse.y:3557 read_token streams the input once; the
/// per-appended-line whole-buffer rescans this state replaces were this
/// port's substitute for that model.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct ParamScanState {
    single: bool,
    double: bool,
    ansi_single: bool,
    escaped: bool,
    comment_start: bool,
    in_comment: bool,
}

impl Default for ParamScanState {
    fn default() -> Self {
        Self {
            single: false,
            double: false,
            ansi_single: false,
            escaped: false,
            // The fresh scan starts word-initial: a '#' there opens a
            // comment (parse_comment), so comment state begins armed.
            comment_start: true,
            in_comment: false,
        }
    }
}

/// Advance the parameter-expansion scan over `chars[resume..]` from
/// `state`, returning the char index of a `${` whose body scan is still
/// undecided (the park — the fresh scan would return `true` there; a
/// longer buffer may let the body close, so the answer must be re-derived
/// from exactly this position) or `None` when the scan consumed the whole
/// slice without an unterminated `${` (the fresh scan's `false`).
pub(crate) fn param_residuals_advance(
    chars: &[char],
    resume: usize,
    state: &mut ParamScanState,
) -> Option<usize> {
    let mut index = resume;
    // GNU parse.y consumes a shell comment in the lexer (read_token hands a
    // word-initial '#' to parse_comment) before any expansion scanning, so a
    // dollar-brace opener inside a comment never starts a parameter
    // expansion. Track just enough quote state to keep quoted '#' literal.
    let ParamScanState {
        single,
        double,
        ansi_single,
        escaped,
        comment_start,
        in_comment,
    } = state;
    while index < chars.len() {
        let ch = chars[index];
        if *in_comment {
            if ch == '\n' {
                *in_comment = false;
                *comment_start = true;
            }
            index += 1;
            continue;
        }
        if *escaped {
            *escaped = false;
            *comment_start = false;
            index += 1;
            continue;
        }
        if ch == '\n' && !*single && !*double && !*ansi_single {
            *comment_start = true;
            index += 1;
            continue;
        }
        if ch == '#' && !*single && !*double && !*ansi_single && *comment_start {
            *in_comment = true;
            index += 1;
            continue;
        }
        if ch.is_whitespace() && !*single && !*double && !*ansi_single {
            *comment_start = true;
            index += 1;
            continue;
        }
        if *ansi_single {
            if ch == '\\' {
                *escaped = true;
            } else if ch == '\'' {
                *ansi_single = false;
            }
            *comment_start = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !*single {
            *escaped = true;
            *comment_start = false;
            index += 1;
            continue;
        }
        if ch == '\'' && !*double {
            *single = !*single;
            *comment_start = false;
            index += 1;
            continue;
        }
        if ch == '"' && !*single {
            *double = !*double;
            *comment_start = false;
            index += 1;
            continue;
        }
        if ch == '$' && !*single && !*double && chars.get(index + 1) == Some(&'\'') {
            *ansi_single = true;
            *comment_start = false;
            index += 2;
            continue;
        }
        if chars[index] == '$' && !*single && chars.get(index + 1) == Some(&'{') {
            // GNU parse.y:5305 read_token_word(): text inside single quotes
            // is literal until the closing `'` — `${` there never opens a
            // parameter expansion, so an unclosed-looking `${` in a quoted
            // word must not keep the logical line open (rubash#190:
            // `echo 'x=${bad' joined every later line into line 1, so a
            // later expansion failure's same-line skip discarded the whole
            // script tail). Inside double quotes `${` still expands and
            // still continues, matching the test below.
            //
            // rubash#281: the zero-copy char-slice API
            // (scan_braced_parameter_body_chars, rubash#185) replaces the
            // per-`${` String copy of the ENTIRE remaining buffer — nvm.sh
            // carries 1644 `${` occurrences, so the copy made this scan
            // O(buffer^2) per call (the same shape as the continuation.rs
            // `${` arm). The slice INCLUDES the `${` opener (the scanner
            // requires it at position 0) and `scan.end` is the CHAR count
            // of the body past the closing `}`.
            //
            // Checkpoint (perf15): an undecided body scan PARKS here and is
            // re-derived on the next appended text; the fresh scan returns
            // `true` at this position, so park = answer true.
            let body = &chars[index..];
            let context = BraceContext {
                outer_double_quote: false,
                posix: false,
                replacement_context: false,
                initial_state: DolbraceState::Param,
            };
            let Some(scan) = scan_braced_parameter_body_chars(body, context) else {
                return Some(index);
            };
            index += 2 + scan.end;
            *comment_start = false;
            continue;
        }
        *comment_start = false;
        index += 1;
    }
    None
}

pub(super) fn has_unclosed_parameter_expansion(input: &str) -> bool {
    let chars = input.chars().collect::<Vec<_>>();
    let mut state = ParamScanState::default();
    param_residuals_advance(&chars, 0, &mut state).is_some()
}
pub(super) fn opens_function_body_after_previous_signature(input: &str, output: &[Token]) -> bool {
    if input.trim() != "{" {
        return false;
    }

    output
        .iter()
        .rev()
        .find(|token| token.kind != TokenKind::Semicolon)
        .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == ")")
}

pub(super) fn unquoted_brace_group_depth(input: &str) -> usize {
    let chars = input.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    let mut case_depth = 0usize;
    let mut word = String::new();
    let mut word_boundary = true;
    let mut current_word_boundary = true;
    // A word-initial unquoted '#' comments out the rest of the line
    // (parse.y read_token -> parse_comment), so braces inside comment text
    // must not move the brace-group depth.
    let mut comment_start = true;
    let mut in_comment = false;

    while index < chars.len() {
        let ch = chars[index];
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                comment_start = true;
                word.clear();
                word_boundary = true;
                current_word_boundary = true;
            }
            index += 1;
            continue;
        }
        if escaped {
            escaped = false;
            comment_start = false;
            index += 1;
            continue;
        }
        if ansi_single {
            if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                ansi_single = false;
            }
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            comment_start = false;
            index += 2;
            continue;
        }
        if ch == '#' && !single && !double && !ansi_single && comment_start {
            in_comment = true;
            index += 1;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            comment_start = false;
            index += 1;
            continue;
        }
        if single || double {
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '$' && chars.get(index + 1) == Some(&'{') {
            index = skip_braced_parameter_in_chars(&chars, index + 2);
            comment_start = false;
            continue;
        }
        if ch.is_whitespace() {
            comment_start = true;
        } else {
            comment_start = false;
        }
        update_brace_group_case_depth(
            &chars,
            index,
            ch,
            &mut word,
            &mut case_depth,
            &mut word_boundary,
            &mut current_word_boundary,
        );
        match ch {
            '{' if case_depth == 0 => depth += 1,
            '}' if case_depth == 0 => depth = depth.saturating_sub(1),
            _ => {}
        }
        index += 1;
    }

    depth
}

fn update_brace_group_case_depth(
    chars: &[char],
    index: usize,
    ch: char,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
) {
    if ch == '_' || ch.is_ascii_alphanumeric() {
        if word.is_empty() {
            *current_word_boundary = *word_boundary;
        }
        word.push(ch);
        return;
    }

    if word.is_empty() {
        if brace_group_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    let reserved_word_allows_next = update_brace_group_reserved_word_depth(
        chars,
        index,
        word,
        *current_word_boundary,
        case_depth,
    );
    word.clear();
    *word_boundary = reserved_word_allows_next || brace_group_separator_allows_reserved_word(ch);
}

fn update_brace_group_reserved_word_depth(
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
        if word.is_empty() {
            if brace_group_separator_allows_reserved_word(ch) {
                word_boundary = true;
            } else if !ch.is_whitespace() {
                word_boundary = false;
            }
            scan += 1;
            continue;
        }
        let reserved_word_allows_next =
            word_boundary && brace_group_reserved_word_allows_next(&word);
        word.clear();
        word_boundary = reserved_word_allows_next || brace_group_separator_allows_reserved_word(ch);
        scan += 1;
    }

    word == "esac" && word_boundary
}

fn brace_group_reserved_word_allows_next(word: &str) -> bool {
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

fn brace_group_separator_allows_reserved_word(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | ')' | '{' | '\n')
}

pub(super) fn skip_braced_parameter_in_chars(chars: &[char], index: usize) -> usize {
    // rubash#281: zero-copy scan (rubash#185 API) — the String copy below
    // re-collected the whole remaining buffer per `${`. `index` sits just
    // past a `${` (the caller matched it), so the slice starts at the `$`
    // and `scan.end` is the CHAR count of the body past the closing `}`.
    let body = &chars[index - 2..];
    let context = BraceContext {
        outer_double_quote: false,
        posix: false,
        replacement_context: false,
        initial_state: DolbraceState::Param,
    };
    if let Some(scan) = scan_braced_parameter_body_chars(body, context) {
        return index + scan.end;
    }

    let mut index = index;
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
        if !single && !double {
            if ch == '{' {
                depth += 1;
            } else if ch == '}' {
                depth -= 1;
                if depth == 0 {
                    return index + 1;
                }
            }
        }
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::{has_unclosed_brace_group, unquoted_brace_group_depth};

    #[test]
    fn comment_with_dollar_brace_does_not_continue_logical_line() {
        // braces.tests regression: a comment mentioning an unbalanced
        // parameter expansion must not swallow the rest of the script
        // (parse.y consumes the comment before expansion scanning).
        assert!(!has_unclosed_brace_group(
            "# make sure the dollar brace is parsed as a word expansion"
        ));
        assert!(!has_unclosed_brace_group(
            "echo hi # don't treat quoted '#' as comment start"
        ));
        assert!(!has_unclosed_brace_group(
            "echo hi # comment mentions an unbalanced dollar brace here"
        ));
    }

    #[test]
    fn unclosed_parameter_expansion_still_continues() {
        assert!(has_unclosed_brace_group("echo ${unclosed"));
        assert!(has_unclosed_brace_group("echo \"${unclosed"));
        assert!(has_unclosed_brace_group("echo ${a#b"));
    }

    #[test]
    fn comment_braces_do_not_move_group_depth() {
        assert_eq!(unquoted_brace_group_depth("{ # } still open"), 1);
        assert_eq!(unquoted_brace_group_depth("{\n# }\n}"), 0);
        assert_eq!(unquoted_brace_group_depth("{ echo hi; }"), 0);
    }
}
