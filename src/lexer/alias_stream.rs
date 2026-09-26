//! GNU parse.y alias expansion ported to the input-text layer.
//!
//! GNU expands aliases while reading tokens: `alias_expand_token`
//! (parse.y:3249 / y.tab.c:5620) pushes the replacement text back onto the
//! parser input through `push_string` (parse.y:2055 / y.tab.c:4433), so the
//! alias value and the remaining source share one character stream. An
//! unclosed quote inside the alias value continues into the following
//! input (alias4.sub: `alias foo="echo 'Error:"` + `foo bar'` prints
//! `Error: bar`, and `foo x` is an unexpected-EOF syntax error), and
//! alias-introduced `;`, newlines, or redirections are real syntax.
//!
//! Word-level expansion after parsing cannot reproduce any of that — the
//! tokenizer has already consumed the unclosed quote before the executor
//! sees a word list. The script driver therefore expands each command
//! group's text before the tokenizer's quote-balance decisions, using the
//! alias table live at that moment — the same instant GNU sees it.

/// Alias lookup: name -> (replacement text, expand_next). `expand_next` is
/// alias.c's AL_EXPANDNEXT (value ends in blank → the next token is also
/// alias-expanded; parse.y:2098 sets PST_ALEXPNEXT when the pushed string
/// pops).
pub(crate) type AliasLookup<'a> = dyn Fn(&str) -> Option<(String, bool)> + 'a;

/// Last-token kinds needed by GNU's command-position predicates:
/// `command_token_position` (parse.y:3157), `assignment_acceptable`
/// (parse.y:3164), `reserved_word_acceptable` (y.tab.c:8258), and the
/// special-case token rules (y.tab.c:5725 special_case_tokens).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tok {
    Start,
    Newline,
    Semi,
    SemiSemi,
    SemiAnd,
    SemiSemiAnd,
    And,
    AndAnd,
    OrOr,
    Pipe,
    BarAnd,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Bang,
    Word,
    Assign,
    RedirOp,
    RedirTarget,
    If,
    Then,
    Elif,
    Else,
    Fi,
    While,
    Until,
    Do,
    Done,
    For,
    Select,
    Case,
    In,
    Esac,
    Function,
    Time,
    TimeOpt,
    TimeIgn,
    Coproc,
    CondStart,
    CondEnd,
    ArithCmd,
}

/// y.tab.c:8258 reserved_word_acceptable — last tokens that allow the next
/// word to be a reserved word (and, through command_token_position, an
/// alias candidate). `case`, `for`, `select`, `in`, `function`, and `[[`
/// are deliberately absent: the word following them is an operand, not a
/// command.
fn reserved_ok(t: Tok) -> bool {
    matches!(
        t,
        Tok::Start
            | Tok::Newline
            | Tok::Semi
            | Tok::SemiSemi
            | Tok::SemiAnd
            | Tok::SemiSemiAnd
            | Tok::LParen
            | Tok::RParen
            | Tok::Pipe
            | Tok::And
            | Tok::LBrace
            | Tok::RBrace
            | Tok::AndAnd
            | Tok::ArithCmd
            | Tok::Bang
            | Tok::BarAnd
            | Tok::CondEnd
            | Tok::Do
            | Tok::Done
            | Tok::Elif
            | Tok::Else
            | Tok::Esac
            | Tok::Fi
            | Tok::If
            | Tok::OrOr
            | Tok::Then
            | Tok::Time
            | Tok::TimeOpt
            | Tok::TimeIgn
            | Tok::Coproc
            | Tok::Until
            | Tok::While
    )
}

/// parse.y:3157 command_token_position: the next word may be a command
/// name (alias candidate). ASSIGNMENT_WORD, a non-redirection token inside
/// a redirection list, and anything reserved_word_acceptable — except the
/// case-pattern separators `;;` `;&` `;;&`, which introduce a pattern.
fn command_pos(last: Tok, last2: Tok) -> bool {
    if last == Tok::Assign || last == Tok::RedirTarget {
        return true;
    }
    // y.tab.c:8301: `coproc NAME cmd` — a WORD right after COPROC leaves
    // the following word in command position too.
    if last == Tok::Word && last2 == Tok::Coproc {
        return true;
    }
    !matches!(last, Tok::SemiSemi | Tok::SemiAnd | Tok::SemiSemiAnd) && reserved_ok(last)
}

