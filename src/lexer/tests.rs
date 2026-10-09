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
fn multiline_dquoted_comsub_join_gate_matches_full_scan() {
    // rubash#292: the join gate's residual checkpoint must reproduce the
    // per-prefix full-scan answers. `x="$(fo` alone is OPEN (the unit skip
    // fails on the prefix and the residual double-quote state would never
    // let a naive per-line scan close it); only re-deriving the atomic
    // skip over the joined buffer closes the comsub at the `)` inside the
    // double quotes — exactly what the full scan of the accumulated
    // logical line does. Early finalization would split the word and drop
    // the `echo after` line into the open quote.
    let multi = tokenize("x=\"$(fo\no)\"\necho after\n");
    let single = tokenize("x=\"$(foo)\"\necho after\n");
    // The comsub body keeps the physical newline (GNU runs `fo` and `o` as
    // two commands), so the values differ by that newline — but the token
    // SHAPE must match the closed one-line form: one assignment word, then
    // `echo after` as normal words. An early gate finalization would split
    // the word and swallow the trailing line into the open quote.
    assert_eq!(multi.len(), single.len());
    assert!(multi[0].value.contains("$(fo\no)"));
    assert!(multi.iter().any(|token| token.value == "echo"));
    assert!(multi.iter().any(|token| token.value == "after"));
}

#[test]
fn backslash_continuation_inside_comsub_invalidates_checkpoint() {
    // The pop path drops the trailing `\` and appends the next line WITHOUT
    // the '\n' separator, so a two-character lookahead (`$(`, `<<`, ...)
    // can newly straddle the join — the checkpoint is hard-invalidated
    // there and the gate full-scans. The joined text equals the one-line
    // form byte for byte, so the token values must match it.
    let multi = tokenize("$(echo a\\\nb)\necho after\n");
    let single = tokenize("$(echo ab)\necho after\n");
    assert_eq!(multi.len(), single.len());
    assert_eq!(multi[0].value, single[0].value);
    assert!(multi.iter().any(|token| token.value == "after"));
}

#[test]
fn multiline_case_in_comsub_stays_one_word_through_gate() {
    // rubash#292 + the existing fn-level case-pattern tests: through the
    // feeder, `$(case a in a) echo x` must stay open across the newline
    // and close at `esac)` (the pattern `)` is gated by case depth both in
    // the full scan and in the parked re-derivation).
    let tokens = tokenize("$(case a in a) echo x\nesac)\necho after\n");
    assert!(tokens.iter().any(|token| token.value == "after"));
    assert!(tokens
        .iter()
        .any(|token| token.value.contains("case a in a) echo x")));
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
    let tokens = tokenize("cat <<EOF\r\nbody\r\nEOF\r\n");
    let body = tokens
        .iter()
        .find(|token| token.kind == TokenKind::HereDocBody)
        .map(|token| token.value.as_str())
        .expect("heredoc body token");
    // On unix the delimiter itself carries the CR (EOF\r matches EOF\r),
    // so the heredoc IS terminated and the body keeps the CR from the
    // content line — GNU semantics: CR is literal data (make_cmd.c).
    assert!(body.contains("body\r\n"), "CR stays in the body: {body:?}");
}

/// rubash#215: an escaped quote plus a backtick inside one ANSI-C string
/// (`$'a\'b`c'`) must lex as ONE assignment word whose value carries the
/// decoded data via the walker's carriers. GNU parse.y:5546-5558
/// read_token_word → parse_matched_pair (parse.y:3877) with P_ALLOWESC: `\'`
/// never closes the span and the backtick inside it is string data, so no
/// downstream re-scan may see a live backtick (the executor's
/// unclosed-`$(` gate used to kill the whole script with "unexpected EOF
/// while looking for matching `)'").
#[test]
fn ansi_c_escaped_quote_with_backtick_stays_one_assignment_word() {
    let input = "x=$'a\\'b`c'\ny=1\n";
    assert!(!has_unclosed_quotes(input));
    assert!(!has_unclosed_input_syntax_posix(input, false));
    assert!(unclosed_input_close_char_posix(input, false).is_none());
    let tokens = tokenize(input);
    assert_eq!(tokens[0].kind, TokenKind::Assignment);
    // \u{1c} is the quoted-RHS prefix (mark_quoted_assignment_value),
    // \u{e010} the decoded `'` data marker, \u{1a} the decoded backtick.
    assert_eq!(tokens[0].value, "x=\u{1c}a\u{e010}b\u{1a}c");
    // The decoded backtick travels as the DATA_BACKTICK carrier, so the
    // executor gate (has_unclosed_command_substitution) cannot mistake it
    // for an open command substitution.
    assert!(!crate::lexer::has_unclosed_command_substitution(
        &tokens[0].value
    ));
}

