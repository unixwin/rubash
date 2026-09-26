use super::heredoc_scan::skip_heredoc_in_chars_with_closure;

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
#[derive(Clone, Copy)]
struct UnclosedDelim {
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
/// `${ ' followed by a FUNSUB_CHAR (parser.h:83-85: space, tab, newline,
/// '|', '(') is a ksh-style function substitution parsed by parse_comsub
/// (parse.y:5506): its `}' only closes at command position, so
/// `_[${ a }]' is unterminated while `_[${ a; }]' runs `a'.
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
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        let top = stack.last().copied();
        if ch == '\n' {
            line += 1;
        }
        if let Some(d) = top {
            if d.escapes && ch == '\\' {
                // An escaped character is word text everywhere.
                comment_start = false;
                i += 2;
                continue;
            }
            if ch == d.close && !(d.funsub && !d.term_ready) {
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
            // `$(# c )` cannot close the substitution (comsub-posix).
            if d.close == ')' || d.funsub {
                if !(d.close == ')' && d.report_open) && ch == '#' && comment_start {
                    while i < chars.len() && chars[i] != '\n' {
                        i += 1;
                    }
                    continue;
                }
                comment_start = ch.is_whitespace()
                    || matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
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
                            "if" | "then" | "else" | "elif" | "while" | "until" | "do"
                                | "in" | "!" | "time" | "coproc" | "case"
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
                    array_list: false,
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
                    array_list: false,
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
                    array_list: false,
                });
            }
            '$' => {
                match chars.get(i + 1) {
                    Some('{') => {
                        // parse.y:5506: `${' followed by a FUNSUB_CHAR is a
                        // function substitution parsed as commands; a
                        // parameter expansion takes parse_matched_pair
                        // (yyerror path: EOF line either way).
                        let funsub = chars
                            .get(i + 2)
                            .is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '|' | '('));
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
                            array_list: false,
                        });
                        if funsub {
                            comment_start = true;
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
                            array_list: false,
                        });
                        // A fresh substitution body starts at a token
                        // boundary: `$(#c` is a comment.
                        comment_start = true;
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
                                array_list: false,
                            });
                            i += 1;
                        }
                        i += 1;
                    }
                    Some('\'') if top.is_none() => {
                        // ANSI-C $'...': single-quote close, escapes live.
                        stack.push(UnclosedDelim {
                            close: '\'',
                            open_line: line,
                            escapes: true,
                            report_open: true,
                            funsub: false,
                            command: false,
                            term_ready: false,
                            array_list: false,
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
                        array_list: is_array_list,
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
                    array_list: false,
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
                    array_list: false,
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
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    let mut comment_start = true;
    let mut in_comment = false;
    let chars = input.chars().collect::<Vec<_>>();
    let mut index = 0usize;

    while index < chars.len() {
        let ch = chars[index];
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                comment_start = true;
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

        if ch == '\n' && !single && !double && !ansi_single {
            comment_start = true;
            index += 1;
            continue;
        }

        if ch == '#' && !single && !double && !ansi_single && comment_start {
            in_comment = true;
            index += 1;
            continue;
        }

        if ch.is_whitespace() && !single && !double && !ansi_single {
            comment_start = true;
            index += 1;
            continue;
        }

        if ch == '\\' && (!single || ansi_single) {
            escaped = true;
            comment_start = false;
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
        if ch == '$' && !single && chars.get(index + 1) == Some(&'{') {
            let body: String = chars[index + 2..].iter().collect();
            if !double {
                let context = crate::lexer::dolbrace::BraceContext {
                    outer_double_quote: false,
                    posix: false,
                    replacement_context: false,
                    initial_state: crate::lexer::dolbrace::DolbraceState::Param,
                };
                if let Some(scan) =
                    crate::lexer::dolbrace::scan_braced_parameter_body(&body, context)
                {
                    index += 2 + body[..scan.end].chars().count();
                    comment_start = false;
                    continue;
                }
                // Unterminated/odd expansion: fall through and let the caller
                // treat the input as having unclosed syntax.
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
                        crate::lexer::dolbrace::scan_braced_parameter_body(&body, context)
                    {
                        index += 2 + body[..scan.end].chars().count();
                        comment_start = false;
                        closed = true;
                        break;
                    }
                }
                if closed {
                    continue;
                }
                // Neither scan closes the span: fall through; the trailing
                // quote toggles leave the state as-is and the caller reports
                // unclosed input, matching the lexer's own fallback swallow.
            }
        }

        // Skip a command substitution `$(...)` (and `$((...))` arithmetic) as a
        // self-contained unit.
        // A balanced command substitution owns its nested quote state, even
        // when the substitution itself appears inside an outer double quote.
        if ch == '$' && !single && chars.get(index + 1) == Some(&'(') {
            if let Some(end) = skip_parenthesized_unit(&chars, index + 1) {
                index = end;
                comment_start = false;
                continue;
            }
        }

        // Skip a backtick command substitution as a self-contained unit.
        if ch == '`' && !single && !double {
            if let Some(end) = skip_backtick_unit(&chars, index) {
                index = end;
                comment_start = false;
                continue;
            }
        }

        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            comment_start = false;
            index += 2;
            continue;
        }

        match ch {
            '\'' if ansi_single => {
                ansi_single = false;
                comment_start = false;
            }
            '\'' if !double && !ansi_single => {
                single = !single;
                comment_start = false;
            }
            '"' if !single && !ansi_single => {
                double = !double;
                comment_start = false;
            }
            _ => {
                if !single && !double && !ansi_single {
                    comment_start = false;
                }
            }
        }
        index += 1;
    }

    single || double || ansi_single
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
    let chars = input.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut compound_depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    let mut in_comment = false;
    let mut comment_start = true;
    let mut word = String::new();
    let mut word_pure = true;
    let mut seen_command_word = false;
    let mut declaration_context = false;

    while index < chars.len() {
        let ch = chars[index];

        if in_comment {
            if ch == '\n' {
                in_comment = false;
                comment_start = true;
                if compound_depth == 0 {
                    seen_command_word = false;
                    declaration_context = false;
                }
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

        if ch == '\n' && !single && !double && !ansi_single {
            comment_start = true;
            if compound_depth == 0 {
                if !word.is_empty() {
                    classify_top_level_word(
                        &word,
                        &mut seen_command_word,
                        &mut declaration_context,
                    );
                    word.clear();
                    word_pure = true;
                }
                seen_command_word = false;
                declaration_context = false;
            }
            index += 1;
            continue;
        }

        if ch == '#' && !single && !double && !ansi_single && comment_start {
            in_comment = true;
            index += 1;
            continue;
        }

        if ch.is_whitespace() && !single && !double && !ansi_single {
            comment_start = true;
            if compound_depth == 0 && !word.is_empty() {
                classify_top_level_word(&word, &mut seen_command_word, &mut declaration_context);
                word.clear();
                word_pure = true;
            }
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
            word_pure = false;
            index += 1;
            continue;
        }

        if ch == '\\' && !single {
            escaped = true;
            comment_start = false;
            word_pure = false;
            index += 1;
            continue;
        }

        if ch == '$' && !single && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            comment_start = false;
            word_pure = false;
            index += 2;
            continue;
        }

        // A `${...}` parameter expansion is opaque to parenthesis balancing.
        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'{') {
            let body: String = chars[index + 2..].iter().collect();
            let context = crate::lexer::dolbrace::BraceContext {
                outer_double_quote: false,
                posix: false,
                replacement_context: false,
                initial_state: crate::lexer::dolbrace::DolbraceState::Param,
            };
            if let Some(scan) = crate::lexer::dolbrace::scan_braced_parameter_body(&body, context) {
                index += 2 + body[..scan.end].chars().count();
            } else {
                index += 2;
            }
            comment_start = false;
            word_pure = false;
            continue;
        }

        // A `$(...)` / `$((...))` command substitution is opaque to
        // parenthesis balancing; an unbalanced one is reported by
        // has_unclosed_command_substitution instead.
        if ch == '$' && !single && chars.get(index + 1) == Some(&'(') {
            if let Some(end) = skip_parenthesized_unit(&chars, index + 1) {
                index = end;
                comment_start = false;
                word_pure = false;
                continue;
            }
            return false;
        }

        if ch == '`' && !single && !double {
            if let Some(end) = skip_backtick_unit(&chars, index) {
                index = end;
                comment_start = false;
                word_pure = false;
                continue;
            }
            return false;
        }

        if ch == '\'' && !double {
            single = !single;
            comment_start = false;
            word_pure = false;
            index += 1;
            continue;
        }

        if ch == '"' && !single {
            double = !double;
            comment_start = false;
            word_pure = false;
            index += 1;
            continue;
        }

        if single || double {
            index += 1;
            continue;
        }

        if ch == '(' && compound_depth == 0 {
            let opens_compound = word_pure
                && word
                    .strip_suffix('=')
                    .map(|head| {
                        let head = head.strip_suffix('+').unwrap_or(head);
                        valid_identifier(head)
                    })
                    .unwrap_or(false)
                && (!seen_command_word || declaration_context);
            if opens_compound {
                compound_depth = 1;
                word.clear();
                word_pure = true;
                // A `#` may start a comment right after the opening paren.
                comment_start = true;
                index += 1;
                continue;
            }
            // A subshell/grouping paren ends the assignment-prefix region:
            // GNU reports `echo a=(b` immediately instead of continuing.
            if !word.is_empty() {
                classify_top_level_word(&word, &mut seen_command_word, &mut declaration_context);
                word.clear();
                word_pure = true;
            }
            seen_command_word = true;
            comment_start = true;
            index += 1;
            continue;
        }

        if ch == '(' && compound_depth > 0 {
            compound_depth += 1;
            comment_start = true;
            index += 1;
            continue;
        }

        if ch == ')' {
            if compound_depth > 0 {
                compound_depth -= 1;
                word.clear();
                word_pure = true;
            } else {
                if !word.is_empty() {
                    classify_top_level_word(
                        &word,
                        &mut seen_command_word,
                        &mut declaration_context,
                    );
                    word.clear();
                    word_pure = true;
                }
                seen_command_word = true;
            }
            comment_start = true;
            index += 1;
            continue;
        }

        if ch == ';' || ch == '|' || ch == '&' {
            if compound_depth == 0 && !word.is_empty() {
                classify_top_level_word(&word, &mut seen_command_word, &mut declaration_context);
                word.clear();
                word_pure = true;
                seen_command_word = false;
                declaration_context = false;
            }
            comment_start = true;
            index += 1;
            continue;
        }

        word.push(ch);
        comment_start = false;
        index += 1;
    }

    compound_depth > 0
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
    // GNU read_token (parse.y:3630-3643): `#` introduces a comment only at
    // a token boundary — after whitespace, a separator (`;&|()<>`), or at
    // the start. `word.is_empty()` alone is wrong: `$`, quotes and other
    // non-alphanumeric word characters never reach `word`, so `$(echo $#)`
    // and `$(echo 'a'#b)` would misread `#` as a comment.
    let mut token_boundary = true;
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
                return Some(next);
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
            update_command_substitution_case_depth(
                chars,
                index,
                ch,
                &mut word,
                &mut case_depth,
                &mut word_boundary,
                &mut current_word_boundary,
            );
            // GNU read_token_word (parse.y:5377-5397): outside quotes a
            // backslash quotes the next character — it can never act as a
            // paren delimiter, so `$(echo \)` does not close the
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
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        token_boundary = ch.is_whitespace()
            || matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
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
    let mut index = 0usize;
    let mut depth = 0usize;
    let mut backtick = false;
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    let mut comment_start = true;
    let mut in_comment = false;
    let mut case_depth = 0usize;
    let mut parameter_depth = 0usize;
    let mut parameter_single = false;
    let mut word = String::new();
    let mut word_boundary = true;
    let mut current_word_boundary = true;

    while index < chars.len() {
        let ch = chars[index];
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                comment_start = true;
            }
            index += 1;
            continue;
        }
        if escaped {
            escaped = false;
            comment_start = false;
            // A backslash-quoted character is word text (placeholder: `c\ase`
            // is not `case`), so a following `#` stays mid-word (`\;#` in
            // comsub1.sub). A quoted newline is handled by the `\` arm.
            word.push('\u{1}');
            index += 1;
            continue;
        }
        if ch == '\n' && !single && !double && !ansi_single && !backtick && depth == 0 {
            comment_start = true;
            index += 1;
            continue;
        }
        if ch == '#'
            && !single
            && !double
            && !ansi_single
            && !backtick
            && depth == 0
            && comment_start
        {
            in_comment = true;
            index += 1;
            continue;
        }
        if ch.is_whitespace() && !single && !double && !ansi_single && !backtick && depth == 0 {
            comment_start = true;
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
        if depth > 0 && ch == '`' && !single {
            index = skip_backtick_substitution(&chars, index);
            comment_start = false;
            continue;
        }
        if ch == '\'' && parameter_depth > 0 && !ansi_single {
            parameter_single = !parameter_single;
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
        if single {
            index += 1;
            continue;
        }
        if ch == '`' && depth == 0 {
            backtick = !backtick;
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '$'
            && chars.get(index + 1) == Some(&'{')
            && !single
            && !ansi_single
            && !parameter_single
        {
            parameter_depth += 1;
            // parse.y parse_matched_pair: inside `${...}` a `#` is parameter
            // operator text (the length operator in `${#x}`), never a
            // comment introducer — comment_start must clear here like every
            // other consumed non-space character, or `${#x}` is swallowed
            // to EOL and the `}` is never matched.
            comment_start = false;
            index += 2;
            continue;
        }
        if ch == '}' && parameter_depth > 0 {
            parameter_depth = parameter_depth.saturating_sub(1);
            parameter_single = false;
            comment_start = false;
            index += 1;
            continue;
        }
        // A balanced command substitution owns its nested quote state even
        // inside an outer double-quoted word. Keep it atomic here as well as
        // in has_unclosed_quotes; an unbalanced unit falls through so this
        // checker still reports the missing closing delimiter.
        if ch == '$' && !single && chars.get(index + 1) == Some(&'(') && !parameter_single {
            if let Some(end) = skip_parenthesized_unit(&chars, index + 1) {
                index = end;
                comment_start = false;
                continue;
            }
            if chars.get(index + 2) == Some(&'(') {
                if let Some(end) = skip_arithmetic_substitution(&chars, index + 3) {
                    index = end;
                    comment_start = false;
                    continue;
                }
                // POSIX permits command substitution when the text after
                // "$((" is not a valid arithmetic expression.
            }
            depth += 1;
            if depth == 1 {
                case_depth = 0;
                word.clear();
                word_boundary = true;
                current_word_boundary = true;
            }
            // The fast skip failed, so the body is scanned char-by-char
            // from here — and a substitution body begins at a token
            // boundary: `$(#c` is a comment.
            comment_start = true;
            index += 2;
            continue;
        }
        // GNU read_token_word (parse.y:3630-3643): `#` at a token boundary
        // begins a comment through end of line. `comment_start` tracks that
        // boundary; `word.is_empty()` alone is wrong because `$`, quotes and
        // other non-alphanumeric word characters never reach `word`
        // (`$(echo $#)`). `parameter_depth` keeps `${#x}` parameter text
        // out of the comment rule.
        if depth > 0
            && ch == '#'
            && !single
            && !double
            && !ansi_single
            && !backtick
            && parameter_depth == 0
            && comment_start
        {
            while index + 1 < chars.len() && chars[index + 1] != '\n' {
                index += 1;
            }
            word.clear();
            word_boundary = true;
            current_word_boundary = true;
            comment_start = true;
            index += 1;
            continue;
        }
        if depth > 0 && !ansi_single && !backtick {
            update_command_substitution_case_depth(
                &chars,
                index,
                ch,
                &mut word,
                &mut case_depth,
                &mut word_boundary,
                &mut current_word_boundary,
            );
        }
        if depth > 0
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) == Some(&'<')
        {
            index += 3;
            continue;
        }
        if depth > 0 && ch == '<' && chars.get(index + 1) == Some(&'<') {
            let (next, closes) = skip_heredoc_in_chars_with_closure(&chars, index);
            if closes.is_some() {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return (depth, backtick, ansi_single, parameter_depth);
                }
            }
            index = next;
            continue;
        }
        if backtick
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) == Some(&'<')
        {
            index += 3;
            continue;
        }
        if backtick && ch == '<' && chars.get(index + 1) == Some(&'<') {
            index = skip_heredoc_in_chars_with_closure(&chars, index).0;
            continue;
        }
        // GNU read_token_word (parse.y:5404-5418): inside double quotes a
        // `)` is literal text — it never balances a `$(` parenthesis. All
        // constructs that stay live inside `"..."` (`\x`, `$(`, `` ` ``,
        // `${`) were handled by the arms above; anything left is inert.
        if double {
            index += 1;
            continue;
        }
        if depth > 0 && case_depth == 0 && !ansi_single && ch == '(' {
            depth += 1;
        } else if depth > 0 && case_depth == 0 && !ansi_single && ch == ')' {
            depth -= 1;
        }
        if !single && !double && !ansi_single && !backtick {
            // GNU read_token: a token boundary follows whitespace and the
            // shell separators; every other live character continues or
            // begins a word, so a following `#` is mid-word text.
            comment_start = ch.is_whitespace()
                || matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
        }
        index += 1;
    }

    // parameter_depth covers both ${param...} (GNU reads past newlines in
    // parse_matched_pair looking for the closing `}`) and the nofork
    // command substitution `${ command; }` (parser.h:83 FUNSUB_CHAR,
    // parse.y:5407 PST_FUNSUBST close) — comsub2.tests splits
    // `echo ${ printf ...` + `}` across lines and must keep reading.
    (depth, backtick, ansi_single, parameter_depth)
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

fn update_command_substitution_case_depth(
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
        if command_substitution_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    let reserved_word_allows_next = match word.as_str() {
        "case" if *current_word_boundary => {
            *case_depth += 1;
            false
        }
        "esac" if *current_word_boundary && !case_pattern_starts_with_esac_chars(chars, index) => {
            *case_depth = case_depth.saturating_sub(1);
            true
        }
        "for" | "select" | "while" | "until" | "then" | "do" | "else" | "elif" | "in" | "fi"
        | "done"
            if *current_word_boundary =>
        {
            true
        }
        _ => false,
    };
    word.clear();
    *word_boundary =
        reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
}

fn command_substitution_separator_allows_reserved_word(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | ')' | '\n')
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

#[cfg(test)]
mod tests {
    use super::has_unclosed_quotes;

    #[test]
    fn command_substitution_quotes_do_not_leak_from_outer_double_quote() {
        let input = r#"echo \"$(echo \"\${IFS+'}'z}\")\""#;
        assert!(!has_unclosed_quotes(input));
    }
}