/// One pushed expansion being consumed: GNU's pushed_string_list entry.
/// `end` is the buffer index where the pushed text ends; when the scan
/// position reaches it the alias leaves AL_BEINGEXPANDED and, when the
/// alias value ended in a blank, PST_ALEXPNEXT is set (parse.y:2098-2107).
struct Boundary {
    end: usize,
    expander: String,
    expand_next: bool,
}

/// Expand every alias that GNU's reader would expand while scanning
/// `source`, given the alias table in `lookup`. Returns the rewritten
/// source; everything outside the replaced words is byte-identical.
pub(crate) fn expand_aliases_in_source(source: &str, lookup: &AliasLookup<'_>) -> String {
    let source = expand_comsub_alias_bodies(source, lookup);
    let source = source.as_str();
    let mut buf: Vec<char> = source.chars().collect();
    let mut pos = 0usize;
    // AL_BEINGEXPANDED stack (parse.y:3259) — an alias does not expand
    // while its own pushed text is still being consumed.
    let mut expanding: Vec<String> = Vec::new();
    // Pending pushed-text boundaries, innermost last (LIFO like GNU's
    // pushed_string_list).
    let mut bounds: Vec<Boundary> = Vec::new();
    let mut alexpnext = false; // PST_ALEXPNEXT
    let mut last = Tok::Start;
    let mut last2 = Tok::Start;
    let mut casepat = false; // PST_CASEPAT
    let mut case_needed = 0usize; // esacs_needed_count
    let mut expect_in = 0usize; // expecting_in_token
    let mut expect_in_cmd = Tok::Start; // expecting_in_command
    let mut cond = 0usize; // inside [[ ]]
    let mut open_brace = 0usize; // open_brace_count for `}` recognition
                                 // Heredoc state: `<<`/<<- record their delimiter word; after the next
                                 // newline the body lines are raw data — GNU reads them with
                                 // here_document_to_fd, outside the token stream, so no alias expansion
                                 // ever applies inside a heredoc body.
    let mut expect_heredoc: Option<bool> = None; // strip_tabs for <<-
    let mut pending_heredocs: Vec<(String, bool)> = Vec::new();

    macro_rules! emit {
        ($tok:expr) => {{
            last2 = last;
            last = $tok;
            alexpnext = false;
        }};
    }

    while pos < buf.len() {
        // shell_getc boundary pops (parse.y:2081 pop_string).
        while bounds.last().is_some_and(|b| pos >= b.end) {
            let b = bounds.pop().expect("checked");
            expanding.retain(|name| name != &b.expander);
            if b.expand_next {
                alexpnext = true;
            }
        }
        if pos >= buf.len() {
            break;
        }
        let c = buf[pos];
        match c {
            ' ' | '\t' => pos += 1,
            '\n' => {
                emit!(Tok::Newline);
                pos += 1;
                // After the line carrying a `<<` operator ends, consume the
                // heredoc body lines verbatim until each delimiter
                // (parse.y here_document_to_fd): body text is never token
                // input, so it must not be scanned for aliases.
                while let Some((delimiter, strip_tabs)) = pending_heredocs.first().cloned() {
                    let line_start = pos;
                    let mut line_end = pos;
                    while line_end < buf.len() && buf[line_end] != '\n' {
                        line_end += 1;
                    }
                    let line: String = buf[line_start..line_end].iter().collect();
                    let candidate = if strip_tabs {
                        line.trim_start_matches('\t')
                    } else {
                        line.as_str()
                    };
                    pos = (line_end + 1).min(buf.len());
                    if candidate == delimiter {
                        pending_heredocs.remove(0);
                    }
                    if line_end >= buf.len() {
                        pending_heredocs.clear();
                        break;
                    }
                }
            }
            '#' => {
                // PST_COMMENT: to end of line (the newline itself emits a
                // token on the next iteration).
                while pos < buf.len() && buf[pos] != '\n' {
                    pos += 1;
                }
            }
            ';' => {
                if pos + 1 < buf.len() && buf[pos + 1] == ';' {
                    if pos + 2 < buf.len() && buf[pos + 2] == '&' {
                        emit!(Tok::SemiSemiAnd);
                        casepat = true; // y.tab.c:6118
                        pos += 3;
                    } else {
                        emit!(Tok::SemiSemi);
                        casepat = true; // y.tab.c:6069
                        pos += 2;
                    }
                } else if pos + 1 < buf.len() && buf[pos + 1] == '&' {
                    emit!(Tok::SemiAnd);
                    casepat = true; // y.tab.c:6118
                    pos += 2;
                } else {
                    emit!(Tok::Semi);
                    pos += 1;
                }
            }
            '&' => {
                if pos + 1 < buf.len() && buf[pos + 1] == '&' {
                    emit!(Tok::AndAnd);
                    pos += 2;
                } else if pos + 1 < buf.len() && buf[pos + 1] == '>' {
                    emit!(Tok::RedirOp);
                    pos += if pos + 2 < buf.len() && buf[pos + 2] == '>' {
                        3
                    } else {
                        2
                    };
                } else {
                    emit!(Tok::And);
                    pos += 1;
                }
            }
            '|' => {
                if pos + 1 < buf.len() && buf[pos + 1] == '|' {
                    emit!(Tok::OrOr);
                    pos += 2;
                } else if pos + 1 < buf.len() && buf[pos + 1] == '&' {
                    emit!(Tok::BarAnd);
                    pos += 2;
                } else {
                    emit!(Tok::Pipe);
                    pos += 1;
                }
            }
            '(' => {
                // parse_dparen (y.tab.c:7218): `((` is an arithmetic command
                // when a matching `))` exists, else a nested subshell.
                if pos + 1 < buf.len() && buf[pos + 1] == '(' {
                    if let Some(end) = match_dparen(&buf, pos) {
                        emit!(Tok::ArithCmd);
                        pos = end;
                    } else {
                        emit!(Tok::LParen);
                        pos += 1;
                    }
                } else {
                    emit!(Tok::LParen);
                    pos += 1;
                }
            }
            ')' => {
                if casepat {
                    casepat = false; // y.tab.c:6146
                }
                emit!(Tok::RParen);
                pos += 1;
            }
            '<' | '>' => {
                // `<<` / `<<-` declare a heredoc whose next word is the
                // delimiter; `<<<` is a herestring (no body to skip).
                if c == '<' && buf.get(pos + 1) == Some(&'<') && buf.get(pos + 2) != Some(&'<') {
                    expect_heredoc = Some(buf.get(pos + 2) == Some(&'-'));
                }
                emit!(Tok::RedirOp);
                pos += redir_op_len(&buf, pos);
            }
            _ => {
                // Word token: quoted regions and substitutions stay opaque.
                let ws = pos;
                let (end, quoted) = scan_word(&buf, pos);
                pos = end;
                let word: String = buf[ws..end].iter().collect();

                // A word right after a redirection operator is its target
                // (file, fd, or heredoc delimiter): not command position,
                // but PST_ALEXPNEXT still expands it (alias `c='< '` makes
                // `a2 foo c file` read `file` as the expanded target,
                // alias7.sub). A `<<`/`<<-` target also declares the body
                // delimiter, dequoted for the line comparison.
                if last == Tok::RedirOp && !alexpnext {
                    if let Some(strip_tabs) = expect_heredoc.take() {
                        pending_heredocs.push((dequote_heredoc_delimiter(&word), strip_tabs));
                    }
                    emit!(Tok::RedirTarget);
                    continue;
                }

                // y.tab.c:8085: an all-digit word before < or > is the fd
                // of a redirection, as is the {varname} form — but only
                // when they share one input read: a digit word ending at a
                // pushed-alias boundary does not merge with a < or > in the
                // outer text (`alias foo='echo 0'` + `foo>&2` prints `0` to
                // stderr; GNU read the `0` as an argument, not fd 0).
                if (word.chars().all(|d| d.is_ascii_digit()) && !word.is_empty()
                    || is_var_redir_name(&word))
                    && pos < buf.len()
                    && (buf[pos] == '<' || buf[pos] == '>')
                    && !bounds.iter().any(|b| b.end == end)
                {
                    emit!(Tok::RedirOp);
                    pos += redir_op_len(&buf, pos);
                    continue;
                }

                // special_case_tokens (y.tab.c:5725) run BEFORE alias
                // expansion: `in`, `do`, and `esac` in their grammar slots
                // can never be alias text.
                if word == "in"
                    && ((last == Tok::Word && matches!(last2, Tok::For | Tok::Case | Tok::Select))
                        || (expect_in > 0 && matches!(last, Tok::Word | Tok::Newline)))
                {
                    if expect_in_cmd == Tok::Case {
                        casepat = true; // y.tab.c:5736
                        case_needed += 1;
                    }
                    expect_in = expect_in.saturating_sub(1);
                    expect_in_cmd = Tok::Start;
                    emit!(Tok::In);
                    continue;
                }
                if word == "do" && last == Tok::Word && matches!(last2, Tok::For | Tok::Select) {
                    expect_in = expect_in.saturating_sub(1);
                    expect_in_cmd = Tok::Start;
                    emit!(Tok::Do);
                    continue;
                }
                if word == "esac" && case_needed > 0 && last == Tok::In {
                    case_needed -= 1;
                    casepat = false;
                    emit!(Tok::Esac);
                    continue;
                }
                // `time` is a reserved word only immediately after
                // ; \n || && & (y.tab.c:5695 comment + TIMEOPT/TIMEIGN).
                if word == "-p" && last == Tok::Time {
                    emit!(Tok::TimeOpt);
                    continue;
                }
                if word == "--" && matches!(last, Tok::Time | Tok::TimeOpt) {
                    emit!(Tok::TimeIgn);
                    continue;
                }

                // alias_expand_token (parse.y:3249): unquoted word in a
                // command position (PST_ALEXPNEXT or
                // assignment_acceptable) with a live alias that is not
                // currently being expanded.
                let eligible = alexpnext || (!casepat && command_pos(last, last2));
                if eligible && !quoted && !expanding.iter().any(|name| name == &word) {
                    if let Some((value, expand_next)) = lookup(&word) {
                        // push_string: splice the replacement text in place
                        // and re-read it under the same parser state.
                        let mut value_chars: Vec<char> = value.chars().collect();
                        // A pushed string end is a token boundary for the
                        // fd-prefix rule (y.tab.c:8085): digits at the end
                        // of pushed text do not merge with a < or > from
                        // the outer input. A literal space reproduces that
                        // boundary in flat text (`alias foo='echo 0'` +
                        // `foo>&2` is `echo 0 >&2`, printing `0` on stderr).
                        if value_chars.last().is_some_and(|ch| ch.is_ascii_digit())
                            && matches!(buf.get(end), Some('<') | Some('>'))
                        {
                            value_chars.push(' ');
                        }
                        let vlen = value_chars.len();
                        let removed = end - ws;
                        buf.splice(ws..end, value_chars);
                        let delta = vlen as isize - removed as isize;
                        for b in bounds.iter_mut() {
                            b.end = (b.end as isize + delta) as usize;
                        }
                        bounds.push(Boundary {
                            end: ws + vlen,
                            expander: word.clone(),
                            expand_next,
                        });
                        expanding.push(word);
                        pos = ws;
                        continue;
                    }
                }

                // alexpnext made the word alias-eligible but no alias was
                // defined: it is still the redirection's target.
                if last == Tok::RedirOp {
                    if let Some(strip_tabs) = expect_heredoc.take() {
                        pending_heredocs.push((dequote_heredoc_delimiter(&word), strip_tabs));
                    }
                    emit!(Tok::RedirTarget);
                    continue;
                }

                // CHECK_FOR_RESERVED_WORD (y.tab.c:3167): only when the
                // previous token allows a reserved word. PST_CASEPAT
                // suppresses every reserved word except a pattern-position
                // esac (y.tab.c:5536-5545).
                let esac_suppressed = casepat && matches!(last, Tok::Pipe | Tok::LParen);
                if !quoted && reserved_ok(last) && !(casepat && word != "esac") && !esac_suppressed
                {
                    let tok = match word.as_str() {
                        "if" => Some(Tok::If),
                        "then" => Some(Tok::Then),
                        "elif" => Some(Tok::Elif),
                        "else" => Some(Tok::Else),
                        "fi" => Some(Tok::Fi),
                        "while" => Some(Tok::While),
                        "until" => Some(Tok::Until),
                        "do" => Some(Tok::Do),
                        "done" => Some(Tok::Done),
                        "for" => Some(Tok::For),
                        "select" => Some(Tok::Select),
                        "case" => Some(Tok::Case),
                        "esac" => Some(Tok::Esac),
                        "function" => Some(Tok::Function),
                        "coproc" => Some(Tok::Coproc),
                        "time" => Some(Tok::Time),
                        "!" => Some(Tok::Bang),
                        "{" => Some(Tok::LBrace),
                        "}" if open_brace > 0 => Some(Tok::RBrace),
                        "[[" => Some(Tok::CondStart),
                        "]]" if cond > 0 => Some(Tok::CondEnd),
                        _ => None,
                    };
                    if let Some(tok) = tok {
                        match tok {
                            Tok::Case | Tok::For | Tok::Select => {
                                expect_in_cmd = tok;
                                expect_in += 1;
                            }
                            Tok::Esac => {
                                case_needed = case_needed.saturating_sub(1);
                                casepat = false;
                            }
                            Tok::LBrace => open_brace += 1,
                            Tok::RBrace => open_brace = open_brace.saturating_sub(1),
                            Tok::CondStart => cond += 1,
                            Tok::CondEnd => cond = cond.saturating_sub(1),
                            _ => {}
                        }
                        emit!(tok);
                        continue;
                    }
                }
                if cond > 0 {
                    // Inside [[ ]] words are expression operands.
                    emit!(Tok::Word);
                    continue;
                }
                if is_assignment_syntax(&word) && command_pos(last, last2) && !casepat {
                    emit!(Tok::Assign);
                } else {
                    emit!(Tok::Word);
                }
            }
        }
    }
    buf.iter().collect()
}