/// rubash#215 follow-on: a backtick in a trailing COMMENT after an
/// assignment whose ANSI-C body contains `\'` must not hold the line open.
/// GNU parse.y:3992 P_ALLOWESC/LEX_PASSNEXT keeps `\'` inside the span; the
/// comment's backtick is comment text (parse.y:3630 comment scan).
#[test]
fn ansi_c_assignment_with_backtick_comment_closes_cleanly() {
    let input = "w=$'a\\'b`c' # bug `>\necho ok\n";
    assert!(!has_unclosed_quotes(input));
    assert!(!has_unclosed_input_syntax_posix(input, false));
}

/// rubash#251: a multi-byte UTF-8 character in a double-quoted `${...}`
/// alternate (`x="${SCM_THEME_PROMPT_CLEAN:-✔}"`, oh-my-bash
/// omb-prompt-base.sh) must not corrupt the char→byte translation in
/// scan_braced_parameter — the closing quote terminates the word and the
/// following line lexes normally. GNU parse.y parse_matched_pair scans
/// characters, not bytes.
#[test]
fn quoted_dolbrace_utf8_alternate_closes_word() {
    let tokens = tokenize("x=\"${V:-\u{2714}}\"\necho ok\n");
    assert_eq!(tokens[0].kind, crate::lexer::TokenKind::Assignment);
    // The word ends at the closing quote; `echo` is a separate word.
    assert!(tokens.iter().any(|t| t.value == "echo"));
}

#[test]
fn assignment_comsub_body_case_keeps_inner_quotes() {
    // rubash#276: GNU parse.y:4451 parse_comsub parses the `$(...)` body
    // with the real grammar, so a case clause inside the body owns its
    // pattern `)` and the substitution closes at the LAST `)`. The
    // assignment-value quote removal (remove_shell_quotes_assignment ->
    // copy_dollar_paren_body_raw) must copy the body verbatim through the
    // whole `case ... esac` — cutting it at the pattern `)` sent the
    // following `'z\n'` through ordinary quote removal, and the re-parse
    // swallowed the backslash (`w=$(case z in z) printf 'z\n' ;; esac)`
    // stored `zn` where GNU stores `z`).
    let input: String = std::iter::once("w=$(case z in z) printf 'z")
        .chain(std::iter::once("\\"))
        .chain(std::iter::once("n' ;; esac)"))
        .collect();
    let tokens = tokenize(&input);
    assert_eq!(tokens[0].kind, crate::lexer::TokenKind::Assignment);
    let expected_body: String = std::iter::once("$(case z in z) printf 'z")
        .chain(std::iter::once("\\"))
        .chain(std::iter::once("n' ;; esac)"))
        .collect();
    assert!(
        tokens[0].value.contains(&expected_body),
        "{}",
        tokens[0].value
    );

    // Same body inside double quotes: the cooked value still carries the
    // inner single quotes (verified against WSL GNU Bash 5.3.0,
    // target/resid1/p276class.sh A2).
    let quoted = format!("v=\"{input}\"");
    let tokens = tokenize(&quoted);
    assert_eq!(tokens[0].kind, crate::lexer::TokenKind::Assignment);
    assert!(tokens[0].value.contains("'z"), "{}", tokens[0].value);
}

#[test]
fn glued_brace_closer_is_word_text_not_group_closer() {
    // rubash#278: `}` is not in shell_break_chars (syntax.h:29-30), so
    // read_token_word collects `}}` as ONE word and CHECK_FOR_RESERVED_WORD
    // (parse.y:3174-3175) never yields the reserved `}` — the brace group
    // never closes and the parse is GNU's "unexpected end of file from `{'
    // command" EOF error, never a function definition (verified against WSL
    // GNU Bash 5.3.0, target/resid1/p278*.sh).
    let tokens = tokenize("f() { :; }}\necho after\n");
    assert!(tokens.iter().any(|token| token.value == "}}"));
    assert!(!tokens.iter().any(|token| token.kind == TokenKind::Keyword
        && token.value.contains('}')
        && token.value != "("
        && token.value != ")"));
}

/// rubash#465: a keyword-form function definition nested inside a folded
/// brace group must count as a nested opener. GNU's grammar reads the body
/// `{` at command position (parse.y:1056 function_def: FUNCTION WORD
/// function_body) — not gated by reserved_word_acceptable — so
/// `{ function i { :; } }` folds as ONE balanced group token. Without the
/// `function NAME' chain the inner `{` was not counted while the body's
/// `}' still closed the scan, and the fold ended one `}` early — the
/// enclosing definition's closer was misattributed and a later nested
/// `function` definition reported "unexpected end of file from `{'".
#[test]
fn nested_keyword_function_body_counts_as_group_opener() {
    let tokens = tokenize("{ function i { :; } }\necho after\n");
    let group = tokens
        .iter()
        .find(|token| {
            token.kind == TokenKind::Keyword
                && token.value.starts_with('{')
                && token.value.ends_with('}')
        })
        .expect("folded group token");
    // Both closing braces are INSIDE the fold: the group value must end at
    // the outer `}', not the inner function body's.
    assert!(
        group.value.contains("} }"),
        "group must span both closers, got {:?}",
        group.value
    );
    assert!(tokens
        .iter()
        .any(|token| token.kind == TokenKind::Word && token.value == "after"));
}

