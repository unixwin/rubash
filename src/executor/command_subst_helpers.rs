use crate::executor::markers::DATA_DOLLAR;
use crate::executor::substitution_metadata::bytes_to_shell_text;

/// Whether a collected `${...}` body ends inside an unterminated
/// single-quoted span, scanned with the same state machine
/// [`collect_braced_parameter_body_into`] uses (plain `'`, `$'...'` with
/// backslash escapes, `"..."`, backslash pairs). GNU subst.c:1954
/// extract_dollar_brace_string: outside the POSIX+compat>42 literal
/// case, a `'` inside `${...}` opens skip_single_quoted — an
/// unterminated span swallows the closing `}` and the extraction fails.
pub(in crate::executor) fn braced_body_ends_in_single_quote(body: &str) -> bool {
    let mut chars = body.chars().peekable();
    let mut single = false;
    let mut double = false;
    let mut ansi_c = false;
    while let Some(ch) = chars.next() {
        if single {
            // DATA_SQUOTE (\u{17}) also closes: it is the lexer's data
            // marking of the span's closing quote (see the match below).
            if ch == '\'' || ch == '\u{17}' {
                single = false;
            }
            continue;
        }
        if ansi_c {
            if ch == '\\' {
                chars.next();
            } else if ch == '\'' {
                ansi_c = false;
            }
            continue;
        }
        if double {
            if ch == '"' {
                double = false;
            } else if ch == '\\' {
                chars.next();
            }
            continue;
        }
        match ch {
            '\\' => {
                chars.next();
            }
            '$' if chars.peek().copied() == Some('\'') => {
                chars.next();
                ansi_c = true;
            }
            '\'' => single = true,
            // DATA_SQUOTE: a quote the lexer already classified as data
            // because its own span scan closed the expansion earlier —
            // for the extraction verdict it is the span's closing quote
            // (rubash#1109 paired form `${uq:-don'}'t}`).
            '\u{17}' => single = !single,
            '"' => double = true,
            _ => {}
        }
    }
    single || ansi_c
}

impl super::Executor {
    /// rubash#1109 (GNU subst.c:1954 extract_dollar_brace_string): when
    /// the POSIX literal-quote gate does NOT apply (not posix, or compat
    /// <= 42), a `'` inside a double-quoted/heredoc `${...}` opens a
    /// skip_single_quoted span; if that span is unterminated the brace
    /// extraction fails with `bad substitution: no closing `}' in
    /// "<word>"` and the word expansion fails (status 1, command
    /// abandoned). Returns true when the verdict was raised — the
    /// caller then contributes no value for this fragment. (The parse
    /// gate, parse.y:4035 compat>41, is the lexer's side; a word that
    /// reaches expansion has already passed it.)
    pub(in crate::executor) fn no_closing_brace_verdict(
        &self,
        body: &str,
        terminated: bool,
        quoted_context: bool,
        word: &str,
    ) -> bool {
        if terminated || !quoted_context {
            return false;
        }
        if self.posix_mode_enabled()
            && crate::builtins::shopt::compatibility_level(&self.shell_state.env_vars) > 42
        {
            return false;
        }
        if !braced_body_ends_in_single_quote(body) {
            return false;
        }
        self.write_diagnostic_fd2(
            format!(
                "{}bad substitution: no closing `}}' in {}\n",
                self.diagnostic_prefix(),
                word
            )
            .as_bytes(),
        );
        // Not the parameter_bad_substitution latch: that one takes
        // FORCE_EOF in a noninteractive POSIX shell (subst.c:10288),
        // but the extract_dollar_brace_string failure is an
        // expand_word_error with DISCARD semantics in every mode (GNU
        // 5.3.20 probe: compat42+posix `echo "C=${uq:-don't}"` reports
        // the error, abandons the command with status 1, and the next
        // line runs). The dedicated latch is consumed next to the
        // bad-substitution checks with ExpansionFailure(1).
        self.no_closing_brace_failure.set(true);
        true
    }

