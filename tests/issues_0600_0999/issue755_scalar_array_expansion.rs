//! rubash#755: a SCALAR whose text happens to look like a parenthesised list
//! must not be re-parsed as an array by `"${x[@]}"` / `${x[*]}` / `${x[i]}`.
//!
//! GNU decides array-ness from the variable's CELL TYPE (subst.c
//! get_var_and_type: VT_VARIABLE for a scalar, VT_ARRAYVAR/VT_ASSOCVAR for an
//! array), never from the value's text. rubash stored every variable as a
//! string and `is_array_storage()` accepted any `(...)` body, so
//! `x='(a b c)'` expanded to three elements and `"${x[@]}"` — the documented
//! "scalar or array" idiom — silently changed the word count.
//!
//! Every expected byte below was measured against WSL GNU Bash 5.3.0
//! (script-file oracle, separated stdout/stderr, `cd` to the probe dir).

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

struct Run {
    stdout: String,
    stderr: String,
    status: Option<i32>,
}

/// Run one LF script named `probe.sh` in a private scratch dir (so nothing it
/// writes can land in the repo root) and capture the streams separately.
fn run(script: &str) -> Run {
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash755-{}-{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    std::fs::write(dir.join("probe.sh"), script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code(),
    }
}

/// The headline: one element, count 1, and `set --` sees one positional.
#[test]
fn quoted_at_list_on_scalar_is_one_whole_element() {
    let out = run(concat!(
        "x='(a b c)'\n",
        "printf '[%s] len=%s\\n' \"${x[@]}\" \"${#x[@]}\"\n",
        "set -- \"${x[@]}\"\n",
        "echo \"argc=$#\"\n",
    ));
    assert_eq!(out.stdout, "[(a b c)] len=1\nargc=1\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, Some(0));
}

/// Element 0 is the whole string, any other index is absent, `[*]` is the one
/// element, and `${#x}` is the whole-string length (7), not element 0's (1).
#[test]
fn scalar_subscript_reads_are_single_element() {
    let out = run(concat!(
        "x='(a b c)'\n",
        "printf 'z[%s]\\n' \"${x[0]}\"\n",
        "printf 'x[%s]\\n' \"${x[1]}\"\n",
        "printf 's[%s]\\n' \"${x[*]}\"\n",
        "printf 'l[%s]\\n' \"${#x}\"\n",
    ));
    assert_eq!(out.stdout, "z[(a b c)]\nx[]\ns[(a b c)]\nl[7]\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, Some(0));
}

/// Slicing a scalar `[@]` is substring semantics on the string, and `${!x[@]}`
/// names the single index 0 — never a fanned-out element list.
#[test]
fn scalar_slice_and_keys_follow_scalar_semantics() {
    let out = run(concat!(
        "x='(a b c)'\n",
        "printf 'sl[%s]\\n' \"${x[@]:1}\"\n",
        "printf 'k[%s]\\n' \"${!x[@]}\"\n",
    ));
    assert_eq!(out.stdout, "sl[a b c)]\nk[0]\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, Some(0));
}

/// Unquoted `[@]`/`[*]` return the same one-element value; GNU then field
/// splits that single word on the default IFS, exactly as here.
#[test]
fn unquoted_scalar_list_field_splits_the_single_value() {
    let out = run(concat!(
        "x='(a b c)'\n",
        "printf 'u1<%s>\\n' ${x[@]}\n",
        "set -- ${x[@]}\n",
        "printf 'u3=%s\\n' \"$#\"\n",
    ));
    assert_eq!(out.stdout, "u1<(a>\nu1<b>\nu1<c)>\nu3=3\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, Some(0));
}

/// Guard: a genuine array still expands as an array — count, per-element
/// fields, and spaced elements intact.
#[test]
fn genuine_array_still_expands_as_array() {
    let out = run(concat!(
        "arr=('p' 'q r' 's')\n",
        "printf 'n=%s\\n' \"${#arr[@]}\"\n",
        "printf '<%s>\\n' \"${arr[@]}\"\n",
        "printf 'e[%s]\\n' \"${arr[0]}\"\n",
    ));
    assert_eq!(out.stdout, "n=3\n<p>\n<q r>\n<s>\ne[p]\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, Some(0));
}

/// Guard: a real array whose ELEMENT text looks like a parenthesised list is
/// still two elements — the type comes from the cell, not the text.
#[test]
fn genuine_array_with_parenthesised_elements_stays_two_elements() {
    let out = run(concat!(
        "arr=('(a b c)' 'z')\n",
        "printf 'n=%s\\n' \"${#arr[@]}\"\n",
        "printf '<%s>\\n' \"${arr[@]}\"\n",
        "printf 'sl[%s]\\n' \"${arr[@]:1}\"\n",
    ));
    assert_eq!(out.stdout, "n=2\n<(a b c)>\n<z>\nsl[z]\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, Some(0));
}

/// Guard: copying a genuine array into a declared local/global array with the
/// quoted `"${foo[@]}"` idiom still fans out per element — the declared cell
/// carries the prefixed storage, so it must not be mistaken for the
/// rubash#755 scalar shape.
#[test]
fn declared_compound_copy_of_array_still_fans_out() {
    let out = run(concat!(
        "f() {\n",
        "  foo=(a 'b c' d)\n",
        "  local v=(\"${foo[@]}\")\n",
        "  printf '<%s>\\n' \"${v[@]}\"\n",
        "}\n",
        "f\n",
        "declare d=(\"${foo[@]}\")\n",
        "printf 'd<%s>\\n' \"${d[@]}\"\n",
    ));
    assert_eq!(out.stdout, "<a>\n<b c>\n<d>\nd<a>\nd<b c>\nd<d>\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, Some(0));
}

/// Guard: the plain control (`y='plain'`) was already correct and stays so.
#[test]
fn plain_scalar_control_unchanged() {
    let out = run(concat!(
        "y='plain'\n",
        "printf 'control:[%s] len=%s\\n' \"${y[@]}\" \"${#y[@]}\"\n",
    ));
    assert_eq!(out.stdout, "control:[plain] len=1\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, Some(0));
}

/// The `[@]`/`[*]` default-operator and transform forms treat a scalar base
/// as one element too — `${x[@]-d}` must not fan out and `${x[@]@A}` renders
/// the scalar assignment.
#[test]
fn scalar_operator_and_transform_forms_are_single_element() {
    let out = run(concat!(
        "x='(a b c)'\n",
        "printf 'o1[%s]\\n' \"${x[@]-d}\"\n",
        "printf 'o2[%s]\\n' \"${x[@]:-d}\"\n",
        "printf 'o5[%s]\\n' \"${x[*]-d}\"\n",
        "printf 'o6[%s]\\n' \"${x[*]:-d}\"\n",
        "printf 'o12[%s]\\n' \"${x[@]@A}\"\n",
        "printf 'o13[%s]\\n' \"${x[@]@a}\"\n",
        "printf 'o17[%s]\\n' \"${x[@]:0:2}\"\n",
    ));
    assert_eq!(
        out.stdout,
        "o1[(a b c)]\no2[(a b c)]\no5[(a b c)]\no6[(a b c)]\no12[x='(a b c)']\no13[]\no17[(a]\n"
    );
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, Some(0));
}

/// Guard: the same operator/transform forms on a genuine array still fan out
/// per element (or join for `[*]`).
#[test]
fn genuine_array_operator_and_transform_forms_unchanged() {
    let out = run(concat!(
        "arr=('p' 'q r')\n",
        "printf 'g1<%s>\\n' \"${arr[@]-d}\"\n",
        "printf 'g2<%s>\\n' \"${arr[*]-d}\"\n",
        "printf 'g3<%s>\\n' \"${arr[@]@A}\"\n",
        "printf 'g4<%s>\\n' \"${arr[@]@a}\"\n",
        "printf 'g5<%s>\\n' \"${arr[@]:0:1}\"\n",
    ));
    assert_eq!(
        out.stdout,
        "g1<p>\ng1<q r>\ng2<p q r>\ng3<declare>\ng3<-a>\ng3<arr=([0]=\"p\" [1]=\"q r\")>\ng4<a>\ng4<a>\ng5<p>\n"
    );
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, Some(0));
}

/// Guard: an empty but SET scalar is one empty element, not zero.
#[test]
fn empty_scalar_is_one_empty_element() {
    let out = run(concat!(
        "e=''\n",
        "printf 'n=%s\\n' \"${#e[@]}\"\n",
        "printf 'v[%s]\\n' \"${e[@]}\"\n",
    ));
    assert_eq!(out.stdout, "n=1\nv[]\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, Some(0));
}