/// rubash#465 end-to-end shape (mirkop.sh): a completed `&& { ... <<< ""; }`
/// group (whose herestring operator blocks its own fold) followed by a
/// keyword-form function whose body holds a nested keyword-form function.
/// The later definition's fold must keep both `}` lines: the outer body
/// token spans to the LAST `}`, leaving no stray top-level closer, and the
/// trailing statement survives.
#[test]
fn herestring_group_then_nested_function_definitions_fold_balanced() {
    let script = concat!(
        "f() {\n",
        "  [[ a == b ]] && {\n",
        "    ((0)) && {\n",
        "      g <<< \"\"\n",
        "    }\n",
        "  }\n",
        "}\n",
        "function h {\n",
        "  function i {\n",
        "    printf X\n",
        "  }\n",
        "}\n",
    );
    let tokens = tokenize(script);
    // No group token may be left unclosed, and no standalone `}' closer may
    // trail the folded h body: the fold must reach the outer `}`.
    let folded = tokens
        .iter()
        .filter(|token| {
            token.kind == TokenKind::Keyword
                && token.value.starts_with('{')
                && token.value.contains('\n')
        })
        .count();
    assert!(folded >= 1, "h's multi-line body should fold: {tokens:?}");
    let h_body = tokens
        .iter()
        .find(|token| {
            token.kind == TokenKind::Keyword
                && token.value.starts_with('{')
                && token.value.contains("function i")
        })
        .expect("folded h body token");
    assert!(
        h_body.value.matches('}').count() >= 2
            && h_body.value.trim_end().ends_with('}')
            && h_body.value.contains("printf X"),
        "h body fold must include the nested function and both closers, got {:?}",
        h_body.value
    );
}

#[test]
fn unclosed_brace_group_tokens_keep_physical_lines_at_eof() {
    // The leftover logical line at end of input spans every physical line
    // the open construct accumulated; GNU line_number is physical, so the
    // EOF diagnostic numbers from the last physical line, not the line
    // where the group opened (rubash#278: `{\ncmd1\ncmd2` reports the EOF
    // at line 4, matching GNU).
    let tokens = tokenize("{\ncmd1\ncmd2\n");
    let cmd_lines: Vec<usize> = tokens
        .iter()
        .filter(|token| matches!(token.value.as_str(), "cmd1" | "cmd2"))
        .map(|token| token.position)
        .collect();
    assert_eq!(cmd_lines, vec![2, 3]);
}

#[test]
fn dbg_redirect_fields() {
    // print_comsub redirect rendering: the ordered redirects list carries
    // the fd inside the operator and duplications carry a leading `&` on
    // the target (rubash#274; verified shape for `2> /dev/null 2>&1`).
    let tokens = crate::lexer::tokenize("nosuchcmd-y > /dev/null 2>&1");
    let ast = crate::parser::parse(&tokens);
    let Some(command) = ast.commands.first() else {
        panic!("no command");
    };
    let redirects: Vec<(String, String)> = command
        .redirects
        .iter()
        .map(|redirect| (redirect.operator.clone(), redirect.target.clone()))
        .collect();
    assert_eq!(
        redirects,
        vec![
            (">".to_string(), "/dev/null".to_string()),
            ("2>&".to_string(), "&1".to_string())
        ]
    );
}

#[test]
fn extglob_group_swallows_bracket_for_subscript_scan() {
    // GNU parse.y:5464-5490: while extended_glob is live, PATTERN_CHAR + `(`
    // hands the balanced group to parse_matched_pair — the body's `[`/`|`
    // never open an array-subscript hunt. `shopt -s extglob` has already run
    // when the later line parses (incremental read-then-execute), so the
    // pre-scan must track it and keep `echo +(a|b[)*` a plain word
    // (rubash#317; extglob.tests:220 179-line band).
    assert_eq!(
        unclosed_array_subscript_line("shopt -s extglob\necho +(a|b[)*\n"),
        None
    );
    // Without the shopt the old (extglob-off) model is unchanged: GNU's
    // error there is the yacc `(` syntax error, not a `]` EOF.
    assert!(unclosed_array_subscript_line("echo +(a|b[)*\n").is_some());
    // A genuinely unclosed group is a `)`-shaped EOF, owned by the generic
    // close-char scan, not this one.
    assert_eq!(
        unclosed_array_subscript_line("shopt -s extglob\necho +(a|b[\n"),
        None
    );
    // Unchanged true positive: `|` puts `b` at command position, `[` opens
    // the subscript (parse.y:5635-5643, rubash#221).
    assert_eq!(unclosed_array_subscript_line("a | b[c\n"), Some((1, false)));
    // shopt -u restores the extglob-off model mid-script.
    assert!(
        unclosed_array_subscript_line("shopt -s extglob\nshopt -u extglob\necho +(a|b[)*\n")
            .is_some()
    );
}

