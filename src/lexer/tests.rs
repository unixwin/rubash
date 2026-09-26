use super::*;
use crate::executor::markers::STORAGE_WORD_PREFIX;

#[test]
fn test_parameter_pattern_quotes_stay_in_one_word() {
    let input = r##"echo "${a#'$('}"##;
    let tokens = tokenize(input);
    let expected = "$".to_string() + "{a#'$('}";
    assert_eq!(tokens[1].value, expected);
}

#[test]
fn test_tokenize_simple() {
    let tokens = tokenize("ls -la");
    assert!(tokens.len() >= 2);
    assert_eq!(tokens[0].value, "ls");
    assert_eq!(tokens[1].value, "-la");
}

#[test]
fn test_tokenize_empty() {
    assert!(tokenize("").is_empty());
}

#[test]
fn test_empty_quoted_heredoc_delimiter_reads_until_eof() {
    let tokens = tokenize("cat <<''\nhi\nthere\n''");

    assert!(tokens.iter().any(|token| token.kind == TokenKind::HereDoc));
    let body = tokens
        .iter()
        .find(|token| token.kind == TokenKind::HereDocBody)
        .map(|token| token.value.as_str());
    assert_eq!(body, Some("__RUBASH_HD1__\x1fhi\nthere\n''\n"));
}

