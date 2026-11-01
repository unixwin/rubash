//! rubash#2433 — scalar -> array conversion must keep the old value
//! verbatim as element 0.
//!
//! GNU: assign_array_element (arrayfunc.c:262) reaches
//! bind_array_variable (arrayfunc.c:288-290), which calls
//! convert_var_to_array (arrayfunc.c:74-100) for a not-yet-array
//! variable; the conversion inserts the old scalar value unchanged as
//! the single element 0 — it is never word-split. Subscript
//! resolution runs BEFORE the conversion against the variable as it
//! stands (assign_array_element_internal, arrayfunc.c:425-434):
//! `array_p (entry) == 0` yields max index 0, and an absent entry
//! resolves no negative subscript at all.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle.
//! Before the fix, rubash parsed the scalar's text as compound array
//! storage, so `x="(a b)"` converted to elements {0:a, 1:b}.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2433-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(dir: &Path, script: &str) -> (String, String) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    let normalize = |text: &str| {
        text.replace(&script_path.to_string_lossy().into_owned(), "SCRIPT")
    };
    (
        normalize(&String::from_utf8_lossy(&output.stdout)),
        normalize(&String::from_utf8_lossy(&output.stderr)),
    )
}

#[test]
fn issue2433_parenthesized_scalar_survives_verbatim() {
    let dir = scratch("main");
    let (out, err) = run_script(
        &dir,
        r#"x="(a b)"
x[1]=z
echo "whole=[${x[0]}] elem1=[${x[1]}] count=${#x[@]}"
"#,
    );
    assert_eq!(err, "");
    assert_eq!(out, "whole=[(a b)] elem1=[z] count=2\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2433_plain_and_padded_scalars_seed_element_zero() {
    let dir = scratch("seed");
    let (out, err) = run_script(
        &dir,
        r#"y=abc
y[2]=q
echo "y0=[${y[0]}] y2=[${y[2]}] n=${#y[@]}"
w="  pad  "
w[1]=z
echo "w0=[${w[0]}] n=${#w[@]}"
s="one two"
s[5]=z
echo "keys=${!s[@]}"
"#,
    );
    assert_eq!(err, "");
    assert_eq!(
        out,
        "y0=[abc] y2=[q] n=2\nw0=[  pad  ] n=2\nkeys=0 5\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2433_negative_subscript_resolves_against_the_single_element() {
    let dir = scratch("neg");
    let (out, err) = run_script(
        &dir,
        r#"x="(a b)"
x[-1]=z
echo "whole=[${x[0]}] count=${#x[@]}"
"#,
    );
    assert_eq!(err, "");
    assert_eq!(out, "whole=[z] count=1\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2433_too_negative_subscript_fails_and_leaves_the_scalar() {
    // Resolution runs before conversion (arrayfunc.c:425-434): with
    // max index 0, -2 is a bad subscript and the variable is never
    // converted at all.
    let dir = scratch("neg2");
    let (out, err) = run_script(
        &dir,
        r#"x="(a b)"
x[-2]=z
echo "whole=[${x[0]}] count=${#x[@]}"
"#,
    );
    assert_eq!(out, "whole=[(a b)] count=1\n");
    assert_eq!(err, "SCRIPT: line 2: x[-2]: bad array subscript\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2433_append_onto_the_converted_element() {
    let dir = scratch("append");
    let (out, err) = run_script(
        &dir,
        r#"x="(a b)"
x[0]+=Q
echo "e0=[${x[0]}] count=${#x[@]}"
"#,
    );
    assert_eq!(err, "");
    assert_eq!(out, "e0=[(a b)Q] count=1\n");
    let _ = fs::remove_dir_all(&dir);
}