/// rubash#380: the `$(` body is a fresh command stream (subst.c:7143
/// command_substitute -> parse_and_execute; parse.y:4451 parse_comsub), so
/// its FIRST word sits at command position and IS a reserved word — a
/// leading `case' must open the case-depth machine. The pattern-list
/// forms then push pattern-parens (4928cfa6) whose pops bypass the
/// case-depth guard, and pattern-position `case'/`esac' stay WORD data
/// (parse.y:3177-3186 CHECK_FOR_RESERVED_WORD inside PST_CASEPAT; `esac'
/// is pattern text only after a `|' or pattern-list `(' token,
/// parse.y:3181/3183). GNU-verified vs WSL bash 5.3.0: every shape below
/// runs with rc=0 and an empty/`x` result.
///
/// SPLIT OF OWNERSHIP: the tokenizer-side span (skip_cmd_subst) and the
/// executor machine are fixed in this branch; the admission oracle
/// (`unclosed_input_close_char_posix`) lives in captain-exclusive
/// src/lexer/continuation.rs — the `has_unclosed_input_syntax_posix`
/// assertions stay RED until the captain applies the wt31/casepat
/// continuation.rs diff (comsub-body word boundary + `esac`
/// previous-token rule).
#[test]
fn comsub_leading_case_paren_keyword_patterns_close() {
    let forms = [
        "v=$(case y in (b|case) echo x;; esac)\n",
        "v=$(case y in (case|b) echo x;; esac)\n",
        "v=$(case y in (case) echo x;; esac)\n",
        "v=$(case y in (b|c|case) echo x;; esac)\n",
        "v=$(case y in (case|esac) echo x;; esac)\n",
        "v=$(case y in (b|case) echo x;; (case|d) echo y;; esac)\n",
    ];
    for input in forms {
        assert!(
            !has_unclosed_quotes(input),
            "quotes misjudged for {input:?}"
        );
        // Tokenizer side (skip.rs) — green on this branch.
        let tokens = tokenize(input);
        assert_eq!(tokens[0].kind, TokenKind::Assignment, "for {input:?}");
        assert_eq!(
            tokens[0].value,
            input.trim_end(),
            "comsub span for {input:?}"
        );
        // Admission side (continuation.rs) — needs the captain diff.
        assert!(
            !has_unclosed_input_syntax_posix(input, false),
            "admission rejected {input:?} (needs the #380 continuation.rs diff)"
        );
        assert!(
            unclosed_input_close_char_posix(input, false).is_none(),
            "oracle residue for {input:?}"
        );
    }
}

/// rubash#380 controls: genuinely unclosed forms must still be rejected —
/// the pattern-paren bookkeeping may not swallow a REAL missing `)`.
#[test]
fn comsub_leading_case_missing_close_stays_unclosed() {
    // Missing the comsub's `)`: GNU reports `unexpected EOF while looking
    // for matching `)'`.
    let open = "v=$(case y in (b|case) echo x;; esac\n";
    assert!(has_unclosed_input_syntax_posix(open, false));
    // Missing `esac` AND `)`: the case never closes, so the `)` cannot
    // arrive.
    let no_esac = "v=$(case y in (b|case) echo x;;\n";
    assert!(has_unclosed_input_syntax_posix(no_esac, false));
}

/// rubash#380 nested forms: a `$(`/`case` inside a clause body of an
/// outer comsub case — GNU runs both with rc=0.
#[test]
fn comsub_leading_case_nested_constructs_close() {
    let nested = [
        "v=$(case y in (b|case) echo $(case b in (b) echo n;; esac);; esac)\n",
        "v=$(case y in (b|case) case z in (c) echo zz;; esac;; esac)\n",
    ];
    for input in nested {
        let tokens = tokenize(input);
        assert_eq!(tokens[0].kind, TokenKind::Assignment, "for {input:?}");
        assert_eq!(
            tokens[0].value,
            input.trim_end(),
            "comsub span for {input:?}"
        );
        assert!(
            !has_unclosed_input_syntax_posix(input, false),
            "admission rejected nested form {input:?} (needs the #380 continuation.rs diff)"
        );
    }
}

