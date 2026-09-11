use super::*;

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
    // trailing `'..."` single-quoted segment balances the word. The
    // unclosed-input pre-flight and the line collector must agree, otherwise
    // the script is rejected with "unexpected end of file" before execution.
    let line = "(echo -n '28 '; printf '%s\\n' \"${IFS+\"'\"x ~ x'}'x\"'}\"x}\" #') 2>&-";
    assert!(
        !has_unclosed_input_syntax(line),
        "posix-interleaved quotes must not read as unclosed input"
    );
    let tokens = tokenize_with_initial_posix(line, true);
    assert!(
        tokens.iter().any(|token| {
            token.value.starts_with('\x1d')
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
        word.value.starts_with('\x1d'),
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

    assert_eq!(words[0].value, "a[\" \"]=15");
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
