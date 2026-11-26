//! Issue rubash#311 regressions (wt13/sweepfix lane).
//!
//! `set -u` (nounset) falsely reported `unbound variable` for expansions
//! nested in the PATTERN of a `${var#pattern}` / `${var%pattern}` /
//! `${var//pat/repl}` expansion — bash-completion `_comp__reassemble_words`
//! (`line=${line#*"${COMP_WORDS[i]}"}`) and liquidprompt
//! (`${display_path//[!\/]}`) both died on it.
//!
//! GNU semantics (vendored third_party/bash, subst.c):
//! - `subst.c:9807 parameter_brace_expand` extracts the parameter name with
//!   `string_extract (string, &t_index, "#%^,:-=?+/@}", SX_VARNAME)`
//!   (`subst.c:791`): the reference ends at the FIRST unescaped operator
//!   character, and a well-formed `[...]` subscript group is skipped as
//!   part of the reference (`subst.c:812-818`, `skipsubscript` + the
//!   `string[ni] == RBRACK` check). Brackets after the operator are
//!   pattern text, never a subscript: `${line#[[:space:]]}` references
//!   `line`; `${a[5]#z}` references element `a[5]`.
//! - The unbound check (`subst.c:10170-10180`) applies to the substring /
//!   patsub / casemod / attribute (`@`) / `#` / `%` / bare forms only, and
//!   `err_unboundvar(name)` carries the bare pre-operator reference
//!   (`${UNSET#pat}` reports `UNSET`, `${a[5]#z}` reports `a[5]`,
//!   `${1#pat}` reports `1`).
//!
//! Root cause fixed in `nounset_braced_parameter_is_unbound`
//! (src/executor/parameter_errors.rs): the checker ran
//! `parse_array_subscript` over the WHOLE `${...}` body including the
//! operator tail, so a bracket class in the pattern matched as a
//! subscript (`line#[[:space:]]` → base `line#`, sub `[:space:]`), and
//! the whole body was echoed as the diagnostic name. It also bailed on
//! ANY `-`/`=`/`+`/`@` anywhere in the body, so those characters inside
//! patterns suppressed real unbound reports. The checker now splits the
//! pre-operator reference first (`parameter_reference_len`, mirroring
//! string_extract) and applies the default/assign/alternate and
//! all-elements bails to the OPERATOR, not the pattern text.
//!
//! Every expectation is byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-29; matrices under
//! target/sweepfix/ and the issue's target/issue-suites/results/ecosweep2/).

use std::process::Command;