/// rubash#380 `esac` previous-token rule (parse.y:3181/3183): `esac' is
/// pattern text after `|' or the pattern-list `(' and the keyword
/// everywhere else on a boundary. GNU-verified shapes.
#[test]
fn comsub_esac_pattern_positions_follow_previous_token_rule() {
    // `a|esac)` and `(esac)` are PATTERN text: the case stays open past
    // them and closes at the final keyword `esac`.
    let patterns = [
        "v=$(case b in a|esac) echo hit;; esac)\n",
        "v=$(case b in (esac) echo hit;; esac)\n",
    ];
    for input in patterns {
        assert!(
            !has_unclosed_input_syntax_posix(input, false),
            "pattern-esac rejected {input:?}"
        );
    }
    // Backquote and funsub bodies carry the same grammar (GNU rc=0) and
    // their closers are not `)', so the text scanners keep them closed.
    assert!(!has_unclosed_input_syntax_posix(
        "v=`case y in (b|case) echo x;; esac`\n",
        false
    ));
    assert!(!has_unclosed_input_syntax_posix(
        "v=${ case y in (b|case) echo x;; esac; }\n",
        false
    ));
}

#[test]
fn continuation_join_tokens_keep_physical_lines_and_shared_logical_line() {
    // GNU parse.y:2846: the unquoted `\<newline>` pair is elided from the
    // token text but the reader still bumps line_number, so a token that
    // STARTS after the join sits on the later PHYSICAL line while every
    // command of the joined list shares one LOGICAL line (rubash#411:
    // `readonly RO=1; \` + `RO=2; echo after` reports `line 2`, and the
    // DISCARD abort (eval.c:111) still kills `echo after` on the joined
    // line — the skip needs the logical identity, not the physical line).
    let tokens = tokenize("readonly RO=1; \\\nRO=2; echo after\n");
    let ro2 = tokens
        .iter()
        .find(|token| token.value == "RO=2")
        .expect("assignment token");
    assert_eq!(ro2.position, 2, "assignment on the joined physical line");
    assert_eq!(ro2.logical_line, 1, "assignment on the first logical line");
    let echo = tokens
        .iter()
        .find(|token| token.value == "echo")
        .expect("echo token");
    assert_eq!(echo.position, 2, "echo on the joined physical line");
    assert_eq!(echo.logical_line, 1, "echo on the first logical line");
    // A multi-join chain advances one physical line per join while the
    // logical line stays fixed.
    let tokens = tokenize("echo \\\nnever; \\\nRO=4\n");
    let ro4 = tokens
        .iter()
        .find(|token| token.value == "RO=4")
        .expect("third-line token");
    assert_eq!(ro4.position, 3);
    assert_eq!(ro4.logical_line, 1);
    // A word split MID-WORD by the continuation keeps the word's START
    // line for `position` (GNU's Simple->line word-completion rule is
    // approximated separately by simple_command_first_word_end_line).
    let tokens = tokenize("cmd_a\\\n_b\n");
    let word = tokens
        .iter()
        .find(|token| token.value == "cmd_a_b")
        .expect("joined word");
    assert_eq!(word.position, 1);
    assert_eq!(word.logical_line, 1);
}

#[test]
fn comsub_case_word_glued_pattern_close_arms_region_and_rearm() {
    // rubash#405: the streaming skip.rs case-depth machine must clear the
    // PST_CASEPAT pattern region at the `)` GLUED to the last pattern word
    // (`x)` — parse.y:3787-3788 terminates the pattern list wherever the
    // unquoted `)` arrives) and re-arm it at `;;`/`;&` (parse.y:3710/3759)
    // so a `case`/`esac` WORD in a later pattern list stays pattern text
    // and the final `esac` after `done` is recognized as the keyword.
    // Without it the comsub scan swallows everything after its `)`.
    let forms = [
        "v=$(case k in x) for f in 1 2; do printf x; done esac)\n",
        "v=$(case k in else|done|time|esac) for f in 1 2 3; do printf x; done esac)\n",
        "v=$(case y in (b|case) echo x;; (case|d) echo y;; esac)\n",
        "v=$(case k in x) echo body;; esac)\n",
        "v=$(case k in x) echo a;& y) echo b;; esac)\n",
    ];
    for input in forms {
        let tokens = tokenize(input);
        assert_eq!(tokens[0].kind, TokenKind::Assignment, "for {input:?}");
        assert_eq!(
            tokens[0].value,
            input.trim_end(),
            "comsub span for {input:?}"
        );
        assert!(
            !has_unclosed_input_syntax_posix(input, false),
            "admission rejected {input:?}"
        );
    }
}

