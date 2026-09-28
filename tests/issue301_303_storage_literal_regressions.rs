//! Issue rubash#301 and rubash#303 regression: storage/literal domain.
//!
//! #301: a backslash inside a single-quoted alias body (and, generally, any
//! quote-span backslash of an argument-position `NAME='...'`-shaped word) was
//! dropped from the stored value — `alias path='echo -e ${PATH//:/\\n}'`
//! (mathiasbynens/dotfiles .aliases:148 @b7c7894) stored `...\n...` instead
//! of `...\\n...`.
//!
//! GNU anchors: parse.y:5419-5436 read_token_word keeps a `'...'` span
//! verbatim in the token; subst.c:11881-11932 (expand_word_internal case
//! '\'') extracts the span and hands it to add_quoted_string, whose
//! quote_string (subst.c:4773) CTLESC-protects every character — the span's
//! backslashes are data. builtins/alias.def:113-139 alias_builtin stores the
//! already-expanded word text after the first `=` verbatim (add_alias), so
//! no pass may read the span's `\` as source escape syntax.
//!
//! #303: a single-quoted `${@}` in a compound array assignment expanded
//! instead of staying literal — `A=('${@}')` must store ONE element with the
//! literal text `${@}` (GNU), but rubash expanded it to the (empty) `$@` and
//! left the quotes as data (`['']`).
//!
//! GNU anchors: arrayfunc.c:557-610 expand_compound_array_assignment ->
//! parse_string_to_word_list (arrayfunc.c:581, y.tab.c:9374) re-tokenizes the
//! compound body with the REAL tokenizer (PST_COMPASSIGN|PST_REPARSE|
//! PST_STRING), so `'${@}'` is one W_QUOTED word; expand_words_no_vars
//! (arrayfunc.c:609 -> subst.c:12590) then never sees live `${...}` syntax
//! inside the span (subst.c:11881 case '\'' -> add_quoted_string ->
//! quote_string CTLESC-protects the content). Fieldsplit-lane leftover
//! (ef9b4ee7): hoist_data_single_quotes skipped `${...}` bodies even INSIDE
//! a hoisted '...' span, leaving a live `${@}` for the walker to expand.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Alias EXPANSION (`path` as a command word) needs the alias table at
/// parse time, and GNU parses a `-c` string before running any command in
/// it, so `shopt -s expand_aliases` cannot arm later words of the same `-c`
/// string (GNU 5.3.0: `bash -c 'shopt -s expand_aliases; alias e=...; e'`
/// reports `e: command not found`). Alias-use cases therefore run from a
/// script file, the way the WSL GNU baseline was captured.
fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue301-{:x}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0),
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_clean(stderr: &str) {
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
}

// ---------------------------------------------------------------------------
// rubash#301: alias storage keeps single-quoted backslashes
// ---------------------------------------------------------------------------

/// The reporter's exact line (mathiasbynens/dotfiles .aliases:148): the
/// stored alias value must keep the `\\n` pair byte-for-byte, `alias path`
/// must reprint it, and using the alias must reparse `\\n` as one literal
/// backslash + n for the patsub replacement.
#[test]
fn mathiasbynens_path_alias_stores_double_backslash() {
    let (stdout, stderr, _) = rubash_script(
        "shopt -s expand_aliases\nP=/a:/b:/c\nalias path='echo -e ${P//:/\\\\n}'\nalias path\npath\nunalias path\n",
    );
    assert_clean(&stderr);
    assert_eq!(
        stdout, "alias path='echo -e ${P//:/\\\\n}'\n/a\n/b\n/c\n",
        "alias storage + use must match GNU 5.3.0 byte-for-byte"
    );
}

