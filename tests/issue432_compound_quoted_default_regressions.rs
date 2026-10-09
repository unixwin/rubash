//! rubash#432: a quoted whole-word default-value expansion inside a
//! compound array assignment must keep its product as ONE element.
//! `a=("${v:=x y}")` stored elements `x` + `y` (count 2) because the
//! `${name:=word}` / `${name-word}` family fell through the whole-word
//! parameter fast path to the embedded walker, whose re-quoted splitter
//! treated the default's space as live IFS whitespace. GNU treats the
//! wholly double-quoted word as W_QUOTED (subst.c:12050): the default
//! word is the expansion result and never field-splits — ONE element
//! `x y`, with the `:=` side effect assigning `v` the same string.
//! Unquoted tokens keep GNU's field split (`a=(${v:=x y})` -> 2
//! elements); non-compound contexts are unaffected.
//!
//! Expectations follow the GNU Bash 5.3.0 semantics recorded in the
//! issue body (wt96/sweepfix family, adjacent to the wt66 fix).

use std::process::Command;

/// Run a script FILE in its own scratch directory and return
/// (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i432-{}-{}",
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

fn assert_script(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, expected_stdout, "stdout mismatch");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0), "exit code mismatch");
}

/// The issue's minimal form: `:=` default with a space, quoted element.
/// ONE element `x y`; the assignment side effect gives `v` the default.
#[test]
fn issue_repro_quoted_colon_equals_space_default_is_one_element() {
    assert_script(
        concat!(
            "v=\"\"\n",
            "a=(\"${v:=x y}\")\n",
            "echo \"count=${#a[@]}\"\n",
            "echo \"elem0=${a[0]}\"\n",
            "echo \"v=$v\"\n",
        ),
        "count=1\nelem0=x y\nv=x y\n",
    );
}

/// No-space default keeps one element.
#[test]
fn quoted_colon_equals_compact_default() {
    assert_script(
        "unset v\na=(\"${v:=xy}\")\necho \"count=${#a[@]} ${a[0]} $v\"\n",
        "count=1 xy xy\n",
    );
}

/// The `:-` sibling: default used for an unset variable, `v` stays unset.
#[test]
fn quoted_colon_dash_space_default_is_one_element() {
    assert_script(
        concat!(
            "unset v\n",
            "a=(\"${v:-p q}\")\n",
            "echo \"count=${#a[@]} ${a[0]}\"",
            "\necho \"set=${v+yes}\"\n",
        ),
        "count=1 p q\nset=\n",
    );
}

/// The bare `=` sibling assigns and keeps one glued element.
#[test]
fn quoted_bare_equals_space_default_is_one_element() {
    assert_script(
        concat!(
            "unset v\n",
            "a=(\"${v=de fault}\")\n",
            "echo \"count=${#a[@]} ${a[0]} $v\"\n",
        ),
        "count=1 de fault de fault\n",
    );
}

/// A SET variable keeps its own value (the default is not used) and the
/// quoted element still never splits.
#[test]
fn quoted_default_with_set_variable_keeps_value() {
    assert_script(
        concat!(
            "v=set\n",
            "a=(\"${v:-x y}\")\n",
            "b=(\"${v=de fault}\")\n",
            "echo \"${#a[@]}:${a[0]} ${#b[@]}:${b[0]} v=$v\"\n",
        ),
        "1:set 1:set v=set\n",
    );
}

/// The `:?` sibling with a set variable uses the variable's value.
#[test]
fn quoted_colon_question_set_variable() {
    assert_script(
        "v=ok\na=(\"${v:?err}\")\necho \"count=${#a[@]} ${a[0]}\"\n",
        "count=1 ok\n",
    );
}

/// The `:?` sibling with an UNSET variable errors (message on stderr,
/// rc 1) instead of storing the alternate.
#[test]
fn quoted_colon_question_unset_variable_errors() {
    let (stdout, stderr, code) = rubash_file("unset v\na=(\"${v:?err msg}\")\n");
    assert_eq!(code, Some(1), "exit code mismatch");
    assert!(stderr.contains("err msg"), "stderr: {stderr}");
    assert_eq!(stdout, "");
}

/// Unquoted token: GNU field-splits the default (2 elements), and the
/// `:=` side effect still assigns the full default.
#[test]
fn unquoted_colon_equals_space_default_field_splits() {
    assert_script(
        concat!(
            "unset v\n",
            "a=(${v:=x y})\n",
            "echo \"count=${#a[@]} ${a[0]}|${a[1]}\"\n",
            "echo \"v=$v\"\n",
        ),
        "count=2 x|y\nv=x y\n",
    );
}

/// declare's compound RHS takes the same path.
#[test]
fn declare_compound_quoted_default_is_one_element() {
    assert_script(
        concat!(
            "unset v\n",
            "declare -a a=(\"${v:=x y}\")\n",
            "echo \"count=${#a[@]} ${a[0]} $v\"\n",
        ),
        "count=1 x y x y\n",
    );
}

/// Subscript element form: W_NOSPLIT stores the whole default at the key.
#[test]
fn subscript_element_quoted_default_is_whole_value() {
    assert_script(
        concat!("unset v\n", "A[3]=\"${v:=x y}\"\n", "echo \"${A[3]}|$v\"\n",),
        "x y|x y\n",
    );
}

/// Scalar assignment RHS (non-compound context) already glued: guard.
#[test]
fn scalar_assignment_quoted_default_stays_glued() {
    assert_script(
        "unset v\nw=\"${v:=a b}\"\necho \"w=$w v=$v\"\n",
        "w=a b v=a b\n",
    );
}

/// The default word respects IFS: a colon default under `IFS=:` still
/// stores ONE element (quoted token never splits on any IFS character).
#[test]
fn quoted_default_ignores_ifs_character() {
    assert_script(
        concat!(
            "unset v\n",
            "IFS=:\n",
            "a=(\"${v:=x:y}\")\n",
            "echo \"count=${#a[@]} ${a[0]} $v\"\n",
        ),
        "count=1 x:y x:y\n",
    );
}

/// `+`/`:+` alternates keep their dedicated fanout path (rubash#315).
#[test]
fn plus_alternate_guard_idioms_stay_green() {
    assert_script(
        concat!(
            "b=(\"\" x)\n",
            "c=( ${b[@]+\"${b[@]:0:2}\"} )\n",
            "echo \"count=${#c[@]} [${c[0]}][${c[1]}]\"\n",
            "h=( ${b[@]+\"\"} )\n",
            "echo \"hcount=${#h[@]}\"\n",
        ),
        "count=2 [][x]\nhcount=1\n",
    );
}