    /// rubash#2441 (GNU subst.c extract_delimited_string failure site,
    /// reported via extract_arithmetic_subst): a `$[...]` span whose
    /// extraction finds no closing `]` is `bad substitution: no closing
    /// `]' in <string>` and the command is discarded with status 1
    /// (exp_jump_to_top_level(DISCARD) — every mode, like the brace
    /// verdict above, so it shares the no_closing_brace_failure latch
    /// and its consumers). <string> is the whole string handed to that
    /// expansion level (segment content for a double-quoted word, the
    /// whole heredoc body for a heredoc walk). Prints at most once per
    /// command: GNU jumps at the first failing word/span, so a latch
    /// already set means an earlier verdict owns the diagnostic.
    /// Returns true when the verdict applies — the caller contributes
    /// no value for this span.
    pub(in crate::executor) fn no_closing_bracket_verdict(&self, string: &str) -> bool {
        if !self.no_closing_brace_failure.replace(true) {
            self.write_diagnostic_fd2(
                format!(
                    "{}bad substitution: no closing `]' in {}\n",
                    self.diagnostic_prefix(),
                    string
                )
                .as_bytes(),
            );
        }
        true
    }
}

/// rubash#2441/rubash#2227: the <string> GNU names for a failed `$[...]`
/// extraction in a non-heredoc walk. GNU expand_word_internal
/// (subst.c:11692-11723) hands each double-quoted segment
/// (string_extract_double_quoted, subst.c:852) to a recursive expansion
/// level, so extract_arithmetic_subst's failing scan runs against THE
/// SEGMENT and `bad substitution: no closing `]' in <string>`
/// (subst.c:1505) names the segment — not the whole word prefix. The
/// cookers re-emit the doomed segment's opening `"` (quotes.rs
/// reopen_doomed_segment), so the segment is the text after the LAST
/// raw `"` / E302 opener in the word prefix through the failing `$[`
/// plus the collector's consumed body. `tail` is the walker iterator's
/// unconsumed remainder at the verdict — with it the span's end in the
/// word is exact (the segment-bounded collector stops at the cut
/// quote and leaves the post-cut text unconsumed).
pub(in crate::executor) fn dollar_bracket_no_closing_string(
    word: &str,
    expression: &str,
    tail: &str,
) -> String {
    let total = word.chars().count();
    let span_end = total.saturating_sub(expression.chars().count() + tail.chars().count());
    let mut string: Vec<char> = word.chars().take(span_end).collect();
    // Start at the last double-quote delimiter in the prefix — the
    // doomed segment's re-emitted opener (raw `"` in the generic
    // channels, E302 in the assignment hoist channel). A raw `"` from a
    // single-quoted span also qualifies: in GNU's word string quotes
    // are quotes, and expand_word_internal would open a segment there
    // too. Data-quote carriers (DATA_DQUOTE 0x18 etc.) do not: GNU's
    // segment machine passes an escaped quote by (subst.c:989).
    if let Some(pos) = string.iter().rposition(|&ch| ch == '"' || ch == '\u{E302}') {
        string.drain(..=pos);
    }
    let mut string: String = string.into_iter().collect();
    let mut previous_was_backslash = false;
    for ch in expression.chars() {
        // The generic channels' collector stops before the cut quote, so
        // the expression rarely reaches one; the assignment hoist
        // channel's collector is not segment-bounded and consumes its
        // E302 cut survivor as an unterminated quote unit — stop there.
        if !previous_was_backslash && matches!(ch, '"' | '\'' | '`' | '\u{E302}') {
            break;
        }
        string.push(ch);
        previous_was_backslash = ch == '\\' && !previous_was_backslash;
    }
    string
}

