//! Issue rubash#538: GNU setattr.def:590-614 set_var_attribute ("only called
//! by readonly/export") absorbs from the temporary environment ONLY the
//! variables the builtin's operands name — `FOO=val export BAR` leaves FOO
//! unset while `FOO=val export FOO` persists — and variables.c:3211
//! bind_variable marks every binding exported while `set -a`
//! (mark_modified_vars) is on, including a valueless `declare v`.

#![cfg(windows)]

use std::process::{Command, Stdio};

fn run_c(script: &str) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash -c");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn prefix_assignment_absorbs_only_named_operands() {
    // WSL GNU bash 5.3.0: `FOO=val export BAR; echo "${FOO-unset}"` -> unset.
    let (code, stdout, stderr) = run_c("FOO=val export BAR; echo \"${FOO-unset}\"");
    assert_eq!(stdout, "unset\n", "stderr={stderr:?}");
    assert_eq!(code, Some(0));

    // readonly shares the rule.
    let (_, stdout, _) = run_c("FOO=val readonly BAR; echo \"${FOO-unset}\"");
    assert_eq!(stdout, "unset\n");

    // The named form persists (both sides agree).
    let (_, stdout, _) = run_c("FOO=val export FOO; echo \"${FOO-unset}\"");
    assert_eq!(stdout, "val\n");

    // A mixed operand list absorbs exactly the named one.
    let (_, stdout, _) = run_c("FOO=val export BAR FOO; echo \"${FOO-unset}\"");
    assert_eq!(stdout, "val\n");

    // Plain external/other-builtin prefixes were already transient.
    let (_, stdout, _) = run_c("FOO=val true; echo \"${FOO-unset}\"");
    assert_eq!(stdout, "unset\n");
}

#[test]
fn allexport_marks_valueless_declarations() {
    // WSL GNU bash 5.3.0: `set -a; declare v; declare -p v` ->
    // `declare -x v`.
    let (_, stdout, _) = run_c("set -a; declare v; declare -p v");
    assert_eq!(stdout, "declare -x v\n");

    // The scalar assignment form keeps its existing behavior.
    let (_, stdout, _) = run_c("set -a; v=1; declare -p v");
    assert_eq!(stdout, "declare -x v=\"1\"\n");

    // Without set -a a valueless declare stays unexported.
    let (_, stdout, _) = run_c("declare w; declare -p w");
    assert_eq!(stdout, "declare -- w\n");
}