/// Scan one word starting at `pos`: returns (end, quoted). Quoting and
/// substitution constructs are consumed opaquely; the word ends at an
/// unquoted shell break (blank, newline, or metachar).
fn scan_word(buf: &[char], mut pos: usize) -> (usize, bool) {
    let mut quoted = false;
    while pos < buf.len() {
        match buf[pos] {
            ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')' | '<' | '>' => break,
            '\\' => {
                quoted = true;
                pos += if pos + 1 < buf.len() { 2 } else { 1 };
            }
            '\'' => {
                quoted = true;
                pos = skip_single_quote(buf, pos);
            }
            '"' => {
                quoted = true;
                pos = skip_double_quote(buf, pos);
            }
            '`' => {
                pos = skip_backtick(buf, pos);
            }
            '$' if pos + 1 < buf.len() => match buf[pos + 1] {
                '\'' => {
                    quoted = true;
                    pos = skip_ansi_quote(buf, pos + 2);
                }
                '"' => {
                    quoted = true;
                    pos = skip_double_quote(buf, pos + 1);
                }
                '(' => pos = skip_dollar_paren(buf, pos),
                '{' => pos = skip_dollar_brace(buf, pos),
                _ => pos += 1,
            },
            _ => pos += 1,
        }
    }
    (pos, quoted)
}

