use super::heredoc_scan::{
    skip_heredoc_in_chars_decided, skip_heredoc_in_chars_with_closure, skip_heredoc_top_level,
};

pub(super) fn ends_with_unquoted_backslash(input: &str) -> bool {
    // GNU parse.y shell_getc remove_quoted_newline: backslash-newline is ignored
    // unless the current delimiter is a single quote (qc == '\''). The dstack
    // correctly nests quotes inside command substitutions: a single quote
    // inside $(...) is quoted even when the substitution itself is inside
    // double quotes (quote.tests: echo "$(echo 'foo\↵bar')" must keep the
    // backslash). The old boolean single/double tracker treated '\'' as
    // literal when double==true, so the trailing '\' inside that nested
    // single was misclassified as unquoted and the logical line was joined
    // with the backslash removed (foobar instead of foo\↵bar).
    // Track a delimiter stack mirroring parse.y dstack for the cases that
    // affect this probe: ' " ` and $(.
    let chars: Vec<char> = input.chars().collect();
    let mut stack: Vec<char> = Vec::new();
    let mut i = 0usize;
    // Track word boundary for comment detection: a `#` at the top level
    // (no quote on stack) at a word boundary starts a comment, and a
    // trailing backslash inside a comment is NOT a line continuation.
    let mut comment_start = true;
    while i < chars.len() {
        let ch = chars[i];
        let top = stack.last().copied();
        if top == Some('\'') {
            if ch == '\'' {
                stack.pop();
                comment_start = false;
            }
            i += 1;
            continue;
        }
        if top == Some('"') {
            if ch == '\\' {
                // Inside double quotes a backslash escapes the next char
                // (parse.y parse_matched_pair LEX_PASSNEXT). Consume both
                // so a trailing '\' inside double is correctly seen as
                // an escaped newline that should be removed.
                if i + 1 < chars.len() {
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if ch == '"' {
                stack.pop();
                comment_start = false;
                i += 1;
                continue;
            }
            if ch == '$' && i + 1 < chars.len() && chars[i + 1] == '(' {
                stack.push('(');
                i += 2;
                continue;
            }
            if ch == '`' {
                stack.push('`');
                i += 1;
                continue;
            }
            // Single quote inside double quotes is literal (POSIX).
            i += 1;
            continue;
        }
        if top == Some('`') {
            if ch == '\\' {
                if i + 1 < chars.len() {
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if ch == '`' {
                stack.pop();
                comment_start = false;
            }
            i += 1;
            continue;
        }
        if top == Some('(') {
            // Inside command substitution quoting resets: ' " ` and nested $(
            // are delimiters again (parse.y read_token_word / parse_matched_pair).
            // A `#` at a word boundary starts a comment here too — a `\`
            // inside the comment is literal text, not a line continuation
            // (`$(echo x # \` + `)` in comsub-posix.tests).
            if ch == '#' && comment_start {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                if i >= chars.len() {
                    return false;
                }
                continue;
            }
            if ch.is_whitespace() {
                comment_start = true;
                i += 1;
                continue;
            }
            // GNU read_token: shell separators also begin a fresh token,
            // so `;#x` / `(#x` are comments while `$#` / `a#` are not.
            comment_start = matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
            if ch == '\'' {
                stack.push('\'');
                i += 1;
                continue;
            }
            if ch == '"' {
                stack.push('"');
                i += 1;
                continue;
            }
            if ch == '`' {
                stack.push('`');
                i += 1;
                continue;
            }
            if ch == '$' && i + 1 < chars.len() && chars[i + 1] == '(' {
                stack.push('(');
                // A fresh substitution body starts at a token boundary:
                // `$(#c` is a comment, not word text.
                comment_start = true;
                i += 2;
                continue;
            }
            if ch == ')' {
                stack.pop();
                i += 1;
                continue;
            }
            if ch == '\\' {
                if i + 1 < chars.len() {
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            i += 1;
            continue;
        }
        // Top-level (no quote or other)
        // A `#` at a word boundary starts a comment — the rest of the
        // line is not scanned for backslash-newline continuation. Only
        // return early when the comment runs to EOF; a later line can
        // still end in a real continuation.
        if ch == '#' && comment_start {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            if i >= chars.len() {
                return false;
            }
            continue;
        }
        if ch.is_whitespace() {
            comment_start = true;
            i += 1;
            continue;
        }
        comment_start = matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
        if ch == '\'' {
            stack.push('\'');
            i += 1;
            continue;
        }
        if ch == '"' {
            stack.push('"');
            i += 1;
            continue;
        }
        if ch == '`' {
            stack.push('`');
            i += 1;
            continue;
        }
        if ch == '$' && i + 1 < chars.len() && chars[i + 1] == '(' {
            stack.push('(');
            // A fresh substitution body starts at a token boundary:
            // `$(#c` is a comment, not word text.
            comment_start = true;
            i += 2;
            continue;
        }
        if ch == '\\' {
            if i + 1 < chars.len() {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        i += 1;
    }

    if stack.last() == Some(&'\'') {
        return false;
    }
    let trailing_backslashes = input.chars().rev().take_while(|ch| *ch == '\\').count();
    trailing_backslashes % 2 == 1
}

/// One pending matched-pair construct on the delimiter stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UnclosedDelim {
    /// Close delimiter of this construct.
    close: char,
    /// Line the construct opened on.
    open_line: usize,
    /// Backslash escapes the next char inside (false only in '...').
    escapes: bool,
    /// parse_matched_pair site (parse.y:3912): report the open line.
    /// Otherwise the yyerror site (parse.y:6891) reports the EOF line.
    report_open: bool,
    /// `${ ' function-substitution (parse.y:5506 FUNSUB_CHAR ->
    /// parse_comsub) or a `{ }' command group: `}' only closes at command
    /// position, after a command terminator.
    funsub: bool,
    /// `name=(` array list: GNU exits 1 on EOF (matched-pair in an
    /// assignment), not 2 like a syntax error.
    array_list: bool,
    /// A complete command ended here (subshell `( )' or `{ }' group).
    /// Closing such a construct resumes the enclosing funsub at command
    /// position; expansions like `$(...)'/`${...}' are mid-word and do not.
    command: bool,
    /// funsub: the last significant char was a command terminator
    /// (';', '&', '|', newline, or a closed command construct).
    term_ready: bool,
    /// rubash#380: a pattern-list `(` pushed while the case pattern region is armed —
    /// pattern punctuation (parse.y:3383 case_item_patten), not a subshell; its `)`
    /// is the pattern terminator, so the pop bypasses the case-depth guard.
    pattern_paren: bool,
    /// perf19: the case-clause depth when this delimiter was pushed — a
    /// `)` only closes this delimiter when the case depth has returned to
    /// its push-time level (a `case` opened OUTSIDE never blocks the
    /// closer; one opened INSIDE holds it through its `esac`).
    case_depth_at_push: usize,
}

/// GNU parse.y reports `unexpected EOF while looking for matching `X'' where
/// X is the close delimiter of the innermost unclosed matched-pair
/// construct: `}' for `${...}', `)' for `$(...)' / `( ... )' / `$(( ... ))',
/// ``' for backquotes, and the quote characters themselves.
///
/// Two different C sites produce the diagnostic and they number lines
/// differently: parse_matched_pair (parse.y:3912, quotes, backquotes and
/// `$(( )` arithmetic) reports `start_lineno` — the line the construct
/// opened on — while the `$(`/`${` paths take the yyerror route
/// (parse.y:6891) and report `line_number` at EOF.
///
/// `${ ' followed by a FUNSUB_CHAR (parser.h:85, the live `#else` arm —
/// space, tab, newline, `|'; the parser.h:83 spelling that also lists `(' is
/// inside `#if 0' dead code) is a ksh-style function substitution parsed by
/// parse_comsub (parse.y:5506): its `}' only closes at command position, so
/// `_[${ a }]' is unterminated while `_[${ a; }]' runs `a'. A `(' right after
/// `${' is NOT a funsub opener: it takes parse_matched_pair
/// (parse.y:5513, P_FIRSTCLOSE|P_DOLBRACE) where `(' is an ordinary
/// character and the first unquoted `}' closes — `${(M)x}' parses fine and
/// fails later as a runtime `bad substitution' (subst.c:10278).
///
/// Returns (close char, open line, EOF line, report_open_line) for the
/// innermost pending construct: callers print `open_line' when
/// report_open_line is set, otherwise the line at EOF.
/// POSIX mode honors the Austin Group Interp 221 rule already implemented in
/// `dolbrace::scan_braced_parameter`: inside `"${...}"` a `'` is literal text,
/// not a quote opener, so `"${IFS+'bar}` is complete input. Outside POSIX
/// mode (or outside double quotes) the `'` still opens a quote that can keep
/// the input unclosed.
fn squote_is_literal_in_posix_braced_dquote(stack: &[UnclosedDelim]) -> bool {
    let mut in_brace = false;
    for d in stack.iter().rev() {
        match d.close {
            '}' if !d.funsub => in_brace = true,
            '"' => return in_brace,
            // `'`, '`', `)` (`$(`/subshell/arithmetic) and funsub `}` reset the
            // parse context: quotes inside them behave normally.
            _ => return false,
        }
    }
    false
}

/// Returns `(close_delimiter, open_line, eof_line, report_open, command)`.
/// `command` marks a `( ...`/`{ ...; }` command-position construct (GNU's
/// yyerror names it "from `(' command"), as opposed to `$(`/`$((`/quote
/// matched pairs that take the "matching `X'" wording. `array_list` marks
/// `name=(` constructs — GNU exits 1 rather than 2 for those.
pub(crate) fn unclosed_input_close_char_posix(
    input: &str,
    posix: bool,
) -> Option<(char, usize, usize, bool, bool, bool)> {
    let chars: Vec<char> = input.chars().collect();
    let mut stack: Vec<UnclosedDelim> = Vec::new();
    let mut line = 1usize;
    let mut comment_start = true;
    // GNU parse.y: a bare `(` only opens a subshell/array-list when the
    // parser expects a command (start, after a separator, after a command
    // keyword like `if`/`in`) or directly follows `=` in an assignment
    // word (`ddd=(aaa` spans lines). A `(` mid-command is an immediate
    // syntax error, not a continuation — tracking this keeps `echo (`
    // from swallowing the following lines into a dead group.
    let mut at_command = true;
    let mut cur_word = String::new();
    // perf19 case-pattern tracking — the same word machine the parked
    // close-char scan carries (`close_char_operator_context`,
    // update_command_substitution_case_depth; GNU parse.y:1037 case
    // grammar: a pattern's `)` is a case token, not the enclosing
    // subshell/`$(` closer).
    let mut case_word = String::new();
    let mut case_depth = 0usize;
    let mut case_word_boundary = true;
    let mut case_current_word_boundary = true;
    let mut case_in_stage = 0u8;
    // parse.y:29 PST_CASEPAT (set at :3379/3396, cleared at :3787-3788):
    // inside a pattern list reserved words are word data — the region bit
    // keeps a pattern-position `case` from inflating case_depth (rubash#380).
    let mut case_pattern_region = false;
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        let top = stack.last().copied();
        if ch == '\n' {
            line += 1;
        }
        if close_char_operator_context(top) {
            update_command_substitution_case_depth_staged_chars(
                &chars,
                i,
                ch,
                &mut case_word,
                &mut case_depth,
                &mut case_word_boundary,
                &mut case_current_word_boundary,
                &mut case_in_stage,
                &mut case_pattern_region,
            );
        } else if top.is_some_and(|d| d.close == '\'' || d.close == '"') {
            case_word.clear();
            case_word_boundary = false;
        }
        // perf19: heredoc body opacity (make_cmd.c:512 make_here_document
        // reads the body raw; parse.y:3120 gather_here_documents, and
        // parse.y:3557 read_token streams past the terminator). Same arm
        // and context gate as the parked scan; the jumped newlines still
        // advance the diagnostic line counter.
        if ch == '<'
            && chars.get(i + 1) == Some(&'<')
            && chars.get(i + 2) != Some(&'<')
            && close_char_operator_context(top)
        {
            if let Some((next, terminator_found)) = skip_heredoc_top_level(&chars, i) {
                line += chars[i..next].iter().filter(|c| **c == '\n').count();
                comment_start = true;
                cur_word.clear();
                case_word.clear();
                case_word_boundary = true;
                if let Some(d) = stack.last_mut() {
                    if d.funsub {
                        d.term_ready = true;
                    }
                } else {
                    at_command = true;
                }
                i = next;
                continue;
            }
        }
        // `<<<` here-string (parse.y:3690-3706): one operator, consumed
        // atomically so its second `<` is never a `<<` opener.
        if ch == '<'
            && chars.get(i + 1) == Some(&'<')
            && chars.get(i + 2) == Some(&'<')
            && close_char_operator_context(top)
        {
            comment_start = true;
            cur_word.clear();
            i += 3;
            continue;
        }
        if let Some(d) = top {
            if d.escapes && ch == '\\' {
                // An escaped character is word text everywhere.
                comment_start = false;
                i += 2;
                continue;
            }
            // perf19: a `)` while a case clause is open is a PATTERN
            // terminator (parse.y:1037), not this delimiter's closer.
            if ch == d.close
                && !(d.funsub && !d.term_ready)
                && (d.pattern_paren || !(d.close == ')' && case_depth > d.case_depth_at_push))
            {
                stack.pop();
                // A closed subshell or brace group is a complete command:
                // an enclosing function substitution's `}' may now close.
                if let Some(parent) = stack.last_mut() {
                    if parent.funsub && d.command {
                        parent.term_ready = true;
                    }
                }
                // `)` is a shell separator: a following `#` starts a
                // comment (`x=$(a)#c`). Quote/`}` closes stay mid-word.
                comment_start = d.close == ')';
                // A completed '...'/"..." span is WORD MATERIAL for the
                // at_command tracker (rubash#322): read_token returns one
                // WORD token for `"#F"`, so the whitespace after it marks
                // a non-command position exactly like a bare word does —
                // otherwise `[[ "#F" =~ (a #c) ]]` kept at_command through
                // the quoted LHS and the regexp `(` opened a phantom
                // subshell whose `#` commented out the closing `) ]]`.
                if d.close == '"' || d.close == '\'' {
                    cur_word.push('"');
                }
                i += 1;
                continue;
            }
            if d.close == '\'' {
                // Literal context: nothing else is special inside '...'.
                i += 1;
                continue;
            }
            // Inside `$(...)` / `( ... )` / `${ cmds; }` (report_open false —
            // the `$((` inner paren reports open and is arithmetic text
            // where `#` is the base operator, never a comment) a `#` at a
            // token boundary comments through end of line, so the `)` in
            // `$(# c )` cannot close the substitution (comsub-posix). An
            // array list (`name=(`, also report_open — parse_matched_pair's
            // start_lineno report) is element text where `#` IS a comment:
            // a `)` inside the comment must not close the list (probe
            // 2026-09-27: `declare -a x=(\n 1 # c )` at EOF → GNU reports
            // "unexpected EOF while looking for matching `)'" + exit 1).
            if d.close == ')' || d.funsub {
                if !(d.close == ')' && d.report_open && !d.array_list) && ch == '#' && comment_start
                {
                    while i < chars.len() && chars[i] != '\n' {
                        i += 1;
                    }
                    continue;
                }
                comment_start =
                    ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
            }
            if d.funsub {
                // Track command-terminator state: `${ cmd }' without a
                // separator before `}' never terminates (parse_comsub).
                let d = stack.last_mut().unwrap();
                match ch {
                    ';' | '&' | '|' | '\n' => d.term_ready = true,
                    c if !c.is_whitespace() => d.term_ready = false,
                    _ => {}
                }
            }
        } else {
            // Top-level `\` quotes the next character as literal word text
            // (parse.y read_token_word): an escaped `(` in `\<...` must not
            // open a paren delimiter. `\<newline>` is a continuation that
            // joins the word across lines.
            if ch == '\\' && i + 1 < chars.len() {
                if chars[i + 1] == '\n' {
                    line += 1;
                } else if chars[i + 1] != '=' {
                    cur_word.push(chars[i + 1]);
                }
                comment_start = false;
                i += 2;
                continue;
            }
            // Top-level comment: a word-initial '#' consumes to EOL
            // (parse.y read_token -> parse_comment).
            if ch == '#' && comment_start {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            if ch.is_whitespace()
                || matches!(ch, ';' | '&' | '|' | '(' | ')' | '{' | '}' | '<' | '>')
            {
                comment_start = true;
            } else {
                comment_start = false;
            }
            match ch {
                // `(` is excluded: the push arm below gates on the state
                // BEFORE it — a mid-command `(` must not mark itself as
                // command position. `)` likewise: a stray `)` is a parse
                // error left to the parser, but after it a command follows.
                '\n' | ';' | '&' | '|' | ')' | '{' | '}' => {
                    at_command = true;
                    cur_word.clear();
                }
                '<' | '>' => {
                    // Redirect operator: a filename word follows, so `>(` is
                    // not command position.
                    cur_word.clear();
                }
                c if c.is_whitespace() => {
                    if !cur_word.is_empty() {
                        at_command = matches!(
                            cur_word.as_str(),
                            "if" | "then"
                                | "else"
                                | "elif"
                                | "while"
                                | "until"
                                | "do"
                                | "in"
                                | "!"
                                | "time"
                                | "coproc"
                                | "case"
                        );
                        cur_word.clear();
                    }
                }
                c if c.is_alphanumeric() || c == '_' || (c == '=' && !cur_word.is_empty()) => {
                    cur_word.push(c)
                }
                _ => {}
            }
        }
        let in_double = top.is_some_and(|d| d.close == '"');
        match ch {
            '\'' if !in_double => {
                // POSIX + Interp 221: `'` inside `"${...}"` is literal.
                if posix && squote_is_literal_in_posix_braced_dquote(&stack) {
                    comment_start = false;
                    i += 1;
                    continue;
                }
                // parse_matched_pair reports start_lineno for quotes.
                stack.push(UnclosedDelim {
                    close: '\'',
                    open_line: line,
                    escapes: false,
                    report_open: true,
                    funsub: false,
                    command: false,
                    term_ready: false,
                    case_depth_at_push: 0,
                    array_list: false,
                    pattern_paren: false,
                });
            }
            '"' => {
                stack.push(UnclosedDelim {
                    close: '"',
                    open_line: line,
                    escapes: true,
                    report_open: true,
                    funsub: false,
                    command: false,
                    term_ready: false,
                    case_depth_at_push: 0,
                    array_list: false,
                    pattern_paren: false,
                });
            }
            '`' => {
                stack.push(UnclosedDelim {
                    close: '`',
                    open_line: line,
                    escapes: true,
                    report_open: true,
                    funsub: false,
                    command: false,
                    term_ready: false,
                    case_depth_at_push: 0,
                    array_list: false,
                    pattern_paren: false,
                });
            }
            '$' => {
                match chars.get(i + 1) {
                    Some('{') => {
                        // parse.y:5506: `${' followed by a FUNSUB_CHAR is a
                        // function substitution parsed as commands; a
                        // parameter expansion takes parse_matched_pair
                        // (yyerror path: EOF line either way).
                        // FUNSUB_CHAR is parser.h:85 (`#else' arm): blank,
                        // newline or `|' only — NOT `(' (that spelling is
                        // the `#if 0' dead arm at parser.h:83), so
                        // `${(M)x}' is a parameter brace whose first
                        // unquoted `}' closes (P_FIRSTCLOSE).
                        let funsub = chars
                            .get(i + 2)
                            .is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '|'));
                        stack.push(UnclosedDelim {
                            close: '}',
                            open_line: line,
                            escapes: true,
                            // `${param` is a parse_matched_pair: EOF names
                            // the `${` line. The `${ ' funsub variant is a
                            // command context and reports the EOF line.
                            report_open: !funsub,
                            funsub,
                            command: false,
                            term_ready: false,
                            case_depth_at_push: 0,
                            array_list: false,
                            pattern_paren: false,
                        });
                        if funsub {
                            comment_start = true;
                            // P380FIX (captain diff): the funsub body is a
                            // command context (parse.y:5506 — `${ cmds; }'),
                            // so its first word sits at command position like
                            // `$(`'s does; the `{` was consumed here without
                            // feeding the word machine and the FUNSUB_CHAR
                            // whitespace never restores the boundary.
                            case_word_boundary = true;
                        }
                        i += 1;
                    }
                    Some('(') => {
                        // $( EOF takes the yyerror path: line_number at EOF.
                        // `$((` is a single arithmetic construct parsed by
                        // parse_matched_pair instead: EOF names the `$(`
                        // line even when only the inner `)` was closed.
                        stack.push(UnclosedDelim {
                            close: ')',
                            open_line: line,
                            escapes: true,
                            report_open: chars.get(i + 2) == Some(&'('),
                            funsub: false,
                            command: false,
                            term_ready: false,
                            case_depth_at_push: case_depth,
                            array_list: false,
                            pattern_paren: false,
                        });
                        // A fresh substitution body starts at a token
                        // boundary: `$(#c` is a comment.
                        comment_start = true;
                        // P380FIX (captain diff): the comsub body is a fresh
                        // command stream (subst.c:7143 command_substitute ->
                        // parse_and_execute; parse.y:4451 parse_comsub), so
                        // its first word sits at command position and IS a
                        // reserved word — `$(case y in ...)` must open the
                        // case-depth machine. The `(` is consumed here
                        // without feeding the word machine, and the `$` feed
                        // cleared the boundary, so restore it explicitly
                        // (skip_cmd_subst enters after `$(` with
                        // word_boundary=true — same invariant). rubash#380.
                        if chars.get(i + 2) != Some(&'(') {
                            case_word_boundary = true;
                        }
                        if chars.get(i + 2) == Some(&'(') {
                            // $(( ... )) arithmetic nests a second ')' and is
                            // parsed by parse_matched_pair: start_lineno.
                            stack.push(UnclosedDelim {
                                close: ')',
                                open_line: line,
                                escapes: true,
                                report_open: true,
                                funsub: false,
                                command: false,
                                term_ready: false,
                                case_depth_at_push: case_depth,
                                array_list: false,
                                pattern_paren: false,
                            });
                            i += 1;
                        }
                        i += 1;
                    }
                    Some('\'')
                        if top
                            .is_none_or(|d| d.close == '}' || d.close == ')' || d.close == '`') =>
                    {
                        // ANSI-C $'...': single-quote close, escapes live.
                        // parse.y:4062-4068 parse_matched_pair: inside a
                        // grouping construct ($(...), ${...}, subshell,
                        // backtick) a `$'` opens a nested P_ALLOWESC unit —
                        // its \' escapes stay inside and the enclosing
                        // construct's quote state never sees them
                        // (rubash#222/t0286: `${foo/$a/$''}` must not read
                        // as an unclosed `'`). Double quotes stay excluded:
                        // `"` is not in the guard set.
                        stack.push(UnclosedDelim {
                            close: '\'',
                            open_line: line,
                            escapes: true,
                            report_open: true,
                            funsub: false,
                            command: false,
                            term_ready: false,
                            case_depth_at_push: 0,
                            array_list: false,
                            pattern_paren: false,
                        });
                        i += 1;
                    }
                    _ => {}
                }
            }
            '(' if top.is_none() => {
                // Command-position `(` opens a subshell; `name=(` in an
                // assignment word opens an array list (`declare -a ddd=(aaa`
                // continues on the next line). A `(` elsewhere is a parse
                // error for the parser, not a pending delimiter.
                if at_command || (cur_word.len() > 1 && cur_word.ends_with('=')) {
                    let is_subshell = at_command && cur_word.is_empty();
                    let is_array_list = !is_subshell;
                    stack.push(UnclosedDelim {
                        close: ')',
                        open_line: line,
                        escapes: true,
                        // Array lists take parse_matched_pair's start_lineno
                        // report (`ddd=(aaa` EOF names the `(` line); a
                        // command-position subshell reports the EOF line.
                        report_open: !is_subshell,
                        funsub: false,
                        // GNU yyerror "from `(' command" applies only to a
                        // command-position subshell; `name=(...` is an
                        // array-list matched pair ("matching `)'"). The
                        // `at_command` flag is only refreshed at word
                        // boundaries, so a pending `name=` word means this
                        // `(` is array text, not a subshell.
                        command: is_subshell,
                        term_ready: false,
                        case_depth_at_push: case_depth,
                        array_list: is_array_list,
                        pattern_paren: false,
                    });
                }
                comment_start = true;
                at_command = true;
                cur_word.clear();
            }
            '(' if top.is_some_and(|d| d.close == ')' || (d.close == '}' && d.funsub)) => {
                // Subshell nested inside `$(...)`/`( ... )`/`${ ...; }`:
                // the body is command context where `(` is legal. An
                // immediately adjacent `(` (`((x`) is the arithmetic
                // construct's inner paren — a matched pair, not a command.
                stack.push(UnclosedDelim {
                    close: ')',
                    open_line: line,
                    escapes: true,
                    // `((x` arithmetic takes parse_matched_pair's
                    // start_lineno report like `$((` does; a real nested
                    // subshell reports the EOF line (yyerror).
                    report_open: i > 0 && chars[i - 1] == '(',
                    funsub: false,
                    command: !(i > 0 && chars[i - 1] == '('),
                    term_ready: false,
                    case_depth_at_push: case_depth,
                    array_list: false,
                    pattern_paren: case_depth > 0 && case_pattern_region,
                });
                comment_start = true;
            }
            '{' if top.is_some_and(|d| d.close == '}' && d.funsub) => {
                // `{ cmd; }' group inside a function substitution: `}' only
                // closes after a command terminator, same rule.
                stack.push(UnclosedDelim {
                    close: '}',
                    open_line: line,
                    escapes: true,
                    report_open: false,
                    funsub: true,
                    command: true,
                    term_ready: false,
                    case_depth_at_push: 0,
                    array_list: false,
                    pattern_paren: false,
                });
            }
            _ => {}
        }
        i += 1;
    }
    let d = *stack.last()?;
    // GNU parse.y:6883 yyerror path reports `line_number` at EOF. The lexer
    // consumes the final physical line before seeing EOF, so the reported
    // line is the last content line + 1: for input ending in '\n' `line`
    // already counts the (empty) next line; without a trailing newline the
    // pending last line still has to be stepped past.
    let eof_line = if input.ends_with('\n') {
        line
    } else {
        line + 1
    };
    Some((
        d.close,
        d.open_line,
        eof_line,
        d.report_open,
        d.command,
        d.array_list,
    ))
}

/// rubash#463: True when the word token's scanned span `raw` ends inside an
/// unclosed word-attached process substitution. GNU read_token_word's
/// shellexp arm (parse.y:5494-5524) folds the `(...)` body of a `<(`/`>(`
/// that directly follows word text into the SAME token, and the
/// parse_matched_pair LEX_GTLT discipline (parse.y:4147-4153) reads
/// newlines as ordinary body characters — so a body spanning physical lines
/// keeps read_secondary_line pulling input until the matching `)` arrives.
/// Only the span-final state matters: an earlier `<(`/`>(` that already
/// closed cannot hold the word open, and a body-closing `)` followed by more
/// word text re-arms nothing.
///
/// Quote/escape/backtick aware with nested-paren depth (`$( ... )` bodies
/// contribute their own balanced parens, mirroring skip_cmd_subst's depth
/// discipline). This is the JOIN GATE's oracle — the full re-lex of the
/// joined logical line re-runs the real word scanner (skip_word_inner's
/// `<(`/`>(` arm), so the gate only needs to answer "did the word scan hit
/// end-of-line inside the body".
pub(crate) fn word_ends_in_open_process_substitution(raw: &str) -> bool {
    let chars: Vec<char> = raw.chars().collect();
    let len = chars.len();
    let mut index = 0usize;
    let mut single = false;
    let mut double = false;
    while index < len {
        let ch = chars[index];
        if single {
            if ch == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if double {
            if ch == '\\' {
                index += 2;
                continue;
            }
            if ch == '"' {
                double = false;
            }
            index += 1;
            continue;
        }
        match ch {
            '\\' => index += 2,
            '\'' => {
                single = true;
                index += 1;
            }
            '"' => {
                double = true;
                index += 1;
            }
            '`' => {
                // Backtick unit: the body scan never sees its interior.
                index += 1;
                while index < len && chars[index] != '`' {
                    if chars[index] == '\\' {
                        index += 1;
                    }
                    index += 1;
                }
                index += 1;
            }
            '<' | '>'
                if chars.get(index + 1) == Some(&'(')
                    && index > 0
                    && !" \t\r\n|&;<>()".contains(chars[index - 1]) =>
            {
                // Word-attached introducer (GNU syntax.h:84 shellexp): scan
                // the `(...)` body with the same quote/escape discipline.
                let mut depth = 1usize;
                let mut scan = index + 2;
                let mut body_single = false;
                let mut body_double = false;
                while scan < len && depth > 0 {
                    let body_ch = chars[scan];
                    if body_single {
                        if body_ch == '\'' {
                            body_single = false;
                        }
                    } else if body_double {
                        if body_ch == '\\' {
                            scan += 1;
                        } else if body_ch == '"' {
                            body_double = false;
                        }
                    } else {
                        match body_ch {
                            '\\' => scan += 1,
                            '\'' => body_single = true,
                            '"' => body_double = true,
                            '(' => depth += 1,
                            ')' => depth -= 1,
                            _ => {}
                        }
                    }
                    scan += 1;
                }
                if depth > 0 {
                    return true;
                }
                index = scan;
                continue;
            }
            _ => index += 1,
        }
    }
    false
}

pub(super) fn has_unclosed_quotes(input: &str) -> bool {
    // TODO(parse.y): Bash reads parser input with full quoting state,
    // continuations, command substitutions, arithmetic contexts, and here-doc
    // deferral. This tracks only enough single/double quote state to keep a
    // multi-line alias definition as one parser unit.
    //
    // Crucially, quote characters that live *inside* a parameter expansion
    // (`${...}`), command substitution (`$(...)` / backticks), or ANSI-C string
    // (`$'...'`) must NOT toggle the surrounding word's quote state. GNU Bash
    // scans these nested contexts with their own quote rules; a `'` inside
    // `${IFS+'}'z}` is balanced within the expansion and must not leak a
    // dangling single-quote into the rest of the line.
    //
    // rubash#292 plan-B shape (perf8): this is the full-buffer oracle; the
    // join gate (GroupScanFeeder::advance_quotes_scan) drives
    // quotes_residuals_advance incrementally over the same bytes.
    let chars = input.chars().collect::<Vec<_>>();
    let mut state = QuotesResidualState::default();
    let _ = quotes_residuals_advance(&chars, 0, &mut state);
    state.is_open()
}

/// Every scan local of the unclosed-quote checker above, checkpointable
/// across physical lines (rubash#292 plan-B shape, perf8 lane).
///
/// GNU anchor: parse.y:3557 read_token — a streaming reader whose quote
/// state (parse.y:5305 read_token_word) advances character by character and
/// never re-scans consumed text. The full-buffer loop above is this port's
/// substitute for that streaming model; its locals are a left-to-right DFA
/// over the same bytes, so in isolation they are resumable: carrying them
/// across a physical-line boundary reproduces the full-buffer scan bit for
/// bit. The one thing that is NOT position-local is the set of atomic
/// forward skips (`${` span scan, `$(...)`/backtick unit skips): their
/// outcome is a function of text up to the END of the current buffer, so a
/// skip that failed on a shorter prefix may succeed on a longer one.
/// [`quotes_residuals_advance`] reports those positions as a
/// [`QuotesResidualPark`] so the caller re-derives the decision instead of
/// committing the shorter prefix's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QuotesResidualState {
    pub(crate) single: bool,
    pub(crate) double: bool,
    pub(crate) ansi_single: bool,
    pub(crate) escaped: bool,
    pub(crate) comment_start: bool,
    pub(crate) in_comment: bool,
}

impl Default for QuotesResidualState {
    fn default() -> Self {
        // The original scan's initial locals: only comment_start starts set.
        Self {
            single: false,
            double: false,
            ansi_single: false,
            escaped: false,
            comment_start: true,
            in_comment: false,
        }
    }
}

impl QuotesResidualState {
    /// The join gate's question: does any quote flag still keep the logical
    /// line open?
    pub(crate) fn is_open(&self) -> bool {
        self.single || self.double || self.ansi_single
    }
}

/// A re-derivation point returned by [`quotes_residuals_advance`]: the
/// scan's trajectory from `pos` onward depends on a forward decision the
/// current buffer leaves undecided (a `${` span, `$(` unit or backtick unit
/// whose atomic skip failed, or a unit that closed through an `esac)`
/// lookahead that ran off the end), so a longer buffer may take a different
/// branch there. Resuming at `pos` with `snapshot` re-runs that decision
/// with the longer buffer's text — exactly what the full scan of the longer
/// buffer does (same soundness rules as [`ComsubResidualPark`], rubash#292).
pub(crate) struct QuotesResidualPark {
    /// Char index (absolute, into the buffer passed to the advance fn)
    /// where the undecided decision starts.
    pub(crate) pos: usize,
    /// Scan state at `pos`, before the undecided arm mutated anything.
    pub(crate) snapshot: QuotesResidualState,
}

/// Advance the unclosed-quote scan over `chars[from..]` starting from
/// `state` (restored from a previous checkpoint or `Default`), leaving
/// `state` as the scan's end state, and return the FIRST undecided
/// position, if any.
///
/// Equivalence contract (same discipline as `comsub_residuals_advance`):
/// scanning a buffer fully (`from == 0`, `Default`) reproduces the old
/// whole-buffer loop byte for byte (the arms below are its arms,
/// unchanged); resuming at a park re-derives every buffer-length-dependent
/// decision, so the end state after each prefix equals the full scan of
/// that prefix. Arms that only read position-local state (quote toggles,
/// `$'`, comments, escapes) need no park: the machine is memoryless given
/// position + state, and the '\n' separators the join loop inserts
/// guarantee no two-character lookahead (`$(`, `${`, `$'`) straddles a
/// resume boundary except after a backslash-continuation pop — which
/// invalidates the checkpoint instead (see the feeder).
pub(crate) fn quotes_residuals_advance(
    chars: &[char],
    from: usize,
    state: &mut QuotesResidualState,
) -> Option<QuotesResidualPark> {
    let mut index = from.min(chars.len());
    let mut park: Option<QuotesResidualPark> = None;

    while index < chars.len() {
        let ch = chars[index];
        if state.in_comment {
            if ch == '\n' {
                state.in_comment = false;
                state.comment_start = true;
            }
            index += 1;
            continue;
        }

        if state.escaped {
            state.escaped = false;
            state.comment_start = false;
            index += 1;
            continue;
        }

        // GNU parse.y:5546-5558 read_token_word → parse_matched_pair
        // (parse.y:3877) with P_ALLOWESC: inside `$'...'` a `\` escapes the
        // next byte (parse.y:3992 sets LEX_PASSNEXT, so `\'` never closes)
        // and the closing `'` is the ONLY other special character — the
        // nesting arms (parse.y:4052 shellquote → backtick/`$(`/quote
        // recursion) run only when `open != close`, so a backtick or `$(` in
        // an ANSI-C body is string data. Without this arm the backtick-skip
        // below fired mid-`$'...'` and consumed through the span's closing
        // quote into a following comment's backtick, leaving a dangling
        // ansi_single that held the line open to EOF (rubash#215:
        // `w=$'a\'b`c' # bug `>` reported "unexpected end of file").
        // Same arm shape as compound_residuals_advance below and
        // comsub_residuals_advance further down.
        if state.ansi_single {
            if ch == '\\' {
                state.escaped = true;
            } else if ch == '\'' {
                state.ansi_single = false;
            }
            state.comment_start = false;
            index += 1;
            continue;
        }

        if ch == '\n' && !state.single && !state.double && !state.ansi_single {
            state.comment_start = true;
            index += 1;
            continue;
        }

        if ch == '#' && !state.single && !state.double && !state.ansi_single && state.comment_start
        {
            state.in_comment = true;
            index += 1;
            continue;
        }

        if ch.is_whitespace() && !state.single && !state.double && !state.ansi_single {
            state.comment_start = true;
            index += 1;
            continue;
        }

        if ch == '\\' && (!state.single || state.ansi_single) {
            state.escaped = true;
            state.comment_start = false;
            index += 1;
            continue;
        }

        // Skip a parameter expansion `${...}` as a self-contained unit so that
        // quotes inside its word/operator body do not affect outer state.
        // Inside double quotes the unit must still be skipped: the quote
        // toggles below are dolbrace-style (`'` only when !double, `"` only
        // when !single), so body quotes of `"${IFS+"'"x ~ x'}"` would
        // otherwise leak a dangling double-quote state and report the line
        // as unclosed (posixexp2 case 28). The span scan is mode-dependent —
        // POSIX honors the Interp 221 big hammer (`'` literal, first `}`
        // closes), non-POSIX honors quotes — so try POSIX first and fall back
        // to the non-POSIX scan; if neither closes, the line is unclosed.
        //
        // rubash#281 (captain-applied perf3 proposal): zero-copy body
        // view via the rubash#185 chars API — the String copy
        // re-collected the ENTIRE remaining input per `${` (nvm.sh: 1644
        // occurrences -> O(n^2); prescan 4540ms -> 27ms). The slice
        // INCLUDES the `${` opener: scan_braced_parameter_body_chars
        // requires it at position 0 — slicing from index+2 would report
        // every span unclosed.
        //
        // perf8 park (rubash#292 discipline): a span that did not close on
        // THIS buffer is not decided — a longer buffer may close it and
        // jump past body quotes that the committed fall-through below let
        // toggle the outer state. A successful close IS decided (the span
        // scanner is a forward scan that stops at its own `}`; it never
        // reads past the close it found, so a longer buffer takes the same
        // trajectory).
        if ch == '$' && !state.single && chars.get(index + 1) == Some(&'{') {
            let body = &chars[index..];
            if !state.double {
                let context = crate::lexer::dolbrace::BraceContext {
                    outer_double_quote: false,
                    posix: false,
                    replacement_context: false,
                    initial_state: crate::lexer::dolbrace::DolbraceState::Param,
                };
                if let Some(scan) =
                    crate::lexer::dolbrace::scan_braced_parameter_body_chars(body, context)
                {
                    index += 2 + scan.end;
                    state.comment_start = false;
                    continue;
                }
                // Unterminated/odd expansion: fall through and let the caller
                // treat the input as having unclosed syntax — but park first
                // so a longer buffer re-derives the span decision.
                if park.is_none() {
                    park = Some(QuotesResidualPark {
                        pos: index,
                        snapshot: state.clone(),
                    });
                }
            } else {
                let mut closed = false;
                for posix_mode in [true, false] {
                    let context = crate::lexer::dolbrace::BraceContext {
                        outer_double_quote: true,
                        posix: posix_mode,
                        replacement_context: false,
                        initial_state: crate::lexer::dolbrace::DolbraceState::Param,
                    };
                    if let Some(scan) =
                        crate::lexer::dolbrace::scan_braced_parameter_body_chars(body, context)
                    {
                        index += 2 + scan.end;
                        state.comment_start = false;
                        closed = true;
                        break;
                    }
                }
                if closed {
                    continue;
                }
                // Neither scan closes the span: park (the trailing quote
                // toggles leave the state as-is and the caller reports
                // unclosed input, matching the lexer's own fallback swallow)
                // — a longer buffer may still close the span.
                if park.is_none() {
                    park = Some(QuotesResidualPark {
                        pos: index,
                        snapshot: state.clone(),
                    });
                }
            }
        }

        // Skip a command substitution `$(...)` (and `$((...))` arithmetic) as a
        // self-contained unit.
        // A balanced command substitution owns its nested quote state, even
        // when the substitution itself appears inside an outer double quote.
        //
        // perf8 parks (rubash#292 discipline): (P1) the atomic skip failed on
        // THIS buffer — the committed fall-through consumes `$` and `(` as
        // ordinary text, but a longer buffer may close the unit atomically
        // and jump its whole body (whose quote bytes then never toggle the
        // outer state); (P2) the unit closed through an `esac)` case-pattern
        // lookahead that ran off the buffer end undecided — future text can
        // flip the internal case depth and move (or undo) the closure point.
        // Every other internal decision of the skip is prefix-monotone.
        if ch == '$' && !state.single && chars.get(index + 1) == Some(&'(') {
            if let Some((end, unit_decided)) = skip_parenthesized_unit_ex(chars, index + 1) {
                if !unit_decided && park.is_none() {
                    park = Some(QuotesResidualPark {
                        pos: index,
                        snapshot: state.clone(),
                    });
                }
                index = end;
                state.comment_start = false;
                continue;
            }
            if park.is_none() {
                park = Some(QuotesResidualPark {
                    pos: index,
                    snapshot: state.clone(),
                });
            }
        }

        // Skip a backtick command substitution as a self-contained unit.
        // perf8 park: an unclosed backtick unit is not decided — the
        // committed fall-through lets the body's `"`/`'` bytes toggle the
        // outer quote state, but a longer buffer closes the unit and jumps
        // its body wholesale.
        if ch == '`' && !state.single && !state.double {
            if let Some(end) = skip_backtick_unit(chars, index) {
                index = end;
                state.comment_start = false;
                continue;
            }
            if park.is_none() {
                park = Some(QuotesResidualPark {
                    pos: index,
                    snapshot: state.clone(),
                });
            }
        }

        // perf19: TOP-LEVEL heredoc — the body is raw text. GNU
        // make_cmd.c:512 `make_here_document` (driven by parse.y:3120
        // gather_here_documents) reads the body through
        // `read_secondary_line` with NO quoting state; the lexer
        // (parse.y:5305 read_token_word) never processes a body character
        // and the reader continues past the terminator line
        // (parse.y:3557 read_token). A quote scan that toggles `'`/`"`
        // inside the body mis-derives the rest of the group (perf19: an
        // apostrophe in configure's `<<_ACEOF` C commentary held
        // `single` open across 19578 lines). Park while the terminator
        // has not arrived (perf17 prefix-stable rule); empty delimiter
        // keeps the fall-through.
        if !state.single
            && !state.double
            && !state.ansi_single
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) != Some(&'<')
        {
            if let Some((next, terminator_found)) = skip_heredoc_top_level(&chars, index) {
                if !terminator_found && park.is_none() {
                    park = Some(QuotesResidualPark {
                        pos: index,
                        snapshot: state.clone(),
                    });
                }
                state.comment_start = true;
                index = next;
                continue;
            }
        }
        // `<<<` here-string (parse.y:3690-3706): one three-character
        // operator — consume it atomically so its second `<` is never
        // re-read as a `<<` heredoc opener.
        if !state.single
            && !state.double
            && !state.ansi_single
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) == Some(&'<')
        {
            index += 3;
            continue;
        }

        if ch == '$' && !state.single && !state.double && chars.get(index + 1) == Some(&'\'') {
            state.ansi_single = true;
            state.comment_start = false;
            index += 2;
            continue;
        }

        match ch {
            '\'' if state.ansi_single => {
                state.ansi_single = false;
                state.comment_start = false;
            }
            '\'' if !state.double && !state.ansi_single => {
                state.single = !state.single;
                state.comment_start = false;
            }
            '"' if !state.single && !state.ansi_single => {
                state.double = !state.double;
                state.comment_start = false;
            }
            _ => {
                if !state.single && !state.double && !state.ansi_single {
                    state.comment_start = false;
                }
            }
        }
        index += 1;
    }

    park
}

const DECLARATION_COMMAND_WORDS: [&str; 5] = ["declare", "typeset", "local", "export", "readonly"];

fn is_identifier_head(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if is_identifier_head(first) => {}
        _ => return false,
    }
    chars.all(is_identifier_char)
}

/// `name=` / `name+=` / `name[subscript]=...` prefix (anything may follow `=`).
fn assignment_prefix_word(word: &str) -> bool {
    let Some(eq) = word.find('=') else {
        return false;
    };
    let head = &word[..eq];
    let head = head.strip_suffix('+').unwrap_or(head);
    let Some(open) = head.find('[') else {
        return valid_identifier(head);
    };
    head.ends_with(']') && valid_identifier(&head[..open])
}

/// True when the logical line ends inside an unclosed compound array
/// assignment: a `name=(` / `name+=(` word whose parentheses are still open
/// at end of line. GNU parse.y keeps reading physical lines until the
/// matching `)` closes the compound assignment word, so the line-oriented
/// collector must not finalize the logical line there (ISSUE #78: a
/// multi-line `plugins=(...)` rc assignment used to execute its elements as
/// commands).
///
/// GNU only continues for an assignment word in command position, in the
/// assignment-prefix region of a command (`x=1 a=(...`), or as an operand of
/// a declaration builtin (`declare -a b=(...`). Any other adjacent `name=(`
/// (`echo a=(b`) is an immediate syntax error in GNU, so it must NOT swallow
/// the following lines: the collector finalizes the logical line and the
/// parser reports the error.
pub(super) fn has_unclosed_compound_assignment(input: &str) -> bool {
    // rubash#292 plan-B shape (perf8): this is the full-buffer oracle; the
    // join gate (GroupScanFeeder::advance_compound_scan) drives
    // compound_residuals_advance incrementally over the same bytes.
    let chars = input.chars().collect::<Vec<_>>();
    let mut state = CompoundResidualState::default();
    let _ = compound_residuals_advance(&chars, 0, &mut state);
    state.is_open()
}

/// Every scan local of the unclosed-compound-assignment checker,
/// checkpointable across physical lines (rubash#292 plan-B shape, perf8
/// lane).
///
/// GNU anchor: parse.y:3557 read_token streams tokens (the `name=(`
/// compound-assignment word keeps the reader open until its matching `)`,
/// parse.y read_token_word assignment handling / gather the array list);
/// this checker's char-machine is a left-to-right DFA over the same bytes,
/// so its locals are resumable in isolation. The non-position-local
/// decisions are the atomic forward skips (`${` span, `$(...)`/backtick
/// units) plus the two EARLY TERMINALS (`return false` on an unbalanced
/// `$(`/backtick unit): both overrode any accumulated `compound_depth`,
/// which [`CompoundResidualState::forced_closed`] records so the parked
/// answer stays exact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompoundResidualState {
    pub(crate) compound_depth: usize,
    pub(crate) single: bool,
    pub(crate) double: bool,
    pub(crate) ansi_single: bool,
    pub(crate) escaped: bool,
    pub(crate) in_comment: bool,
    pub(crate) comment_start: bool,
    pub(crate) word: String,
    pub(crate) word_pure: bool,
    pub(crate) seen_command_word: bool,
    pub(crate) declaration_context: bool,
    /// An unbalanced `$(`/backtick unit forced the original loop's early
    /// `return false` — the terminal answer is false regardless of the
    /// accumulated `compound_depth` at that point.
    pub(crate) forced_closed: bool,
}

impl Default for CompoundResidualState {
    fn default() -> Self {
        // The original scan's initial locals: comment_start and word_pure
        // start set; everything else is empty/false.
        Self {
            compound_depth: 0,
            single: false,
            double: false,
            ansi_single: false,
            escaped: false,
            in_comment: false,
            comment_start: true,
            word: String::new(),
            word_pure: true,
            seen_command_word: false,
            declaration_context: false,
            forced_closed: false,
        }
    }
}

impl CompoundResidualState {
    /// The join gate's question: is a `name=(` compound assignment still
    /// open? `forced_closed` encodes the original scan's early `return
    /// false` (an unbalanced `$(`/backtick unit reports "no compound
    /// assignment" even at compound_depth > 0).
    pub(crate) fn is_open(&self) -> bool {
        !self.forced_closed && self.compound_depth > 0
    }
}

/// A re-derivation point returned by [`compound_residuals_advance`] (same
/// soundness rules as [`QuotesResidualPark`] / [`ComsubResidualPark`],
/// rubash#292).
pub(crate) struct CompoundResidualPark {
    /// Char index (absolute, into the buffer passed to the advance fn)
    /// where the undecided decision starts.
    pub(crate) pos: usize,
    /// Scan state at `pos`, before the undecided arm mutated anything.
    pub(crate) snapshot: CompoundResidualState,
}

/// Advance the compound-assignment scan over `chars[from..]` starting from
/// `state`, leaving `state` as the scan's end state, and return the FIRST
/// undecided position, if any. See [`quotes_residuals_advance`] for the
/// equivalence contract; the compound machine's parks are:
/// - a `${` span whose body scan did not close on this buffer (the
///   committed path steps past `${` and scans the body as ordinary text,
///   whose `(`/`)`/quote bytes a longer buffer would jump wholesale);
/// - a `$(` unit whose atomic skip failed (the original loop's early
///   `return false` — `forced_closed` — is committed for THIS prefix, and
///   the park re-derives the skip on the next line);
/// - a unit that closed through an off-the-end `esac)` lookahead;
/// - an unclosed backtick unit (early `return false`, same treatment).
pub(crate) fn compound_residuals_advance(
    chars: &[char],
    from: usize,
    state: &mut CompoundResidualState,
) -> Option<CompoundResidualPark> {
    let mut index = from.min(chars.len());
    let mut park: Option<CompoundResidualPark> = None;

    while index < chars.len() {
        let ch = chars[index];

        if state.in_comment {
            if ch == '\n' {
                state.in_comment = false;
                state.comment_start = true;
                if state.compound_depth == 0 {
                    state.seen_command_word = false;
                    state.declaration_context = false;
                }
            }
            index += 1;
            continue;
        }

        if state.escaped {
            state.escaped = false;
            state.comment_start = false;
            index += 1;
            continue;
        }

        if ch == '\n' && !state.single && !state.double && !state.ansi_single {
            state.comment_start = true;
            if state.compound_depth == 0 {
                if !state.word.is_empty() {
                    let mut seen_command_word = state.seen_command_word;
                    let mut declaration_context = state.declaration_context;
                    classify_top_level_word(
                        &state.word,
                        &mut seen_command_word,
                        &mut declaration_context,
                    );
                    state.seen_command_word = seen_command_word;
                    state.declaration_context = declaration_context;
                    state.word.clear();
                    state.word_pure = true;
                }
                state.seen_command_word = false;
                state.declaration_context = false;
            }
            index += 1;
            continue;
        }

        if ch == '#' && !state.single && !state.double && !state.ansi_single && state.comment_start
        {
            state.in_comment = true;
            index += 1;
            continue;
        }

        if ch.is_whitespace() && !state.single && !state.double && !state.ansi_single {
            state.comment_start = true;
            if state.compound_depth == 0 && !state.word.is_empty() {
                let mut seen_command_word = state.seen_command_word;
                let mut declaration_context = state.declaration_context;
                classify_top_level_word(
                    &state.word,
                    &mut seen_command_word,
                    &mut declaration_context,
                );
                state.seen_command_word = seen_command_word;
                state.declaration_context = declaration_context;
                state.word.clear();
                state.word_pure = true;
            }
            index += 1;
            continue;
        }

        if state.ansi_single {
            if ch == '\\' {
                state.escaped = true;
            } else if ch == '\'' {
                state.ansi_single = false;
            }
            state.comment_start = false;
            state.word_pure = false;
            index += 1;
            continue;
        }

        if ch == '\\' && !state.single {
            state.escaped = true;
            state.comment_start = false;
            state.word_pure = false;
            index += 1;
            continue;
        }

        if ch == '$' && !state.single && chars.get(index + 1) == Some(&'\'') {
            state.ansi_single = true;
            state.comment_start = false;
            state.word_pure = false;
            index += 2;
            continue;
        }

        // A `${...}` parameter expansion is opaque to parenthesis balancing.
        // perf8 park (rubash#292 discipline): a span that did not close on
        // THIS buffer is not decided — the committed path below steps past
        // `${` and scans the body as ordinary text (its `(`/`)` bytes
        // balancing parens, its quotes toggling), but a longer buffer closes
        // the span and jumps its whole body.
        if ch == '$' && !state.single && !state.double && chars.get(index + 1) == Some(&'{') {
            // rubash#281 (captain-applied perf3 proposal): zero-copy view;
            // slice includes the `${` opener per the chars API contract.
            let body = &chars[index..];
            let context = crate::lexer::dolbrace::BraceContext {
                outer_double_quote: false,
                posix: false,
                replacement_context: false,
                initial_state: crate::lexer::dolbrace::DolbraceState::Param,
            };
            if let Some(scan) =
                crate::lexer::dolbrace::scan_braced_parameter_body_chars(body, context)
            {
                index += 2 + scan.end;
            } else {
                if park.is_none() {
                    park = Some(CompoundResidualPark {
                        pos: index,
                        snapshot: state.clone(),
                    });
                }
                index += 2;
            }
            state.comment_start = false;
            state.word_pure = false;
            continue;
        }

        // A `$(...)` / `$((...))` command substitution is opaque to
        // parenthesis balancing; an unbalanced one is reported by
        // has_unclosed_command_substitution instead — the original loop
        // returned false IMMEDIATELY there (`forced_closed`), and the park
        // re-derives the skip so a longer buffer that closes the unit
        // resumes the scan instead.
        if ch == '$' && !state.single && chars.get(index + 1) == Some(&'(') {
            if let Some((end, unit_decided)) = skip_parenthesized_unit_ex(chars, index + 1) {
                if !unit_decided && park.is_none() {
                    park = Some(CompoundResidualPark {
                        pos: index,
                        snapshot: state.clone(),
                    });
                }
                index = end;
                state.comment_start = false;
                state.word_pure = false;
                continue;
            }
            if park.is_none() {
                park = Some(CompoundResidualPark {
                    pos: index,
                    snapshot: state.clone(),
                });
            }
            state.forced_closed = true;
            return park;
        }

        if ch == '`' && !state.single && !state.double {
            if let Some(end) = skip_backtick_unit(chars, index) {
                index = end;
                state.comment_start = false;
                state.word_pure = false;
                continue;
            }
            if park.is_none() {
                park = Some(CompoundResidualPark {
                    pos: index,
                    snapshot: state.clone(),
                });
            }
            state.forced_closed = true;
            return park;
        }

        if ch == '\'' && !state.double {
            state.single = !state.single;
            state.comment_start = false;
            state.word_pure = false;
            index += 1;
            continue;
        }

        if ch == '"' && !state.single {
            state.double = !state.double;
            state.comment_start = false;
            state.word_pure = false;
            index += 1;
            continue;
        }

        if state.single || state.double {
            index += 1;
            continue;
        }

        if ch == '(' && state.compound_depth == 0 {
            let opens_compound = state.word_pure
                && state
                    .word
                    .strip_suffix('=')
                    .map(|head| {
                        let head = head.strip_suffix('+').unwrap_or(head);
                        valid_identifier(head)
                    })
                    .unwrap_or(false)
                && (!state.seen_command_word || state.declaration_context);
            if opens_compound {
                state.compound_depth = 1;
                state.word.clear();
                state.word_pure = true;
                // A `#` may start a comment right after the opening paren.
                state.comment_start = true;
                index += 1;
                continue;
            }
            // A subshell/grouping paren ends the assignment-prefix region:
            // GNU reports `echo a=(b` immediately instead of continuing.
            if !state.word.is_empty() {
                let mut seen_command_word = state.seen_command_word;
                let mut declaration_context = state.declaration_context;
                classify_top_level_word(
                    &state.word,
                    &mut seen_command_word,
                    &mut declaration_context,
                );
                state.seen_command_word = seen_command_word;
                state.declaration_context = declaration_context;
                state.word.clear();
                state.word_pure = true;
            }
            state.seen_command_word = true;
            state.comment_start = true;
            index += 1;
            continue;
        }

        if ch == '(' && state.compound_depth > 0 {
            state.compound_depth += 1;
            state.comment_start = true;
            index += 1;
            continue;
        }

        if ch == ')' {
            if state.compound_depth > 0 {
                state.compound_depth -= 1;
                state.word.clear();
                state.word_pure = true;
            } else {
                if !state.word.is_empty() {
                    let mut seen_command_word = state.seen_command_word;
                    let mut declaration_context = state.declaration_context;
                    classify_top_level_word(
                        &state.word,
                        &mut seen_command_word,
                        &mut declaration_context,
                    );
                    state.seen_command_word = seen_command_word;
                    state.declaration_context = declaration_context;
                    state.word.clear();
                    state.word_pure = true;
                }
                state.seen_command_word = true;
            }
            state.comment_start = true;
            index += 1;
            continue;
        }

        if ch == ';' || ch == '|' || ch == '&' {
            if state.compound_depth == 0 && !state.word.is_empty() {
                let mut seen_command_word = state.seen_command_word;
                let mut declaration_context = state.declaration_context;
                classify_top_level_word(
                    &state.word,
                    &mut seen_command_word,
                    &mut declaration_context,
                );
                state.seen_command_word = seen_command_word;
                state.declaration_context = declaration_context;
                state.word.clear();
                state.word_pure = true;
                state.seen_command_word = false;
                state.declaration_context = false;
            }
            state.comment_start = true;
            index += 1;
            continue;
        }

        state.word.push(ch);
        state.comment_start = false;
        index += 1;
    }

    park
}

fn classify_top_level_word(
    word: &str,
    seen_command_word: &mut bool,
    declaration_context: &mut bool,
) {
    if !*seen_command_word && DECLARATION_COMMAND_WORDS.contains(&word) {
        *declaration_context = true;
        return;
    }
    if assignment_prefix_word(word) {
        // Assignment words keep the command inside its prefix region.
        return;
    }
    if *declaration_context && (word.starts_with('-') || word.starts_with('+')) {
        // Declaration builtin flags (`declare -a`).
        return;
    }
    *seen_command_word = true;
}

/// Skip a balanced `(` ... `)` unit starting at `open` (the index of the `(`),
/// returning the index just past the matching `)`. Returns `None` if unbalanced.
fn skip_parenthesized_unit(chars: &[char], open: usize) -> Option<usize> {
    skip_parenthesized_unit_ex(chars, open).map(|(end, _)| end)
}

/// rubash#292 plan B variant of [`skip_parenthesized_unit`]: the `Some`
/// payload additionally reports whether the closure decision is *decided*
/// by the current buffer. The unit's internal `esac)` case-pattern
/// lookahead (`case_pattern_starts_with_esac_chars_ex`) scans PAST the
/// unit's closing `)` looking for `;;` / `esac` / `)` evidence; when that
/// scan runs off the end of the buffer undecided, future text can flip the
/// internal case depth and move (or undo) this closure point, so the
/// caller must park at the `$(` instead of trusting the jump. Every other
/// internal decision is prefix-monotone (a forward scan that already
/// closed keeps its closure point on longer buffers).
fn skip_parenthesized_unit_ex(chars: &[char], open: usize) -> Option<(usize, bool)> {
    let mut depth = 0usize;
    let mut index = open;
    let mut single = false;
    let mut double = false;
    // A case pattern list owns its closing `)` (parse.y `case_item`), so the
    // pattern `)` must not balance the command-substitution parenthesis. Track
    // case depth the same way `has_unclosed_command_substitution` does; without
    // it `$(case a in a) echo x\nesac)` looks balanced at the pattern `)` and
    // the logical line is finalized before `esac)`.
    let mut case_depth = 0usize;
    let mut word = String::new();
    let mut word_boundary = true;
    let mut current_word_boundary = true;
    let mut parameter_depth = 0usize;
    // rubash#380: pattern-list region bit (parse.y:29 PST_CASEPAT).
    let mut case_pattern_region = false;
    // GNU read_token (parse.y:3630-3643): `#` introduces a comment only at
    // a token boundary — after whitespace, a separator (`;&|()<>`), or at
    // the start. `word.is_empty()` alone is wrong: `$`, quotes and other
    // non-alphanumeric word characters never reach `word`, so `$(echo $#)`
    // and `$(echo 'a'#b)` would misread `#` as a comment.
    let mut token_boundary = true;
    let mut undecided = false;
    while index < chars.len() {
        let ch = chars[index];
        if single {
            if ch == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if double {
            // GNU skip_double_quoted (parse.y): inside double quotes a
            // backslash escapes the following character, so `\"` does not
            // close the quote and neither member of the pair can balance a
            // parenthesis. Consume both characters; a backslash before a
            // non-special char is harmless to skip since neither byte is
            // significant to this balancer.
            if ch == '\\' {
                index += 2;
                continue;
            }
            if ch == '"' {
                double = false;
            }
            index += 1;
            continue;
        }
        // A `<<<` here-string is ONE redirection operator (GNU read_token
        // builds the operator word from consecutive `<`s before dispatching
        // it, parse.y redirection handling), so the second `<` must not pair
        // with the third into a `<<` heredoc header — without this arm,
        // `$(grep -- x <<< "$v")` misreads `<<` mid-operator, the delimiter
        // scan swallows to end-of-input, and the atomic unit skip fails
        // every time (the outer machine's own `<<<` arms below consume the
        // three characters plainly). token_boundary mirrors the `<<` arm:
        // the operand after the operator begins a fresh token.
        if !single
            && !double
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) == Some(&'<')
        {
            token_boundary = true;
            index += 3;
            continue;
        }
        // Skip a here-document body so its ) and quote bytes stay opaque to
        // parenthesis balancing, mirroring how GNU make_here_document reads
        // the body from the input stream (parse.y gather_here_documents)
        // instead of feeding it back to the token scanner. Matches the heredoc
        // skip already present in has_unclosed_command_substitution below.
        if !single
            && !double
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) != Some(&'<')
        {
            let (next, closes) = skip_heredoc_in_chars_with_closure(chars, index);
            if closes.is_some() {
                return Some((next, !undecided));
            }
            index = next;
            // A heredoc terminator ends on its own line, so the next
            // character begins a fresh token.
            token_boundary = true;
            continue;
        }
        // `parameter_depth` keeps `${#x}` out of the comment rule.
        if ch == '#' && token_boundary && parameter_depth == 0 {
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
        if ch == '$' && chars.get(index + 1) == Some(&'{') {
            parameter_depth += 1;
            token_boundary = false;
            index += 2;
            continue;
        }
        if ch == '}' && parameter_depth > 0 {
            parameter_depth -= 1;
            token_boundary = false;
            index += 1;
            continue;
        }
        // Quoted text is a literal word and cannot begin a reserved word.
        if !single && !double {
            update_command_substitution_case_depth_ex(
                chars,
                index,
                ch,
                &mut word,
                &mut case_depth,
                &mut word_boundary,
                &mut current_word_boundary,
                &mut undecided,
                &mut case_pattern_region,
            );
            // GNU read_token_word (parse.y:5377-5397): outside quotes a
            // backslash quotes the next character — it can never act as
            // a paren delimiter, so `$(echo \)` does not close the
            // substitution (comsub-posix.tests:42). The quoted character is
            // word text (a placeholder, since `c\ase` is not `case`), so a
            // following `#` stays mid-word (`\;#` in comsub1.sub); a quoted
            // newline is a line continuation, not word content.
            if ch == '\\' {
                if chars.get(index + 1).is_some_and(|next| *next != '\n') {
                    word.push('\u{1}');
                }
                token_boundary = false;
                index += 2;
                continue;
            }
        }
        match ch {
            '\'' => single = true,
            '"' => double = true,
            '(' if case_depth == 0 => depth += 1,
            ')' if case_depth == 0 => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((index + 1, !undecided));
                }
            }
            _ => {}
        }
        token_boundary =
            ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
        index += 1;
    }
    None
}