#[test]
fn test_command_substitution_here_string_does_not_swallow_following_heredoc() {
    let tokens = tokenize("echo $(\ncat <<< \"comsub here-string\"\n)\ncat <<''\nhi\nthere\n''");

    let bodies = tokens
        .iter()
        .filter(|token| token.kind == TokenKind::HereDocBody)
        .map(|token| token.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(bodies, vec!["__RUBASH_HD1__\x1fhi\nthere\n''\n"]);
}

#[test]
fn test_nested_braced_parameter_stays_in_one_word() {
    let tokens = tokenize("echo ${outer:-${inner:-fallback}} ${array[${idx:-0}]}");

    assert_eq!(tokens[1].value, "${outer:-${inner:-fallback}}");
    assert_eq!(tokens[2].value, "${array[${idx:-0}]}");
    assert!(tokens
        .iter()
        .all(|token| token.value != "}" || token.kind == TokenKind::HereDocBody));
}

#[test]
fn test_braced_parameter_single_quotes_follow_gnu_pairing() {
    // GNU parse.y pairs the first `'` with the quote inside `$'`, leaving
    // the final `'` unmatched: bash 5.2 reports "unexpected EOF while
    // looking for matching `'" for this input.
    assert!(has_unclosed_input_syntax("echo ${IFS+'bar} ${v/$'\\''/x}"));
}

#[test]
fn braced_quote_tokens_follow_gnu_pairing() {
    let tokens = tokenize("echo ${IFS+'}'z}");
    assert_eq!(tokens[0].value, "echo");
    assert_eq!(tokens[1].value, "${IFS+'}'z}");

    let tokens = tokenize("v=${IFS+'}'z}");
    let assignment = tokens
        .iter()
        .find(|token| token.kind == TokenKind::Assignment)
        .expect("assignment token");
    assert!(assignment.value.contains("${IFS+'}'z}"));
}

#[test]
fn runtime_set_o_posix_switches_dolbrace_scan() {
    // GNU parses lazily: after `set -o posix` runs, single quotes inside a
    // double-quoted `${...}` are literal (Austin Group Interp 221), so the
    // first `}` closes the expansion.
    let source = "set -o posix\necho \"${IFS+'}'z}\"\n";
    let tokens = tokenize_with_initial_posix(source, false);
    assert!(
        tokens
            .iter()
            .any(|token| token.value == "\"${IFS+'}'z}\"" || token.raw == "\"${IFS+'}'z}\""),
        "posix-mode scan must close the expansion at the first `}}`: {tokens:?}"
    );

    // Before the switch the same text keeps the non-posix pairing where the
    // quote protects the first `}`.
    let source = "echo \"${IFS+'a'bc}\"\nset -o posix\n";
    let tokens = tokenize_with_initial_posix(source, false);
    assert!(
        tokens
            .iter()
            .any(|token| token.value == "\"${IFS+'a'bc}\"" || token.raw == "\"${IFS+'a'bc}\""),
        "non-posix scan must keep the quoted pairing: {tokens:?}"
    );
}

#[test]
fn posix_interleaved_quotes_whole_line_balance() {
    // posixexp2 case 28: `"${IFS+"'"x ~ x'}'x"'}"x}" #'` closes the `${...}`
    // span at the first `}` in POSIX mode (single quotes literal inside the
    // double-quoted body), the double quote closes right after `'x`, and the
    // trailing `'..."` single-quoted segment balances the word. But GNU
    // parse.y still treats a `#` at a token boundary inside `( ... )` as a
    // comment through end of line, so the `')` closing the subshell is
    // comment text and the line IS unclosed — GNU 5.3.0 reports
    // "unexpected end of file from `(' command" for this line in isolation
    // (in the suite the subshell stays open into the following line).
    let line = "(echo -n '28 '; printf '%s\\n' \"${IFS+\"'\"x ~ x'}'x\"'}\"x}\" #') 2>&-";
    assert!(
        has_unclosed_input_syntax(line),
        "`#' inside an open `( ... )` comments through EOL, so the line is unclosed"
    );
    let tokens = tokenize_with_initial_posix(line, true);
    assert!(
        tokens.iter().any(|token| {
            token.value.starts_with(STORAGE_WORD_PREFIX)
                && token.value.contains("${IFS+")
                && token.value.ends_with(" #")
        }),
        "the quoted word must stay one token with the quoted-word marker: {tokens:?}"
    );
}

#[test]
fn posix_quoted_alternate_word_value_keeps_quote_structure() {
    // The de-quoted value of the case-28 word: `${...}` closes at the first
    // `}`, the literal-in-dquote `'` before the closing `"` travels as the
    // protected-literal marker, and the single-quoted tail keeps its `"` as
    // data. The expansion stage owns the rest.
    let line = "printf '%s\\n' \"${IFS+\"'\"x ~ x'}'x\"'}\"x}\" #\"";
    let tokens = tokenize_with_initial_posix(line, true);
    let word = tokens
        .iter()
        .find(|token| token.value.contains("${IFS+"))
        .expect("braced word token");
    assert!(
        word.value.starts_with(STORAGE_WORD_PREFIX),
        "fully-quoted mixed word must carry the quoted-word marker: {:?}",
        word.value
    );
    assert!(
        word.value.contains("\x17"),
        "literal-in-dquote `'` must be protected: {:?}",
        word.value
    );
}

#[test]
fn test_comment_skip() {
    let tokens = tokenize("ls # comment");
    assert_eq!(tokens[0].value, "ls");
    assert!(tokens
        .iter()
        .skip(1)
        .all(|token| token.kind == TokenKind::Semicolon));
}

#[test]
fn test_large_single_quoted_unicode_word_tokenizes() {
    let payload = "▀".repeat(4096);
    let script = format!("v='{}'\n:", payload);
    let tokens = tokenize(&script);
    let assignment = tokens
        .iter()
        .find(|token| token.kind == TokenKind::Assignment)
        .expect("assignment token");

    assert_eq!(
        assignment.value.strip_prefix("v=\x1c"),
        Some(payload.as_str())
    );
}

#[test]
fn test_escaped_quote_array_assignment_stays_one_word() {
    let tokens = tokenize(r#"a[\" \"]=15; echo after"#);
    let words = tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::Word | TokenKind::Assignment))
        .collect::<Vec<_>>();

    // `value` is transport text: the escaped quotes inside the subscript are
    // carried as DATA_DQUOTE markers (quotes.rs), so assert the decoded form.
    assert_eq!(
        crate::locale::decode_to_visible_text(&words[0].value),
        "a[\" \"]=15"
    );
    assert_eq!(words[0].raw, r#"a[\" \"]=15"#);
    assert_eq!(words[1].value, "echo");
    assert_eq!(words[2].value, "after");
}

#[test]
fn heredoc_body_paren_does_not_close_command_substitution() {
    // GNU make_here_document reads the here-doc body from the input stream,
    // so a ) inside the body never closes the surrounding $(). The fast-path
    // paren balancer must skip the here-doc body like the slow path already does.
    assert!(has_unclosed_command_substitution(
        "echo $(cat <<eof\nhere doc with )"
    ));
    assert!(has_unclosed_command_substitution(
        "echo $(cat <<eof\nhere doc with )\neof"
    ));
    assert!(!has_unclosed_command_substitution(
        "echo $(cat <<eof\nhere doc with )\neof\n)"
    ));
}

#[test]
fn case_pattern_paren_does_not_close_command_substitution() {
    // parse.y `case_item` owns the pattern list closing `)`, so it must not
    // balance the surrounding $(). A multi-line case whose `esac` sits on its
    // own line inside $() must stay one logical line, otherwise the sub-word is
    // truncated at the newline and the parser reports
    // `syntax error in command substitution` (comsub-posix line 81).
    assert!(has_unclosed_command_substitution(
        "echo $(case a in a) echo x"
    ));
    assert!(has_unclosed_command_substitution(
        "echo $(case a in a) echo x\nesac"
    ));
    assert!(!has_unclosed_command_substitution(
        "echo $(case a in a) echo x\nesac)"
    ));
    assert!(!has_unclosed_command_substitution(
        "echo $(case a in a) echo x;; esac)"
    ));
    // A pattern `)` still leaves an unterminated case open when `esac` is missing.
    assert!(has_unclosed_command_substitution(
        "echo $(case a in a) echo x)"
    ));
}

#[test]
fn nested_heredoc_in_command_substitution_is_collected_before_next_command() {
    let tokens = tokenize("echo $(cat <<EOF)\nfoo\nbar\nEOF\necho after");
    let substitution = tokens
        .iter()
        .find(|token| token.kind == TokenKind::CommandSubst)
        .expect("nested command substitution token");
    assert!(substitution.value.contains("foo\nbar\nEOF"));
    assert!(tokens.iter().any(|token| token.value == "after"));
}

#[test]
fn trailing_unquoted_backslash_keeps_stdin_line_open() {
    // parse.y:5379-5384 read_token_word: backslash-newline is removed as a
    // pair ("ignored in all cases except when quoted with single quotes"),
    // so the incremental stdin driver must keep reading (PS2) instead of
    // submitting the line with the backslash dropped.
    use crate::script_driver::stdin_source_needs_more;
    assert!(stdin_source_needs_more("echo abc\\\n"));
    assert!(stdin_source_needs_more("echo abc\\"));
    // An escaped backslash terminates the line: `echo a\` (two backslashes)
    // runs now instead of waiting for a continuation line.
    assert!(!stdin_source_needs_more("echo a\\\\\n"));
    // Single quotes make the backslash literal: 'xx\' closes and runs.
    assert!(!stdin_source_needs_more("echo 'xx\\\\'\n"));
    // A retained CR (CRLF physical line) is an escaped carriage return,
    // not a continuation.
    assert!(!stdin_source_needs_more("echo a\\\r\n"));
}

/// rubash#144: a `\'` escaped quote consumes two characters without opening
/// quote state (parse.y:5366-5398 read_token_word), so the following `'`
/// re-enters a fresh single-quoted span (parse.y:5419-5437 shellquote
/// branch). Inside that span every byte is literal data —
/// subst.c:11882-11886 never consults the command-substitution scanner —
/// so a backtick there must travel as the DATA_BACKTICK carrier in the
/// assignment value, or expand_backtick_substitution_typed executes the
/// body (`x='a'\''`b`'` ran `b` and stored `a'`).
#[test]
fn escaped_quote_reenters_single_quotes_with_literal_backticks() {
    use crate::executor::markers::{DATA_BACKTICK, DATA_DOLLAR, DATA_SQUOTE};
    let tokens = tokenize("x='a'\\''`b`'");
    let assignment = tokens
        .iter()
        .find(|token| token.kind == TokenKind::Assignment)
        .expect("assignment token");
    // `'a'` -> a, `\'` -> data quote, `` '`b`' `` -> backtick-carrier b
    // backtick-carrier: value must NOT contain a live backtick for the
    // assignment expander's comsub fast paths to claim.
    assert!(assignment.raw.contains("\\''"));
    let expected = format!("a{DATA_SQUOTE}{DATA_BACKTICK}b{DATA_BACKTICK}");
    assert!(
        assignment.value.ends_with(&expected),
        "value {:?} should end with {:?}",
        assignment.value,
        expected
    );
    // Lone backtick in the re-entered span stays literal too (c03: fatal EOF).
    let tokens = tokenize("x='a'\\''b`c'");
    let assignment = tokens
        .iter()
        .find(|token| token.kind == TokenKind::Assignment)
        .expect("assignment token");
    assert!(
        !assignment.value.contains('`'),
        "value {:?}",
        assignment.value
    );
    // A `$` in the re-entered span is data (c06 family / `u='a'\''$h'`).
    let tokens = tokenize("x='a'\\''$h'");
    let assignment = tokens
        .iter()
        .find(|token| token.kind == TokenKind::Assignment)
        .expect("assignment token");
    assert!(
        assignment.value.contains(DATA_DOLLAR),
        "value {:?} must carry the protected dollar",
        assignment.value
    );
    assert!(!assignment.value.contains('$'));
}

/// rubash#144 assignment form mixing a re-entered `'...'` span with a live
/// `$(...)`: the `$(` branch must protect the span content too
/// (`x='a'\''`b`'$(echo z)` — GNU keeps `` `b` `` literal and runs only z).
#[test]
fn assignment_dollar_paren_branch_protects_reentered_span() {
    let tokens = tokenize("x='a'\\''`b`'$(echo z)");
    let assignment = tokens
        .iter()
        .find(|token| token.kind == TokenKind::Assignment)
        .expect("assignment token");
    let backticks: &str = &assignment
        .value
        .chars()
        .filter(|ch| *ch == '`')
        .collect::<String>();
    assert!(
        backticks.is_empty(),
        "assignment value must not carry a live backtick: {:?}",
        assignment.value
    );
    assert!(assignment.value.contains("$(echo z)"));
}

/// bash_completion:188 (`_comp_dequote__regex_safe_word`) and ssh.bash:457
/// (`_comp_cmd_scp__path_esc`) forms: the whole RHS is one assignment word
/// whose re-entered single-quoted spans contain backticks, `"` and `$`.
#[test]
fn completion_regex_forms_stay_one_word_with_data_carriers() {
    let tokens = tokenize("_r='^([^\\'\\''\"`;&|<>()!]|'$rq'|$rp')*$'");
    let words: Vec<&Token> = tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Assignment)
        .collect();
    assert_eq!(words.len(), 1, "one assignment word: {tokens:?}");
    let value = &words[0].value;
    assert!(!value.contains('`'), "value {value:?}");
    assert!(
        value.contains("$rq"),
        "unquoted $name expands later: {value:?}"
    );
    let tokens = tokenize("_p='[][(){}<>\"'\"'\"'\",:;^&!$=?`\\\\|[:space:]]'");
    let assignment = tokens
        .iter()
        .find(|token| token.kind == TokenKind::Assignment)
        .expect("assignment token");
    assert!(
        !assignment.value.contains('`'),
        "value {:?}",
        assignment.value
    );
    assert!(assignment.raw.ends_with("]'"));
}