fn skip_single_quote(buf: &[char], mut pos: usize) -> usize {
    pos += 1;
    while pos < buf.len() && buf[pos] != '\'' {
        pos += 1;
    }
    (pos + 1).min(buf.len())
}

fn skip_ansi_quote(buf: &[char], mut pos: usize) -> usize {
    while pos < buf.len() {
        match buf[pos] {
            '\\' => pos = (pos + 2).min(buf.len()),
            '\'' => return pos + 1,
            _ => pos += 1,
        }
    }
    pos
}

fn skip_double_quote(buf: &[char], mut pos: usize) -> usize {
    pos += 1;
    while pos < buf.len() {
        match buf[pos] {
            '\\' => pos = (pos + 2).min(buf.len()),
            '"' => return pos + 1,
            '`' => pos = skip_backtick(buf, pos),
            '$' if pos + 1 < buf.len() && buf[pos + 1] == '(' => {
                pos = skip_dollar_paren(buf, pos);
            }
            '$' if pos + 1 < buf.len() && buf[pos + 1] == '{' => {
                pos = skip_dollar_brace(buf, pos);
            }
            _ => pos += 1,
        }
    }
    pos
}

fn skip_backtick(buf: &[char], mut pos: usize) -> usize {
    pos += 1;
    while pos < buf.len() {
        match buf[pos] {
            '\\' => pos = (pos + 2).min(buf.len()),
            '`' => return pos + 1,
            _ => pos += 1,
        }
    }
    pos
}

