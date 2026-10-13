//! wt56-bash-completion-syntax, layer 3 — the `${` body scan inside the `$[`
//! EOF route must end at the FIRST unquoted `}` (P_FIRSTCLOSE).
//!
//! Root cause: `db_scan_dolbrace_body` (src/lexer/skip.rs) counted a bare ``
//! as `${` nesting depth. GNU parse.y's read_token_word dolbrace arm (the
//! P_FIRSTCLOSE|P_DOLBRACE parse_matched_pair family, parse.y:5506-5527 with
//! the parse.y:3877 matched-pair recursion) ends the expansion at the first
//! unquoted `}`; a `{` in the pattern/operator text is WORD DATA — only a
//! nested `${` recurses. Byte anchors, WSL GNU bash 5.3.0 (2026-10-13):
//!
//! - `z='a{}c'; echo ${z%{}}` prints `a{}c}` — the pattern `{` did not nest,
//!   so the expansion ended at the first `}` and the second survived as
//!   literal;
//! - `z='x{a{bQ'; echo ${z%{a{b}}` prints `x{a{bQ}` likewise;
//! - `x=abc; echo ${x%[<{().[]*} 'A}' B` prints `abc A} B` — the expansion
//!   closed inside `[<{().[]`'s `}` and the quoted `}` stayed a separate
//!   word.
//!
//! With the bare-`{` depth count, scop/bash-completion's
//! `option2=${option2%%[<{().[]*}` (bash_completion:1755, scop@43d0dc5, the
//! `_comp_compgen_help__parse` helper) phantom-opened a nest: the `$[`
//! unclosed-route scan (issue rubash#543 family) walked past the real `}`
//! through the rest of the library and a much later quote reported
//! `unexpected EOF while looking for matching `'' attributed to line 1755 —
//! plus a bogus `unexpected end of file from `if' command on line 1753`
//! prefix re-run. The library's `source` died rc=2 where GNU bash 5.3.0
//! sources the whole file clean (rc=0).
//!
//! The minimal composite below embeds the two smallest upstream spans that
//! reproduce the shape (scop/bash-completion@43d0dc5, bash_completion:1753-1761
//! and bash_completion:2117-2136 — GPL, minimal line set per the wt56
//! discipline, cited by source line).

use std::io::Write;
use std::process::{Command, Stdio};

/// Run a SCRIPT FILE and return (stdout+stderr text, exit code).
fn rubash_file(script: &str) -> (String, Option<i32>) {
    let path = std::env::temp_dir().join(format!(
        "rubash_wt56_{}_{}.sh",
        std::process::id(),
        script.len()
    ));
    std::fs::write(&path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (text, output.status.code())
}

/// Feed a script through `rubash -s` stdin (the journey's stdin shape).
fn rubash_stdin(script: &str) -> (String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -s");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write script");
    let output = child.wait_with_output().expect("wait");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (text, output.status.code())
}

