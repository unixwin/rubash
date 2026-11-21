//! rubash#288: a `[sub]=value` compound array element must undergo exactly
//! ONE quote-removal pass, matching GNU's pipeline —
//! `expand_compound_array_assignment` (arrayfunc.c:557) runs
//! `parse_string_to_word_list` (tokenizes the raw body, quoting intact) and
//! then `expand_words_no_vars` (arrayfunc.c:610), after which
//! `assign_compound_array_list` stores `val = w + len + 2` (arrayfunc.c:839)
//! verbatim. The old rubash composition ran the storage unquote ON TOP of
//! `remove_shell_quotes`, re-reading literal backslashes as escape syntax:
//! `A=([0]='\[\e[33m\]Z')` stored `[e[33m]Z` and oh-my-bash agnoster printed
//! ` [e[33m]` noise in the prompt.
//!
//! Verified expectations come from WSL GNU Bash 5.3.0 (/usr/local/bin/bash)
//! script-file probes (`od -c` byte streams).

use std::process::Command;

fn rubash_raw(script: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    output.stdout
}

fn element_zero(script_head: &str) -> Vec<u8> {
    rubash_raw(&format!("{script_head}\nprintf '%s' \"${{V[0]}}\""))
}

const PROMPT_ESC: &[u8] = b"\\[\\e[33m\\]Z";

#[test]
fn single_quoted_subscript_element_keeps_backslashes() {
    assert_eq!(
        element_zero("V=([0]='\\[\\e[33m\\]Z')"),
        PROMPT_ESC,
        "parse.y:5305 read_token_word: '...' interior is literal"
    );
}

#[test]
fn declare_single_quoted_subscript_element_keeps_backslashes() {
    assert_eq!(
        element_zero("declare -a V=([0]='\\[\\e[33m\\]Z')"),
        PROMPT_ESC
    );
}

#[test]
fn double_quoted_subscript_element_keeps_non_cbsdquote_escapes() {
    // parse.y:5390-5393 + syntax.h:26 slashify_in_quotes = "\\`$\"\n":
    // `\e`/`\[`/`\]` inside "..." keep BOTH characters.
    assert_eq!(element_zero("V=([0]=\"\\[\\e[33m\\]Z\")"), PROMPT_ESC);
    assert_eq!(
        element_zero("declare -a V=([0]=\"\\[\\e[33m\\]Z\")"),
        PROMPT_ESC
    );
}

#[test]
fn dollar_single_quoted_subscript_element_decodes_once() {
    // GNU parse.y:5564-5571: ansiexpand's result is re-wrapped in
    // sh_single_quote, so the decoded bytes stay quoted through the one
    // removal. Expected bytes: \ [ ESC [ 3 3 m \ ] Z.
    assert_eq!(
        element_zero("V=([0]=$'\\[\\e[33m\\]Z')"),
        b"\\[\x1b[33m\\]Z"
    );
    // The `\]` unknown-escape pair keeps its backslash (GNU strtrans.c
    // ansicstr default branch) instead of collapsing to `]`.
    assert_eq!(element_zero("V=([0]=$'\\]')"), b"\\]");
}

#[test]
fn bare_subscript_element_still_drops_unquoted_backslashes() {
    // parse.y:5368-5397: an unquoted backslash quotes exactly the next
    // character, which survives WITHOUT the backslash.
    assert_eq!(element_zero("V=([0]=\\[\\eZ\\])"), b"[eZ]");
}

#[test]
fn append_subscript_element_keeps_backslashes() {
    assert_eq!(
        rubash_raw("V=(a)\nV+=('\\[\\e[44m\\]W')\nprintf '%s' \"${V[1]}\""),
        b"\\[\\e[44m\\]W"
    );
    assert_eq!(
        rubash_raw("V=(a)\nV+=([5]='\\[\\e[44m\\]W')\nprintf '%s' \"${V[5]}\""),
        b"\\[\\e[44m\\]W"
    );
}