/// GNU expands aliases while reading a `$(...)` body — parse_comsub feeds
/// the same token reader, so alias_expand_token applies inside the
/// substitution (comsub5.sub: `alias switch=case` + `$( switch foo in foo)
/// ...)` runs to the `)` after `esac`, not `foo)`). The substitution's
/// extent is therefore decided on post-alias text; iterate because a splice
/// can move the close paren (`case` protects a mid-body `)`, and an alias
/// can contribute the closing `)` itself: `short='echo ok 8 )'`).
/// `'` bodies and `\` escapes hide `$(`; `"` and `` ` `` interiors do not.
fn expand_comsub_alias_bodies(source: &str, lookup: &AliasLookup<'_>) -> String {
    if !source.contains("$(")
        && !source.contains("${ ")
        && !source.contains("${\t")
        && !source.contains("${\n")
        && !source.contains("${|")
        && !source.contains("${(")
    {
        return source.to_string();
    }
    let mut chars: Vec<char> = source.chars().collect();
    let mut pos = 0usize;
    let mut in_double = false;
    let mut changed = false;
    while pos < chars.len() {
        match chars[pos] {
            '\\' => pos = (pos + 2).min(chars.len()),
            '\'' if !in_double => pos = skip_single_quote(&chars, pos),
            '"' => {
                in_double = !in_double;
                pos += 1;
            }
            '`' if !in_double => pos = skip_backtick(&chars, pos),
            '$' if chars.get(pos + 1) == Some(&'(') => {
                pos = splice_substitution_body(
                    &mut chars,
                    pos,
                    lookup,
                    &mut changed,
                    |chars, open| crate::lexer::skip_parenthesized_unit_corrected(chars, open),
                );
            }
            // Bash 5.3 funsub (parser.h:83-85 FUNSUB_CHAR, parse.y:5506):
            // `${ ' followed by blank, newline, '|' or '(' parses a command
            // list like `$(` — its body gets the same one-pass expansion.
            '$' if chars.get(pos + 1) == Some(&'{')
                && chars
                    .get(pos + 2)
                    .is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '|' | '(')) =>
            {
                pos = splice_substitution_body(
                    &mut chars,
                    pos,
                    lookup,
                    &mut changed,
                    skip_funsub_body,
                );
            }
            _ => pos += 1,
        }
    }
    if changed {
        chars.into_iter().collect()
    } else {
        source.to_string()
    }
}