/// Backslash class inside a single-quoted alias body: pairs survive storage
/// (`\\` stays `\\`), specials stay (`\$`, `` \` ``, `\"`, `\n`), and the
/// embedded-quote splice (`'\''`) keeps its backslash.
#[test]
fn alias_body_backslash_class_survives_storage() {
    let (stdout, stderr, _) = rubash(
        "shopt -s expand_aliases; alias b1='a\\\\b'; alias b1; \
         alias b3='\\\\n'; alias b3; \
         alias b4='a\\$b'; alias b4; \
         alias b5='a\\`b'; alias b5; \
         alias b6='a\\\"b'; alias b6; \
         alias b8='x\\\\$y'; alias b8; \
         alias b9='a\\'\\''b \\c'; alias b9; \
         unalias -a",
    );
    assert_clean(&stderr);
    assert_eq!(
        stdout,
        "alias b1='a\\\\b'\n\
         alias b3='\\\\n'\n\
         alias b4='a\\$b'\n\
         alias b5='a\\`b'\n\
         alias b6='a\\\"b'\n\
         alias b8='x\\\\$y'\n\
         alias b9='a\\'\\''b \\c'\n",
        "single-quote span backslashes are storage data, not escape syntax"
    );
}

/// Alias reparse semantics after correct storage: the value re-enters the
/// parser, so `a\\b` prints `a\b`, and `\\$v` keeps the second `$v` literal.
#[test]
fn alias_use_reparse_keeps_gnu_escape_semantics() {
    let (stdout, stderr, _) = rubash_script(
        "shopt -s expand_aliases\nalias e='echo a\\\\b'\ne\nv=ok\nalias r='echo $v \\\\$v'\nr\nunalias -a\n",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "a\\b\nok \\ok\n");
}

/// Real scalar assignments through the same lexer path are unchanged.
#[test]
fn scalar_assignment_quoted_rhs_backslash_unchanged() {
    let (stdout, stderr, _) = rubash(
        "v='a\\\\b'; printf '[%s]' \"$v\"; \
         w='a\\ b'; printf '[%s]' \"$w\"; \
         u='a\\$b'; printf '[%s]' \"$u\"; echo",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "[a\\\\b][a\\ b][a\\$b]\n");
}

// ---------------------------------------------------------------------------
// rubash#301 (class): argument-position `NAME='...'`-shaped words
// ---------------------------------------------------------------------------

/// The whole class matrix from the probe ladder: fully-quoted RHS, partial
/// quotes on either side, quoted name part, multiple `=`, adjacent operands.
/// GNU expands each as an ORDINARY word whose quote-span backslashes are
/// data (parse.y:5419 -> subst.c:11881 add_quoted_string).
#[test]
fn assignment_shaped_argument_words_keep_span_backslashes() {
    let (stdout, stderr, _) = rubash(
        "echo n='a\\\\b'; \
         echo o='a\\\\b'x; \
         echo p=x'a\\\\b'; \
         echo q='a\\\\b'\"c\"; \
         echo r='n'='a\\\\b'; \
         echo s=n''='a\\\\b'; \
         echo t='a\\\\b'q='c\\\\d'; \
         printf '[%s]' k='a\\\\b'; echo",
    );
    assert_clean(&stderr);
    assert_eq!(
        stdout,
        "n=a\\\\b\n\
         o=a\\\\bx\n\
         p=xa\\\\b\n\
         q=a\\\\bc\n\
         r=n=a\\\\b\n\
         s=n=a\\\\b\n\
         t=a\\\\bq=c\\\\d\n\
         [k=a\\\\b]\n"
    );
}

/// Guard matrix: single backslash pairs (`\c`) in argument words already
/// matched GNU and must keep doing so; glob metachars stay escaped.
#[test]
fn single_backslash_pairs_and_glob_guards_unchanged() {
    let (stdout, stderr, _) = rubash(
        "echo n='a\\ b'; \
         echo n='a\\*b'; \
         echo n='a\\?b'; \
         echo n='a\\nb'; \
         echo n='a\\{b'; \
         echo n='a\\(b'; \
         echo n='a\\|b'; \
         echo n='a\\~b'; \
         touch 'aXb' 'aYb'; echo 'a\\*b'; echo m='a\\*b'; rm -f 'aXb' 'aYb'",
    );
    assert_clean(&stderr);
    assert_eq!(
        stdout,
        "n=a\\ b\n\
         n=a\\*b\n\
         n=a\\?b\n\
         n=a\\nb\n\
         n=a\\{b\n\
         n=a\\(b\n\
         n=a\\|b\n\
         n=a\\~b\n\
         a\\*b\n\
         m=a\\*b\n"
    );
}