#[test]
fn quotes_inside_double_quoted_element_are_data() {
    // One removal: the `'a'` inside "..." is data and must survive the
    // storage pass (a second removal used to strip it to `a`).
    assert_eq!(element_zero("V=([0]=\"'a'\")"), b"'a'");
    assert_eq!(element_zero("declare -a V=([0]=\"'a'\")"), b"'a'");
    // CBSDQUOTE: `\\` collapses to one backslash, exactly once.
    assert_eq!(element_zero("V=([0]=\"a\\\\b\")"), b"a\\b");
}

#[test]
fn agnoster_symbol_element_stays_glued_with_trailing_space() {
    // oh-my-bash agnoster stores prompt segments like '\[\e[33m\]bolt ' in
    // arrays: quote-span whitespace never field-splits.
    assert_eq!(
        rubash_raw("V=('\\[\\e[33m\\]bolt ')\nprintf '%s' \"${V[0]}\""),
        b"\\[\\e[33m\\]bolt "
    );
}

#[test]
fn mixed_quote_subscript_element_dequotes_positionally() {
    // 'it'\''s -> it's: escape + span adjacency, one removal.
    assert_eq!(element_zero("V=([0]='it'\\''s')"), b"it's");
    // a\'b (escaped quote outside quotes) -> a'b.
    assert_eq!(element_zero("V=([0]=a\\'b)"), b"a'b");
}

#[test]
fn scalar_append_keeps_expansion_result_backslashes() {
    // rubash#288 user-visible path: oh-my-bash agnoster builds its status
    // segment with `symbols+=$REPLY'⚡'` where REPLY holds a `\[\e[33m\]`
    // color prefix. GNU expand_string_assignment (subst.c:4345) dequotes
    // the word ONCE and variables.c's assign_array_element appends the
    // text to element 0 — expansion-produced backslashes are data (GNU
    // CTLESC-protects them via add_quoted_string, subst.c:11862 ->
    // quote_string, subst.c:4773). A second storage-layer unquote read
    // them as escape syntax and stored `[e[33m]⚡`.
    assert_eq!(
        rubash_raw(
            "REPLY='\\[\\e[33m\\]'\nsymbols=()\nsymbols+=$REPLY'⚡'\nprintf '%s' \"${symbols[0]}\"",
        ),
        b"\\[\\e[33m\\]\xe2\x9a\xa1"
    );
    // Double-quoted expansion + quoted suffix concatenates identically.
    assert_eq!(
        rubash_raw(
            "REPLY='\\[\\e[33m\\]'\nsymbols=()\nsymbols+=\"$REPLY\"'Q'\nprintf '%s' \"${symbols[0]}\"",
        ),
        b"\\[\\e[33m\\]Q"
    );
    // Appending to an existing element keeps both halves verbatim.
    assert_eq!(
        rubash_raw(
            "REPLY='\\[\\e[33m\\]'\nsymbols=(x)\nsymbols+=$REPLY'Q'\nprintf '%s' \"${symbols[0]}\"",
        ),
        b"x\\[\\e[33m\\]Q"
    );
    // `arr+=word` binds ONE scalar (variables.c assign_array_element): the
    // word never field-splits, matching `s=(a); s+="x y"` -> [0]="ax y".
    assert_eq!(
        rubash_raw("s=(a)\ns+=\"x y\"\nprintf '%s' \"${s[0]}\""),
        b"ax y"
    );
}