/// Skip a backtick command substitution starting at `open` (the index of the
/// opening backtick), returning the index just past the closing backtick.
/// Returns `None` if unbalanced.
fn skip_backtick_unit(chars: &[char], open: usize) -> Option<usize> {
    let mut index = open + 1;
    while index < chars.len() {
        if chars[index] == '\\' {
            index += 2;
            continue;
        }
        if chars[index] == '`' {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

/// Residual unclosed `$(` depth after scanning `input` — how many top-level
/// `)` tokens a multi-line substitution still needs. Used by the parser's
/// stray-`)` guard: a `)` token is a legitimate closer while this is > 0.
pub(crate) fn unclosed_command_substitution_depth(input: &str) -> usize {
    comsub_residuals(input).0
}

pub(crate) fn has_unclosed_command_substitution(input: &str) -> bool {
    let (depth, backtick, ansi_single, parameter_depth) = comsub_residuals(input);
    depth > 0 || backtick || ansi_single || parameter_depth > 0
}

fn comsub_residuals(input: &str) -> (usize, bool, bool, usize) {
    let chars = input.chars().collect::<Vec<_>>();
    let mut state = ComsubResidualState::default();
    let _ = comsub_residuals_advance(&chars, 0, &mut state);
    (
        state.depth,
        state.backtick,
        state.ansi_single,
        state.parameter_depth,
    )
}

/// Every scan local of the command-substitution residual checker below,
/// checkpointable across physical lines (rubash#292 plan B).
///
/// GNU reads a script once, token by token (`parse.y:3557 read_token`): the
/// reader state advances per token and never re-scans consumed text. This
/// checker's char-machine is a left-to-right DFA over exactly the same
/// bytes, so its locals are, in isolation, resumable: carrying them across
/// a physical-line boundary reproduces the full-buffer scan bit for bit.
/// The one thing that is NOT position-local is the set of atomic forward
/// skips (`skip_parenthesized_unit`, `skip_arithmetic_substitution`, the
/// heredoc skip, the `esac)` case-pattern lookahead): their outcome is a
/// function of text up to the END of the current buffer, so a decision
/// that failed (or ran off the end undecided) on a shorter prefix may
/// succeed on a longer one. [`comsub_residuals_advance`] reports those
/// positions as a [`ComsubResidualPark`] so the caller re-derives the
/// decision instead of committing the shorter prefix's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ComsubResidualState {
    pub(crate) depth: usize,
    pub(crate) backtick: bool,
    pub(crate) single: bool,
    pub(crate) double: bool,
    pub(crate) ansi_single: bool,
    pub(crate) escaped: bool,
    pub(crate) comment_start: bool,
    pub(crate) in_comment: bool,
    pub(crate) case_depth: usize,
    pub(crate) parameter_depth: usize,
    pub(crate) parameter_single: bool,
    pub(crate) word: String,
    pub(crate) word_boundary: bool,
    pub(crate) current_word_boundary: bool,
    /// rubash#380: pattern-list region bit (parse.y:29 PST_CASEPAT).
    pub(crate) case_pattern_region: bool,
}

impl Default for ComsubResidualState {
    fn default() -> Self {
        // The original scan's initial locals: only the three boundary
        // flags start set.
        Self {
            depth: 0,
            backtick: false,
            single: false,
            double: false,
            ansi_single: false,
            escaped: false,
            comment_start: true,
            in_comment: false,
            case_depth: 0,
            parameter_depth: 0,
            parameter_single: false,
            word: String::new(),
            word_boundary: true,
            current_word_boundary: true,
            case_pattern_region: false,
        }
    }
}

impl ComsubResidualState {
    /// The join gate's question: does any substitution depth or flag still
    /// keep the logical line open?
    pub(crate) fn is_open(&self) -> bool {
        self.depth > 0 || self.backtick || self.ansi_single || self.parameter_depth > 0
    }
}

/// A re-derivation point returned by [`comsub_residuals_advance`]: the
/// scan's trajectory from `pos` onward depends on a forward decision the
/// current buffer leaves undecided, so a longer buffer may take a
/// different branch there (an atomic unit skip that failed, a unit that
/// closed through an `esac)` lookahead that ran off the end, or a
/// top-level-backtick heredoc whose terminator line has not arrived).
/// Resuming the scan at `pos` with `snapshot` re-runs that decision with
/// the longer buffer's text — exactly what the full scan of the longer
/// buffer does — which is what keeps the per-prefix answers identical to
/// the full-buffer scan this checkpoint replaces (same soundness rules as
/// `BraceScanResume`, rubash#176/#178/#241).
pub(crate) struct ComsubResidualPark {
    /// Char index (absolute, into the buffer passed to the advance fn)
    /// where the undecided decision starts.
    pub(crate) pos: usize,
    /// Scan state at `pos`, before the undecided arm mutated anything.
    pub(crate) snapshot: ComsubResidualState,
}

/// Advance the residual scan over `chars[from..]` starting from `state`
/// (restored from a previous checkpoint or `Default`), leaving `state` as
/// the scan's end state, and return the FIRST undecided position, if any.
///
/// Equivalence contract: scanning a buffer fully (`from == 0`, `Default`)
/// reproduces the old whole-buffer loop byte for byte (the arms below are
/// its arms, unchanged); resuming at a park re-derives every
/// buffer-length-dependent decision, so the end state after each prefix
/// equals the full scan of that prefix. Arms that only read
/// position-local state (quotes, `$'`, `${`, comments, escapes) need no
/// park: the char-machine is memoryless given position + state, and the
/// '\n' separators the join loop inserts guarantee no two-character
/// lookahead (`$(`, `${`, `$'`, `<<`) straddles a resume boundary except
/// after a backslash-continuation pop — which invalidates the checkpoint
/// instead (see the feeder).
pub(crate) fn comsub_residuals_advance(
    chars: &[char],
    from: usize,
    state: &mut ComsubResidualState,
) -> Option<ComsubResidualPark> {
    let mut index = from.min(chars.len());
    let mut park: Option<ComsubResidualPark> = None;

    while index < chars.len() {
        let ch = chars[index];
        if state.in_comment {
            if ch == '\n' {
                state.in_comment = false;
                state.comment_start = true;
            }
            index += 1;
            continue;
        }
        if state.escaped {
            state.escaped = false;
            state.comment_start = false;
            // A backslash-quoted character is word text (placeholder: `c\ase`
            // is not `case`), so a following `#` stays mid-word (`\;#` in
            // comsub1.sub). A quoted newline is handled by the `\` arm.
            state.word.push('\u{1}');
            index += 1;
            continue;
        }
        if ch == '\n'
            && !state.single
            && !state.double
            && !state.ansi_single
            && !state.backtick
            && state.depth == 0
        {
            state.comment_start = true;
            index += 1;
            continue;
        }
        if ch == '#'
            && !state.single
            && !state.double
            && !state.ansi_single
            && !state.backtick
            && state.depth == 0
            && state.comment_start
        {
            state.in_comment = true;
            index += 1;
            continue;
        }
        if ch.is_whitespace()
            && !state.single
            && !state.double
            && !state.ansi_single
            && !state.backtick
            && state.depth == 0
        {
            state.comment_start = true;
            index += 1;
            continue;
        }
        if state.ansi_single {
            if ch == '\\' {
                state.escaped = true;
            } else if ch == '\'' {
                state.ansi_single = false;
            }
            state.comment_start = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !state.single {
            state.escaped = true;
            state.comment_start = false;
            index += 1;
            continue;
        }
        if ch == '$' && !state.single && !state.double && chars.get(index + 1) == Some(&'\'') {
            state.ansi_single = true;
            state.comment_start = false;
            index += 2;
            continue;
        }
        if state.depth > 0 && ch == '`' && !state.single {
            index = skip_backtick_substitution(&chars, index);
            state.comment_start = false;
            continue;
        }
        if ch == '\'' && state.parameter_depth > 0 && !state.ansi_single {
            state.parameter_single = !state.parameter_single;
            index += 1;
            continue;
        }
        if ch == '\'' && !state.double {
            state.single = !state.single;
            state.comment_start = false;
            index += 1;
            continue;
        }
        if ch == '"' && !state.single {
            state.double = !state.double;
            state.comment_start = false;
            index += 1;
            continue;
        }
        if state.single {
            index += 1;
            continue;
        }
        if ch == '`' && state.depth == 0 {
            state.backtick = !state.backtick;
            state.comment_start = false;
            index += 1;
            continue;
        }
        if ch == '$'
            && chars.get(index + 1) == Some(&'{')
            && !state.single
            && !state.ansi_single
            && !state.parameter_single
        {
            state.parameter_depth += 1;
            // parse.y parse_matched_pair: inside `${...}` a `#` is parameter
            // operator text (the length operator in `${#x}`), never a
            // comment introducer — comment_start must clear here like every
            // other consumed non-space character, or `${#x}` is swallowed
            // to EOL and the `}` is never matched.
            state.comment_start = false;
            index += 2;
            continue;
        }
        if ch == '}' && state.parameter_depth > 0 {
            state.parameter_depth = state.parameter_depth.saturating_sub(1);
            state.parameter_single = false;
            state.comment_start = false;
            index += 1;
            continue;
        }
        // A balanced command substitution owns its nested quote state even
        // inside an outer double-quoted word. Keep it atomic here as well as
        // in has_unclosed_quotes; an unbalanced unit falls through so this
        // checker still reports the missing closing delimiter.
        if ch == '$'
            && !state.single
            && chars.get(index + 1) == Some(&'(')
            && !state.parameter_single
        {
            if let Some((end, unit_decided)) = skip_parenthesized_unit_ex(&chars, index + 1) {
                // rubash#292: the unit closed, but through an `esac)`
                // lookahead that ran off the end of the buffer undecided —
                // future text can move the closure point, so park at this
                // `$(` and re-derive on the next line.
                if !unit_decided && park.is_none() {
                    park = Some(ComsubResidualPark {
                        pos: index,
                        snapshot: state.clone(),
                    });
                }
                index = end;
                state.comment_start = false;
                continue;
            }
            // rubash#292: the atomic skip failed on THIS buffer; a longer
            // buffer may close the unit (with priority over the `$((`
            // arithmetic fallback below), so park before committing the
            // char-by-char fallback.
            if park.is_none() {
                park = Some(ComsubResidualPark {
                    pos: index,
                    snapshot: state.clone(),
                });
            }
            if chars.get(index + 2) == Some(&'(') {
                if let Some(end) = skip_arithmetic_substitution(&chars, index + 3) {
                    index = end;
                    state.comment_start = false;
                    continue;
                }
                // POSIX permits command substitution when the text after
                // "$((" is not a valid arithmetic expression.
            }
            state.depth += 1;
            if state.depth == 1 {
                state.case_depth = 0;
                state.word.clear();
                state.word_boundary = true;
                state.current_word_boundary = true;
            }
            // The fast skip failed, so the body is scanned char-by-char
            // from here — and a substitution body begins at a token
            // boundary: `$(#c` is a comment.
            state.comment_start = true;
            index += 2;
            continue;
        }
        // GNU read_token_word (parse.y:3630-3643): `#` at a token boundary
        // begins a comment through end of line. `comment_start` tracks that
        // boundary; `word.is_empty()` alone is wrong because `$`, quotes and
        // other non-alphanumeric word characters never reach `word`
        // (`$(echo $#)`). `parameter_depth` keeps `${#x}` parameter text
        // out of the comment rule.
        if state.depth > 0
            && ch == '#'
            && !state.single
            && !state.double
            && !state.ansi_single
            && !state.backtick
            && state.parameter_depth == 0
            && state.comment_start
        {
            while index + 1 < chars.len() && chars[index + 1] != '\n' {
                index += 1;
            }
            state.word.clear();
            state.word_boundary = true;
            state.current_word_boundary = true;
            state.comment_start = true;
            index += 1;
            continue;
        }
        if state.depth > 0 && !state.ansi_single && !state.backtick {
            // The `esac)` lookahead inside is re-derived by the enclosing
            // park whenever it runs off the buffer end (depth > 0 requires
            // a parked `$(` fallback below it), so its undecided status is
            // deliberately ignored here.
            update_command_substitution_case_depth(
                &chars,
                index,
                ch,
                &mut state.word,
                &mut state.case_depth,
                &mut state.word_boundary,
                &mut state.current_word_boundary,
                &mut state.case_pattern_region,
            );
        }
        if state.depth > 0
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) == Some(&'<')
        {
            index += 3;
            continue;
        }
        if state.depth > 0 && ch == '<' && chars.get(index + 1) == Some(&'<') {
            let (next, closes) = skip_heredoc_in_chars_with_closure(&chars, index);
            if closes.is_some() {
                state.depth = state.depth.saturating_sub(1);
                if state.depth == 0 {
                    return park;
                }
            }
            index = next;
            continue;
        }
        if state.backtick
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) == Some(&'<')
        {
            index += 3;
            continue;
        }
        if state.backtick && ch == '<' && chars.get(index + 1) == Some(&'<') {
            let (next, _closes, terminator_found) = skip_heredoc_in_chars_decided(&chars, index);
            // rubash#292 + perf17: park ONLY while the terminator line has
            // not arrived. Once it has, the heredoc's resume index is
            // prefix-stable — the terminator search compares whole lines
            // only (the join loop appends complete '\n'-terminated lines),
            // so a longer buffer cannot move the first match — and the
            // committed jump past the body is exactly what the fresh scan
            // of every longer prefix does. GNU never re-reads a consumed
            // heredoc body either (make_cmd.c:512 make_here_document reads
            // each body line once through read_secondary_line, then the
            // reader continues past the terminator; parse.y:3120
            // gather_here_documents, parse.y:3557 read_token streams).
            //
            // Before perf17 this arm parked on `closes.is_none()` — which
            // is also true for every heredoc whose HEADER line carries no
            // `)` (i.e. every ordinary heredoc inside an open backtick)
            // even after its terminator arrived, so the park never cleared
            // and every later candidate line re-derived the whole
            // accumulated tail: GNU bash's own configure -n parked once at
            // the `<<_ACEOF` of `cat confdefs.h - <<_ACEOF >conftest.$ac_ext`
            // and re-walked 534 M chars over 2406 candidate lines
            // (perf11's #1 leftover, 95% of the gather).
            if !terminator_found && park.is_none() {
                park = Some(ComsubResidualPark {
                    pos: index,
                    snapshot: state.clone(),
                });
            }
            index = next;
            continue;
        }
        // perf19: TOP-LEVEL heredoc (`depth == 0`, no open backtick, outside
        // quotes and `${...}`): the body is raw text. GNU make_cmd.c:512
        // `make_here_document` (driven by parse.y:3120 gather_here_documents)
        // reads the body line by line through `read_secondary_line` without
        // any quoting state — the lexer (parse.y:5305 read_token_word, fed by
        // parse.y:3557 read_token) never sees a body character and continues
        // PAST the terminator line. A group-completeness scan that instead
        // applies quote/backtick rules across the body mis-derives: bash's
        // own configure parks an `as_fn_*` heredoc whose body contains
        // `can't` (an apostrophe in C commentary) and reports the whole
        // remaining script as one open construct (perf19: 19578 lines, one
        // group). Skip uses `skip_heredoc_top_level`: the PST_EOFTOKEN
        // pushback branches (make_cmd.c:602-611, parse.y:4513 parse_comsub
        // sets the flag only inside a command substitution) are disabled —
        // at top level only the exact delimiter line terminates. Park while
        // the terminator has not arrived (perf17's prefix-stable rule: whole
        // lines only); an empty delimiter returns None and keeps the
        // pre-existing fall-through.
        if state.depth == 0
            && !state.backtick
            && !state.double
            && state.parameter_depth == 0
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) != Some(&'<')
        {
            if let Some((next, terminator_found)) = skip_heredoc_top_level(&chars, index) {
                if !terminator_found && park.is_none() {
                    park = Some(ComsubResidualPark {
                        pos: index,
                        snapshot: state.clone(),
                    });
                }
                // The jump lands just past the terminator line's '\n' — a
                // fresh line start, exactly the '\n' arm's comment_start.
                state.comment_start = true;
                index = next;
                continue;
            }
        }
        // `<<<` here-string (parse.y:3690-3706 read_token lexes the
        // three-character operator atomically): consume all three so the
        // second `<` is never re-read as a `<<` heredoc opener — the
        // same shape as the depth>0 arm above.
        if state.depth == 0
            && !state.backtick
            && !state.double
            && state.parameter_depth == 0
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) == Some(&'<')
        {
            index += 3;
            continue;
        }
        // GNU read_token_word (parse.y:5404-5418): inside double quotes a
        // `)` is literal text — it never balances a `$(` parenthesis. All
        // constructs that stay live inside `"..."` (`\x`, `$(`, `` ` ``,
        // `${`) were handled by the arms above; anything left is inert.
        if state.double {
            index += 1;
            continue;
        }
        if state.depth > 0 && state.case_depth == 0 && !state.ansi_single && ch == '(' {
            state.depth += 1;
        } else if state.depth > 0 && state.case_depth == 0 && !state.ansi_single && ch == ')' {
            state.depth -= 1;
        }
        if !state.single && !state.double && !state.ansi_single && !state.backtick {
            // GNU read_token: a token boundary follows whitespace and the
            // shell separators; every other live character continues or
            // begins a word, so a following `#` is mid-word text.
            state.comment_start =
                ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
        }
        index += 1;
    }

    // parameter_depth covers both ${param...} (GNU reads past newlines in
    // parse_matched_pair looking for the closing `}`) and the nofork
    // command substitution `${ command; }` (parser.h:83 FUNSUB_CHAR,
    // parse.y:5407 PST_FUNSUBST close) — comsub2.tests splits
    // `echo ${ printf ...` + `}` across lines and must keep reading.
    park
}