#[test]
fn aliases_never_expand_inside_compound_array_assignment() {
    // parse.y:5652-5673 read_token_word: `NAME=` + `(` hands the whole
    // `( ... )` to parse_compound_assignment (parse.y:7104) INSIDE the same
    // word; element words are read with last_read_token = WORD — "we won't
    // be in a command position and so alias expansion won't happen"
    // (parse.y:7113-7117). Bug family: niubash 1.3.0 + oh-my-bash
    // powerbash10k corrupted `OMB_VERSINFO=(1 0 0 0 master noarch)` when a
    // default `alias 1='cd -'` was live, and the later version arithmetic
    // died on the bare `-` element (`niu: -: arithmetic syntax error`).
    let lookup = |name: &str| match name {
        "zq" => Some(("echo ZQ".to_string(), false)),
        "1" => Some(("cd -".to_string(), false)),
        _ => None,
    };
    // The reported corruption: element words are never alias candidates.
    let forms = [
        "A=(zq 0)",
        "OMB_VERSINFO=(1 0 0 0 master noarch)",
        "V=(a zq b)",          // non-first element too
        "declare -a E=(zq 0)", // declaration utility argument
        "export F=(zq 0)",
        "G+=(zq 0)",         // append form
        "J=([k]=zq)",        // subscripted element
        "K=('zq' \"zq\")",   // quoted elements
        "N=( if then )",     // reserved words stay words
        "M=(zq)tail",        // word glued to the closer
        "A=(\n  zq\n  0\n)", // newlines are whitespace
        "S=(# comment\nzq)", // comments skipped (parse.y:7142)
        "h() { g=(zq 0); }", // inside a function body
        "if true; then I=(zq 0); fi",
    ];
    for form in forms {
        let out = expand_aliases_in_source(&format!("{form}\n"), &lookup, false);
        assert_eq!(out, format!("{form}\n"), "leaked in {form:?}");
    }
}

#[test]
fn command_position_aliases_still_expand_around_array_assignments() {
    // The fix must not over-reach: alias candidates remain exactly GNU's
    // command positions (parse.y:3157 command_token_position), which
    // includes the word AFTER a whole assignment word (ASSIGNMENT_WORD)
    // and the first word of a subshell — but not argument position.
    let lookup = |name: &str| match name {
        "zq" => Some(("echo ZQ".to_string(), false)),
        _ => None,
    };
    let cases = [
        ("zq hi\n", "echo ZQ hi\n"),
        ("( zq hi )\n", "( echo ZQ hi )\n"),
        ("x=1 zq hi\n", "x=1 echo ZQ hi\n"),
        ("A=(zq 0) zq hi\n", "A=(zq 0) echo ZQ hi\n"),
        ("echo zq\n", "echo zq\n"),
    ];
    for (input, want) in cases {
        let out = expand_aliases_in_source(input, &lookup, false);
        assert_eq!(out, want, "for {input:?}");
    }
}

#[test]
fn alexpnext_still_expands_inside_compound_array_assignment() {
    // parse.y:3254: alias_expand_token fires on PST_ALEXPNEXT alone, even
    // inside a compound assignment — an alias whose value ends in a blank
    // (AL_EXPANDNEXT, popped at parse.y:2098) and OPENS the array literal
    // makes the next element word expandable. Verified byte-for-byte
    // against WSL GNU Bash 5.3.0: `alias opener='MULTI=( '` + `opener zq 0)`
    // yields elements echo ZQ 0.
    let lookup = |name: &str| match name {
        "opener" => Some(("MULTI=( ".to_string(), true)),
        "zq" => Some(("echo ZQ".to_string(), false)),
        _ => None,
    };
    let out = expand_aliases_in_source("opener zq 0)\n", &lookup, false);
    // Two blanks before `echo`: the alias value's trailing blank plus the
    // input's separator — the splice is textual; word splitting happens in
    // the executor, which is why the e2e output matches GNU byte-for-byte.
    assert_eq!(out, "MULTI=(  echo ZQ 0)\n");
}

/// wt56/alias-cond: the `[[ ]]` interior is alias-inert. GNU reads the
/// conditional through parse_cond_command's recursive descent (read_token
/// cond branch parse.y:3586-3604, parse_cond_command parse.y:5254), whose
/// direct read_token calls bypass yylex — the ONLY `last_read_token'
/// bookkeeper (parse.y:3076-3078). The token therefore stays frozen at
/// COND_START, which reserved_word_acceptable never accepts, so
/// alias_expand_token's assignment_acceptable gate (parse.y:3254) can
/// never fire inside `[[ ]]`. Verified byte-for-byte against WSL GNU Bash
/// 5.3.0 (wt56-aliascond matrix, 2026-10-02); the extglob cells hold with
/// the shopt off as well — the frozen token protects the split fragments
/// just like the consumed whole word.
#[test]
fn cond_interior_words_are_never_alias_candidates() {
    let lookup = |name: &str| match name {
        "zz" => Some(("echo ZZ".to_string(), false)),
        ".." => Some(("cd ..".to_string(), false)),
        _ => None,
    };
    let forms = [
        // The owner's production shape (bash_completion line 1376 via
        // oh-my-bash `alias ..='cd ..'`): the fragment after the extglob
        // `)` used to land in pseudo command position and splice
        // `cd ..` into the pattern.
        "if [[ $cur != ?(*/).. ]]; then echo hit; fi",
        "[[ $cur == ?(*/).. ]] && echo y",
        "[[ zz == zz ]]",
        "[[ ( zz == zz ) ]]",
        "[[ a == b || zz == zz ]]",
        "[[ a == b && zz == zz ]]",
        "[[ ! zz == zz ]]",
        "[[ ab == zz* ]]",
        "[[ abc =~ zz ]]",
        "[[ -f zz ]]",
        "[[ ((zz)) == zz ]]",
        "[[ a == b ]] > zz",
    ];
    for form in forms {
        let out = expand_aliases_in_source(&format!("{form}\n"), &lookup, false);
        assert_eq!(out, format!("{form}\n"), "leaked in {form:?}");
    }
}