#[test]
fn single_quoted_brace_element_stays_literal() {
    // unixwin/niubash#1060: a single-quoted `{m,n}` / `{m..n}` inside a
    // compound array literal is quoted DATA for GNU — brace_gobbler
    // (braces.c:638) opens its quoted state on the `'` characters in the
    // raw word (braces.c:729-734) and skips the braces, so
    // expand_compound_array_assignment (arrayfunc.c:557 -> 581
    // parse_string_to_word_list -> 610 expand_words_no_vars) stores the
    // element verbatim. Rubash stores the `'...'` span wrapped in the
    // ASSIGN_HOISTED_SQUOTE (U+E307) carrier pair; the brace scanners in
    // expand/braces.rs must honor that delimiter exactly like the E301 /
    // literal `'` forms — and the unquoted brace groups in the same word
    // must still expand (rubash#3970).
    // Every expectation below verified against WSL GNU bash 5.3.20
    // (/usr/local/bin/bash) script-file probes.
    assert_eq!(
        rubash_raw("a=('X{16,}Y')\nprintf '<%s> %s' \"${a[0]}\" \"${#a[@]}\""),
        b"<X{16,}Y> 1"
    );
    assert_eq!(
        rubash_raw("a=('a{1,2}b')\nprintf '<%s> %s' \"${a[0]}\" \"${#a[@]}\""),
        b"<a{1,2}b> 1"
    );
    assert_eq!(
        rubash_raw("a=('a{1..3}b')\nprintf '<%s> %s' \"${a[0]}\" \"${#a[@]}\""),
        b"<a{1..3}b> 1"
    );
    // A quoted span containing whitespace braces keeps both the braces
    // and the single element.
    assert_eq!(
        rubash_raw("a=('a b{1,2}')\nprintf '<%s> %s' \"${a[0]}\" \"${#a[@]}\""),
        b"<a b{1,2}> 1"
    );
    // declare -a and += take the same compound expansion path.
    assert_eq!(
        rubash_raw("declare -a a=('X{16,}Y')\nprintf '<%s> %s' \"${a[0]}\" \"${#a[@]}\""),
        b"<X{16,}Y> 1"
    );
    assert_eq!(
        rubash_raw("a=(x)\na+=('X{16,}Y')\nprintf '<%s> %s' \"${a[1]}\" \"${#a[@]}\""),
        b"<X{16,}Y> 2"
    );
    // A [sub]= element's quoted braces stay literal too.
    assert_eq!(
        rubash_raw("a=([3]='x{1,2}y')\nprintf '<%s> %s' \"${a[3]}\" \"${#a[@]}\""),
        b"<x{1,2}y> 1"
    );
    // Mixed quoting: the UNQUOTED brace group still expands while the
    // quoted half stays literal — one check for the whole class, not per
    // symptom (AGENTS.md rubash#117 rule).
    assert_eq!(
        rubash_raw("a=('a'{1,2})\nprintf '<%s> %s' \"${a[*]}\" \"${#a[@]}\""),
        b"<a1 a2> 2"
    );
    assert_eq!(
        rubash_raw("a=('pre'{1,2}'post')\nprintf '<%s> %s' \"${a[*]}\" \"${#a[@]}\""),
        b"<pre1post pre2post> 2"
    );
    assert_eq!(
        rubash_raw("a=('x{a,b}'{1,2})\nprintf '<%s> %s' \"${a[*]}\" \"${#a[@]}\""),
        b"<x{a,b}1 x{a,b}2> 2"
    );
    // Both halves quoted braces + unquoted braces on one element.
    assert_eq!(
        rubash_raw("a=('pre'{a,b}'mid'{1,2}'post')\nprintf '<%s> %s' \"${a[*]}\" \"${#a[@]}\""),
        b"<preamid1post preamid2post prebmid1post prebmid2post> 4"
    );
    // Unquoted brace expansion inside a compound literal is unchanged.
    assert_eq!(
        rubash_raw("a=(x{1,2}y)\nprintf '<%s> %s' \"${a[*]}\" \"${#a[@]}\""),
        b"<x1y x2y> 2"
    );
}

/// Bind `element` (a `V...` compound-assignment fragment) in a for-loop over
/// `for_word` and return element 0's bytes.
fn for_element_zero(for_word: &str, element: &str) -> Vec<u8> {
    rubash_raw(&format!(
        "V=()\nfor v in {for_word}; do V{element}; done\nprintf '%s' \"${{V[0]}}\""
    ))
}

