//! #609 slice 1: fast RHS (`literal`, `$name`, `"$name"`) contract — the same
//! observable behavior as the full expansion (GNU subst.c:3524
//! do_assignment_internal; variables.c:3225 bind_variable).

use std::process::Command;

fn run(script: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash")
}

fn out(script: &str) -> String {
    let o = run(script);
    assert!(o.status.success(), "failed: {script}");
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn fast_rhs_shapes_bind_plain_scalars() {
    assert_eq!(
        out("S=abc; t=$S; u=\"$S\"; v=lit; e=; echo \"$t|$u|$v|$e|\""),
        "abc|abc|lit||\n"
    );
}

#[test]
fn fast_rhs_sees_latest_binding() {
    assert_eq!(out("a=1; b=$a; a=2; c=$a; echo \"$b|$c\""), "1|2\n");
}

#[test]
fn fast_rhs_unset_expands_empty() {
    assert_eq!(out("t=$gone; echo \"[$t]\""), "[]\n");
}

#[test]
fn fast_rhs_fallthrough_keeps_semantics() {
    assert!(!run("set -u; t=$gone").status.success());
    assert_eq!(out("readonly r=v\nr=w\necho rc=$?"), "rc=1\n");
    assert_eq!(out("t=$LINENO; echo $t"), "1\n");
    assert_eq!(out("declare -i n=2+3; m=$n; echo $m"), "5\n");
}

// #609 slice 2: dual-store fast bind contract — marks-free plain scalar
// binds (GNU variables.c:3225 bind_variable -> :3054, one hash insert)
// keep both stores in lockstep; identical rebinds are no-ops; special
// names and attributed variables fall through to the generic bind.

#[test]
fn fast_bind_repeated_same_value_stays_in_sync() {
    let o = out("S=abc; t=$S; t=$S; t=$S; declare -p t S; echo \"$t\"");
    let mut lines: Vec<&str> = o.lines().collect();
    assert_eq!(lines.pop(), Some("abc"));
    lines.sort_unstable();
    assert_eq!(lines, ["declare -- S=\"abc\"", "declare -- t=\"abc\""]);
}

#[test]
fn fast_bind_special_names_fall_through() {
    assert_eq!(out("SECONDS=5; echo got=$SECONDS"), "got=5\n");
    assert_eq!(out("PATH=/bin; echo $PATH"), "/bin\n");
    assert_eq!(out("x=1\nreadonly x\nx=1\necho rc=$?"), "rc=1\n");
    assert_eq!(
        out("declare -u u=abc; v=$u; u=$v; declare -p u"),
        "declare -u u=\"ABC\"\n"
    );
}

#[test]
fn fast_bind_nameref_and_export_fall_through() {
    assert_eq!(
        out("t=old; declare -n r=t; s=$t; r=$s; declare -p t; echo \"$r\""),
        "declare -- t=\"old\"\nold\n"
    );
    assert_eq!(
        out("set -a; S=abc; q=$S; declare -p q"),
        "declare -x q=\"abc\"\n"
    );
}
