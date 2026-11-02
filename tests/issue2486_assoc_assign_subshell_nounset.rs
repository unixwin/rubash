//! rubash#2486 (face B) — under `set -u`, an associative-array element
//! assignment inside a BACKGROUND subshell falsely reported the subscript
//! as an unbound variable (`building[$1]=1` with `$1`=app ->
//! `app: unbound variable`), while the same call at top level and in a
//! plain `( ... )` subshell worked.
//!
//! GNU anchor: the async child is a FORK (execute_cmd.c:1761-1763
//! execute_in_subshell), so it inherits the parent's variable attributes
//! whole. For an associative array the subscript of `name[sub]=value` is
//! WORD-expanded (arrayfunc.c:392-408 assign_array_element_internal ->
//! expand_subscript_string at arrayfunc.c:396, subst.c:11063 — plain word
//! expansion, then dequote) and used literally as the key
//! (arrayfunc.c:408 bind_assoc_variable); the expanded key is never
//! re-interpreted as a variable reference, so no unbound check of it ever
//! runs. Only INDEXED arrays evaluate the subscript arithmetically
//! (arrayfunc.c:425 array_expand_index -> expand_arith_string + evalexp),
//! where a bare unset name under `set -u` IS fatal (expr.c) — the
//! contrast case pinned below matches GNU.
//!
//! Root cause on the rubash side: the background `( ... ) &` body is a
//! respawned `rubash -c` child, and its env transport iterated the
//! write-behind VarTable directly, so the `__RUBASH_*_VARS` attribute
//! markers were never materialized: `declare -A building=()` crossed as a
//! plain string, the child treated `building` as an INDEXED array, the
//! subscript took the arithmetic path, and `app` raised nounset.

use std::path::PathBuf;
use std::process::Command;

fn rubash_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2486b-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("probe.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("probe.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The parallel-worker shape from the issue: function assigning
/// `building[$1]=1`, called at top level, in a plain subshell, and in a
/// background subshell — all must succeed under `set -u` with byte-equal
/// output and empty stderr (GNU 5.3.20 script-file oracle).
#[test]
fn issue2486_setu_assoc_element_assign_all_positions() {
    let script = concat!(
        "set -u\n",
        "declare -A building=()\n",
        "b1() { building[$1]=1; echo \"set $1 ok\"; }\n",
        "b1 app\n",
        "( b1 app > log1.txt 2>&1 )\n",
        "( b1 app > log2.txt 2>&1 ) &\n",
        "wait\n",
        "cat log1.txt log2.txt\n",
        "echo main-ok\n",
    );
    let (stdout, stderr, code) = rubash_file("worker", script);
    assert_eq!(
        stdout,
        concat!("set app ok\n", "set app ok\n", "set app ok\n", "main-ok\n",),
        "stdout: {stdout}"
    );
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

/// The literal-subscript form in a background subshell: `building[app]=1`
/// has no `$` at all, yet the pre-fix child lost the assoc attribute and
/// reported `app: unbound variable`. Must succeed, and the assignment must
/// land (`${building[app]}` reads back `1` inside the same child).
#[test]
fn issue2486_setu_bg_subshell_literal_assoc_subscript() {
    let script = concat!(
        "set -u\n",
        "declare -A building=()\n",
        "( building[app]=1; echo \"v=${building[app]}\" ) > log.txt 2>&1 &\n",
        "wait\n",
        "cat log.txt\n",
        "echo main-ok\n",
    );
    let (stdout, stderr, code) = rubash_file("literal", script);
    assert_eq!(stdout, "v=1\nmain-ok\n", "stdout: {stdout}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

/// The background child must see the array's ASSOCIATIVE attribute itself:
/// `declare -p building` inside `( ... ) &` reports `-A` with the parent's
/// elements (GNU fork inheritance; execute_cmd.c:1761-1763). The respawned
/// child still stamps `-x` on transported shell vars (every env pair is an
/// export from the child's viewpoint — a pre-existing transport artifact
/// for ALL unexported vars, out of #2486's scope), so the pin asserts the
/// assoc attribute and the element, not the export column.
#[test]
fn issue2486_bg_child_declares_assoc_attribute() {
    let script = concat!(
        "declare -A building=([k]=v)\n",
        "( declare -p building ) > log.txt 2>&1 &\n",
        "wait\n",
        "cat log.txt\n",
    );
    let (stdout, stderr, code) = rubash_file("declarep", script);
    assert!(
        stdout.contains("declare -A")
            && stdout.contains("building=([k]=")
            && stdout.contains("\"v\""),
        "stdout: {stdout}"
    );
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

/// `set +u` control: the same background assignment works with nounset
/// off in every rubash build (the pre-fix bug was only AMPLIFIED by
/// `set -u`, but the attribute loss also mis-stored keys there).
#[test]
fn issue2486_plusu_bg_subshell_assoc_assign() {
    let script = concat!(
        "set +u\n",
        "declare -A building=()\n",
        "b1() { building[$1]=1; echo \"set $1 ok\"; }\n",
        "( b1 app > log.txt 2>&1 ) &\n",
        "wait\n",
        "cat log.txt\n",
        "echo main-ok\n",
    );
    let (stdout, stderr, code) = rubash_file("plusu", script);
    assert_eq!(stdout, "set app ok\nmain-ok\n", "stdout: {stdout}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

/// Contrast (GNU-matching, must keep firing): for an INDEXED array the
/// subscript IS arithmetic (arrayfunc.c:425), so `idx[$1]` with a NON-
/// numeric `$1` string under `set -u` reports the bare name unbound —
/// exactly like GNU (WSL 5.3.20 prints `five: unbound variable` for
/// `i2 five` there). The fix must not silence the indexed family.
#[test]
fn issue2486_setu_indexed_subscript_arithmetic_still_fires() {
    let script = concat!(
        "set -u\n",
        "declare -a idx=()\n",
        "i2() { idx[$1]=7; echo \"idx $1 ok\"; }\n",
        "( i2 five > log.txt 2>&1 )\n",
        "cat log.txt\n",
        "echo main-ok\n",
    );
    let (stdout, stderr, code) = rubash_file("indexed", script);
    assert_eq!(stdout, "main-ok\n", "stdout: {stdout}");
    assert!(
        stderr.contains("five: unbound variable"),
        "stderr: {stderr}"
    );
    let _ = code;
}