// ---------------------------------------------------------------------------
// rubash#155 / #130: brace-group join fast path
// ---------------------------------------------------------------------------

/// Admission whitelist of the rubash#155 fast path (mod.rs
/// `brace_join_active`): a line is only admitted when no byte can open or
/// close any construct the join iteration tracks.
#[test]
fn brace_join_fast_path_line_admission() {
    assert!(brace_join_fast_path_line("x=1"));
    assert!(brace_join_fast_path_line("echo plain words; and-more.args"));
    assert!(brace_join_fast_path_line(""));
    assert!(brace_join_fast_path_line(": route-table/if.cfg"));
    // Every byte class a consumer between the append and the join
    // `continue` reacts to disqualifies the line.
    assert!(!brace_join_fast_path_line("v='q'"));
    assert!(!brace_join_fast_path_line("v=\"q\""));
    assert!(!brace_join_fast_path_line("echo ${x}"));
    assert!(!brace_join_fast_path_line("}"));
    assert!(!brace_join_fast_path_line("{ nested"));
    assert!(!brace_join_fast_path_line("cat <<EOF"));
    assert!(!brace_join_fast_path_line("run & (bg)"));
    assert!(!brace_join_fast_path_line("x=$(echo hi)"));
    assert!(!brace_join_fast_path_line("x=`echo hi`"));
    assert!(!brace_join_fast_path_line("echo a\\"));
    assert!(!brace_join_fast_path_line("# comment"));
    assert!(!brace_join_fast_path_line("set -o posix"));
}