#[test]
fn fully_single_quoted_for_word_decodes_all_quote_carriers() {
    // rubash#629: the fully-single-quoted for-word fast path
    // (command_words.rs) restored only the protected-dollar carrier, leaving
    // the lexer's DATA_DQUOTE (0x18, lexer/quotes.rs b'"' arm) and
    // DATA_BACKTICK (0x1a) carriers live in the bound loop variable, so they
    // reached the compound element as raw C0 bytes. GNU keeps no carrier for
    // these: parse.y:3877 parse_matched_pair reads the `'...'` interior
    // literally (subst.c:4807 dequote_string is the only removal), so the
    // element must hold `"` / `` ` `` exactly. Cover every compound form that
    // serializes back through quote_array_value: `=`, `+=`,
    // `[i]=`, and `+=([i]=)`.
    let expected: &[u8] = b"he said \"hi\"";
    for element in ["=(\"$v\")", "+=(\"$v\")", "=([0]=\"$v\")", "+=([0]=\"$v\")"] {
        assert_eq!(
            for_element_zero("'he said \"hi\"'", element),
            expected,
            "compound element `{element}` leaked a data carrier byte"
        );
    }
    // The backtick carrier must decode too (a bare `` ` `` is literal data
    // inside `'...'`; GNU never rescans for command substitution there).
    assert_eq!(
        for_element_zero("'a`b'", "+=(\"$v\")"),
        b"a`b",
        "backtick carrier (0x1a) leaked"
    );
    // The dollar carrier already decoded before #629; pin it so the fix
    // cannot regress it.
    assert_eq!(for_element_zero("'a$b'", "+=(\"$v\")"), b"a$b");
}

#[test]
fn for_word_carrier_decode_leaves_clean_paths_unchanged() {
    // `a[0]="$v"` (indexed element assignment, not compound) and any
    // for-word that is not fully single quoted already ran through the
    // walker's whole-family decode; they must stay clean.
    assert_eq!(
        rubash_raw(
            "V=()\nfor v in 'he said \"hi\"'; do V[0]=\"$v\"; done\nprintf '%s' \"${V[0]}\"",
        ),
        b"he said \"hi\""
    );
    assert_eq!(
        for_element_zero("x'he said \"hi\"'", "+=(\"$v\")"),
        b"xhe said \"hi\""
    );
}

#[test]
fn compound_double_quoted_element_with_expansion_stays_one_element() {
    // rubash#1504 (external report, Azathothas): GNU
    // expand_compound_array_assignment (arrayfunc.c:557) tokenizes the raw
    // body with parse_string_to_word_list (arrayfunc.c:581 — the word
    // `"a $v"` is ONE W_QUOTED word) and expand_words_no_vars
    // (arrayfunc.c:610 -> subst.c:13219 expand_word_list_internal) never
    // field-splits a W_QUOTED word, so the quoted expansion result stays a
    // single element however much IFS whitespace it produced. Verified
    // against GNU bash 5.3.20 script-file probes.
    assert_eq!(
        rubash_raw("v='b c'\na=(x \"a $v\")\nprintf '<%s>' \"${a[@]}\""),
        b"<x><a b c>"
    );
    assert_eq!(
        rubash_raw("v='b c'\na=(\"a $v\")\nprintf '<%s>' \"${a[@]}\""),
        b"<a b c>"
    );
    // The wholly-quoted whole-expansion element already worked; pin it.
    assert_eq!(
        rubash_raw("v='b c'\na=(\"$v\")\nprintf '<%s>' \"${a[@]}\""),
        b"<b c>"
    );
    // A quoted element whose expansion is empty keeps its literal part.
    assert_eq!(
        rubash_raw("a=(\"a $unset_v\")\nprintf '<%s>' \"${a[@]}\""),
        b"<a >"
    );
}

#[test]
fn compound_quoted_element_with_newline_value_stays_one_element() {
    // rubash#1504 case 3: a quoted expansion result containing a newline
    // is still one element (GNU: the newline is quoted data, CTLESC
    // protected, never an IFS split point inside a W_QUOTED word).
    // The plain form asserts through ${a[0]} + the element count: the
    // "${a[@]}" rendering of a newline-bearing element leaks a U+E401
    // carrier byte on BOTH the baseline and this tree — that is the
    // separate rubash#1505 sentinel-leak issue, not #1504 splitting
    // (verified byte-identical against the unmodified master binary).
    assert_eq!(
        rubash_raw("vn=$'d\\ne'\na=(\"$vn\")\nprintf '%s/%s' \"${#a[@]}\" \"${a[0]}\""),
        b"1/d\ne"
    );
    assert_eq!(
        rubash_raw("vn=$'d\\ne'\ndeclare -a a=(\"$vn\")\nprintf '<%s>' \"${a[@]}\""),
        b"<d\ne>"
    );
    assert_eq!(
        rubash_raw("vn=$'d\\ne'\nf() { local -a a=(\"$vn\"); printf '<%s>' \"${a[@]}\"; }\nf"),
        b"<d\ne>"
    );
}