/// parse.y:3481-3484 (special_case_tokens): `]]' is COND_END on
/// PST_CONDEXPR alone — no reserved_word_acceptable(last) condition —
/// and it runs BEFORE alias expansion (parse.y:5743-5750), so the
/// everyday `[[ a == b ]]` shape closes the conditional and the token
/// AFTER it is a normal command position again. Verified against GNU
/// 5.3.0: `[[ a == a ]] && zz` prints ZZ, `[[ a == b ]] zz` splices and
/// then fails to parse in both engines.
#[test]
fn cond_end_is_recognized_and_command_position_resumes() {
    let lookup = |name: &str| match name {
        "zz" => Some(("echo ZZ".to_string(), false)),
        _ => None,
    };
    let cases = [
        ("[[ a == b ]]; zz\n", "[[ a == b ]]; echo ZZ\n"),
        ("[[ a == b ]] && zz\n", "[[ a == b ]] && echo ZZ\n"),
        ("[[ a == b ]] zz\n", "[[ a == b ]] echo ZZ\n"),
        // `]]` after an interior `(`/`)` is the closer, never alias text.
        ("[[ ( a == b ) ]] && zz\n", "[[ ( a == b ) ]] && echo ZZ\n"),
        // A later conditional still tracks open/close correctly.
        (
            "[[ a == b ]]; [[ c == d ]]; zz\n",
            "[[ a == b ]]; [[ c == d ]]; echo ZZ\n",
        ),
    ];
    for (input, want) in cases {
        let out = expand_aliases_in_source(input, &lookup, false);
        assert_eq!(out, want, "for {input:?}");
    }
}

/// parse.y:5464-5477 (read_token_word): with `extended_glob' on, a ksh
/// extglob opener `X(` (PATTERN_CHAR syntax.h:90-91: @ * + ? !) consumes
/// its matched `(...)` into the SAME word via parse_matched_pair, so the
/// pattern interior and anything glued after its `)` are one token —
/// never separate alias candidates in argument, assignment-RHS, or
/// case-pattern position (the pattern word keeps PST_CASEPAT intact for
/// the clause's own `)` at parse.y:3787). Verified byte-for-byte against
/// WSL GNU Bash 5.3.0 with `shopt -s extglob`.
#[test]
fn extglob_pattern_words_stay_single_words() {
    crate::lexer::set_parse_extended_glob(true);
    let lookup = |name: &str| match name {
        "zz" => Some(("echo ZZ".to_string(), false)),
        ".." => Some(("cd ..".to_string(), false)),
        _ => None,
    };
    let forms = [
        "echo @(zz)/tail",
        "x=?(zz)file",
        "case zz in ?(a)|zz) echo pat;; *) echo star;; esac",
        "case x in @(zz|..)) echo pat;; *) echo star;; esac",
        "case ../x in ?(*/)..) echo pat;; *) echo star;; esac",
        "[[ $cur != ?(*/).. ]] && echo y",
        "[[ ab == ?(zz|..) ]] && echo y",
    ];
    for form in forms {
        let out = expand_aliases_in_source(&format!("{form}\n"), &lookup, false);
        assert_eq!(out, format!("{form}\n"), "leaked in {form:?}");
    }
    crate::lexer::set_parse_extended_glob(false);
}
#[test]
fn dparen_arith_group_open_at_end_matches_gnu_reader() {
    // rubash#435: the feeder join gate's walker. GNU parse.y:4904
    // parse_dparen reads the arithmetic command body across newlines
    // (parse.y:4963 parse_arith_cmd), so an unclosed command-position `((`
    // means the construct is incomplete; a closed one (including the
    // nested-subshell reinterpretation, parse.y:4938-4948) does not.
    let joined = |s: &str| s.contains("((") && crate::lexer::dparen_arith_group_open_at_end(s);
    // Multi-line comma chain (ble.sh color.sh:538 shape): open after line 1.
    assert!(joined("((h1=H%120,h2=120-h1,"));
    // Closed on the next line: the whole construct is complete.
    assert!(!joined(
        "((h1=H%120,h2=120-h1,
  x+=1))"
    ));
    // Nested ternary groups (init-term.sh:313 shape): open mid-chain.
    assert!(joined(
        "((j1=(i1==3?6:
      (i1==6?3:"
    ));
    // `&&`-continued chain inside the group stays open (keymap.vi shape).
    assert!(joined(
        "((index=(bol+${COLUMNS:-eol})/2,
  index>eol&&(index=eol),"
    ));
    // Nested-subshell reinterpretation closes: `((echo hi); cat` is a
    // subshell command, not an open arithmetic group (parse.y:4938-4948).
    assert!(!joined("((echo hi); cat"));
    // A `$((` unit still open inside the body keeps the line open
    // (core-syntax/comsub shape `(( c=$((a+1),`).
    assert!(joined("(( c=$((a+1),"));
    // Comments are data inside P_ARITH (rubash#222): the group stays open.
    assert!(joined(
        "(( 1+2
# comment"
    ));
    // Single-line arithmetic commands close and never join.
    assert!(!joined("(( 1+2 ))"));
}