/// nvm.sh shape (rubash#130): one `{` group spanning N physical lines of
/// inert statements. The fast path skips the per-line full-buffer
/// re-tokenization; the accepted stream must still be the folded group
/// keyword attributed to the opening line.
#[test]
fn multiline_brace_group_folds_into_one_keyword() {
    let mut script = String::from("{\n");
    for i in 0..300 {
        script.push_str(&format!("x={}\n", i));
    }
    script.push_str("}\n");
    let tokens = tokenize(&script);
    let group = tokens
        .iter()
        .find(|token| {
            token.kind == TokenKind::Keyword
                && token.value.starts_with('{')
                && token.value.ends_with('}')
        })
        .expect("folded group token");
    assert!(group.value.contains("x=299"));
    assert_eq!(group.position, 1, "group attributed to its opening line");
}

/// Fast-path (inert) and slow-path (reactive) lines interleave inside one
/// joined group; quotes, `${...}` and `$(...)` on slow lines must not
/// derail the join, and the closing `}` line still folds everything.
#[test]
fn brace_join_mixes_fast_and_slow_lines() {
    let script =
        "{\nx=1\ny=2\nv=\"quoted $x tail\"\nw=$(echo sub)\necho ${v:-d}\nz=3\n}\necho after\n";
    let tokens = tokenize(script);
    let group = tokens
        .iter()
        .find(|token| {
            token.kind == TokenKind::Keyword
                && token.value.starts_with('{')
                && token.value.ends_with('}')
        })
        .expect("folded group token");
    assert!(group.value.contains("quoted $x tail"));
    assert!(group.value.contains("${v:-d}"));
    assert!(tokens
        .iter()
        .any(|token| token.kind == TokenKind::Word && token.value == "after"));
}