fn skip_backtick_substitution(chars: &[char], mut index: usize) -> usize {
    index += 1;
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
            return index + 1;
        }
        index += 1;
    }
    index
}

/// Skip a `$((...))` expansion while checking its own parenthesis and quote
/// state. Arithmetic `#` is an operator-context character, not a shell
/// comment introducer.
fn skip_arithmetic_substitution(chars: &[char], mut index: usize) -> Option<usize> {
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
        if !single && !double {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' && depth > 0 {
                depth -= 1;
            } else if ch == ')' && chars.get(index + 1) == Some(&')') {
                return Some(index + 2);
            }
        }
        index += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// perf9 (#292B family, third wave): the candidate-line TEXT scanners of
// stdin_source_text_needs_more, parked per appended group line.
//
// GNU anchor: parse.y:3557 read_token — a streaming reader that advances
// token by token and never re-scans consumed text. The group reader
// (script_driver.rs read_next_source_group) instead re-ran the FULL text
// scan battery over the whole accumulated `pending` group at every
// candidate line: has_unclosed_quotes, has_unclosed_command_substitution &&
// !command_substitutions_balanced, unclosed_array_subscript_line,
// unclosed_input_close_char_posix, and the function-body delimiters — an
// O(group^2) amplifier measured at 3.56 G chars per arm for GNU bash's own
// configure (24 753 lines): full `-n` 22.7 s vs GNU 5.3.0's 37 ms. The
// quotes and comsub residual machines above (perf8) cover the first two
// predicates; the machines below cover the remaining text scanners.
//
// Same discipline as the #292 family: each advance fn's arms are the
// oracle's arms, unchanged; every forward decision whose outcome depends on
// text past the current buffer end (a `$(` unit skip, a `${` span scan, an
// `esac)` case-pattern lookahead, a buffer-tail `${`/`$(`/`$'`/`\` whose
// two-character lookahead is not fully visible) parks at its position, so
// the next appended line re-derives exactly the decision the full scan of
// the longer buffer would make. The oracles in skip.rs stay authoritative
// and untouched (their callers unchanged); equivalence is enforced by the
// per-prefix + fuzz tests at the bottom of this file.
// ---------------------------------------------------------------------------