pub(in crate::executor) fn collect_braced_parameter_name(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> String {
    collect_braced_parameter_body(chars).0
}

/// Collects the `${`-body and reports whether the closing `}` was found.
/// GNU subst.c parameter_brace_expand verdicts an unterminated `${body`
/// (`PS1='a${bad'`) as `bad substitution` naming the whole word; the
/// prompt pass needs the termination bit to raise that verdict (the
/// ordinary word paths keep their historical silent behavior).
pub(in crate::executor) fn collect_braced_parameter_body(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> (String, bool) {
    let mut name = String::new();
    let terminated = collect_braced_parameter_body_into(chars, &mut name);
    (name, terminated)
}

/// Appending form of [`collect_braced_parameter_body`]: the caller supplies
/// the buffer so hot walkers can collect straight into a pooled synthetic
/// `${...}` word instead of staging a per-fragment `String` (GNU
/// subst.c:9777 parameter_brace_expand scans the same body inline — the
/// intermediate is a port artifact, not GNU state).
pub(in crate::executor) fn collect_braced_parameter_body_into(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    body: &mut String,
) -> bool {
    let mut nested = 0usize;
    let mut single = false;
    let mut double = false;
    let mut ansi_c = false;
    while let Some(ch) = chars.next() {
        // Inside single quotes everything is literal, including backslashes;
        // only the next single quote ends the quoted region.
        if single {
            if ch == '\'' {
                single = false;
            }
            body.push(ch);
            continue;
        }
        // Inside a $'...' ANSI-C span a backslash escapes the following
        // character, so '\'' is data rather than the closing quote (GNU parse.y
        // extract_dollar_brace_string). Without this the escaped quote ends the span
        // early and the trailing '}' is swallowed, leaving a heredoc body
        // such as 7: ${x%'$'a\t\\'b'}' unexpanded.
        if ansi_c {
            body.push(ch);
            if ch == '\\' {
                if let Some(escaped) = chars.next() {
                    body.push(escaped);
                }
            } else if ch == '\'' {
                ansi_c = false;
            }
            continue;
        }
        if double {
            if ch == '"' {
                double = false;
                body.push(ch);
                continue;
            }
            if ch == '\\' {
                body.push(ch);
                if let Some(escaped) = chars.next() {
                    body.push(escaped);
                }
                continue;
            }
            body.push(ch);
            continue;
        }
        // GNU Bash scans `\` plus the following character as one unit
        // (extract_dollar_brace_string advances by two). `\\` is a literal
        // backslash, `\}` a literal closing brace; neither closes the name.
        if ch == '\\' {
            if let Some(escaped) = chars.next() {
                body.push('\\');
                body.push(escaped);
            } else {
                body.push('\\');
            }
            continue;
        }
        if ch == '$' && chars.peek().copied() == Some('\'') {
            // $'...' is an ANSI-C quoted span, a quoting mode distinct
            // from a plain single quote: backslashes inside it escape data.
            chars.next();
            ansi_c = true;
            body.push('$');
            body.push('\'');
            continue;
        }
        if ch == '\'' {
            single = true;
            body.push(ch);
            continue;
        }
        if ch == '"' {
            double = true;
            body.push(ch);
            continue;
        }
        if ch == '[' {
            body.push(ch);
            continue;
        }
        if ch == ']' {
            body.push(ch);
            continue;
        }
        if ch == '$' && chars.peek().copied() == Some('{') {
            chars.next();
            nested += 1;
            body.push('$');
            body.push('{');
            continue;
        }
        if ch == '}' {
            if nested == 0 {
                return true;
            }
            nested -= 1;
            body.push(ch);
            continue;
        }
        body.push(ch);
    }
    false
}

pub(in crate::executor) fn decode_old_style_backtick_source(source: &str) -> String {
    let mut output = String::new();
    let mut chars = source.chars().peekable();
    let mut single = false;
    let mut double = false;
    while let Some(ch) = chars.next() {
        if ch == '\'' && !double {
            single = !single;
            output.push(ch);
            continue;
        }

        if ch == '"' && !single {
            double = !double;
            output.push(ch);
            continue;
        }

        if ch != '\\' {
            push_backtick_source_char(&mut output, ch, single);
            continue;
        }

        // rubash#745: inside a backtick body the backslash is consumed only
        // before `$`, a backtick or another backslash; everything else keeps
        // it (parse.y parse_matched_pair under P_BACKQUOTE: "backslash is
        // only special if it precedes $, `, or \"). `\"` therefore stays,
        // and the double-quoted case is handled one layer up in
        // lexer::quotes (a backtick inside a double-quoted word receives the
        // already-consumed quote).
        //
        //   recho `echo 'a\' b`        -> argv[1]=<a\>  (backslash kept)
        //   recho `echo "(\")"`        -> (" )           (kept, dquote rules apply)
        //
        // The old arms special-cased `\\"` and `\"` while `double` was set
        // and emitted DATA_DQUOTE directly, which skipped this step, so
        // `"\\\\"` (four backslashes in double quotes) decoded to `\"`
        // instead of `\`.
        match chars.next() {
            Some(next @ ('$' | '`' | '\\')) => {
                push_backtick_source_char(&mut output, next, single);
            }
            Some('\n') => {}
            Some('\r') if chars.peek().copied() == Some('\n') => {
                chars.next();
            }
            Some(next) => {
                output.push('\\');
                push_backtick_source_char(&mut output, next, single);
            }
            None => output.push('\\'),
        }
    }
    // rubash#745: a word-final ODD run of backslashes. GNU hands the body to
    // parse_and_execute with EOF immediately after it, so a lone trailing
    // `\\` stays literal (`\`printf '%s' \\`` yields one backslash). The
    // re-parse here terminates the text with a newline instead, where `\\`
    // plus newline is a line continuation and the character disappears.
    // Double it so the parser's own escape pass lands on the same single
    // backslash; an even run already terminates on a literal backslash.
    if output.chars().rev().take_while(|ch| *ch == '\\').count() % 2 == 1 {
        output.push('\\');
    }
    output
}

fn push_backtick_source_char(output: &mut String, ch: char, single: bool) {
    if single && ch == '$' {
        output.push(DATA_DOLLAR);
    } else {
        output.push(ch);
    }
}

pub(super) fn unescape_remaining_shell_escapes(value: &str) -> String {
    unescape_remaining_shell_escapes_cow(value).into_owned()
}

/// Borrowing form: the escape-consuming loop only rewrites positions at a
/// `\`; a value with none copies through verbatim, so the marker-restore
/// chains on the expansion hot path can borrow instead of staging an output
/// String (GNU dequote_string walks the string in place).
pub(super) fn unescape_remaining_shell_escapes_cow(value: &str) -> std::borrow::Cow<'_, str> {
    if !value.contains('\\') {
        return std::borrow::Cow::Borrowed(value);
    }
    std::borrow::Cow::Owned(unescape_remaining_shell_escapes_inner(value))
}

fn unescape_remaining_shell_escapes_inner(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let mut lookahead = chars.clone();
            if lookahead.next() == Some('\\') && lookahead.next() == Some('\'') {
                chars.next();
                chars.next();
                output.push('\'');
                continue;
            }
            if let Some(
                next @ ('\'' | '"' | '\\' | '$' | '`' | '(' | ')' | '{' | '}' | ';' | '&' | '|'
                | '<' | '>' | '!' | '*' | '?' | '#' | ' '),
            ) = chars.peek().copied()
            {
                chars.next();
                output.push(next);
                continue;
            }
        }
        output.push(ch);
    }
    output
}