/// Run a script FILE in its own scratch directory (relative name `case.sh`
/// keeps the diagnostic prefix stable) and return (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i311-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The original report: a nested `${A[i]}` inside the pattern of
/// `${r#pattern}` under `set -u` with everything set must expand, not
/// error. GNU prints `[abc]`, rc=0.
#[test]
fn nounset_nested_array_element_in_pattern_expands() {
    let (stdout, stderr, code) =
        rubash_file("set -u\nA=(x0 x1)\ni=0\nr=abc\nr=${r#*\"${A[i]}\"}\necho \"[$r]\"\n");
    assert_eq!(stdout, "[abc]\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// The shape matrix from the issue (ecosweep2 lane): bracket classes,
/// negated classes and quoted nested expansions in `#`/`%`/`//` patterns
/// with all variables set. GNU prints the eleven lines below, rc=0,
/// empty stderr.
#[test]
fn nounset_pattern_shape_matrix_matches_gnu() {
    let script = concat!(
        "set -u\n",
        "line=abc; p2=/x; s2=a,c\n",
        "A=(x0 x1); i=0\n",
        "echo \"t1: ${line#[[:space:]]}\"\n",
        "echo \"t2: ${line%[[:space:]]}\"\n",
        "echo \"t3: ${p2#[!\\/]}\"\n",
        "echo \"t4: ${s2//[[:alpha:]]}\"\n",
        "echo \"t5: ${s2//[a,]}\"\n",
        "echo \"t6: ${p2//[!\\/]}\"\n",
        "echo \"t7: ${s2#*\"${A[i]}\"}\"\n",
        "echo \"t8: ${line#x}\"\n",
        "echo \"t9: ${line%%[[:space:]]*}\"\n",
        "echo \"t10: ${s2//,}\"; echo \"${s2//,/;}\"; echo \"${s2/[a]/X}\"\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    let expected = concat!(
        "t1: abc\n",
        "t2: abc\n",
        "t3: /x\n",
        "t4: ,\n",
        "t5: c\n",
        "t6: /\n",
        "t7: a,c\n",
        "t8: abc\n",
        "t9: abc\n",
        "t10: ac\na;c\nX,c\n",
    );
    assert_eq!(stdout, expected);
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// True positives keep firing, with GNU's NAME (the bare pre-operator
/// reference, err_unboundvar(name) at subst.c:10176) — never the pattern
/// text: `${UNSET#pat}` reports `UNSET` (was `UNSET#pat`), `${a[5]#z}`
/// reports `a[5]` (was silently missing), `${1#pat}` reports `1`.
#[test]
fn nounset_pattern_operator_reports_bare_reference_name() {
    let cases: &[(&str, &str, &str)] = &[
        // (function body, expected stderr tail, expected rc)
        ("echo \"${UNSET#pat}\"", "UNSET: unbound variable", "1"),
        ("echo \"${UNSET%pat}\"", "UNSET: unbound variable", "1"),
        ("echo \"${UNSET:1}\"", "UNSET: unbound variable", "1"),
        ("echo \"${UNSET/pat/rep}\"", "UNSET: unbound variable", "1"),
        ("echo \"${UNSET^}\"", "UNSET: unbound variable", "1"),
        ("echo \"${UNSET#[a-]}\"", "UNSET: unbound variable", "1"),
        ("a=(x0); echo \"${a[5]#z}\"", "a[5]: unbound variable", "1"),
        ("echo \"${1#pat}\"", "1: unbound variable", "1"),
    ];
    for (body, message, code) in cases {
        let script = format!("set -u\n{body}\n");
        let (stdout, stderr, rc) = rubash_file(&script);
        assert!(
            stdout.is_empty(),
            "case {body:?} stdout not empty: {stdout}"
        );
        assert!(
            stderr.trim_end().ends_with(message),
            "case {body:?} stderr {stderr:?} lacks {message:?}"
        );
        assert_eq!(
            rc.map(|c| c.to_string()).as_deref(),
            Some(*code),
            "case {body:?}"
        );
    }
}

/// Guard forms stay never-unbound: default/assign/alternate operators
/// (subst.c:10170 checks only substring/patsub/casemod/@/#/%/bare) and
/// `[@]`/`[*]` all-element references behind operators.
#[test]
fn nounset_default_and_all_element_guards_stay_quiet() {
    let script = concat!(
        "set -u\n",
        "a=(x0)\n",
        "echo \"[${UNSET-x}]\"\n",
        "echo \"[${UNSET:-d}]\"\n",
        "echo \"[${a[5]-d}]\"\n",
        "echo \"[${UNSET:=z}]\"\n",
        "echo \"[${UNSET:+y}]\"\n",
        "echo \"[${a[@]#?}]\"\n",
        "echo \"[${a[*]//x/}\"\n",
        "declare -A h=([k]=v); echo \"[${h[k]#v}]\"\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "[x]\n[d]\n[d]\n[z]\n[y]\n[0]\n[0\n[]\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// Subscript evaluation still reports the unset subscript NAME first
/// (nameref25.sub ok 4 semantics; the subscript is evaluated before the
/// operator, GNU arrayfunc.c + subst.c parameter_brace_expand_word).
#[test]
fn nounset_pattern_reports_unset_subscript_name_first() {
    for body in ["echo \"${A[i]#z}\"", "x=(v); echo \"${x[abc]#z}\""] {
        let script = format!("set -u\nA=(x0)\n{body}\n");
        let (stdout, stderr, rc) = rubash_file(&script);
        assert!(stdout.is_empty());
        assert_eq!(
            stderr.trim_end(),
            match body {
                "echo \"${A[i]#z}\"" => "case.sh: line 3: i: unbound variable",
                _ => "case.sh: line 3: abc: unbound variable",
            }
        );
        assert_eq!(rc, Some(1));
    }
}

/// Attribute/transform operators report for an unset base
/// (subst.c:10170 includes `c == '@'`), while `[@]`-referenced transform
/// forms on empty arrays stay quiet (all-element guard). Each reporting
/// probe is fatal under nounset, so it runs in its own script file.
#[test]
fn nounset_transform_operator_reports_unset_base() {
    for body in [
        "echo \"${UNSET@Q}\"",
        "echo \"${UNSET@[Q]}\"",
        "echo \"${x@}\"",
    ] {
        let (stdout, stderr, rc) = rubash_file(&format!("set -u\n{body}\n"));
        assert!(stdout.is_empty(), "case {body:?} stdout: {stdout}");
        let expected = match body {
            "echo \"${UNSET@Q}\"" | "echo \"${UNSET@[Q]}\"" => "UNSET: unbound variable",
            _ => "x: unbound variable",
        };
        assert!(
            stderr.trim_end().ends_with(expected),
            "case {body:?} stderr {stderr:?} lacks {expected:?}"
        );
        assert_eq!(rc, Some(1), "case {body:?}");
    }
    // All-element transform forms never report, even on empty arrays.
    let (stdout, stderr, rc) = rubash_file(concat!(
        "set -u\n",
        "empty=()\n",
        "set_arr=(e0)\n",
        "echo \"[${empty[@]@Q}]\"\n",
        "echo \"[${set_arr[@]@Q}]\"\n",
    ));
    assert_eq!(stdout, "[]\n['e0']\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
}

/// The bash-completion `_comp__reassemble_words` shape: a `${COMP_WORDS[i]}`
/// slice inside the pattern of `line=${line#*"${COMP_WORDS[i]}"}` under
/// `set -u` in a function (issue blast radius: `_comp_initialize -n :`).
#[test]
fn nounset_completion_reassemble_words_shape_expands() {
    let script = concat!(
        "set -u\n",
        "COMP_WORDS=(ssh-keygen -)\n",
        "i=0\n",
        "_reassemble() {\n",
        "  local line=\"ssh-keygen -\"\n",
        "  line=${line#*\"${COMP_WORDS[i]}\"}\n",
        "  echo \"[$line]\"\n",
        "}\n",
        "_reassemble\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "[ -]\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// Set-variable slices and case-modification behind operators with
/// everything bound (value paths that must stay untouched).
#[test]
fn nounset_set_variable_operator_forms_expand() {
    let script = concat!(
        "set -u\n",
        "s=hello\n",
        "echo \"${s:1:3}\"\n",
        "echo \"${s//l/L}\"\n",
        "echo \"${s^}\"\n",
        "echo \"${s:0:2}${s:2}\"\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, "ell\nheLLo\nHello\nhello\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// A scalar whose VALUE text looks like array storage stays bound under
/// nounset (GNU variables.c find_variable routes by cell type —
/// array_p/assoc_p — never by value text). liquidprompt
/// tools/config-from-doc.sh assigns `avalue` the literal `()` via
/// `IFS=' ' read -r v avalue`; the old text heuristic misrouted it as an
/// empty array and killed the sourced script at its printf
/// (`avalue: unbound variable`, test_tools rc=2). An EMPTY ARRAY cell
/// keeps the unbound report (new-exp15 `-uc`).
#[test]
fn nounset_scalar_with_paren_text_stays_bound() {
    let script = concat!(
        "set -u\n",
        "x='()'\n",
        "echo \"A[$x]\"\n",
        "e='(  )'\n",
        "echo \"E[$e]\"\n",
        "f='(\"\")'\n",
        "echo \"F[$f]\"\n",
        "y='(a b)'\n",
        "echo \"B[$y]\"\n",
        "printf 'v ()\n' | { IFS=' ' read -r v avalue; echo \"R[$avalue]\"; }\n",
        "arr=(e0)\n",
        "echo \"D[${arr[0]}]\"\n",
    );
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        stdout,
        "A[()]\nE[(  )]\nF[(\"\")]\nB[(a b)]\nR[()]\nD[e0]\n"
    );
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));

    // The true positive stays: scalar-form `${e}` of an empty array cell
    // reports unbound (GNU new-exp15).
    let (stdout, stderr, rc) = rubash_file("set -u\ne=()\necho \"${e}\"\n");
    assert!(stdout.is_empty());
    assert_eq!(stderr.trim_end(), "case.sh: line 3: e: unbound variable");
    assert_eq!(rc, Some(1));
}