/// Mirror of skip.rs `skip_backtick_corrected` (verbatim; private there).
/// Prefix-monotone: `Some(end)` stops at its own closing backtick.
fn skip_backtick_corrected_chars(chars: &[char], mut index: usize) -> Option<usize> {
    index += 1;
    while index < chars.len() {
        if chars[index] == '\\' {
            index += 2;
            continue;
        }
        if chars[index] == '`' {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

/// Mirror of skip.rs `skip_arith_substitution_corrected` (verbatim; private
/// there). Prefix-monotone: the `))` close reads only two visible chars.
fn skip_arith_substitution_corrected_chars(chars: &[char], mut index: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    while index < chars.len() {
        let ch = chars[index];
        if single {
            if ch == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if double {
            if ch == '\\' {
                index += 2;
                continue;
            }
            if ch == '"' {
                double = false;
            }
            index += 1;
            continue;
        }
        match ch {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '\\' => {
                index += 2;
                continue;
            }
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            ')' if chars.get(index + 1) == Some(&')') => {
                return Some(index + 2);
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Mirror of skip.rs `case_pattern_starts_with_esac_rest` over the char
/// slice directly (the original materializes `[delimiter] + rest` per call;
/// both scan the same span starting at the delimiter). Returns
/// (starts_with_esac_chars, eof_based): `eof_based` marks every branch whose
/// answer consulted text up to the buffer end without concluding — the
/// park-relevant undecided signal for checkpoint callers.
fn case_pattern_starts_with_esac_rest_chars(all: &[char], at: usize) -> (bool, bool) {
    if !matches!(all.get(at), Some(')' | '|')) {
        return (false, false);
    }

    let mut close = at;
    while close < all.len() {
        match all[close] {
            ')' => break,
            ';' | '\n' => return (false, false),
            _ => close += 1,
        }
    }
    if all.get(close) != Some(&')') {
        // No `)` before the end of input: the answer was decided by EOF,
        // and more appended text could still supply the closer.
        return (false, true);
    }

    let mut scan = close + 1;
    let mut word = String::new();
    let mut word_boundary = true;
    while scan < all.len() {
        let ch = all[scan];
        if ch == ';' && all.get(scan + 1) == Some(&';') {
            // `;;` right after `esac)` can be either a case-list separator
            // (esac is a pattern) or an arithmetic-for separator that lives
            // *outside* the command substitution (esac is the keyword and `)`
            // closes the `$(...)`).  GNU arith-for.tests:
            //   for (( $(case x in x) esac);; )); do break; done
            // After `;;`, a `)` (possibly following whitespace/newlines)
            // closes an enclosing `$(...)` or `(( ))`; that cannot be a
            // case-list context, so `esac` is the keyword, not a pattern.
            let mut after = scan + 2;
            while after < all.len() && all[after].is_whitespace() {
                after += 1;
            }
            if all.get(after) == Some(&')') {
                return (false, false);
            }
            // Whitespace running to the end of input leaves the `)` check
            // undecided until more text arrives.
            return (true, after >= all.len());
        }
        if ch == '_' || ch.is_ascii_alphanumeric() {
            word.push(ch);
            scan += 1;
            continue;
        }
        if word == "esac" && word_boundary {
            return (true, false);
        }
        if ch == ')' {
            return (false, false);
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
    // The trailing word ends at end-of-input: a longer input could extend
    // it into `esac' (or past it), so this answer is EOF-based.
    (word == "esac" && word_boundary, true)
}

/// Mirror of skip.rs `update_command_substitution_case_depth` (verbatim
/// arms) plus the `undecided` report: the `esac` arm's case-pattern
/// lookahead consulted text past the buffer end without concluding.
#[allow(clippy::too_many_arguments)]
fn update_command_substitution_case_depth_corrected_ex(
    ch: char,
    single: bool,
    double: bool,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
    lookahead: (&[char], usize),
    case_in_stage: &mut u8,
    undecided: &mut bool,
    case_pattern_region: &mut bool,
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
        if *case_depth > 0 {
            if ch == ')' && *case_pattern_region {
                // parse.y:3787-3788: `)` closes the pattern list.
                *case_pattern_region = false;
            } else if ch == ';'
                && lookahead
                    .0
                    .get(lookahead.1 + 1)
                    .is_some_and(|next| *next == ';' || *next == '&')
            {
                // parse.y:3710/3759: `;;`, `;&`, `;;&` start the next
                // pattern list.
                *case_pattern_region = true;
            }
            if *case_pattern_region && *case_in_stage == 3 && matches!(ch, '(' | '|') {
                *case_in_stage = 4;
            }
        }
        if command_substitution_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    // The word completing while `case' awaits its subject IS the subject
    // (stage 1 -> 2); specific arms below may then rewrite the stage.
    let completing_after_case = *case_in_stage == 1;
    if completing_after_case {
        *case_in_stage = 2;
    }

    // parse.y:3177: inside PST_CASEPAT only ESAC may still be the keyword.
    let in_pattern_region = *case_pattern_region;
    let reserved_word_allows_next = match word.as_str() {
        "case" if *current_word_boundary && !in_pattern_region => {
            *case_depth += 1;
            *case_in_stage = 1;
            *case_pattern_region = false;
            false
        }

        "in" if *case_in_stage == 2 => {
            // GNU special_case_tokens rule 6 (parse.y:3369-3386): this `in'
            // follows the case subject, so it is the IN token even off a
            // reserved-word boundary (`in` after `case SUBJECT `).
            *case_in_stage = 3;
            // parse.y:3379/3396: the IN of a case arms the pattern region.
            *case_pattern_region = true;
            true
        }
        "esac" if *case_in_stage == 3 => {
            // GNU parse.y:3433-3441: `esac' directly after IN is ESAC —
            // the empty case `case WORD in esac'. Unconditional there, so
            // no case-pattern lookahead guard on this arm: the `)` right
            // after `esac` is a stray top-level token, exactly how GNU
            // reports `case x in esac) echo hi;; esac` (syntax error near
            // unexpected token `)', verified vs WSL GNU 5.3.0).
            *case_depth = case_depth.saturating_sub(1);
            *case_in_stage = 0;
            *case_pattern_region = false;
            true
        }
        "esac" if *current_word_boundary => {
            // P380FIX (captain diff): parse.y:3177-3186 — `esac' is
            // pattern text ONLY after a `|' or pattern-list `(' token
            // (backward previous-token test, whitespace-skipped); every
            // other boundary `esac' is the ESAC keyword. rubash#380.
            // The lazily-absent buffer (usize::MAX sentinel) reads as
            // "no pattern separator witnessed" → keyword, matching the
            // old forward fallback's answer for that shape.
            let word_start = lookahead.1.saturating_sub(word.chars().count());
            let mut back = word_start;
            while back > 0 && back <= lookahead.0.len() && lookahead.0[back - 1].is_whitespace() {
                back -= 1;
            }
            let prev_is_pattern_sep =
                back > 0 && back <= lookahead.0.len() && matches!(lookahead.0[back - 1], '|' | '(');
            if prev_is_pattern_sep {
                false
            } else {
                *case_depth = case_depth.saturating_sub(1);
                *case_in_stage = 0;
                *case_pattern_region = false;
                true
            }
        }
        "for" | "select" | "while" | "until" | "then" | "do" | "else" | "elif" | "in" | "fi"
        | "done"
            if *current_word_boundary && !in_pattern_region =>
        {
            *case_in_stage = 0;
            true
        }
        _ => {
            if !completing_after_case {
                *case_in_stage = 0;
            }
            false
        }
    };
    if *case_depth > 0 {
        if ch == ')' && *case_pattern_region {
            *case_pattern_region = false;
        } else if ch == ';'
            && lookahead
                .0
                .get(lookahead.1 + 1)
                .is_some_and(|next| *next == ';' || *next == '&')
        {
            *case_pattern_region = true;
        }
    }
    word.clear();
    *word_boundary =
        reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
}

/// Mirror of skip.rs `skip_parenthesized_unit_corrected` (the corrected
/// case-depth unit skipper; verbatim arms) reporting whether the closure is
/// *decided* by this buffer. A closure whose internal `esac)` case-pattern
/// lookahead (or a nested unit's closure) ran off the buffer end
/// undecided can move on a longer buffer, so checkpoint callers must park
/// at the `$(` instead of trusting the jump; every other internal decision
/// is prefix-monotone. skip.rs's original stays the authoritative oracle.
fn skip_parenthesized_unit_corrected_ex(chars: &[char], open: usize) -> Option<(usize, bool)> {
    let mut depth = 0usize;
    let mut index = open;
    let mut single = false;
    let mut double = false;
    let mut case_depth = 0usize;
    let mut word = String::new();
    let mut word_boundary = true;
    let mut current_word_boundary = true;
    let mut parameter_depth = 0usize;
    // rubash#380: pattern-list region bit (parse.y:29 PST_CASEPAT).
    let mut case_pattern_region = false;
    // `case WORD in' chain tracker (GNU special_case_tokens,
    // parse.y:3369-3386 + 3433-3441) — see the corrected case-depth update.
    let mut case_in_stage = 0u8;
    // GNU read_token (parse.y:3630-3643): `#` introduces a comment only at
    // a token boundary — after whitespace, a separator (`;&|()<>`), or at
    // the start. `word.is_empty()` alone is wrong: `$`, quotes and other
    // non-alphanumeric word characters never reach `word`, so `$(echo $#)`
    // and `$(echo 'a'#b)` would misread `#` as a comment.
    let mut token_boundary = true;
    let mut undecided = false;
    while index < chars.len() {
        let ch = chars[index];
        if single {
            if ch == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if double {
            if ch == '\\' {
                index += 2;
                continue;
            }
            if ch == '"' {
                double = false;
            }
            index += 1;
            continue;
        }
        // Skip heredoc body so its `)` chars stay opaque.
        // GNU parse.y read_token: `<<<` is the here-string redirection
        // operator (LESS_LESS_LESS) whose operand is the next ordinary
        // word — there is no body to skip, and the comsub's `)` after the
        // word still closes the substitution (`$(cat <<< hi)`). Check it
        // BEFORE the `<<` heredoc arm (rubash#168).
        if ch == '<' && chars.get(index + 1) == Some(&'<') && chars.get(index + 2) == Some(&'<') {
            index += 3;
            token_boundary = true;
            continue;
        }
        if ch == '<' && chars.get(index + 1) == Some(&'<') && chars.get(index + 2) != Some(&'<') {
            let (next, _closes) =
                super::heredoc_scan::skip_heredoc_in_chars_with_closure(chars, index);
            index = next;
            // A heredoc terminator ends on its own line, so the next
            // character begins a fresh token.
            token_boundary = true;
            continue;
        }
        // `parameter_depth` keeps `${#x}` text out of the comment rule.
        if ch == '#' && token_boundary && parameter_depth == 0 {
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
        if ch == '$' && chars.get(index + 1) == Some(&'{') {
            parameter_depth += 1;
            token_boundary = false;
            index += 2;
            continue;
        }
        if ch == '}' && parameter_depth > 0 {
            parameter_depth -= 1;
            token_boundary = false;
            index += 1;
            continue;
        }
        // The suffix is consulted ONLY by the corrected case-depth update's
        // `esac` arm — and that scan returns (false, false) without reading
        // the suffix unless the terminating char is `)` or `|` (mirror of
        // the skip.rs materialization rule; see
        // case_pattern_starts_with_esac_rest_chars). The rubash#380
        // previous-token witness additionally needs the buffer at EVERY
        // `esac` completion (a `(&[], usize::MAX)` sentinel stays possible
        // for other shapes).
        let lookahead: (&[char], usize) = if word == "esac" || ch == ';' {
            (chars, index)
        } else {
            (&[], usize::MAX)
        };
        update_command_substitution_case_depth_corrected_ex(
            ch,
            false,
            false,
            &mut word,
            &mut case_depth,
            &mut word_boundary,
            &mut current_word_boundary,
            lookahead,
            &mut case_in_stage,
            &mut undecided,
            &mut case_pattern_region,
        );
        match ch {
            '\'' => single = true,
            '"' => double = true,
            // GNU read_token_word (parse.y:5377-5397): outside quotes a
            // backslash quotes the next character — it can never act as a
            // paren delimiter, so `$(echo \)` does not close the
            // substitution (comsub-posix.tests:42). The quoted character is
            // word text (a placeholder, since `c\ase` is not `case`), so a
            // following `#` stays mid-word (`\;#` in comsub1.sub); a quoted
            // newline is a line continuation, not word content.
            '\\' => {
                if chars.get(index + 1).is_some_and(|next| *next != '\n') {
                    word.push('\u{1}');
                }
                token_boundary = false;
                index += 2;
                continue;
            }
            '`' => {
                if let Some(end) = skip_backtick_corrected_chars(chars, index) {
                    index = end;
                    token_boundary = false;
                    continue;
                }
            }
            '$' if chars.get(index + 1) == Some(&'\'') => {
                index += 2;
                while index < chars.len() {
                    if chars[index] == '\\' {
                        index += 2;
                        continue;
                    }
                    if chars[index] == '\'' {
                        index += 1;
                        break;
                    }
                    index += 1;
                }
                token_boundary = false;
                continue;
            }
            '$' if chars.get(index + 1) == Some(&'(') => {
                if chars.get(index + 2) == Some(&'(') {
                    if let Some(end) = skip_arith_substitution_corrected_chars(chars, index + 3) {
                        index = end;
                        token_boundary = false;
                        continue;
                    }
                } else if let Some((end, inner_decided)) =
                    skip_parenthesized_unit_corrected_ex(chars, index + 1)
                {
                    // A nested unit that closed undecidedly can move on a
                    // longer buffer, which moves this unit's resume point:
                    // propagate the undecided signal so the caller parks.
                    if !inner_decided {
                        undecided = true;
                    }
                    index = end;
                    token_boundary = false;
                    continue;
                }
            }
            '(' if case_depth == 0 => depth += 1,
            ')' if case_depth == 0 => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((index + 1, !undecided));
                }
            }
            _ => {}
        }
        token_boundary =
            ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
        index += 1;
    }
    None
}

/// Every scan local of the corrected command-substitution balance checker
/// (`skip::command_substitutions_balanced`, the oracle), checkpointable
/// across appended group lines (perf9, #292B family).
///
/// GNU anchor: parse.y:3557 read_token — a streaming reader; the oracle's
/// char-machine is a left-to-right DFA over the same bytes, so its locals
/// are resumable in isolation. The non-local decisions are the atomic
/// forward skips (`${` span, backtick unit, `$(...` unit + `$((...))`
/// arithmetic fallback) whose failure makes the oracle return `false`
/// immediately: a failure on a shorter prefix may succeed on a longer one,
/// so the machine parks at the failing opener and the next line re-derives
/// (the `failed` flag carries the oracle's committed `false` answer for the
/// current prefix).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BalancedResidualState {
    pub(crate) single: bool,
    pub(crate) double: bool,
    pub(crate) ansi_single: bool,
    pub(crate) escaped: bool,
    pub(crate) comment_start: bool,
    pub(crate) in_comment: bool,
    /// The oracle returned `false` at a failing skip on the current prefix.
    pub(crate) failed: bool,
}

impl Default for BalancedResidualState {
    fn default() -> Self {
        // The oracle's initial locals: only comment_start starts set.
        Self {
            single: false,
            double: false,
            ansi_single: false,
            escaped: false,
            comment_start: true,
            in_comment: false,
            failed: false,
        }
    }
}

impl BalancedResidualState {
    /// The oracle's answer for the current prefix:
    /// `!command_substitutions_balanced(input)`.
    pub(crate) fn is_unbalanced(&self) -> bool {
        self.failed
    }
}

/// A re-derivation point returned by [`balanced_residuals_advance`] (same
/// contract as [`ComsubResidualPark`]).
pub(crate) struct BalancedResidualPark {
    /// Char index where the undecided decision starts.
    pub(crate) pos: usize,
    /// Scan state at `pos`, before the undecided arm mutated anything.
    pub(crate) snapshot: BalancedResidualState,
}

/// Advance the corrected-balance scan over `chars[from..]` starting from
/// `state` (restored from a checkpoint or `Default`), leaving `state` as
/// the committed end state for this prefix, and return the FIRST undecided
/// position, if any.
///
/// Equivalence contract: a full advance (`from == 0`, `Default`) answers
/// exactly `!skip::command_substitutions_balanced(chars)` — `failed` is set
/// iff the oracle returned `false`; resuming at a park re-derives every
/// buffer-length-dependent decision, so each prefix's committed answer
/// equals the oracle's answer for that prefix. Parks:
/// (B1) a `${` span scan that did not close on this buffer (the oracle
/// returns false; a longer buffer may close it and jump its body quotes);
/// (B2) a backtick unit that did not close (same shape);
/// (B3) a `$(` whose corrected unit skip failed and whose `$((` arithmetic
/// fallback also failed (the oracle returns false);
/// (B4) a `$` at the buffer tail — the `$'`/`${`/`$(` two-character
/// lookahead is not visible;
/// (B5) a `$(` unit that closed through an `esac)` case-pattern lookahead
/// (or nested-unit closure) that ran off the buffer end undecided — the
/// jump is committed but the closure point can move, so the park re-derives
/// (commit + park, the scan continues, exactly like comsub's P2).
pub(crate) fn balanced_residuals_advance(
    chars: &[char],
    from: usize,
    state: &mut BalancedResidualState,
) -> Option<BalancedResidualPark> {
    let mut index = from.min(chars.len());
    let mut park: Option<BalancedResidualPark> = None;

    while index < chars.len() {
        let ch = chars[index];
        if state.in_comment {
            if ch == '\n' {
                state.in_comment = false;
                state.comment_start = true;
            }
            index += 1;
            continue;
        }
        if state.escaped {
            state.escaped = false;
            state.comment_start = false;
            index += 1;
            continue;
        }
        if ch == '\n' && !state.single && !state.double && !state.ansi_single {
            state.comment_start = true;
            index += 1;
            continue;
        }
        if ch == '#' && !state.single && !state.double && !state.ansi_single && state.comment_start
        {
            state.in_comment = true;
            index += 1;
            continue;
        }
        if ch.is_whitespace() && !state.single && !state.double && !state.ansi_single {
            state.comment_start = true;
            index += 1;
            continue;
        }
        if state.ansi_single {
            if ch == '\\' {
                state.escaped = true;
            } else if ch == '\'' {
                state.ansi_single = false;
            }
            state.comment_start = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !state.single {
            state.escaped = true;
            state.comment_start = false;
            index += 1;
            continue;
        }
        // B4: a `$` at the buffer tail — the `$'`/`${`/`$(` lookahead is
        // undecided. Commit the fall-through (below) and park at the `$`.
        if ch == '$' && chars.get(index + 1).is_none() && park.is_none() {
            park = Some(BalancedResidualPark {
                pos: index,
                snapshot: state.clone(),
            });
        }
        if ch == '$' && !state.single && !state.double && chars.get(index + 1) == Some(&'\'') {
            state.ansi_single = true;
            state.comment_start = false;
            index += 2;
            continue;
        }
        if ch == '\'' && !state.double && !state.ansi_single {
            state.single = !state.single;
            state.comment_start = false;
            index += 1;
            continue;
        }
        if ch == '"' && !state.single && !state.ansi_single {
            state.double = !state.double;
            state.comment_start = false;
            index += 1;
            continue;
        }
        if state.single {
            index += 1;
            continue;
        }
        // Skip ${...} parameter expansion so a `$(` inside it is not mistaken
        // for a top-level command substitution.
        if ch == '$' && chars.get(index + 1) == Some(&'{') && !state.double {
            let body = &chars[index..];
            let context = crate::lexer::dolbrace::BraceContext {
                outer_double_quote: state.double,
                posix: false,
                replacement_context: false,
                initial_state: crate::lexer::dolbrace::DolbraceState::Param,
            };
            if let Some(scan) =
                crate::lexer::dolbrace::scan_braced_parameter_body_chars(body, context)
            {
                index += 2 + scan.end;
                state.comment_start = false;
                continue;
            }
            // B1: unterminated ${...}: the oracle returns false; park so a
            // longer buffer re-derives the span decision. A park recorded
            // earlier in this pass (B5) is the re-derivation point — the
            // resume re-runs everything after it, this failing arm included.
            state.failed = true;
            return Some(park.take().unwrap_or(BalancedResidualPark {
                pos: index,
                snapshot: BalancedResidualState {
                    failed: false,
                    ..state.clone()
                },
            }));
        }
        // Skip backtick command substitution.
        if ch == '`' && !state.double {
            if let Some(end) = skip_backtick_corrected_chars(chars, index) {
                index = end;
                state.comment_start = false;
                continue;
            }
            // B2: the backtick unit did not close on this buffer (a park
            // recorded earlier in this pass stays the resume point).
            state.failed = true;
            return Some(park.take().unwrap_or(BalancedResidualPark {
                pos: index,
                snapshot: BalancedResidualState {
                    failed: false,
                    ..state.clone()
                },
            }));
        }
        if ch == '$' && !state.single && chars.get(index + 1) == Some(&'(') {
            if let Some((end, decided)) = skip_parenthesized_unit_corrected_ex(chars, index + 1) {
                // B5: closed undecidedly — commit the jump, park for the
                // re-derivation (first park wins).
                if !decided && park.is_none() {
                    park = Some(BalancedResidualPark {
                        pos: index,
                        snapshot: state.clone(),
                    });
                }
                index = end;
                state.comment_start = false;
                continue;
            }
            // Check for $((...)) arithmetic.
            if chars.get(index + 2) == Some(&'(') {
                if let Some(end) = skip_arith_substitution_corrected_chars(chars, index + 3) {
                    index = end;
                    state.comment_start = false;
                    continue;
                }
            }
            // B3: genuinely unbalanced command substitution (a park
            // recorded earlier in this pass stays the resume point).
            state.failed = true;
            return Some(park.take().unwrap_or(BalancedResidualPark {
                pos: index,
                snapshot: BalancedResidualState {
                    failed: false,
                    ..state.clone()
                },
            }));
        }
        // perf19: TOP-LEVEL heredoc — the body is raw text (GNU
        // make_cmd.c:512 make_here_document reads it through
        // read_secondary_line with no paren state; parse.y:3120
        // gather_here_documents, parse.y:3557 read_token stream past the
        // terminator). A balance scan that counts body parens
        // mis-derives (perf19: configure's `<<_ACEOF` C commentary held
        // the gate's `!balanced` half wrong for the rest of the file).
        // Park while the terminator has not arrived (perf17 prefix-stable
        // rule); empty delimiter keeps the fall-through.
        if !state.single
            && !state.double
            && !state.ansi_single
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) != Some(&'<')
        {
            if let Some((next, terminator_found)) = skip_heredoc_top_level(&chars, index) {
                if !terminator_found && park.is_none() {
                    park = Some(BalancedResidualPark {
                        pos: index,
                        snapshot: state.clone(),
                    });
                }
                state.comment_start = true;
                index = next;
                continue;
            }
        }
        // `<<<` here-string (parse.y:3690-3706): one operator, consumed
        // atomically so its second `<` is never a `<<` opener.
        if !state.single
            && !state.double
            && !state.ansi_single
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) == Some(&'<')
        {
            index += 3;
            continue;
        }
        if !state.single && !state.double && !state.ansi_single {
            state.comment_start = false;
        }
        index += 1;
    }
    park
}

/// Every scan local of the unclosed array-subscript checker
/// (`skip::unclosed_array_subscript_line`, the oracle), checkpointable
/// across appended group lines (perf9, #292B family).
///
/// GNU anchor: parse.y:5635-5643 read_token_word — an unquoted `[` at a
/// command-position identifier (or compound-assignment element start)
/// opens `parse_matched_pair ('[', ']', ..., P_ARRAYSUB)` (parse.y:3906),
/// a forward scan that reads across lines until the matching `]`; EOF
/// reports the `[` line (start_lineno). The oracle is this port's
/// whole-buffer substitute for that streaming read; its locals are a
/// left-to-right DFA over the same bytes, resumable in isolation. The
/// non-local decisions are the array-subscript matched pair itself (an
/// unclosed `[` makes the oracle RETURN its `Some((line, compassign))`
/// answer immediately), the `$(...` corrected unit skip, and the backtick
/// unit skip — each parks so the next line re-derives exactly what the
/// full scan of the longer buffer would decide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SubscriptResidualState {
    pub(crate) single: bool,
    pub(crate) double: bool,
    pub(crate) ansi_single: bool,
    pub(crate) escaped: bool,
    pub(crate) in_comment: bool,
    /// Command-position tracking (parse.y:5899 assignment_acceptable): true
    /// at input start, after a separator/operator or `('/`{', and after the
    /// reserved words that may be followed by a command.
    pub(crate) command_position: bool,
    pub(crate) word: String,
    /// Compound-assignment `name=( ... )` depth (PST_COMPASSIGN); at element
    /// start (right after `(` or whitespace inside the list) a `[` opens a
    /// subscript with no identifier prefix needed.
    pub(crate) compassign_depth: usize,
    pub(crate) element_start: bool,
    pub(crate) line: usize,
    /// The oracle returned `Some((line, compassign))` on the current
    /// prefix: an unclosed `[` swallowed the rest of the buffer
    /// (parse.y:5635: the subscript owns everything ahead of any other
    /// construct).
    pub(crate) reported: Option<(usize, bool)>,
}

impl Default for SubscriptResidualState {
    fn default() -> Self {
        // The oracle's initial locals (skip.rs unclosed_array_subscript_line).
        Self {
            single: false,
            double: false,
            ansi_single: false,
            escaped: false,
            in_comment: false,
            command_position: true,
            word: String::new(),
            compassign_depth: 0,
            element_start: false,
            line: 1,
            reported: None,
        }
    }
}

impl SubscriptResidualState {
    /// The oracle's answer for the current prefix:
    /// `unclosed_array_subscript_line(input).is_some()`.
    pub(crate) fn is_open(&self) -> bool {
        self.reported.is_some()
    }
}

/// A re-derivation point returned by [`subscript_residuals_advance`]
/// (same contract as [`ComsubResidualPark`]).
pub(crate) struct SubscriptResidualPark {
    /// Char index where the undecided decision starts.
    pub(crate) pos: usize,
    /// Scan state at `pos`, before the undecided arm mutated anything.
    pub(crate) snapshot: SubscriptResidualState,
}

/// Advance the array-subscript scan over `chars[from..]` starting from
/// `state` (restored from a checkpoint or `Default`), leaving `state` as
/// the committed end state for this prefix, and return the FIRST undecided
/// position, if any.
///
/// Equivalence contract: a full advance (`from == 0`, `Default`) leaves
/// `reported == skip::unclosed_array_subscript_line(chars)` — payload
/// included ((line, compassign_depth > 0) at the reporting `[`). Parks:
/// (S1) an array subscript `[` whose quote-aware matched-pair scan ran off
/// the buffer end — the oracle RETURNS its `Some` answer there, and a
/// longer buffer may close the pair and continue past it, so the park (at
/// the `[`) re-derives; `reported` carries the committed answer. The
/// advance stops there exactly like the oracle.
/// (S2) a `$(` whose corrected unit skip failed on this buffer (the oracle
/// falls through and treats `$` as word text; a longer buffer may close
/// the unit and skip its whole body) or closed undecidedly (esac-lookahead
/// closure; commit + park, scan continues).
/// (S3) a backtick whose unit did not close on this buffer (the oracle
/// consumes to the buffer end; a longer buffer closes it and jumps).
/// (S4) a `$` at the buffer tail — the `$'`/`$(` two-character lookahead
/// is not visible (the oracle pushes `$` to the word; a longer buffer may
/// take either arm).
pub(crate) fn subscript_residuals_advance(
    chars: &[char],
    from: usize,
    state: &mut SubscriptResidualState,
) -> Option<SubscriptResidualPark> {
    let mut index = from.min(chars.len());
    let mut park: Option<SubscriptResidualPark> = None;

    while index < chars.len() {
        let ch = chars[index];
        if state.in_comment {
            if ch == '\n' {
                state.in_comment = false;
                state.line += 1;
                state.command_position = true;
                state.word.clear();
                state.element_start = false;
            }
            index += 1;
            continue;
        }
        if state.escaped {
            state.escaped = false;
            index += 1;
            continue;
        }
        if state.ansi_single {
            if ch == '\\' {
                state.escaped = true;
            } else if ch == '\'' {
                state.ansi_single = false;
            }
            index += 1;
            continue;
        }
        if state.single {
            if ch == '\'' {
                state.single = false;
            }
            index += 1;
            continue;
        }
        if state.double {
            if ch == '\\' {
                state.escaped = true;
            } else if ch == '"' {
                state.double = false;
            }
            index += 1;
            continue;
        }
        // S4: a `$` at the buffer tail — park before the guards commit the
        // word-text fall-through.
        if ch == '$' && chars.get(index + 1).is_none() && park.is_none() {
            park = Some(SubscriptResidualPark {
                pos: index,
                snapshot: state.clone(),
            });
        }
        // perf19: TOP-LEVEL heredoc — the body is raw text (GNU
        // make_cmd.c:512 make_here_document, parse.y:3120
        // gather_here_documents): no subscript can open inside it, and the
        // body's `[`/`]`/parens must not feed this scan (parse.y:3557
        // read_token never re-reads consumed text). The jumped span's
        // newlines still advance the diagnostic line counter. Park while
        // the terminator has not arrived (perf17 prefix-stable rule);
        // empty delimiter keeps the fall-through.
        if ch == '<' && chars.get(index + 1) == Some(&'<') && chars.get(index + 2) != Some(&'<') {
            if let Some((next, terminator_found)) = skip_heredoc_top_level(&chars, index) {
                if !terminator_found && park.is_none() {
                    park = Some(SubscriptResidualPark {
                        pos: index,
                        snapshot: state.clone(),
                    });
                }
                state.line += chars[index..next].iter().filter(|c| **c == '\n').count();
                state.command_position = true;
                state.word.clear();
                state.element_start = false;
                index = next;
                continue;
            }
        }
        // `<<<` here-string (parse.y:3690-3706): one operator, consumed
        // atomically so its second `<` is never a `<<` opener.
        if ch == '<' && chars.get(index + 1) == Some(&'<') && chars.get(index + 2) == Some(&'<') {
            index += 3;
            continue;
        }
        match ch {
            '\\' => {
                state.escaped = true;
                state.word.clear();
                index += 1;
                continue;
            }
            '#' if state.word.is_empty() => {
                state.in_comment = true;
                index += 1;
                continue;
            }
            '\'' => {
                state.single = true;
                state.word.clear();
                index += 1;
                continue;
            }
            '"' => {
                state.double = true;
                state.word.clear();
                index += 1;
                continue;
            }
            '$' if chars.get(index + 1) == Some(&'\'') => {
                state.ansi_single = true;
                state.word.clear();
                index += 2;
                continue;
            }
            _ => {}
        }
        if ch.is_whitespace() {
            if ch == '\n' {
                state.line += 1;
                state.command_position = true;
                state.word.clear();
                state.element_start = false;
            } else {
                // `if`, `then`, `while`, ... keep the next word in command
                // position — but only when they themselves stood at command
                // position (`echo if a[b` keeps `a` an argument); a command
                // word like `echo` ends it. Whitespace directly after a
                // delimiter (`; `) keeps the delimiter's decision.
                if !state.word.is_empty() {
                    state.command_position = state.command_position
                        && skip_word_is_command_position_boundary(&state.word);
                }
                state.word.clear();
                if state.compassign_depth > 0 {
                    state.element_start = true;
                }
            }
            index += 1;
            continue;
        }
        // `[` opens a subscript when the word prefix is a pure shell
        // identifier at command position (parse.y:5637), or we are at
        // element start inside a compound assignment (parse.y:5638,
        // token_index == 0 && PST_COMPASSIGN).
        if ch == '['
            && ((state.command_position && skip_word_is_pure_identifier(&state.word))
                || (state.compassign_depth > 0 && state.element_start && state.word.is_empty()))
        {
            let snapshot = state.clone();
            // parse_matched_pair ('[', ']'): quote/escape aware, nested
            // `[ ... ]` pairs nest, newlines are consumed by the scan.
            let mut depth = 1usize;
            let mut scan = index + 1;
            let mut q_single = false;
            let mut q_double = false;
            let mut q_escaped = false;
            while scan < chars.len() {
                let c = chars[scan];
                if q_escaped {
                    q_escaped = false;
                    scan += 1;
                    continue;
                }
                if q_single {
                    if c == '\'' {
                        q_single = false;
                    }
                    scan += 1;
                    continue;
                }
                if q_double {
                    if c == '\\' {
                        q_escaped = true;
                    } else if c == '"' {
                        q_double = false;
                    }
                    scan += 1;
                    continue;
                }
                match c {
                    '\\' => q_escaped = true,
                    '\'' => q_single = true,
                    '"' => q_double = true,
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                scan += 1;
            }
            if depth > 0 {
                // S1: EOF inside the subscript — report at the `[` line
                // (parse.y:3906 start_lineno), with the compound-assignment
                // context deciding the exit status (1 inside `name=(`).
                // The oracle returns here; `reported` carries the answer
                // and the park re-derives the pair on the next line.
                state.reported = Some((state.line, state.compassign_depth > 0));
                // A park recorded earlier in this pass (S2/S3/S4) stays the
                // resume point: the re-derivation re-runs this `[` too.
                return Some(park.take().unwrap_or(SubscriptResidualPark {
                    pos: index,
                    snapshot,
                }));
            }
            index = scan + 1;
            state.word.clear();
            state.element_start = false;
            state.command_position = false;
            continue;
        }
        if ch == '(' {
            // `name=(` opens a compound-assignment list; any other `(` is a
            // subshell/grouping whose body starts a fresh command position.
            if state.word.ends_with('=') {
                state.compassign_depth += 1;
                state.element_start = true;
            } else {
                state.command_position = true;
            }
            state.word.clear();
            index += 1;
            continue;
        }
        if ch == ')' {
            state.compassign_depth = state.compassign_depth.saturating_sub(1);
            state.element_start = false;
            state.command_position = true;
            state.word.clear();
            index += 1;
            continue;
        }
        if matches!(ch, ';' | '&' | '|' | '{' | '}') {
            state.command_position = true;
            state.word.clear();
            state.element_start = false;
            index += 1;
            continue;
        }
        if ch == '`' {
            let snapshot = state.clone();
            let mut scan = index + 1;
            while scan < chars.len() && chars[scan] != '`' {
                if chars[scan] == '\\' {
                    scan += 1;
                }
                scan += 1;
            }
            // S3: the unit did not close on this buffer — commit the
            // consume-to-end, park at the backtick for the re-derivation.
            if scan >= chars.len() && park.is_none() {
                park = Some(SubscriptResidualPark {
                    pos: index,
                    snapshot,
                });
            }
            index = (scan + 1).min(chars.len());
            state.word.clear();
            continue;
        }
        if ch == '$' && chars.get(index + 1) == Some(&'(') {
            // Command-substitution body: its internals own their scans in
            // the recursive parse; skip the balanced unit.
            if let Some((end, decided)) = skip_parenthesized_unit_corrected_ex(chars, index + 1) {
                // S2u: closed undecidedly — commit the jump, park for the
                // re-derivation.
                if !decided && park.is_none() {
                    park = Some(SubscriptResidualPark {
                        pos: index,
                        snapshot: state.clone(),
                    });
                }
                index = end;
                state.word.clear();
                continue;
            }
            // S2f: the skip failed on this buffer — the oracle commits the
            // fall-through (`$` becomes word text); park at the `$`.
            if park.is_none() {
                park = Some(SubscriptResidualPark {
                    pos: index,
                    snapshot: state.clone(),
                });
            }
        }
        state.word.push(ch);
        index += 1;
    }
    park
}

/// Every scan local of the matched-pair close-char checker
/// (`unclosed_input_close_char_posix` above, the oracle), checkpointable
/// across appended group lines (perf9, #292B family).
///
/// GNU anchor: parse.y:3557 read_token — a streaming reader whose
/// matched-pair state (parse.y:3877 parse_matched_pair, quotes at
/// parse.y:5305 read_token_word) advances character by character and never
/// re-scans consumed text. The oracle's delimiter stack, line counter,
/// comment/command-position/word trackers are position-local state; the
/// non-local decisions are the buffer-tail two-character lookaheads of the
/// `$` dispatcher and the top-level backslash escape, each of which parks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CloseCharResidualState {
    pub(crate) stack: Vec<UnclosedDelim>,
    pub(crate) line: usize,
    pub(crate) comment_start: bool,
    /// A bare `(` only opens a subshell/array-list at command position or
    /// after `=` in an assignment word (the oracle's at_command tracker).
    pub(crate) at_command: bool,
    pub(crate) cur_word: String,
    /// perf19 case-pattern tracking (parse.y:1037 case_command grammar): a
    /// `)` inside an open case clause is a PATTERN terminator, never the
    /// closer of an enclosing subshell/`$(...`/backquote — see
    /// `close_char_operator_context` below.
    pub(crate) word: String,
    pub(crate) case_depth: usize,
    pub(crate) word_boundary: bool,
    pub(crate) current_word_boundary: bool,
    /// The staged machine's `case SUBJECT in` progress (0 none, 1 after
    /// `case`, 2 subject done, 3 after `in`) — see
    /// update_command_substitution_case_depth_staged_chars.
    pub(crate) case_in_stage: u8,
    /// rubash#380: the pattern-list region bit (parse.y:29 PST_CASEPAT) —
    /// reserved words inside a pattern list are word data. Checkpointed
    /// alongside case_in_stage so a resumed scan matches the oracle.
    pub(crate) case_pattern_region: bool,
}

impl Default for CloseCharResidualState {
    fn default() -> Self {
        // The oracle's initial locals.
        Self {
            stack: Vec::new(),
            line: 1,
            comment_start: true,
            at_command: true,
            cur_word: String::new(),
            word: String::new(),
            case_depth: 0,
            word_boundary: true,
            current_word_boundary: true,
            case_in_stage: 0,
            case_pattern_region: false,
        }
    }
}

impl CloseCharResidualState {
    /// The oracle's answer for the current prefix:
    /// `unclosed_input_close_char_posix(input, _).is_some()` — some
    /// matched-pair construct is still pending. (The diagnostic payload —
    /// close char, open line, report_open/command/array_list — is only
    /// consumed at final-EOF diagnostics, which full-scan the oracle.)
    pub(crate) fn is_open(&self) -> bool {
        !self.stack.is_empty()
    }
}

/// A re-derivation point returned by [`close_char_residuals_advance`]
/// (same contract as [`ComsubResidualPark`]).
pub(crate) struct CloseCharResidualPark {
    /// Char index where the undecided decision starts.
    pub(crate) pos: usize,
    /// Scan state at `pos`, before the undecided arm mutated anything.
    pub(crate) snapshot: CloseCharResidualState,
}

/// Advance the close-char scan over `chars[from..]` starting from `state`
/// (restored from a checkpoint or `Default`), leaving `state` as the
/// committed end state for this prefix, and return the FIRST undecided
/// position, if any. The arms are `unclosed_input_close_char_posix`'s
/// arms, unchanged.
///
/// Parks (all commit + park + continue — the oracle never stops early):
/// (K1) a `${` at the buffer tail — the FUNSUB_CHAR lookahead
/// (parser.h:85) is not visible, so the funsub/report_open split of the
/// pushed delimiter is undecided; the oracle commits the parameter-brace
/// push (funsub=false) for this prefix.
/// (K2) a `$(` at the buffer tail — the `$((` arithmetic lookahead decides
/// report_open and the second (inner-paren) push; the oracle commits the
/// plain `$(` push.
/// (K3) a `$` at the buffer tail — the `$'` ANSI-C arm (guarded by the
/// stack top) is undecided; the oracle commits the inert fall-through.
/// (K4) a top-level `\` at the buffer tail — the escaped-character pair
/// (which feeds cur_word and the array-list `name=(` decision) is
/// undecided; the oracle commits the lone backslash as plain text.
/// Every other decision is position-local or backward-looking
/// (`chars[i-1] == '('`); comment consumption and matched-pair closes are
/// forward-monotone. The loop-head effects at a parked `$`/`\` are pure
/// idempotent assignments (line counting only runs at '\n', which never
/// parks), so resume-at-park reproduces the full scan bit for bit.
#[allow(clippy::too_many_lines)]
/// perf19: is a `<<` heredoc operator / `case` keyword live shell syntax in
/// this scanner context? GNU's reader only lexes operators and keywords
/// where the grammar expects command text: the top level, inside
/// `$(...)`, inside backquotes, inside `${ cmd; }` funsubs, and inside
/// command-position subshells. It does NOT inside quotes (`'...'`,
/// `"..."`, `$'...'` — `<<` and `case` are word data), inside `${param}`
/// expansions, inside array-assignment lists (`x=(`), or inside
/// arithmetic (`$((1<<2))` / `((x<<1))` — `<<` is the shift operator;
/// those tops carry `report_open && !command`, exactly the
/// parse_matched_pair arithmetic pushes).
fn close_char_operator_context(top: Option<UnclosedDelim>) -> bool {
    match top {
        None => true,
        Some(d) => match d.close {
            '\'' | '"' => false,
            '`' => true,
            ')' => !d.report_open || d.command,
            '}' => d.funsub,
            _ => false,
        },
    }
}

pub(crate) fn close_char_residuals_advance(
    chars: &[char],
    from: usize,
    state: &mut CloseCharResidualState,
    posix: bool,
) -> Option<CloseCharResidualPark> {
    let mut i = from.min(chars.len());
    let mut park: Option<CloseCharResidualPark> = None;

    while i < chars.len() {
        let ch = chars[i];
        let top = state.stack.last().copied();
        if ch == '\n' {
            state.line += 1;
        }
        // perf19: case-pattern tracking. GNU's grammar (parse.y:1037
        // case_command: CASE WORD newline_list IN case_clause ESAC)
        // consumes a pattern's `)` as a case token — the subshell/
        // `$(`/backquote closer only arrives at command position after
        // the clause's `esac`. The comsub residual scanner already ports
        // this word machine (`update_command_substitution_case_depth`,
        // the special_case_tokens rules of parse.y:3369-3386); the
        // close-char scan needs the same tracking so a case-pattern `)`
        // does not pop an enclosing `)` delimiter. The feed runs at the
        // loop top so a word completing AT the `)` (`esac)`) is folded
        // before the pop check below reads `case_depth`. Comment bodies
        // are consumed by the arms below after their `#` separator was
        // fed (harmless); quote content is cleared by the helper via the
        // single/double flags derived from the delimiter stack.
        if close_char_operator_context(top) {
            update_command_substitution_case_depth_staged_chars(
                chars,
                i,
                ch,
                &mut state.word,
                &mut state.case_depth,
                &mut state.word_boundary,
                &mut state.current_word_boundary,
                &mut state.case_in_stage,
                &mut state.case_pattern_region,
            );
        } else if top.is_some_and(|d| d.close == '\'' || d.close == '"') {
            // Inside a quote span the word machine clears (the skip.rs
            // variant's `single || double` branch): `es'ac` must never
            // assemble into `esac` (GNU marks quote-bearing words
            // W_QUOTED, never keywords).
            state.word.clear();
            state.word_boundary = false;
        }
        // perf19: heredoc body opacity. GNU make_cmd.c:512
        // `make_here_document` (driven by parse.y:3120
        // gather_here_documents) reads the body as raw lines through
        // `read_secondary_line` — the matched-pair scanner state
        // (parse.y:3877 parse_matched_pair under parse.y:3557 read_token)
        // never processes a body character, and the reader continues past
        // the terminator line. The arm fires only in contexts where a
        // redirection operator is live syntax (`close_char_operator_context`).
        // Park while the terminator has not arrived (perf17 prefix-stable
        // rule); empty delimiter keeps the fall-through. The jumped span's
        // newlines advance the line counter the diagnostic `open_line`
        // reports.
        if ch == '<'
            && chars.get(i + 1) == Some(&'<')
            && chars.get(i + 2) != Some(&'<')
            && close_char_operator_context(top)
        {
            if let Some((next, terminator_found)) = skip_heredoc_top_level(&chars, i) {
                if !terminator_found && park.is_none() {
                    park = Some(CloseCharResidualPark {
                        pos: i,
                        snapshot: state.clone(),
                    });
                }
                state.line += chars[i..next].iter().filter(|c| **c == '\n').count();
                state.comment_start = true;
                state.cur_word.clear();
                state.word.clear();
                state.word_boundary = true;
                if let Some(d) = state.stack.last_mut() {
                    if d.funsub {
                        d.term_ready = true;
                    }
                } else {
                    state.at_command = true;
                }
                i = next;
                continue;
            }
        }
        // `<<<` here-string (parse.y:3690-3706): one operator, consumed
        // atomically so its second `<` is never a `<<` opener (the
        // operator boundary after it matches a single `<`'s).
        if ch == '<'
            && chars.get(i + 1) == Some(&'<')
            && chars.get(i + 2) == Some(&'<')
            && close_char_operator_context(top)
        {
            state.comment_start = true;
            state.cur_word.clear();
            i += 3;
            continue;
        }
        if let Some(d) = top {
            if d.escapes && ch == '\\' {
                // An escaped character is word text everywhere.
                state.comment_start = false;
                i += 2;
                continue;
            }
            // perf19: a `)` while a case clause is open is a PATTERN
            // terminator (parse.y:1037), not this delimiter's closer.
            if ch == d.close
                && !(d.funsub && !d.term_ready)
                && (d.pattern_paren || !(d.close == ')' && state.case_depth > d.case_depth_at_push))
            {
                state.stack.pop();
                // A closed subshell or brace group is a complete command:
                // an enclosing function substitution's `}' may now close.
                if let Some(parent) = state.stack.last_mut() {
                    if parent.funsub && d.command {
                        parent.term_ready = true;
                    }
                }
                // `)` is a shell separator: a following `#` starts a
                // comment (`x=$(a)#c`). Quote/`}` closes stay mid-word.
                state.comment_start = d.close == ')';
                // Completed quote span is word material for the at_command
                // tracker (rubash#322) — keeps the incremental residual
                // consistent with the one-shot scan above.
                if d.close == '"' || d.close == '\'' {
                    state.cur_word.push('"');
                }
                i += 1;
                continue;
            }
            if d.close == '\'' {
                // Literal context: nothing else is special inside '...'.
                i += 1;
                continue;
            }
            // Inside `$(...)` / `( ... )` / `${ cmds; }` (report_open false —
            // the `$((` inner paren reports open and is arithmetic text
            // where `#` is the base operator, never a comment) a `#` at a
            // token boundary comments through end of line, so the `)` in
            // `$(# c )` cannot close the substitution (comsub-posix). An
            // array list (`name=(`, also report_open — parse_matched_pair's
            // start_lineno report) is element text where `#` IS a comment:
            // a `)` inside the comment must not close the list (probe
            // 2026-09-27: `declare -a x=(\n 1 # c )` at EOF → GNU reports
            // "unexpected EOF while looking for matching `)'" + exit 1).
            if d.close == ')' || d.funsub {
                if !(d.close == ')' && d.report_open && !d.array_list)
                    && ch == '#'
                    && state.comment_start
                {
                    while i < chars.len() && chars[i] != '\n' {
                        i += 1;
                    }
                    continue;
                }
                state.comment_start =
                    ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
            }
            if d.funsub {
                // Track command-terminator state: `${ cmd }' without a
                // separator before `}' never terminates (parse_comsub).
                let d = state.stack.last_mut().unwrap();
                match ch {
                    ';' | '&' | '|' | '\n' => d.term_ready = true,
                    c if !c.is_whitespace() => d.term_ready = false,
                    _ => {}
                }
            }
        } else {
            // Top-level `\` quotes the next character as literal word text
            // (parse.y read_token_word): an escaped `(` in `\<...` must not
            // open a paren delimiter. `\<newline>` is a continuation that
            // joins the word across lines.
            if ch == '\\' && i + 1 < chars.len() {
                if chars[i + 1] == '\n' {
                    state.line += 1;
                } else if chars[i + 1] != '=' {
                    state.cur_word.push(chars[i + 1]);
                }
                state.comment_start = false;
                i += 2;
                continue;
            }
            // K4: a top-level `\` at the buffer tail — the pair is
            // undecided; the oracle commits the plain-text fall-through.
            if ch == '\\' && i + 1 >= chars.len() && park.is_none() {
                park = Some(CloseCharResidualPark {
                    pos: i,
                    snapshot: state.clone(),
                });
            }
            // Top-level comment: a word-initial '#' consumes to EOL
            // (parse.y read_token -> parse_comment).
            if ch == '#' && state.comment_start {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            if ch.is_whitespace()
                || matches!(ch, ';' | '&' | '|' | '(' | ')' | '{' | '}' | '<' | '>')
            {
                state.comment_start = true;
            } else {
                state.comment_start = false;
            }
            match ch {
                // `(` is excluded: the push arm below gates on the state
                // BEFORE it — a mid-command `(` must not mark itself as
                // command position. `)` likewise: a stray `)` is a parse
                // error left to the parser, but after it a command follows.
                '\n' | ';' | '&' | '|' | ')' | '{' | '}' => {
                    state.at_command = true;
                    state.cur_word.clear();
                }
                '<' | '>' => {
                    // Redirect operator: a filename word follows, so `>(` is
                    // not command position.
                    state.cur_word.clear();
                }
                c if c.is_whitespace() => {
                    if !state.cur_word.is_empty() {
                        state.at_command = matches!(
                            state.cur_word.as_str(),
                            "if" | "then"
                                | "else"
                                | "elif"
                                | "while"
                                | "until"
                                | "do"
                                | "in"
                                | "!"
                                | "time"
                                | "coproc"
                                | "case"
                        );
                        state.cur_word.clear();
                    }
                }
                c if c.is_alphanumeric()
                    || c == '_'
                    || (c == '=' && !state.cur_word.is_empty()) =>
                {
                    state.cur_word.push(c)
                }
                _ => {}
            }
        }
        let in_double = top.is_some_and(|d| d.close == '"');
        match ch {
            '\'' if !in_double => {
                // POSIX + Interp 221: `'` inside `"${...}"` is literal.
                if posix && squote_is_literal_in_posix_braced_dquote(&state.stack) {
                    state.comment_start = false;
                    i += 1;
                    continue;
                }
                // parse_matched_pair reports start_lineno for quotes.
                state.stack.push(UnclosedDelim {
                    close: '\'',
                    open_line: state.line,
                    escapes: false,
                    report_open: true,
                    funsub: false,
                    command: false,
                    term_ready: false,
                    case_depth_at_push: 0,
                    array_list: false,
                    pattern_paren: false,
                });
            }
            '"' => {
                state.stack.push(UnclosedDelim {
                    close: '"',
                    open_line: state.line,
                    escapes: true,
                    report_open: true,
                    funsub: false,
                    command: false,
                    term_ready: false,
                    case_depth_at_push: 0,
                    array_list: false,
                    pattern_paren: false,
                });
            }
            '`' => {
                state.stack.push(UnclosedDelim {
                    close: '`',
                    open_line: state.line,
                    escapes: true,
                    report_open: true,
                    funsub: false,
                    command: false,
                    term_ready: false,
                    case_depth_at_push: 0,
                    array_list: false,
                    pattern_paren: false,
                });
            }
            '$' => {
                match chars.get(i + 1) {
                    // K3: a `$` at the buffer tail — the arm dispatch is
                    // undecided; the oracle commits the inert fall-through.
                    None if park.is_none() => {
                        park = Some(CloseCharResidualPark {
                            pos: i,
                            snapshot: state.clone(),
                        });
                    }
                    Some('{') => {
                        // parse.y:5506: `${' followed by a FUNSUB_CHAR is a
                        // function substitution parsed as commands; a
                        // parameter expansion takes parse_matched_pair
                        // (yyerror path: EOF line either way).
                        // FUNSUB_CHAR is parser.h:85 (`#else' arm): blank,
                        // newline or `|' only — NOT `(' (that spelling is
                        // the `#if 0' dead arm at parser.h:83), so `${(M)x}'
                        // is a parameter brace whose first unquoted `}'
                        // closes (P_FIRSTCLOSE).
                        //
                        // K1: `${` at the buffer tail — the FUNSUB_CHAR
                        // lookahead is undecided; the oracle commits
                        // funsub=false (parameter brace, report_open).
                        if chars.get(i + 2).is_none() && park.is_none() {
                            park = Some(CloseCharResidualPark {
                                pos: i,
                                snapshot: state.clone(),
                            });
                        }
                        let funsub = chars
                            .get(i + 2)
                            .is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '|'));
                        state.stack.push(UnclosedDelim {
                            close: '}',
                            open_line: state.line,
                            escapes: true,
                            // `${param` is a parse_matched_pair: EOF names
                            // the `${` line. The `${ ' funsub variant is a
                            // command context and reports the EOF line.
                            report_open: !funsub,
                            funsub,
                            command: false,
                            term_ready: false,
                            case_depth_at_push: 0,
                            array_list: false,
                            pattern_paren: false,
                        });
                        if funsub {
                            state.comment_start = true;
                            // P380FIX (captain diff): mirror the one-shot
                            // scan — the funsub body's first word is at
                            // command position (parse.y:5506). rubash#380.
                            state.word_boundary = true;
                        }
                        i += 1;
                    }
                    Some('(') => {
                        // $( EOF takes the yyerror path: line_number at EOF.
                        // `$((` is a single arithmetic construct parsed by
                        // parse_matched_pair instead: EOF names the `$(`
                        // line even when only the inner `)` was closed.
                        //
                        // K2: `$(` at the buffer tail — the `$((` lookahead
                        // is undecided; the oracle commits the plain `$(`
                        // push (report_open=false, no inner-paren push).
                        if chars.get(i + 2).is_none() && park.is_none() {
                            park = Some(CloseCharResidualPark {
                                pos: i,
                                snapshot: state.clone(),
                            });
                        }
                        state.stack.push(UnclosedDelim {
                            close: ')',
                            open_line: state.line,
                            escapes: true,
                            report_open: chars.get(i + 2) == Some(&'('),
                            funsub: false,
                            command: false,
                            term_ready: false,
                            case_depth_at_push: state.case_depth,
                            array_list: false,
                            pattern_paren: false,
                        });
                        // A fresh substitution body starts at a token
                        // boundary: `$(#c` is a comment.
                        state.comment_start = true;
                        // P380FIX (captain diff): mirror the one-shot scan —
                        // the comsub body's first word is at command position
                        // (subst.c:7143 -> parse_and_execute), so the case
                        // word machine must see a word boundary here.
                        // rubash#380.
                        if chars.get(i + 2) != Some(&'(') {
                            state.word_boundary = true;
                        }
                        if chars.get(i + 2) == Some(&'(') {
                            // $(( ... )) arithmetic nests a second ')' and is
                            // parsed by parse_matched_pair: start_lineno.
                            state.stack.push(UnclosedDelim {
                                close: ')',
                                open_line: state.line,
                                escapes: true,
                                report_open: true,
                                funsub: false,
                                command: false,
                                term_ready: false,
                                case_depth_at_push: state.case_depth,
                                array_list: false,
                                pattern_paren: false,
                            });
                            i += 1;
                        }
                        i += 1;
                    }
                    Some('\'')
                        if top
                            .is_none_or(|d| d.close == '}' || d.close == ')' || d.close == '`') =>
                    {
                        // ANSI-C $'...': single-quote close, escapes live.
                        // parse.y:4062-4068 parse_matched_pair: inside a
                        // grouping construct ($(...), ${...}, subshell,
                        // backtick) a `$'` opens a nested P_ALLOWESC unit —
                        // its \' escapes stay inside and the enclosing
                        // construct's quote state never sees them
                        // (rubash#222/t0286: `${foo/$a/$''}` must not read
                        // as an unclosed `'`). Double quotes stay excluded:
                        // `"` is not in the guard set.
                        state.stack.push(UnclosedDelim {
                            close: '\'',
                            open_line: state.line,
                            escapes: true,
                            report_open: true,
                            funsub: false,
                            command: false,
                            term_ready: false,
                            case_depth_at_push: 0,
                            array_list: false,
                            pattern_paren: false,
                        });
                        i += 1;
                    }
                    _ => {}
                }
            }
            '(' if top.is_none() => {
                // Command-position `(` opens a subshell; `name=(` in an
                // assignment word opens an array list (`declare -a ddd=(aaa`
                // continues on the next line). A `(` elsewhere is a parse
                // error for the parser, not a pending delimiter.
                if state.at_command || (state.cur_word.len() > 1 && state.cur_word.ends_with('=')) {
                    let is_subshell = state.at_command && state.cur_word.is_empty();
                    let is_array_list = !is_subshell;
                    state.stack.push(UnclosedDelim {
                        close: ')',
                        open_line: state.line,
                        escapes: true,
                        // Array lists take parse_matched_pair's start_lineno
                        // report (`ddd=(aaa` EOF names the `(` line); a
                        // command-position subshell reports the EOF line.
                        report_open: !is_subshell,
                        funsub: false,
                        // GNU yyerror "from `(' command" applies only to a
                        // command-position subshell; `name=(...` is an
                        // array-list matched pair ("matching `)'"). The
                        // `at_command` flag is only refreshed at word
                        // boundaries, so a pending `name=` word means this
                        // `(` is array text, not a subshell.
                        command: is_subshell,
                        term_ready: false,
                        case_depth_at_push: state.case_depth,
                        array_list: is_array_list,
                        pattern_paren: false,
                    });
                }
                state.comment_start = true;
                state.at_command = true;
                state.cur_word.clear();
            }
            '(' if top.is_some_and(|d| d.close == ')' || (d.close == '}' && d.funsub)) => {
                // Subshell nested inside `$(...)`/`( ... )`/`${ ...; }`:
                // the body is command context where `(` is legal. An
                // immediately adjacent `(` (`((x`) is the arithmetic
                // construct's inner paren — a matched pair, not a command.
                state.stack.push(UnclosedDelim {
                    close: ')',
                    open_line: state.line,
                    escapes: true,
                    // `((x` arithmetic takes parse_matched_pair's
                    // start_lineno report like `$((` does; a real nested
                    // subshell reports the EOF line (yyerror).
                    report_open: i > 0 && chars[i - 1] == '(',
                    funsub: false,
                    command: !(i > 0 && chars[i - 1] == '('),
                    term_ready: false,
                    case_depth_at_push: state.case_depth,
                    array_list: false,
                    pattern_paren: state.case_depth > 0 && state.case_pattern_region,
                });
                state.comment_start = true;
            }
            '{' if top.is_some_and(|d| d.close == '}' && d.funsub) => {
                // `{ cmd; }' group inside a function substitution: `}' only
                // closes after a command terminator, same rule.
                state.stack.push(UnclosedDelim {
                    close: '}',
                    open_line: state.line,
                    escapes: true,
                    report_open: false,
                    funsub: true,
                    command: true,
                    term_ready: false,
                    case_depth_at_push: 0,
                    array_list: false,
                    pattern_paren: false,
                });
            }
            _ => {}
        }
        i += 1;
    }
    park
}

/// Mirror of skip.rs `is_pure_identifier` (verbatim; private there).
fn skip_word_is_pure_identifier(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// Mirror of skip.rs `is_command_position_boundary` (verbatim; private
/// there).
fn skip_word_is_command_position_boundary(word: &str) -> bool {
    matches!(
        word,
        "if" | "then"
            | "elif"
            | "else"
            | "fi"
            | "while"
            | "until"
            | "do"
            | "done"
            | "esac"
            | "!"
            | "time"
            | "coproc"
            | "{"
            | "}"
    )
}

fn update_command_substitution_case_depth(
    chars: &[char],
    index: usize,
    ch: char,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
    case_pattern_region: &mut bool,
) {
    let mut undecided = false;
    update_command_substitution_case_depth_ex(
        chars,
        index,
        ch,
        word,
        case_depth,
        word_boundary,
        current_word_boundary,
        &mut undecided,
        case_pattern_region,
    );
}

/// rubash#292 plan B variant: additionally reports (through `undecided`)
/// whether the `esac` case-pattern lookahead consulted text past the end
/// of `chars` without concluding. The decision itself is still committed
/// exactly as the full-buffer scan commits it; only the caller's
/// checkpoint cares about the flag.
/// perf19: the STAGED case machine (skip.rs's
/// `update_command_substitution_case_depth`, GNU special_case_tokens
/// parse.y:3369-3386 + the parse.y:3433-3441 empty-case `esac`), over a
/// chars slice like the unstaged wrapper above. The close-char scan uses
/// THIS variant: its subshell/`$(`/backquote contexts see bare command
/// text where `case SUBJECT in esac` appears with plain-whitespace word
/// boundaries — the unstaged machine loses the reserved-word boundary
/// after the subject word and never counts the empty case's `esac`
/// (perf19: `( case x in esac )` held the subshell open to EOF).
fn update_command_substitution_case_depth_staged_chars(
    chars: &[char],
    index: usize,
    ch: char,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
    case_in_stage: &mut u8,
    case_pattern_region: &mut bool,
) {
    if ch == '_' || ch.is_ascii_alphanumeric() {
        if word.is_empty() {
            *current_word_boundary = *word_boundary;
        }
        word.push(ch);
        return;
    }

    if word.is_empty() {
        if *case_depth > 0 {
            if ch == ')' && *case_pattern_region {
                // parse.y:3787-3788: `)` closes the pattern list; the
                // clause body begins and reserved words live again.
                *case_pattern_region = false;
            } else if ch == ';'
                && chars
                    .get(index + 1)
                    .is_some_and(|next| *next == ';' || *next == '&')
            {
                // parse.y:3710/3759: `;;`, `;&`, `;;&` end the clause body —
                // the next pattern list begins.
                *case_pattern_region = true;
            }
            if *case_pattern_region && *case_in_stage == 3 && matches!(ch, '(' | '|') {
                // A `(` pattern-list opener or `|` separator ends the
                // directly-after-`in` window in which a bare `esac` closes
                // an empty case (parse.y:3433-3441 needs last_read_token ==
                // IN), so `(esac)` stays pattern text.
                *case_in_stage = 4;
            }
        }
        if command_substitution_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    let completing_after_case = *case_in_stage == 1;
    if completing_after_case {
        *case_in_stage = 2;
    }

    // parse.y:3177: inside PST_CASEPAT only ESAC may still be the keyword.
    let in_pattern_region = *case_pattern_region;
    let reserved_word_allows_next = match word.as_str() {
        "case" if *current_word_boundary && !in_pattern_region => {
            *case_depth += 1;
            *case_in_stage = 1;
            *case_pattern_region = false;
            false
        }
        "in" if *case_in_stage == 2 => {
            // GNU special_case_tokens rule 6: `in' after the case subject
            // is the IN token even off a reserved-word boundary.
            *case_in_stage = 3;
            // parse.y:3379/3396: the IN of a case arms the pattern region.
            *case_pattern_region = true;
            true
        }
        "esac" if *case_in_stage == 3 => {
            // GNU parse.y:3433-3441: `esac' directly after IN is ESAC —
            // the empty case `case WORD in esac' — unconditionally.
            *case_depth = case_depth.saturating_sub(1);
            *case_in_stage = 0;
            *case_pattern_region = false;
            true
        }
        "esac" if *current_word_boundary => {
            // P380FIX (captain diff): parse.y:3177-3186
            // CHECK_FOR_RESERVED_WORD — `esac' stays WORD data ONLY when
            // the previous token is `|' (Posix rule 4) or the pattern-list
            // `(' (phantom rule 4), both meaningful inside PST_CASEPAT;
            // every other boundary `esac' — after `;;', at a clause-body
            // start, after `in' — is the ESAC keyword. The previous
            // token is witnessed by scanning back over whitespace from
            // the word start (the old `)`-then-evidence FORWARD lookahead
            // misjudged `;; esac)` and `x) esac)` whenever a construct
            // OUTSIDE the case supplied the evidence, rubash#380).
            let word_start = index.saturating_sub(word.chars().count());
            let mut back = word_start;
            while back > 0 && chars[back - 1].is_whitespace() {
                back -= 1;
            }
            if back > 0 && matches!(chars[back - 1], '|' | '(') {
                // `esac` heads a pattern list (esac|pat) / sits in pattern
                // position ((esac)): pattern text, not the keyword.
                *case_in_stage = 0;
                false
            } else {
                *case_depth = case_depth.saturating_sub(1);
                *case_in_stage = 0;
                *case_pattern_region = false;
                true
            }
        }
        "for" | "select" | "while" | "until" | "then" | "do" | "else" | "elif" | "in" | "fi"
        | "done"
            if *current_word_boundary && !in_pattern_region =>
        {
            *case_in_stage = 0;
            true
        }
        _ => {
            if !completing_after_case {
                *case_in_stage = 0;
            }
            false
        }
    };
    if *case_depth > 0 {
        if ch == ')' && *case_pattern_region {
            *case_pattern_region = false;
        } else if ch == ';'
            && chars
                .get(index + 1)
                .is_some_and(|next| *next == ';' || *next == '&')
        {
            *case_pattern_region = true;
        }
    }
    word.clear();
    *word_boundary =
        reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
}

fn update_command_substitution_case_depth_ex(
    chars: &[char],
    index: usize,
    ch: char,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
    undecided: &mut bool,
    case_pattern_region: &mut bool,
) {
    if ch == '_' || ch.is_ascii_alphanumeric() {
        if word.is_empty() {
            *current_word_boundary = *word_boundary;
        }
        word.push(ch);
        return;
    }

    if word.is_empty() {
        if *case_depth > 0 {
            if ch == ')' && *case_pattern_region {
                // parse.y:3787-3788: `)` closes the pattern list.
                *case_pattern_region = false;
            } else if ch == ';'
                && chars
                    .get(index + 1)
                    .is_some_and(|next| *next == ';' || *next == '&')
            {
                // parse.y:3710/3759: `;;`, `;&`, `;;&` start the next
                // pattern list.
                *case_pattern_region = true;
            }
        }
        if command_substitution_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    let in_pattern_region = *case_pattern_region;
    let reserved_word_allows_next = match word.as_str() {
        "case" if *current_word_boundary && !in_pattern_region => {
            *case_depth += 1;
            *case_pattern_region = false;
            false
        }
        "esac" if *current_word_boundary => {
            // P380FIX (captain diff): parse.y:3177-3186 — `esac' is
            // pattern text ONLY after a `|' or pattern-list `(' token
            // (backward previous-token test, whitespace-skipped); every
            // other boundary `esac' is the ESAC keyword. rubash#380.
            let word_start = index.saturating_sub(word.chars().count());
            let mut back = word_start;
            while back > 0 && chars[back - 1].is_whitespace() {
                back -= 1;
            }
            if back > 0 && matches!(chars[back - 1], '|' | '(') {
                false
            } else {
                *case_depth = case_depth.saturating_sub(1);
                *case_pattern_region = false;
                true
            }
        }
        "for" | "select" | "while" | "until" | "then" | "do" | "else" | "elif" | "in" | "fi"
        | "done"
            if *current_word_boundary && !in_pattern_region =>
        {
            true
        }
        _ => false,
    };
    if *case_depth > 0 {
        if ch == ')' && *case_pattern_region {
            *case_pattern_region = false;
        } else if ch == ';'
            && chars
                .get(index + 1)
                .is_some_and(|next| *next == ';' || *next == '&')
        {
            *case_pattern_region = true;
        }
    }
    word.clear();
    *word_boundary =
        reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
}

fn command_substitution_separator_allows_reserved_word(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | ')' | '\n')
}

/// rubash#292 plan B variant of the `esac)` case-pattern lookahead: also
/// reports whether the decision was *decided* by the current buffer. A
/// scan that runs off `chars.len()` (matching `)` not yet present, or the
/// post-close `;;` / `esac` / `)` evidence not yet present) returns its
/// partial-text answer with `decided == false`: future text can flip it,
/// so checkpoint callers must re-derive instead of committing.
fn case_pattern_starts_with_esac_chars_ex(chars: &[char], delimiter_index: usize) -> (bool, bool) {
    if !matches!(chars.get(delimiter_index), Some(')' | '|')) {
        return (false, true);
    }

    let mut close = delimiter_index;
    while close < chars.len() {
        match chars[close] {
            ')' => break,
            ';' | '\n' => return (false, true),
            _ => close += 1,
        }
    }
    if chars.get(close) != Some(&')') {
        // Ran off the end before the pattern list's `)`: a longer buffer
        // may find it and continue to opposite evidence.
        return (false, false);
    }

    let mut scan = close + 1;
    let mut word = String::new();
    let mut word_boundary = true;
    while scan < chars.len() {
        let ch = chars[scan];
        if ch == ';' && chars.get(scan + 1) == Some(&';') {
            return (true, true);
        }
        if ch == '_' || ch.is_ascii_alphanumeric() {
            word.push(ch);
            scan += 1;
            continue;
        }
        if word == "esac" && word_boundary {
            return (true, true);
        }
        if ch == ')' {
            return (false, true);
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

    // Off the end with (at best) a partial trailing word: more text can
    // still turn this into `;;`, a boundary `esac`, or `)`.
    (word == "esac" && word_boundary, false)
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

#[cfg(test)]
mod balanced_residual_incremental_tests {
    use super::balanced_residuals_advance;
    use super::BalancedResidualState;
    use crate::lexer::skip::command_substitutions_balanced as oracle;

    /// Drive the checkpoint exactly like `GroupTextScans::advance_balanced`
    /// (restore snapshot at `resume`, advance over the pending mirror,
    /// store park or stable end) and assert at EVERY appended-line prefix
    /// that the committed answer equals the oracle's answer for that
    /// prefix, and the end state equals the full scan of that prefix. The
    /// mirror grows one RAW line (text + '\n') per prefix, exactly like
    /// read_next_source_group accumulates `pending`.
    fn assert_incremental_matches_full(lines: &[&str]) {
        let mut chars: Vec<char> = Vec::new();
        let mut resume = 0usize;
        let mut snapshot = BalancedResidualState::default();
        for (line_index, line) in lines.iter().enumerate() {
            chars.extend(line.chars());
            chars.push('\n');
            let mut state = snapshot.clone();
            let park = balanced_residuals_advance(&chars, resume, &mut state);
            let mut full = BalancedResidualState::default();
            let _ = balanced_residuals_advance(&chars, 0, &mut full);
            let text: String = chars.iter().collect();
            assert_eq!(
                state, full,
                "end state at prefix {line_index} of {lines:?} ({text:?})"
            );
            assert_eq!(
                state.is_unbalanced(),
                !oracle(&text),
                "answer at prefix {line_index} of {lines:?} ({text:?})"
            );
            match park {
                Some(park) => {
                    resume = park.pos;
                    snapshot = park.snapshot;
                }
                None => {
                    resume = chars.len();
                    snapshot = state;
                }
            }
        }
        // The same driving pattern without the final '\n' (the group's last
        // line may be un-terminated): parks cover the tail lookahead.
        let mut chars: Vec<char> = Vec::new();
        let mut resume = 0usize;
        let mut snapshot = BalancedResidualState::default();
        for (line_index, line) in lines.iter().enumerate() {
            chars.extend(line.chars());
            if line_index + 1 < lines.len() {
                chars.push('\n');
            }
            let mut state = snapshot.clone();
            let park = balanced_residuals_advance(&chars, resume, &mut state);
            let mut full = BalancedResidualState::default();
            let _ = balanced_residuals_advance(&chars, 0, &mut full);
            let text: String = chars.iter().collect();
            assert_eq!(state, full, "no-final-nl state at prefix {line_index}");
            assert_eq!(
                state.is_unbalanced(),
                !oracle(&text),
                "no-final-nl answer at prefix {line_index} of {lines:?} ({text:?})"
            );
            match park {
                Some(park) => {
                    resume = park.pos;
                    snapshot = park.snapshot;
                }
                None => {
                    resume = chars.len();
                    snapshot = state;
                }
            }
        }
    }

    #[test]
    fn incremental_matches_full_required_shapes() {
        // Multi-line plain command substitution.
        assert_incremental_matches_full(&["echo $(a", "b)"]);
        // The residual false-positive override this checker exists for
        // (skip.rs bb09aa28): `$(case x in x) esac)` is BALANCED.
        assert_incremental_matches_full(&["echo $(case a in a) echo x", "esac)"]);
        assert_incremental_matches_full(&["$(case x in", "a) :;;", "esac)"]);
        // Unclosed `$(`: unbalanced until the closing line arrives.
        assert_incremental_matches_full(&["x=$(gzip", "-dc file.gz)"]);
        // Backtick substitutions, open and closed.
        assert_incremental_matches_full(&["echo `date", "+%s` after"]);
        assert_incremental_matches_full(&["echo `cat f", "| sort`"]);
        // `$((` arithmetic fallback shapes.
        assert_incremental_matches_full(&["echo $((", "1+", "2))"]);
        assert_incremental_matches_full(&["echo $((", "case x in x) esac;; ", ")"]);
        // `${...}` span skipping with nested quotes in the body.
        assert_incremental_matches_full(&["x=${a:-${b", "}}"]);
        assert_incremental_matches_full(&["x=\"${IFS+'}'z", "}\""]);
        // `$'...'` ANSI-C strings.
        assert_incremental_matches_full(&["echo $'a\\n", "b'"]);
        // Comments and escapes across lines.
        assert_incremental_matches_full(&["echo a #c (", "d", "e"]);
        assert_incremental_matches_full(&["echo a \\", "b ( c"]);
        // Heredoc bodies inside a comsub unit stay opaque to the balancer.
        assert_incremental_matches_full(&["echo $(cat <<eof", "here ) doc", "eof", ")"]);
    }

    #[test]
    fn incremental_matches_full_park_shapes() {
        // B1: a `${` span that only closes on a later line.
        assert_incremental_matches_full(&["echo ${a", "} x"]);
        assert_incremental_matches_full(&["echo ${a:-'", "'}"]);
        // B2: a backtick that only closes on a later line.
        assert_incremental_matches_full(&["echo `a", "b` c"]);
        // B3: a `$(` that only closes on a later line (the failing skip is
        // re-derived per line until the unit closes).
        assert_incremental_matches_full(&["echo $(a", "(b) c)"]);
        assert_incremental_matches_full(&["x=\"$(fo", "o)\""]);
        // B4: a `$` at the buffer tail (`$` then a line break, then the
        // two-character lookahead materializes).
        assert_incremental_matches_full(&["echo $", "(a) b"]);
        assert_incremental_matches_full(&["echo $", "'a' b"]);
        assert_incremental_matches_full(&["echo $", "{a} b"]);
        // B5: closure through an `esac)` lookahead that runs off the buffer
        // end undecided; the following `;;` / `esac` flips the decision.
        assert_incremental_matches_full(&["echo $(case a in x) esac)", ";; )"]);
        assert_incremental_matches_full(&["echo $(case a in x) esac)", "esac)"]);
        // Nested units closing in the opposite order they opened.
        assert_incremental_matches_full(&["echo $(a $(b", "c))"]);
        // quote state straddling a line boundary inside a unit.
        assert_incremental_matches_full(&["echo $(echo 'q", "z')"]);
    }

    /// Deterministic LCG so a failure reproduces bit for bit.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
    }

    /// Seeded fuzz (>=2000 required; 4000 for parity with the perf8
    /// classes): construct-dense random token soups must satisfy the
    /// incremental == full contract at every prefix.
    #[test]
    fn incremental_matches_full_randomized() {
        const FRAGMENTS: &[&str] = &[
            "$( ",
            ") ",
            "$((",
            ")) ",
            "` ",
            "${x}",
            "${ ",
            "}",
            "'a'",
            "\"q",
            "q\"",
            "\\",
            "#c ",
            "case ",
            "esac",
            "esac)",
            " in ",
            "; ",
            ";; ",
            "&&",
            "|",
            "<<EOF",
            "EOF",
            "<<<",
            " x ",
            "echo ",
            "$(case a in b) esac)",
            " a(",
            "(( ",
            " $' ",
            "'",
            "\"",
            "$(",
            "`",
            ")",
            "${a:-${b",
            "${x}'",
            "$('",
            "}\"",
        ];
        let mut rng = Lcg(0x292_0000_0002);
        for _case in 0..4000u64 {
            let fragment_count = 2 + (rng.next() % 6) as usize;
            let mut fragments = Vec::with_capacity(fragment_count);
            for _ in 0..fragment_count {
                fragments.push(FRAGMENTS[(rng.next() as usize) % FRAGMENTS.len()]);
            }
            let max_lines = 4.min(fragment_count) as u64;
            let line_count = (1 + (rng.next() % max_lines)) as usize;
            let mut lines: Vec<String> = vec![String::new(); line_count];
            for (i, fragment) in fragments.iter().enumerate() {
                let target = if line_count == 1 {
                    0
                } else {
                    (i * line_count / fragment_count).min(line_count - 1)
                };
                lines[target].push_str(fragment);
            }
            let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
            assert_incremental_matches_full(&lines);
        }
    }
}

#[cfg(test)]
mod subscript_residual_incremental_tests {
    use super::subscript_residuals_advance;
    use super::SubscriptResidualState;
    use crate::lexer::skip::unclosed_array_subscript_line as oracle;

    /// Same driving pattern as the balanced tests: one raw line (with its
    /// '\n') per prefix; end state and reported payload must equal the full
    /// scan / the skip.rs oracle at every prefix.
    fn assert_incremental_matches_full(lines: &[&str]) {
        let mut chars: Vec<char> = Vec::new();
        let mut resume = 0usize;
        let mut snapshot = SubscriptResidualState::default();
        for (line_index, line) in lines.iter().enumerate() {
            chars.extend(line.chars());
            chars.push('\n');
            let mut state = snapshot.clone();
            let park = subscript_residuals_advance(&chars, resume, &mut state);
            let mut full = SubscriptResidualState::default();
            let _ = subscript_residuals_advance(&chars, 0, &mut full);
            let text: String = chars.iter().collect();
            assert_eq!(
                state, full,
                "end state at prefix {line_index} of {lines:?} ({text:?})"
            );
            assert_eq!(
                state.reported,
                oracle(&text),
                "reported payload at prefix {line_index} of {lines:?} ({text:?})"
            );
            assert_eq!(
                state.is_open(),
                oracle(&text).is_some(),
                "open flag at prefix {line_index} of {lines:?} ({text:?})"
            );
            match park {
                Some(park) => {
                    resume = park.pos;
                    snapshot = park.snapshot;
                }
                None => {
                    resume = chars.len();
                    snapshot = state;
                }
            }
        }
        // Without the final '\n'.
        let mut chars: Vec<char> = Vec::new();
        let mut resume = 0usize;
        let mut snapshot = SubscriptResidualState::default();
        for (line_index, line) in lines.iter().enumerate() {
            chars.extend(line.chars());
            if line_index + 1 < lines.len() {
                chars.push('\n');
            }
            let mut state = snapshot.clone();
            let park = subscript_residuals_advance(&chars, resume, &mut state);
            let mut full = SubscriptResidualState::default();
            let _ = subscript_residuals_advance(&chars, 0, &mut full);
            let text: String = chars.iter().collect();
            assert_eq!(state, full, "no-final-nl state at prefix {line_index}");
            assert_eq!(
                state.reported,
                oracle(&text),
                "no-final-nl reported at prefix {line_index} of {lines:?} ({text:?})"
            );
            match park {
                Some(park) => {
                    resume = park.pos;
                    snapshot = park.snapshot;
                }
                None => {
                    resume = chars.len();
                    snapshot = state;
                }
            }
        }
    }

    #[test]
    fn incremental_matches_full_required_shapes() {
        // parse.y:5635-5643: an unclosed `[` at a command-position
        // identifier swallows the rest of the input (rubash#221).
        assert_incremental_matches_full(&["a[b", "c"]);
        assert_incremental_matches_full(&["arr[x", "=1"]);
        // Inside a compound assignment the subscript reports compassign.
        assert_incremental_matches_full(&["x=([a", "b]=1)"]);
        assert_incremental_matches_full(&["declare -a d=([0", "]=a [1]=b)"]);
        // `echo a[b` stays literal: `a` is an argument, not command
        // position (parse.y:5899).
        assert_incremental_matches_full(&["echo a[b", "c"]);
        // x=a[b has no identifier prefix at `[` — stays an assignment value.
        assert_incremental_matches_full(&["x=a[b", "c"]);
        // Closed subscripts never report.
        assert_incremental_matches_full(&["arr[0]=1", "arr[1]=2"]);
        assert_incremental_matches_full(&["x=([a]=1", "[b]=2)"]);
        // Reserved words keep the next word in command position.
        assert_incremental_matches_full(&["if a[b", "then", "fi"]);
        // Command substitutions own their internals (balanced units skip).
        assert_incremental_matches_full(&["echo $(a [b", "c)"]);
        // Backtick bodies.
        assert_incremental_matches_full(&["echo `a [b", "c`"]);
        // Comments: `#` at word start swallows the line.
        assert_incremental_matches_full(&["# a[b", "c"]);
        // $'...' ANSI-C bodies.
        assert_incremental_matches_full(&["echo $'a[b'", "x[y"]);
    }

    #[test]
    fn incremental_matches_full_park_shapes() {
        // S1: the `[` only closes on a later line — the report fires on
        // every intermediate prefix and clears when the `]` arrives.
        assert_incremental_matches_full(&["a[b", "] =1", "c"]);
        assert_incremental_matches_full(&["a[0", "]", "+1]=x"]);
        // Nested `[ ... ]` pairs inside the subscript.
        assert_incremental_matches_full(&["a[b[0", "]]", "=x"]);
        // Quotes inside the subscript body.
        assert_incremental_matches_full(&["a['b", "']", "=1"]);
        assert_incremental_matches_full(&["a[\"b", "\"]", "=1"]);
        // S2: a `$(...` unit whose skip fails per line until it closes.
        assert_incremental_matches_full(&["a[$(b", "c)]", "=1"]);
        assert_incremental_matches_full(&["a[$(case x in x) esac)", ";; )]", "=1"]);
        // S3: a backtick that only closes on a later line.
        assert_incremental_matches_full(&["a[`b", "c`]", "=1"]);
        // S4: a `$` at the buffer tail whose `$'`/`$(` lookahead
        // materializes on the next line.
        assert_incremental_matches_full(&["a[$", "(b)]"]);
        assert_incremental_matches_full(&["a[$", "'b']"]);
        // Command position tracking across newlines and separators.
        assert_incremental_matches_full(&["x=1;", "a[b", "] =2"]);
        assert_incremental_matches_full(&["f () {", "a[b", "] =1;", "}"]);
    }

    /// Deterministic LCG so a failure reproduces bit for bit.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
    }

    /// Seeded fuzz (>=2000 required; 4000 for parity).
    #[test]
    fn incremental_matches_full_randomized() {
        const FRAGMENTS: &[&str] = &[
            "a[",
            "b]",
            "[",
            "]",
            "=[",
            "x=(",
            "(",
            ")",
            "'q'",
            "\"q",
            "q\"",
            "$(",
            "))",
            "$((",
            "`",
            "`x",
            "$'",
            "'",
            "\"",
            "\\",
            "#c ",
            "if ",
            "then",
            "fi",
            "while ",
            "do",
            "done",
            "case ",
            "esac",
            " in ",
            "; ",
            "&&",
            "|",
            " a ",
            "echo ",
            " ] ",
            "$(case a in b) esac)",
            " a[0]",
            "[[",
            "]]",
            "${x[",
            "]}",
        ];
        let mut rng = Lcg(0x292_0000_0003);
        for _case in 0..4000u64 {
            let fragment_count = 2 + (rng.next() % 6) as usize;
            let mut fragments = Vec::with_capacity(fragment_count);
            for _ in 0..fragment_count {
                fragments.push(FRAGMENTS[(rng.next() as usize) % FRAGMENTS.len()]);
            }
            let max_lines = 4.min(fragment_count) as u64;
            let line_count = (1 + (rng.next() % max_lines)) as usize;
            let mut lines: Vec<String> = vec![String::new(); line_count];
            for (i, fragment) in fragments.iter().enumerate() {
                let target = if line_count == 1 {
                    0
                } else {
                    (i * line_count / fragment_count).min(line_count - 1)
                };
                lines[target].push_str(fragment);
            }
            let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
            assert_incremental_matches_full(&lines);
        }
    }
}

#[cfg(test)]
mod close_char_residual_incremental_tests {
    use super::close_char_residuals_advance;
    use super::unclosed_input_close_char_posix as oracle;
    use super::CloseCharResidualState;

    fn assert_incremental_matches_full(lines: &[&str], posix: bool) {
        let mut chars: Vec<char> = Vec::new();
        let mut resume = 0usize;
        let mut snapshot = CloseCharResidualState::default();
        for (line_index, line) in lines.iter().enumerate() {
            chars.extend(line.chars());
            chars.push('\n');
            let mut state = snapshot.clone();
            let park = close_char_residuals_advance(&chars, resume, &mut state, posix);
            let mut full = CloseCharResidualState::default();
            let _ = close_char_residuals_advance(&chars, 0, &mut full, posix);
            let text: String = chars.iter().collect();
            assert_eq!(
                state, full,
                "end state at prefix {line_index} of {lines:?} ({text:?})"
            );
            assert_eq!(
                state.is_open(),
                oracle(&text, posix).is_some(),
                "open flag at prefix {line_index} of {lines:?} ({text:?})"
            );
            match park {
                Some(park) => {
                    resume = park.pos;
                    snapshot = park.snapshot;
                }
                None => {
                    resume = chars.len();
                    snapshot = state;
                }
            }
        }
        // Without the final '\n'.
        let mut chars: Vec<char> = Vec::new();
        let mut resume = 0usize;
        let mut snapshot = CloseCharResidualState::default();
        for (line_index, line) in lines.iter().enumerate() {
            chars.extend(line.chars());
            if line_index + 1 < lines.len() {
                chars.push('\n');
            }
            let mut state = snapshot.clone();
            let park = close_char_residuals_advance(&chars, resume, &mut state, posix);
            let mut full = CloseCharResidualState::default();
            let _ = close_char_residuals_advance(&chars, 0, &mut full, posix);
            let text: String = chars.iter().collect();
            assert_eq!(state, full, "no-final-nl state at prefix {line_index}");
            assert_eq!(
                state.is_open(),
                oracle(&text, posix).is_some(),
                "no-final-nl open at prefix {line_index} of {lines:?} ({text:?})"
            );
            match park {
                Some(park) => {
                    resume = park.pos;
                    snapshot = park.snapshot;
                }
                None => {
                    resume = chars.len();
                    snapshot = state;
                }
            }
        }
    }

    fn assert_both_modes(lines: &[&str]) {
        assert_incremental_matches_full(lines, false);
        assert_incremental_matches_full(lines, true);
    }

    #[test]
    fn incremental_matches_full_required_shapes() {
        // Multi-line quotes, backticks, `$(...)`, `${...}`.
        assert_both_modes(&["echo 'a", "b' c"]);
        assert_both_modes(&["echo \"a", "b\" c"]);
        assert_both_modes(&["echo `a", "b` c"]);
        assert_both_modes(&["echo $(a", "b) c"]);
        assert_both_modes(&["echo ${a", "} c"]);
        assert_both_modes(&["echo $((", "1+2))"]);
        // Subshells and array lists at command position.
        assert_both_modes(&["(a", "b)"]);
        assert_both_modes(&["x=(1", "2)"]);
        assert_both_modes(&["declare -a d=(a", "b)"]);
        // `${ x; }` function substitution (parse.y:5506 FUNSUB_CHAR).
        assert_both_modes(&["echo ${ a", "; } b"]);
        assert_both_modes(&["echo ${ a", "b"]); // never closed
                                                // `{ cmd; }` group inside a funsub.
        assert_both_modes(&["echo ${ { a; }", "} b"]);
        // Nested subshell inside `$( )`.
        assert_both_modes(&["echo $(( (a", ") ))"]);
        assert_both_modes(&["echo $( (a", ") )"]);
        // Comments at top level and inside substitutions.
        assert_both_modes(&["echo a # ) ' \"", "b"]);
        assert_both_modes(&["echo $(a # ) ' ", "b)"]);
        // `$'` ANSI-C inside grouping constructs (rubash#222).
        assert_both_modes(&["echo ${foo/$a/$'", "'}"]);
        // Escapes and word continuations.
        assert_both_modes(&["echo a\\", "b"]);
        assert_both_modes(&["echo $(a \\", "b)"]);
        // POSIX Interp 221: `'` inside `"${...}"` is literal text.
        assert_both_modes(&["echo \"${IFS+'bar", "}\" x"]);
        // Closed everything.
        assert_both_modes(&["echo a b", "c d"]);
    }

    #[test]
    fn incremental_matches_full_park_shapes() {
        // K1: `${` at the buffer tail — the FUNSUB_CHAR lookahead
        // materializes on the next line (both outcomes).
        assert_both_modes(&["echo ${", " x; } b"]);
        assert_both_modes(&["echo ${", "a} b"]);
        assert_both_modes(&["echo ${", ""]);
        // K2: `$(` at the buffer tail — the `$((` lookahead decides the
        // stack shape (one push vs two).
        assert_both_modes(&["echo $(", "(1+2))"]);
        assert_both_modes(&["echo $(", "a)"]);
        assert_both_modes(&["echo $(", ""]);
        // K3: `$` at the buffer tail.
        assert_both_modes(&["echo $", "'a'"]);
        assert_both_modes(&["echo $", "x"]);
        assert_both_modes(&["echo $", ""]);
        // K4: top-level `\` at the buffer tail.
        assert_both_modes(&["echo a\\", "b"]);
        assert_both_modes(&["echo x=\\", "(a)"]);
        // term_ready funsub closure across lines (`${ a` then `; }`).
        assert_both_modes(&["echo ${ a", "; }"]);
        assert_both_modes(&["echo ${ a; }", ""]);
        // The funsub `}` only closes at command position after a
        // terminator; a closed subshell inside arms it.
        assert_both_modes(&["echo ${ (a)", "; }"]);
        assert_both_modes(&["echo ${ (a", "); }"]);
        // Mid-command `(` is not a delimiter (echo ( stays open? no —
        // oracle decides; the incremental must just agree).
        assert_both_modes(&["echo (a", "b)"]);
        // Backslash-escaped newlines feeding cur_word (array list shape).
        assert_both_modes(&["x=a\\", "=(1 2)"]);
    }

    /// Deterministic LCG so a failure reproduces bit for bit.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
    }

    /// Seeded fuzz (>=2000 required; 4000 for parity) over both POSIX
    /// modes.
    #[test]
    fn incremental_matches_full_randomized() {
        const FRAGMENTS: &[&str] = &[
            "$( ", ") ", "$((", ")) ", "` ", "${", "${ ", "${x}", "}", "'a'", "\"q", "q\"", "\\",
            "#c ", "case ", "esac", " in ", "; ", ";; ", "&&", "|", " x ", "echo ", " a(", "(( ",
            " $' ", "'", "\"", "$(", "`", ")", "={", "${a:-${b", "${x'}", "}\"", "( ", " { ", "} ",
            "a=(", "a=(1 ", "$('", "((", "time ", "coproc ",
        ];
        let mut rng = Lcg(0x292_0000_0004);
        for _case in 0..4000u64 {
            let fragment_count = 2 + (rng.next() % 6) as usize;
            let mut fragments = Vec::with_capacity(fragment_count);
            for _ in 0..fragment_count {
                fragments.push(FRAGMENTS[(rng.next() as usize) % FRAGMENTS.len()]);
            }
            let max_lines = 4.min(fragment_count) as u64;
            let line_count = (1 + (rng.next() % max_lines)) as usize;
            let mut lines: Vec<String> = vec![String::new(); line_count];
            for (i, fragment) in fragments.iter().enumerate() {
                let target = if line_count == 1 {
                    0
                } else {
                    (i * line_count / fragment_count).min(line_count - 1)
                };
                lines[target].push_str(fragment);
            }
            let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
            let posix = _case % 2 == 0;
            assert_incremental_matches_full(&lines, posix);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::has_unclosed_quotes;

    #[test]
    fn command_substitution_quotes_do_not_leak_from_outer_double_quote() {
        let input = r#"echo \"$(echo \"\${IFS+'}'z}\")\""#;
        assert!(!has_unclosed_quotes(input));
    }
}

#[cfg(test)]
mod comsub_residual_incremental_tests {
    use super::{comsub_residuals_advance, ComsubResidualState};

    /// Drive the checkpoint exactly like `GroupScanFeeder::advance_comsub_scan`
    /// (restore snapshot at `resume`, advance over the buffer, store park or
    /// stable end) and assert at EVERY physical-line prefix that:
    ///   1. the advance's end state equals the full scan of that prefix
    ///      (all fourteen fields), and
    ///   2. the open flag the join gate would read equals the full-scan flag.
    /// The logical line is '\n'-joined like `push_main_line` appends (the
    /// no-separator backslash-join is a feeder-level invalidation, covered by
    /// the lexer tokenize tests).
    fn assert_incremental_matches_full(lines: &[&str]) {
        let mut chars: Vec<char> = Vec::new();
        let mut resume = 0usize;
        let mut snapshot = ComsubResidualState::default();
        for (line_index, line) in lines.iter().enumerate() {
            if line_index > 0 {
                chars.push('\n');
            }
            chars.extend(line.chars());
            let mut state = snapshot.clone();
            let park = comsub_residuals_advance(&chars, resume, &mut state);
            let mut full = ComsubResidualState::default();
            let _ = comsub_residuals_advance(&chars, 0, &mut full);
            assert_eq!(state, full, "end state at prefix {line_index} of {lines:?}");
            assert_eq!(
                state.is_open(),
                full.is_open(),
                "open flag at prefix {line_index} of {lines:?}"
            );
            match park {
                Some(park) => {
                    resume = park.pos;
                    snapshot = park.snapshot;
                }
                None => {
                    resume = chars.len();
                    snapshot = state;
                }
            }
        }
    }

    #[test]
    fn incremental_matches_full_required_shapes() {
        // rubash#292 plan B equivalence shapes (task list):
        // cross-line $(case ... esac)
        assert_incremental_matches_full(&["echo $(case a in a) echo x", "esac)"]);
        assert_incremental_matches_full(&["$(case x in", "a) :;;", "esac)"]);
        // nested ${...}
        assert_incremental_matches_full(&["echo ${ printf '%s' a", "} ${x}", "}"]);
        assert_incremental_matches_full(&["echo ${x^${y", "}}"]);
        // heredoc inside a command substitution (body bytes stay opaque)
        assert_incremental_matches_full(&["echo $(cat <<eof", "here doc with )", "eof", ")"]);
        assert_incremental_matches_full(&["echo $(cat <<eof", "body ( \" x", "eof`", ")"]);
        // backslash continuation state (escaped char on the next line)
        assert_incremental_matches_full(&["echo $(echo a\\", "b)"]);
        assert_incremental_matches_full(&["echo $(echo 'q\\", "z')"]);
        // plain multi-line comsub
        assert_incremental_matches_full(&["$(echo a", "b)"]);
        // top-level backtick comsub spanning lines
        assert_incremental_matches_full(&["echo `date", "+%s` after"]);
        // $'...' spanning lines
        assert_incremental_matches_full(&["echo $'a\\n", "b'"]);
    }

    #[test]
    fn incremental_matches_full_park_shapes() {
        // A `$(` opened inside a double-quoted word: the atomic unit skip is
        // what lets the full scan close the unit past the residual
        // double-quote state, so the checkpoint MUST re-derive the skip on
        // the next line (`"$(fo` + `o)"` closes at prefix 2 only via the park).
        assert_incremental_matches_full(&["x=\"$(fo", "o)\""]);
        assert_incremental_matches_full(&["x=\"$(a $(b", "c))\""]);
        // `$((` arithmetic that only balances on a later line.
        assert_incremental_matches_full(&["echo $((", "1+", "2))"]);
        // P2: the unit closes through an `esac)` lookahead that runs off the
        // buffer end undecided; the next line's `;;` flips the internal case
        // decision and keeps the unit open.
        assert_incremental_matches_full(&["echo $(case a in x) esac)", ";; )"]);
        assert_incremental_matches_full(&["echo $(case a in x) esac)", "esac)"]);
        // P3: a heredoc opened inside a top-level backtick substitution.
        assert_incremental_matches_full(&["echo `cat <<EOF", "body ) x", "EOF", "` x"]);
        // Backtick substitution inside a `$(` that stays open across lines.
        assert_incremental_matches_full(&["echo $(x `date", "`)"]);
        // Comment at depth 0 split across lines.
        assert_incremental_matches_full(&["echo a #c", "d", "e"]);
    }

    /// Deterministic LCG so a failure reproduces bit for bit.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
    }

    /// Seeded fuzz: random token soups from a construct-dense alphabet,
    /// split into physical lines at fragment boundaries, must satisfy the
    /// incremental == full contract at every prefix. This is the class
    /// proof for the park mechanism (a green targeted list only proves the
    /// listed cases).
    #[test]
    fn incremental_matches_full_randomized() {
        const FRAGMENTS: &[&str] = &[
            "$( ",
            ") ",
            "$((",
            ")) ",
            "` ",
            "${x}",
            "}",
            "'a'",
            "\"q",
            "q\"",
            "\\",
            "#c ",
            "case ",
            "esac",
            "esac)",
            " in ",
            "; ",
            ";; ",
            "&&",
            "|",
            "<<EOF",
            "EOF",
            "<<-E",
            " x ",
            " y",
            "echo ",
            "$(case a in b) esac)",
            "<<<",
            " a(",
            "(( ",
            " $' ",
            "'",
            "\"",
            "$(",
            "`",
            ")",
            "${ ",
            "${a:-${b",
            "}",
            "\\n",
        ];
        let mut rng = Lcg(0x292_0000_0001);
        for case in 0..4000u64 {
            let fragment_count = 2 + (rng.next() % 6) as usize;
            let mut fragments = Vec::with_capacity(fragment_count);
            for _ in 0..fragment_count {
                fragments.push(FRAGMENTS[(rng.next() as usize) % FRAGMENTS.len()]);
            }
            // Split the fragment list into 1..=4 physical lines.
            let max_lines = 4.min(fragment_count) as u64;
            let line_count = (1 + (rng.next() % max_lines)) as usize;
            let mut lines: Vec<String> = vec![String::new(); line_count];
            for (i, fragment) in fragments.iter().enumerate() {
                let target = if line_count == 1 {
                    0
                } else {
                    (i * line_count / fragment_count).min(line_count - 1)
                };
                lines[target].push_str(fragment);
            }
            let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
            assert_incremental_matches_full(&lines);
        }
    }
}

#[cfg(test)]
mod quotes_residual_incremental_tests {
    use super::{has_unclosed_quotes, quotes_residuals_advance, QuotesResidualState};

    /// Drive the checkpoint exactly like `GroupScanFeeder::advance_quotes_scan`
    /// (restore snapshot at `resume`, advance over the buffer, store park or
    /// stable end) and assert at EVERY physical-line prefix that:
    ///   1. the advance's end state equals the full scan of that prefix
    ///      (all six fields), and
    ///   2. the open flag the join gate would read equals the full-scan flag
    ///      (and the full-buffer oracle `has_unclosed_quotes`).
    /// The logical line is '\n'-joined like `push_main_line` appends (the
    /// no-separator backslash-join is a feeder-level invalidation, covered by
    /// the lexer tokenize tests).
    fn assert_incremental_matches_full(lines: &[&str]) {
        let mut chars: Vec<char> = Vec::new();
        let mut resume = 0usize;
        let mut snapshot = QuotesResidualState::default();
        for (line_index, line) in lines.iter().enumerate() {
            if line_index > 0 {
                chars.push('\n');
            }
            chars.extend(line.chars());
            let mut state = snapshot.clone();
            let park = quotes_residuals_advance(&chars, resume, &mut state);
            let mut full = QuotesResidualState::default();
            let _ = quotes_residuals_advance(&chars, 0, &mut full);
            let text: String = chars.iter().collect();
            assert_eq!(
                state, full,
                "end state at prefix {line_index} of {lines:?} ({text:?})"
            );
            assert_eq!(
                state.is_open(),
                full.is_open(),
                "open flag at prefix {line_index} of {lines:?} ({text:?})"
            );
            assert_eq!(
                state.is_open(),
                has_unclosed_quotes(&text),
                "oracle answer at prefix {line_index} of {lines:?} ({text:?})"
            );
            match park {
                Some(park) => {
                    resume = park.pos;
                    snapshot = park.snapshot;
                }
                None => {
                    resume = chars.len();
                    snapshot = state;
                }
            }
        }
    }

    #[test]
    fn incremental_matches_full_required_shapes() {
        // plain multi-line comsub keeps quotes inside the unit
        assert_incremental_matches_full(&["echo $(echo 'a", "b') x"]);
        // nested ${...} inside a comsub body
        assert_incremental_matches_full(&["echo $(echo ${x", "})"]);
        // heredoc inside a command substitution (body bytes stay opaque)
        assert_incremental_matches_full(&["echo $(cat <<eof", "here doc with )", "eof", ")"]);
        // backslash continuation state (escaped char on the next line)
        assert_incremental_matches_full(&["echo $(echo a\\", "b)"]);
        assert_incremental_matches_full(&["echo $(echo 'q\\", "z')"]);
        // top-level backtick comsub spanning lines
        assert_incremental_matches_full(&["echo `date", "+%s` after"]);
        // $'...' spanning lines
        assert_incremental_matches_full(&["echo $'a\\n", "b'"]);
        // plain multi-line single/double quotes
        assert_incremental_matches_full(&["echo 'a", "b'"]);
        assert_incremental_matches_full(&["echo \"a", "b\""]);
        // comments do not eat quote state
        assert_incremental_matches_full(&["echo a # 'x", "b"]);
    }

    #[test]
    fn incremental_matches_full_park_shapes() {
        // A `$(` opened inside a double-quoted word: the atomic unit skip is
        // what lets the full scan close the unit past the residual
        // double-quote state, so the checkpoint MUST re-derive the skip on
        // the next line (`x="$(fo` + `o)"` closes at prefix 2 only via the park).
        assert_incremental_matches_full(&["x=\"$(fo", "o)\""]);
        assert_incremental_matches_full(&["x=\"$(a $(b", "c))\""]);
        // `$(` whose body only closes on a later line.
        assert_incremental_matches_full(&["echo $(a", "b)"]);
        // `${` span that only closes on a later line: the park must
        // re-derive the span jump (body quotes are data inside the span).
        assert_incremental_matches_full(&["echo ${x", "\"} y"]);
        assert_incremental_matches_full(&["echo ${a:-${b", "}}"]);
        // `${` span inside double quotes: the two-context scan (posix then
        // non-posix) is re-derived per line until one closes.
        assert_incremental_matches_full(&["v=\"${IFS+'}", "'z}\" x"]);
        // backtick unit that only closes on a later line.
        assert_incremental_matches_full(&["echo `a", "b` c"]);
        // P2: the unit closes through an `esac)` lookahead that runs off the
        // buffer end undecided; the next line's `;;` flips the internal case
        // decision and moves the closure point.
        assert_incremental_matches_full(&["echo $(case a in x) esac)", ";; )"]);
        assert_incremental_matches_full(&["echo $(case a in x) esac)", "esac)"]);
        // Comment at top level split across lines.
        assert_incremental_matches_full(&["echo a #c", "d", "e"]);
    }

    /// Deterministic LCG (same generator as the comsub fuzz module).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
    }

    /// Seeded fuzz for the quotes class (rubash#292 discipline, perf8):
    /// random quote/comsub/span-dense soups split into physical lines must
    /// satisfy incremental == full at every prefix, and the incremental open
    /// flag must equal the `has_unclosed_quotes` oracle.
    #[test]
    fn incremental_matches_full_randomized() {
        const FRAGMENTS: &[&str] = &[
            "'",
            "\"",
            "`",
            "$(",
            "${",
            "$'",
            "}",
            ")",
            "'a'",
            "\"q",
            "q\"",
            "`x`",
            "${x}",
            "${a:-${b",
            "$( ",
            "$((",
            ")) ",
            "\\",
            "\\\\",
            "#c ",
            " x ",
            "echo ",
            "case ",
            "esac",
            "esac)",
            ";; ",
            "; ",
            "&&",
            "in ",
            "<<EOF",
            "EOF",
            "<<<",
            "a=(",
            "declare -a ",
            "local ",
        ];
        let mut rng = Lcg(0x7066_3800_0001);
        for case in 0..3000u64 {
            let fragment_count = 2 + (rng.next() % 6) as usize;
            let mut fragments = Vec::with_capacity(fragment_count);
            for _ in 0..fragment_count {
                fragments.push(FRAGMENTS[(rng.next() as usize) % FRAGMENTS.len()]);
            }
            let max_lines = 4.min(fragment_count) as u64;
            let line_count = (1 + (rng.next() % max_lines)) as usize;
            let mut lines: Vec<String> = vec![String::new(); line_count];
            for (i, fragment) in fragments.iter().enumerate() {
                let target = if line_count == 1 {
                    0
                } else {
                    (i * line_count / fragment_count).min(line_count - 1)
                };
                lines[target].push_str(fragment);
            }
            let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
            assert_incremental_matches_full(&lines);
        }
    }
}

#[cfg(test)]
mod compound_residual_incremental_tests {
    use super::{
        compound_residuals_advance, has_unclosed_compound_assignment, CompoundResidualState,
    };

    /// Drive the checkpoint exactly like
    /// `GroupScanFeeder::advance_compound_scan` and assert at EVERY
    /// physical-line prefix that the advance's end state equals the full
    /// scan of that prefix (all twelve fields, `forced_closed` included)
    /// and the open flag equals both the full scan and the
    /// `has_unclosed_compound_assignment` oracle.
    fn assert_incremental_matches_full(lines: &[&str]) {
        let mut chars: Vec<char> = Vec::new();
        let mut resume = 0usize;
        let mut snapshot = CompoundResidualState::default();
        for (line_index, line) in lines.iter().enumerate() {
            if line_index > 0 {
                chars.push('\n');
            }
            chars.extend(line.chars());
            let mut state = snapshot.clone();
            let park = compound_residuals_advance(&chars, resume, &mut state);
            let mut full = CompoundResidualState::default();
            let _ = compound_residuals_advance(&chars, 0, &mut full);
            let text: String = chars.iter().collect();
            assert_eq!(
                state, full,
                "end state at prefix {line_index} of {lines:?} ({text:?})"
            );
            assert_eq!(
                state.is_open(),
                full.is_open(),
                "open flag at prefix {line_index} of {lines:?} ({text:?})"
            );
            assert_eq!(
                state.is_open(),
                has_unclosed_compound_assignment(&text),
                "oracle answer at prefix {line_index} of {lines:?} ({text:?})"
            );
            match park {
                Some(park) => {
                    resume = park.pos;
                    snapshot = park.snapshot;
                }
                None => {
                    resume = chars.len();
                    snapshot = state;
                }
            }
        }
    }

    #[test]
    fn incremental_matches_full_required_shapes() {
        // ISSUE #78 family: multi-line compound array assignment.
        assert_incremental_matches_full(&["plugins=(", "a", "b", ")"]);
        assert_incremental_matches_full(&["declare -a d=(", "1", "2", ")"]);
        assert_incremental_matches_full(&["local arr+=(", "x", ")"]);
        // nested parens inside the compound list
        assert_incremental_matches_full(&["a=(", "b (", "c))"]);
        // words straddling the line boundary (word accumulation is carried)
        assert_incremental_matches_full(&["plug", "ins=(a b", ")"]);
        // `${` span inside the list closing on a later line
        assert_incremental_matches_full(&["arr=(${x", "})"]);
        // `$(` inside the list closing on a later line
        assert_incremental_matches_full(&["arr=($(echo a", "))"]);
        // comments inside the list do not close it
        assert_incremental_matches_full(&["a=(", "1 # ) not a closer", "2", ")"]);
        // quotes inside the list elements
        assert_incremental_matches_full(&["a=(", "'b ()'", "\"c )\"", ")"]);
        // backtick unit inside the list closing on a later line
        assert_incremental_matches_full(&["a=(x `echo y", "`)"]);
    }

    #[test]
    fn incremental_matches_full_park_shapes() {
        // Unbalanced `$(` inside a compound list: forced_closed (the
        // original early `return false`) commits for every prefix where the
        // unit does not close, and the park re-derives it so the prefix
        // that finally closes the unit re-opens the compound answer.
        assert_incremental_matches_full(&["a=(x $(b", "c"]);
        assert_incremental_matches_full(&["a=(x $(b", "c))"]);
        assert_incremental_matches_full(&["a=(x $(b", "c)", ")"]);
        // Unbalanced backtick: same forced_closed discipline.
        assert_incremental_matches_full(&["a=(x `b", "c"]);
        assert_incremental_matches_full(&["a=(x `b", "c`)"]);
        // `${` span whose jump is only decided on the later line.
        assert_incremental_matches_full(&["a=(${x:-'", "'} b", ")"]);
        // An `esac)` lookahead inside a `$(` unit that runs off the end.
        assert_incremental_matches_full(&["a=($(case a in x) esac)", ";; )"]);
        // A `name=(` that only appears on the second line.
        assert_incremental_matches_full(&["echo x", "a=(b", ")"]);
    }

    /// Deterministic LCG (same generator as the comsub fuzz module).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
    }

    /// Seeded fuzz for the compound class: random assignment/command-dense
    /// soups split into physical lines must satisfy incremental == full at
    /// every prefix (including the forced_closed early-terminal).
    #[test]
    fn incremental_matches_full_randomized() {
        const FRAGMENTS: &[&str] = &[
            "a=(",
            "b=(",
            "declare -a ",
            "declare ",
            "local ",
            "export ",
            "readonly ",
            "typeset ",
            "a+=",
            "a=",
            "echo ",
            "x ",
            ")",
            "(",
            "${",
            "}",
            "${x}",
            "$(",
            "$( ",
            ")) ",
            "`",
            "`x`",
            "'",
            "\"",
            "'q'",
            "\"q\"",
            "$'",
            "\\",
            "#c ",
            "; ",
            "&&",
            "|",
            " in ",
            "case ",
            "esac",
            "esac)",
            ";; ",
            "-a ",
            "-f ",
        ];
        let mut rng = Lcg(0x636f_6d70_0001);
        for case in 0..3000u64 {
            let fragment_count = 2 + (rng.next() % 6) as usize;
            let mut fragments = Vec::with_capacity(fragment_count);
            for _ in 0..fragment_count {
                fragments.push(FRAGMENTS[(rng.next() as usize) % FRAGMENTS.len()]);
            }
            let max_lines = 4.min(fragment_count) as u64;
            let line_count = (1 + (rng.next() % max_lines)) as usize;
            let mut lines: Vec<String> = vec![String::new(); line_count];
            for (i, fragment) in fragments.iter().enumerate() {
                let target = if line_count == 1 {
                    0
                } else {
                    (i * line_count / fragment_count).min(line_count - 1)
                };
                lines[target].push_str(fragment);
            }
            let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
            assert_incremental_matches_full(&lines);
        }
    }
}