#[test]
fn word_initial_raw_esc_single_quoted_is_carrier_tagged_niubash_200() {
    // niubash#200: a raw ESC byte right after the opening `'` is DATA (GNU
    // keeps a raw ESC in every position), but it is byte-identical to
    // QUOTED_WORD_PREFIX, so downstream strip_prefix consumers ate it. The
    // lexer must tag the leading data ESC as the U+E000 raw-byte marker pair.
    let tokens = tokenize("printf '%s' '\u{1b}[31mAAA\u{1b}[0m'");
    let value = &tokens[2].value;
    let chars: Vec<u32> = value.chars().map(|c| c as u32).collect();
    // U+E000 (RAW_BYTE_MARKER_ESCAPE) + U+E01C (0x1b) — not a bare U+001B,
    // which the executor's quoted-tilde marker strip would claim.
    assert_eq!(&chars[..2], &[0xE000, 0xE001 + 0x1b]);
    // The mid-string ESC stays verbatim data (never word-initial, never
    // claimed): exactly one carrier tag for the word-initial ESC.
    assert_eq!(
        chars.iter().filter(|&&c| c == 0xE000).count(),
        1,
        "exactly one carrier tag for the word-initial ESC"
    );
}

#[test]
fn word_initial_raw_esc_mid_word_stays_verbatim_niubash_200() {
    // A mid-word ESC is never claimed by strip_prefix, so it needs no tag:
    // `'X<ESC>[32m'` keeps the raw ESC byte verbatim (GNU-identical).
    let tokens = tokenize("printf '%s' 'X\u{1b}[32mBBB'");
    let value = &tokens[2].value;
    assert!(value.starts_with('X'));
    assert!(value.contains('\u{1b}'));
    assert!(
        !value.contains('\u{E000}'),
        "mid-word ESC must not be tagged"
    );
}

#[test]
fn word_initial_raw_esc_double_quoted_and_unquoted_tagged_niubash_200() {
    for (input, word_index) in [
        ("printf '%s' \"\u{1b}[31mC\"", 2usize),
        ("printf '%s' \u{1b}[31mB", 2usize),
    ] {
        let tokens = tokenize(input);
        let chars: Vec<u32> = tokens[word_index].value.chars().map(|c| c as u32).collect();
        assert_eq!(
            &chars[..2],
            &[0xE000, 0xE001 + 0x1b],
            "word-initial raw ESC must be carrier-tagged in every quoting context"
        );
    }
}

#[test]
fn ansi_c_literal_raw_esc_word_initial_tagged_niubash_200() {
    // $'<raw-ESC>...' pushes the already-raw byte verbatim (only \e/\c[/\033
    // get the carrier inside decode_ansi_c_quoted), so the same word-initial
    // collision applies. The textual `$'\033...'` form must stay untouched.
    let raw = tokenize("printf '%s' $'\u{1b}[31mG'");
    let chars: Vec<u32> = raw[2].value.chars().map(|c| c as u32).collect();
    assert_eq!(&chars[..2], &[0xE000, 0xE001 + 0x1b]);
    let textual = tokenize("printf '%s' $'\\033[31mG'");
    assert_eq!(textual[2].value, raw[2].value);
}

#[test]
fn quoted_literal_tilde_marker_still_emitted_niubash_200() {
    // The fix must not disturb the legitimate QUOTED_WORD_PREFIX producer:
    // a quoted literal `~` still carries the U+001B marker for the executor.
    let tokens = tokenize("printf '%s' '~x'");
    assert!(tokens[2]
        .value
        .starts_with(crate::executor::markers::QUOTED_WORD_PREFIX));
    assert!(tokens[2].value.ends_with("~x"));
}