/// Command-substitution capture: strips the line terminator(s) GNU bash
/// removes from a substitution's output.
///
/// GNU (subst.c command_substitute) deletes every trailing newline and keeps
/// any other trailing byte, so on Unix `o=$(printf 'a\r\n')` is `a\r`
/// (WSL 5.3.0 baseline confirms: od prints `a \r`). Windows additionally
/// treats a CRLF pair as one terminator: niubash runs Windows-native tools
/// whose CRT text mode turns every `\n` into `\r\n` (measured: gawk, GoAWK's
/// awk, ugrep, python, jq, xz, curl, 7z), and a native shell reads those
/// pipes without the MSYS text-mode translation that turns `\r\n` back into
/// `\n`. The stray `\r` therefore survives into `$o` and breaks
/// `[ "$o" = x ]`, `case` labels, array keys and concatenated paths with an
/// invisible byte (niubash #120).
///
/// Rule: strip every trailing `\n`; on Windows also a `\r` that immediately
/// precedes a stripped `\n` (part of the CRLF terminator). A lone trailing
/// `\r` not followed by `\n` is preserved everywhere, so the GNU-fidelity
/// case is intact. This is the same policy the lexer applies to CRLF script
/// lines (niubash #106) and the same one `read` applies to its input.
pub(in crate::executor) trait CaptureTerminator {
    fn trim_capture_terminator(&self) -> &str;
}