/// Shared body expansion for `$(` and `${ `: expand the body in ONE pass —
/// alias chains already resolve inside expand_aliases_in_source via
/// pushed-text boundaries. Expansion can move the closer's extent (e.g.
/// `switch`→`case` makes a following `foo)` case syntax, not the closer);
/// the freshly included tail is then new input and gets its own pass.
/// Already-spliced text is never rescanned — a self-referential alias must
/// not fire twice (AL_BEINGEXPANDED, parse.y:3259): `let` → `let --`
/// stays. The loop is bounded so an unclosed body cannot spin.
/// `extent(chars, open)` returns the index just past the closer, like
/// skip_parenthesized_unit_corrected.
fn splice_substitution_body(
    chars: &mut Vec<char>,
    pos: usize,
    lookup: &AliasLookup<'_>,
    changed: &mut bool,
    extent: fn(&[char], usize) -> Option<usize>,
) -> usize {
    let open = pos + 1;
    let body_end = extent(chars, open)
        .unwrap_or(chars.len() + 1)
        .saturating_sub(1)
        .min(chars.len());
    let body: String = chars[open + 1..body_end].iter().collect();
    let expanded = expand_aliases_in_source(&body, lookup);
    if expanded != body {
        chars.splice(open + 1..body_end, expanded.chars());
        *changed = true;
    }
    let mut consumed = expanded.chars().count();
    for _ in 0..8 {
        let new_end = extent(chars, open)
            .unwrap_or(chars.len() + 1)
            .saturating_sub(1)
            .min(chars.len());
        let tail_start = open + 1 + consumed;
        if new_end <= tail_start || tail_start >= chars.len() {
            break;
        }
        let tail: String = chars[tail_start..new_end].iter().collect();
        let tail_expanded = expand_aliases_in_source(&tail, lookup);
        if tail_expanded == tail {
            break;
        }
        chars.splice(tail_start..new_end, tail_expanded.chars());
        *changed = true;
        consumed += tail_expanded.chars().count();
    }
    let end = extent(chars, open).unwrap_or(chars.len() + 1);
    end.max(pos + 2).min(chars.len())
}