/// scop/bash-completion@43d0dc5 bash_completion:1753-1761 verbatim (the
/// `_comp_compgen_help__parse` body tail: the `[<{().[]` pattern and the
/// `[[ =~ ^([^=<{().[]...` continuation pair).
const HELP_PARSE_TAIL: &str = "\
    if [[ $option =~ (\\[((no|dont)-?)\\]). ]]; then
        option2=${option/\"${BASH_REMATCH[1]}\"/}
        option2=${option2%%[<{().[]*}
        options+=(\"${option2/=*/=}\")
        option=${option/\"${BASH_REMATCH[1]}\"/\"${BASH_REMATCH[2]}\"}
    fi

    [[ $option =~ ^([^=<{().[]|\\.[A-Za-z0-9])+=? ]] &&
        options+=(\"$BASH_REMATCH\")
";

/// scop/bash-completion@43d0dc5 bash_completion:2117-2136 verbatim (the
/// `_comp_compgen_pnames` else-branch scan loop the phantom walk used to die
/// in). Closed form: a complete `for ... done` with balanced quotes.
const PNAMES_LOOP: &str = "            local line i=-1\n            for line in \"${psout[@]}\"; do\n                if ((i == -1)); then\n                    # First line, see if it has COMMAND column header. For\n                    # example some busybox ps versions do that, i.e. don't\n                    # respect command=\n                    if [[ $line =~ ^(.*[[:space:]])COMMAND([[:space:]]|$) ]]; then\n                        # It does; store its index.\n                        i=${#BASH_REMATCH[1]}\n                    else\n                        # Nope, fall through to \"regular axo command=\" parsing.\n                        break\n                    fi\n                else\n                    #\n                    line=${line:i}   # take command starting from found index\n                    line=${line%% *} # trim arguments\n                    [[ $line ]] && procs+=(\"$line\")\n                fi\n            done\n";

/// The minimal journey shape: the pattern-brace line inside a function plus a
/// downstream `$(...)`/quote loop, extglob live — GNU sources it rc=0.
fn minimal_composite() -> String {
    format!(
        "shopt -s extglob\nf()\n{{\n{}}}\ng()\n{{\n{}}}\necho MARK_F\ng\necho MARK_DONE\n",
        HELP_PARSE_TAIL, PNAMES_LOOP
    )
}

/// The whole composite sources clean like GNU (rc 0, markers, no diagnostic).
#[test]
fn composite_sources_clean_like_gnu() {
    let composite = minimal_composite();
    let (text, code) = rubash_file(&composite);
    assert_eq!(code, Some(0), "text={text:?}");
    assert!(text.contains("MARK_F\nMARK_DONE"), "text={text:?}");
    assert!(!text.contains("unexpected"), "text={text:?}");

    let (text, code) = rubash_stdin(&composite);
    assert_eq!(code, Some(0), "text={text:?}");
    assert!(text.contains("MARK_F\nMARK_DONE"), "text={text:?}");
    assert!(!text.contains("unexpected"), "text={text:?}");
}

/// The `$[` EOF route must stay SILENT on pattern-brace data: the verdict is
/// None for the composite and for the bare shapes, while the true positive
/// (`echo $[1+2`, issue rubash#543) still reports.
#[test]
fn dollar_bracket_route_stays_silent_on_pattern_brace_data() {
    let composite = minimal_composite();
    assert_eq!(rubash::lexer::unclosed_dollar_bracket_eof(&composite), None);

    // The 1755 line alone, with a downstream quote after the (closed) span:
    // the route must not report — the real `}` closes the `${`.
    let bare = "f()\n{\n    option2=${option2%%[<{().[]*}\n}\necho 'tail'\n";
    assert_eq!(rubash::lexer::unclosed_dollar_bracket_eof(bare), None);

    // GNU first-`}` anchor shapes from the doc header: not unclosed either.
    assert_eq!(
        rubash::lexer::unclosed_dollar_bracket_eof("z='a{}c'\necho ${z%{}}\n"),
        None
    );
    assert_eq!(
        rubash::lexer::unclosed_dollar_bracket_eof("z='x{a{bQ'\necho ${z%{a{b}}\n"),
        None
    );

    // True positive pinned: an actual unterminated `$[` still reports `]`.
    match rubash::lexer::unclosed_dollar_bracket_eof("echo $[1+2\n") {
        Some(report) => assert_eq!(report.closer, ']'),
        None => panic!("unterminated $[ must still report"),
    }
}

/// The GNU first-close semantics byte matrix (every line WSL bash 5.3.0
/// verified, 2026-10-13): pattern braces never nest, the first unquoted `}`
/// closes, and a `}` after the expansion stays literal.
#[test]
fn first_close_brace_semantics_match_gnu() {
    let (text, code) = rubash_file(
        "\
z='a{}c'
echo T1=[${z%{}}]
z='x{a{bQ'
echo T2=[${z%{a{b}}]
x=abc
echo T3=[${x%[<{().[]*} 'A}' B]
echo T4=[${x%%[<{().[]*}]
echo T5=[${x#'\\'}]
o2=--foo=bar
option2=${o2%%[<{().[]*}
echo T6=[$option2]
",
    );
    assert_eq!(code, Some(0), "text={text:?}");
    for expected in [
        "T1=[a{}c}]",
        "T2=[x{a{bQ}]",
        "T3=[abc A} B]",
        "T4=[abc]",
        "T5=[abc]",
        "T6=[--foo=bar]",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text:?}");
    }
    assert!(!text.contains("unexpected"), "text={text:?}");
}

/// Layer 1/2 guard: the single-quoted backslash family (wt56's original
/// `${_word%'\'}` construct, scop/bash-completion:275-277) still sources and
/// expands like GNU across `[[ == ]]`, `[[ =~ ]]`, case, plain echo, and
/// double-quoted contexts, with extglob on (GNU outputs verified 2026-10-13
/// with the identical probe text).
#[test]
fn single_quote_backslash_family_matrix() {
    let (text, code) = rubash_file(
        "\
shopt -s extglob
x=abc
echo M01=[${x%'\\'}]
echo M02=[${x#'\\'}]
_word=a'b'
if [[ ${_word%'\\'} =~ ^a.b$ ]]; then echo M03=HIT; fi
if [[ ${x%'\\'} == abc ]]; then echo M04=HIT; fi
echo M05=[${x%'\\'}-tail]
echo M06=[\"${x%'\\'}\"]
case ${x%'\\'} in abc) echo M07=HIT;; esac
[[ ${x%'\\'} == abc ]] && echo M08=HIT
line=-x
echo M09=[${line##@(*/|-)}]
",
    );
    assert_eq!(code, Some(0), "text={text:?}");
    for expected in [
        "M01=[abc]",
        "M02=[abc]",
        "M04=HIT",
        "M05=[abc-tail]",
        "M06=[abc]",
        "M07=HIT",
        "M08=HIT",
        "M09=[x]",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text:?}");
    }
    assert!(!text.contains("unexpected"), "text={text:?}");
    assert!(!text.contains("looking for matching"), "text={text:?}");
}

/// ANSI-C `$'...'` words inside `${...}` pattern slots keep backslash escapes
/// live (parse.y:3992 LEX_PASSNEXT via P_ALLOWESC) — the plain-`'...` rule
/// (no escapes) must not swallow them. Byte matrix against WSL GNU bash
/// 5.3.0 (2026-10-13), the scop/bash-completion nquote shapes (bash tests
/// nquote2.sub:14-27): `v="'"; t "${v/$'\''/$'\''}"` prints `'  |  '` and
/// `${v/$'\''/x}` prints `x  |  x` on every quoting spelling.
#[test]
fn ansi_c_pattern_words_keep_escapes_like_gnu() {
    let (text, code) = rubash_file(
        "\
v=\"'\"
t() {
    printf '%s  |  %s\\n' \"$1\" \"$2\"
}
t \"${v/$'\\''/$'\\''}\"        \"'\"
t  ${v/$'\\''/$'\\''}         \"'\"
t \"${v/$'\\''/x}\"            \"x\"
t  ${v/$'\\''/x}             \"x\"
t \"${v/x/$'\\''}\"            \"'\"
t  ${v/x/$'\\''}             \"'\"
t \"${v/x/$'\\x5c\\''}\"        \"'\"
t  ${v/x/$'\\x5c\\''}         \"'\"
t \"${v/\\'/\\'}\"              \"'\"
t  ${v/\\'/\\'}               \"'\"
",
    );
    assert_eq!(code, Some(0), "text={text:?}");
    let rows: Vec<&str> = text.lines().filter(|l| l.contains("  |  ")).collect();
    assert_eq!(
        rows,
        vec![
            "'  |  '", "'  |  '", "x  |  x", "x  |  x", "'  |  '", "'  |  '", "'  |  '", "'  |  '",
            "'  |  '", "'  |  '",
        ],
        "text={text:?}"
    );
    assert!(!text.contains("unexpected"), "text={text:?}");
    assert!(!text.contains("looking for matching"), "text={text:?}");
}