/// The fast path skips the per-pass `line_posix_mode_change`
/// recomputation; a `set -o posix` line inside the joined group must still
/// flip the parse mode for the logical lines that follow the group
/// (GNU parses lazily — the switch applies to everything read afterwards).
#[test]
fn posix_switch_inside_multiline_brace_group_applies_after_close() {
    let source = "{\nx=1\nset -o posix\nx=2\n}\necho \"${IFS+'}'z}\"\n";
    let tokens = tokenize_with_initial_posix(source, false);
    assert!(
        tokens.iter().any(|token| token.raw == "\"${IFS+'}'z}\""),
        "posix pairing (first `}}` closes) after an in-group switch: {tokens:?}"
    );
}

// ---------------------------------------------------------------------------
// rubash#140: CRLF line-terminator tolerance is Windows-only
// ---------------------------------------------------------------------------

/// On Windows the trailing `\r` of a CRLF physical line is the terminator,
/// not word data (niubash #106 product decision).
#[cfg(windows)]
#[test]
fn crlf_line_terminator_stripped_on_windows() {
    let tokens = tokenize("x=1\r\n");
    let assignment = tokens
        .iter()
        .find(|token| token.kind == TokenKind::Assignment)
        .expect("assignment token");
    assert!(!assignment.value.contains('\r'), "{tokens:?}");
    // A CRLF heredoc still terminates on its own delimiter line.
    let tokens = tokenize("cat <<EOF\r\nbody\r\nEOF\r\n");
    let body = tokens
        .iter()
        .find(|token| token.kind == TokenKind::HereDocBody)
        .map(|token| token.value.as_str());
    assert_eq!(body, Some("body\n"));
}

/// On unix GNU keeps the `\r` as literal data (rubash#140): the delimiter
/// line of a CRLF heredoc reads `EOF\r`, never matches `EOF`, and the
/// heredoc runs to end of file (make_cmd.c compares the raw line).
#[cfg(unix)]
#[test]
fn crlf_line_terminator_kept_as_data_on_unix() {
    use crate::executor::markers::DATA_DOLLAR_STR;
    let tokens = tokenize("cat <<EOF\r\nbody\r\nEOF\r\n");
    let body = tokens
        .iter()
        .find(|token| token.kind == TokenKind::HereDocBody)
        .map(|token| token.value.as_str())
        .expect("heredoc body token");
    assert!(
        body.starts_with(DATA_DOLLAR_STR),
        "unterminated heredoc keeps the EOF marker: {body:?}"
    );
    assert!(body.contains("body\r\n"), "CR stays in the body: {body:?}");
}