/// Extent of a `${ command; }' funsub body: `open` is the index of `{` —
/// scan to the `}` that returns depth to 0, skipping quoted spans and
/// escape pairs (mirrors Lexer::skip_braced's funsub branch, skip.rs).
/// Returns the index just past the closing `}`.
fn skip_funsub_body(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut index = open + 1;
    let mut single = false;
    let mut double = false;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '\\' && !single {
            index += 2;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '{' if !single && !double => depth += 1,
            '}' if !single && !double => {
                depth -= 1;
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

/// Skip `$( ... )` honoring nested parens and quoting. Case patterns with
/// unbalanced `)` inside the substitution are approximated by depth only.
fn skip_dollar_paren(buf: &[char], mut pos: usize) -> usize {
    pos += 2;
    let mut depth = 1usize;
    while pos < buf.len() && depth > 0 {
        match buf[pos] {
            '\\' => pos = (pos + 2).min(buf.len()),
            '\'' => pos = skip_single_quote(buf, pos),
            '"' => pos = skip_double_quote(buf, pos),
            '`' => pos = skip_backtick(buf, pos),
            '(' => {
                depth += 1;
                pos += 1;
            }
            ')' => {
                depth -= 1;
                pos += 1;
            }
            _ => pos += 1,
        }
    }
    pos
}

fn skip_dollar_brace(buf: &[char], mut pos: usize) -> usize {
    pos += 2;
    let mut depth = 1usize;
    while pos < buf.len() && depth > 0 {
        match buf[pos] {
            '\\' => pos = (pos + 2).min(buf.len()),
            '\'' => pos = skip_single_quote(buf, pos),
            '"' => pos = skip_double_quote(buf, pos),
            '{' => {
                depth += 1;
                pos += 1;
            }
            '}' => {
                depth -= 1;
                pos += 1;
            }
            _ => pos += 1,
        }
    }
    pos
}

/// Match `(( ... ))` from `pos` (which points at the first `(`). Returns
/// the index just past the closing `))`, or None for a nested subshell.
fn match_dparen(buf: &[char], mut pos: usize) -> Option<usize> {
    pos += 2;
    let mut depth = 2usize;
    while pos < buf.len() {
        match buf[pos] {
            '\\' => pos = (pos + 2).min(buf.len()),
            '\'' => pos = skip_single_quote(buf, pos),
            '"' => pos = skip_double_quote(buf, pos),
            '`' => pos = skip_backtick(buf, pos),
            '(' => {
                depth += 1;
                pos += 1;
            }
            ')' => {
                depth -= 1;
                pos += 1;
                if depth == 0 {
                    return Some(pos);
                }
            }
            _ => pos += 1,
        }
    }
    None
}

/// Length of a redirection operator at `pos` (`buf[pos]` is `<` or `>`).
fn redir_op_len(buf: &[char], pos: usize) -> usize {
    let c = buf[pos];
    let next = buf.get(pos + 1).copied();
    let third = buf.get(pos + 2).copied();
    match (c, next, third) {
        ('<', Some('<'), Some('<')) => 3, // <<<
        ('<', Some('<'), Some('-')) => 3, // <<-
        ('<', Some('<'), _) => 2,         // <<
        ('>', Some('>'), _) => 2,         // >>
        ('<', Some('&'), _) => 2,         // <&
        ('>', Some('&'), _) => 2,         // >&
        ('<', Some('>'), _) => 2,         // <>
        ('>', Some('|'), _) => 2,         // >|
        _ => 1,
    }
}

/// Strip quote syntax from a heredoc delimiter word for the body-line
/// comparison (parse.y: the delimiter is remembered dequoted; a quoted
/// delimiter only suppresses body expansion, not matching).
fn dequote_heredoc_delimiter(word: &str) -> String {
    word.chars()
        .filter(|ch| !matches!(ch, '\'' | '"' | '\\'))
        .collect()
}

/// `{name}` redirection target prefix (exec {fd}>file).
fn is_var_redir_name(word: &str) -> bool {
    word.len() > 2
        && word.starts_with('{')
        && word.ends_with('}')
        && word[1..word.len() - 1]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// GNU general.c assignment(): NAME=, NAME+=, NAME[subscript]= — the
/// value may be empty and the subscript may contain anything balanced.
fn is_assignment_syntax(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    let mut i = 0usize;
    if i >= chars.len() || !(chars[i].is_ascii_alphabetic() || chars[i] == '_') {
        return false;
    }
    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
        i += 1;
    }
    if i < chars.len() && chars[i] == '[' {
        // Skip a bracketed subscript (may itself contain quotes/brackets).
        let mut depth = 0usize;
        while i < chars.len() {
            match chars[i] {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    if i < chars.len() && chars[i] == '+' {
        i += 1;
    }
    i < chars.len() && chars[i] == '='
}