impl CaptureTerminator for str {
    fn trim_capture_terminator(&self) -> &str {
        let bytes = self.as_bytes();
        let mut end = bytes.len();
        while end > 0 && bytes[end - 1] == b'\n' {
            end -= 1;
            if cfg!(windows) && end > 0 && bytes[end - 1] == b'\r' {
                end -= 1;
            }
        }
        &self[..end]
    }
}

/// Byte-level form of the same rule, for capture paths that carry `Vec<u8>`
/// until the final text conversion (GNU subst.c read_comsub).
pub(in crate::executor) fn trim_capture_terminator_bytes(bytes: &mut Vec<u8>) {
    while bytes.last() == Some(&b'\n') {
        bytes.pop();
        if cfg!(windows) && bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
}

pub(in crate::executor) fn echo_command_substitution_output(args: &[String]) -> String {
    let mut bytes = echo_raw_output_bytes(args);
    bytes.retain(|byte| *byte != 0);
    bytes_to_shell_text(&bytes)
        .trim_capture_terminator()
        .to_string()
}

fn echo_raw_output_bytes(args: &[String]) -> Vec<u8> {
    let mut output = Vec::new();
    let _ = crate::builtins::echo::write_echo(args.iter().map(String::as_str), &mut output);
    output
}

/// True when any word is a shell control/redirection operator that the
/// word-based command-substitution shortcuts cannot interpret. GNU subst.c
/// parses the whole substitution body into a command list before executing
/// it; a word shortcut that assumes a lone simple command would hand the
/// operators to the command as literal arguments (`$(f a b | wc -l)` runs
/// f with `| wc -l` in its positional params, issue #70).
pub(in crate::executor) fn command_substitution_words_have_operators(words: &[String]) -> bool {
    words.iter().any(|word| command_word_is_operator(word))
}

/// True when the word itself carries redirection syntax: either a bare
/// operator token or an attached form without whitespace (`2>/dev/null`,
/// `>out`, `<in`, `<<EOF`).
pub(in crate::executor) fn command_word_carries_redirect(word: &str) -> bool {
    let body = word.trim_start_matches(|ch: char| ch.is_ascii_digit());
    body.starts_with('<') || body.starts_with('>')
}

fn command_word_is_operator(word: &str) -> bool {
    match word {
        "|" | "|&" | "||" | "&&" | "&" | ";" | ";;" | ";&" | ";;&" => true,
        "<" | ">" | ">>" | ">|" | "<>" | "<<" | "<<<" | "<&" | ">&" | "&>" | "&>>" | "1>"
        | "1>>" | "1>|" | "1<&" | "1>&" | "2>" | "2>>" | "2>|" | "2<&" | "2>&" => true,
        _ => command_word_carries_redirect(word) || word.contains('|'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn echo_command_substitution_uses_builtin_escape_rules() {
        assert_eq!(
            echo_command_substitution_output(&args(&["-e", "a\\nb"])),
            "a\nb"
        );
        assert_eq!(
            echo_command_substitution_output(&args(&["-e", "\\xz"])),
            "\\xz"
        );
        assert_eq!(
            echo_command_substitution_output(&args(&["--help"])),
            "--help"
        );
    }

    #[test]
    fn echo_command_substitution_removes_nul_bytes() {
        assert_eq!(
            echo_command_substitution_output(&args(&["-e", "a\\0b"])),
            "ab"
        );
    }

    #[test]
    fn braced_parameter_collection_closes_at_first_brace_in_bracket_pattern() {
        // GNU parse.y uses P_FIRSTCLOSE for ${...}: the first unquoted '}'
        // closes the expression, even inside a bracket pattern. The
        // remaining `]` is literal text outside the expansion.
        let mut chars = "o%[}]}]".chars().peekable();
        assert_eq!(collect_braced_parameter_name(&mut chars), "o%[");
        assert_eq!(chars.collect::<String>(), "]}]");
    }
}