/// Unquoted and double-quoted RHS escape rules are untouched: `n=a\\b`
/// collapses to `a\b` in GNU, and `"a\\b"` likewise.
#[test]
fn unquoted_and_dquoted_rhs_rules_unchanged() {
    let (stdout, stderr, _) = rubash("echo n=a\\\\b; echo m==\"a\\\\b\"; echo o=\"a\\\\b\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "n=a\\b\nm==a\\b\no=a\\b\n");
}

// ---------------------------------------------------------------------------
// rubash#303: single-quoted `${@}` in compound array assignments
// ---------------------------------------------------------------------------

/// The reporter's exact shape, without and with positional parameters set:
/// `'${@}'` is literal data — one element holding the text `${@}`.
#[test]
fn single_quoted_at_in_compound_assignment_stays_literal() {
    let (stdout, stderr, _) = rubash(
        "A=('${@}'); printf 'len=%d e=[%s]\\n' \"${#A[@]}\" \"${A[0]}\"; \
         set -- 'a b' c; B=('${@}'); printf 'len=%d e=[%s]\\n' \"${#B[@]}\" \"${B[0]}\"",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "len=1 e=[${@}]\nlen=1 e=[${@}]\n");
}

/// The class: every `${...}`/`$param` spelling inside a single-quoted
/// compound element stays literal (`${v}`, `${v:-y}`, `${*}`, `${#}`, `$1`,
/// prefix/suffix text around the span).
#[test]
fn single_quoted_parameter_spellings_in_compound_elements_stay_literal() {
    let (stdout, stderr, _) = rubash(
        "v=x; set -- 'a b' c; \
         A=('a${v}b'); printf 'A=[%s]\\n' \"${A[0]}\"; \
         B=('${v:-y}'); printf 'B=[%s]\\n' \"${B[0]}\"; \
         C=('${*}'); printf 'C=[%s]\\n' \"${C[0]}\"; \
         D=('${#}'); printf 'D=[%s]\\n' \"${D[0]}\"; \
         E=('a$1b'); printf 'E=[%s]\\n' \"${E[0]}\"; \
         F=('a\\$1b'); printf 'F=[%s]\\n' \"${F[0]}\"; \
         G=('a$vb'); printf 'G=[%s]\\n' \"${G[0]}\"",
    );
    assert_clean(&stderr);
    assert_eq!(
        stdout,
        "A=[a${v}b]\n\
         B=[${v:-y}]\n\
         C=[${*}]\n\
         D=[${#}]\n\
         E=[a$1b]\n\
         F=[a\\$1b]\n\
         G=[a$vb]\n"
    );
}

/// Guards: the neighboring forms keep GNU semantics — a DOUBLE-quoted
/// `"${@}"` still fans out one element per positional parameter, an
/// UNQUOTED `$@` still joins and field-splits (rubash#298), a mixed list
/// keeps its element count, and backslash pairs in quoted elements survive.
#[test]
fn neighboring_compound_forms_keep_gnu_semantics() {
    let (stdout, stderr, _) = rubash(
        "set -- 'a b' c; \
         C=(\"${@}\"); printf 'C:len=%d e0=[%s] e1=[%s]\\n' \"${#C[@]}\" \"${C[0]}\" \"${C[1]}\"; \
         F=($@); printf 'F:len=%d e0=[%s] e1=[%s]\\n' \"${#F[@]}\" \"${F[0]}\" \"${F[1]}\"; \
         G=('${@}' x); printf 'G:len=%d e0=[%s] e1=[%s]\\n' \"${#G[@]}\" \"${G[0]}\" \"${G[1]}\"; \
         H=('a\\\\b'); printf 'H=[%s]\\n' \"${H[0]}\"",
    );
    assert_clean(&stderr);
    assert_eq!(
        stdout,
        "C:len=2 e0=[a b] e1=[c]\n\
         F:len=3 e0=[a] e1=[b]\n\
         G:len=2 e0=[${@}] e1=[x]\n\
         H=[a\\\\b]\n"
    );
}
