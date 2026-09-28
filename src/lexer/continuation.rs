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
        // Same arm shape as has_unclosed_compound_assignment below and
        // comsub_residuals further down.
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
            // rubash#281 (captain-applied perf3 proposal): zero-copy body
            // view via the rubash#185 chars API — the String copy
            // re-collected the ENTIRE remaining input per `${` (nvm.sh: 1644
            // occurrences -> O(n^2); prescan 4540ms -> 27ms). The slice
            // INCLUDES the `${` opener: scan_braced_parameter_body_chars
            // requires it at position 0 — slicing from index+2 would report
            // every span unclosed.
            let body = &chars[index..];
            if !double {
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
                        crate::lexer::dolbrace::scan_braced_parameter_body_chars(body, context)
                    {
                        index += 2 + scan.end;
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
            let (next, closes) = skip_heredoc_in_chars_with_closure(&chars, index);
            // rubash#292: the terminator line is not in the buffer yet; a
            // longer buffer closes this heredoc at a position this prefix
            // cannot see (and the body lines in between must stay opaque),
            // so park at the `<<` — no enclosing `$(` park exists at
            // depth == 0.
            if closes.is_none() && park.is_none() {
                park = Some(ComsubResidualPark {
                    pos: index,
                    snapshot: state.clone(),
                });
            }
            index = next;
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

fn update_command_substitution_case_depth(
    chars: &[char],
    index: usize,
    ch: char,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
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
    );
}

/// rubash#292 plan B variant: additionally reports (through `undecided`)
/// whether the `esac` case-pattern lookahead consulted text past the end
/// of `chars` without concluding. The decision itself is still committed
/// exactly as the full-buffer scan commits it; only the caller's
/// checkpoint cares about the flag.
fn update_command_substitution_case_depth_ex(
    chars: &[char],
    index: usize,
    ch: char,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
    undecided: &mut bool,
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
        "esac" if *current_word_boundary => {
            let (starts_with_esac_chars, decided) =
                case_pattern_starts_with_esac_chars_ex(chars, index);
            if !decided {
                *undecided = true;
            }
            if !starts_with_esac_chars {
                *case_depth = case_depth.saturating_sub(1);
                true
            } else {
                false
            }
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