#[test]
fn local_compound_quoted_positional_and_brace_element_shapes() {
    // rubash#1504 case 3 via a positional parameter (the issue's exact
    // shape): the declare/local operand route delivers the element with
    // RAW quote delimiters (no hoisting), and a quoted bare `$1` is one
    // W_QUOTED word for GNU (arrayfunc.c:581) — with a space value as
    // well as a newline value.
    assert_eq!(
        rubash_raw("f3() { local -a a=(\"$1\"); printf '<%s>' \"${a[@]}\"; }\nf3 'a b'"),
        b"<a b>"
    );
    assert_eq!(
        rubash_raw("f3() { local -a a=(\"$1\"); printf '<%s>' \"${a[@]}\"; }\nf3 $'d\\ne'"),
        b"<d\ne>"
    );
    // rubash#1504 case 2 also fires single-line: ANY element carrying a
    // balanced brace group inside quotes ('{}') must not dequote the
    // operand word — braces.c brace_expand is quote-aware, so the group
    // is literal text and brace expansion is a no-op for the word. Real
    // brace expansion of an operand is untouched (GNU expands
    // `declare a={x,y}` to two assignments; a ends as y).
    assert_eq!(
        rubash_raw("v='b c'\nf() { local -a a=('{}' \"a $v\"); printf '<%s>' \"${a[@]}\"; }\nf"),
        b"<{}><a b c>"
    );
    assert_eq!(
        rubash_raw("v='b c'\ndeclare -a a=('{}' \"a $v\")\nprintf '<%s>' \"${a[@]}\""),
        b"<{}><a b c>"
    );
    assert_eq!(rubash_raw("declare a={x,y}\nprintf '%s' \"$a\""), b"y");
    // An ANSI-C quoted element is one quoted word too: the whitespace
    // its decoding produces is data (declare storage must not re-split
    // the `$'...'` serialized form).
    assert_eq!(
        rubash_raw("declare -a a=($'x\\ty')\nprintf '<%s>' \"${a[@]}\""),
        b"<x\ty>"
    );
}

#[test]
fn local_compound_multiline_after_brace_element_keeps_quoted_element() {
    // rubash#1504 case 2 (the curl Authorization-header shape): a
    // multi-line `local -a` whose first line carries a single-quoted
    // brace element must not corrupt the following quoted element.
    assert_eq!(
        rubash_raw(
            "v='b c'\nf2() {\n  local -a a=('{}'\n              \"a $v\")\n  printf '<%s>' \"${a[@]}\"\n}\nf2"
        ),
        b"<{}><a b c>"
    );
}

#[test]
fn fully_single_quoted_for_word_drops_quoted_tilde_prefix() {
    // rubash#629: the same fast path also returned the lexer's
    // QUOTED_WORD_PREFIX (0x1b, markers.rs:107) — the marker a word that
    // begins with a quoted tilde carries so that no later tilde pass touches
    // it. The walker drops it in expand_marked_or_special_word
    // (expand_word.rs:515); the fast path must too, otherwise the loop
    // variable holds `\x1b~` and the compound element serialises the C0 byte.
    // GNU keeps the tilde literal — it is only special unquoted at word start
    // (parse.y:3877 parse_matched_pair reads the span verbatim).
    for word in ["'~'", "'~user'", "'~/'", "'~$x'"] {
        assert_eq!(
            for_element_zero(word, "+=(\"$v\")"),
            word.trim_matches('\'').as_bytes(),
            "quoted-tilde for-word `{word}` kept the word-prefix marker"
        );
    }
    // A tilde that does not begin the word never carried the marker; it must
    // stay a literal tilde.
    assert_eq!(for_element_zero("'x~y'", "+=(\"$v\")"), b"x~y");
    // The bare binding (no compound assignment) must be clean as well.
    assert_eq!(
        rubash_raw("for v in '~'; do printf '%s' \"$v\"; done"),
        b"~"
    );
}
