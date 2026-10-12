//! Issue rubash#541: GNU variables.c initialize_dynamic_variables registers
//! every dynamic parameter as a real cell carrying a dynamic_value getter,
//! so `declare -p RANDOM` renders `declare -i RANDOM="<current>"` (the
//! value re-evaluated at display time) instead of "not found"; and `unset
//! FUNCNAME` unbinds the cell permanently — make_funcname_visible
//! (variables.c:1812) finds no cell on function entry, so `${FUNCNAME[0]}`
//! stays empty instead of rebuilding the stack.

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
fn declare_p_lists_dynamic_variables() {
    // WSL GNU bash 5.3.0: `declare -p RANDOM` -> `declare -i RANDOM="…"`
    // (value varies; pin the shape).
    let (_, stdout, stderr) = run_c("declare -p RANDOM");
    assert!(
        stdout.starts_with("declare -i RANDOM=\"") && stdout.trim_end().ends_with('"'),
        "stdout={stdout:?} stderr={stderr:?}"
    );
    let (_, stdout, _) = run_c("declare -p SECONDS");
    assert!(
        stdout.starts_with("declare -i SECONDS=\""),
        "stdout={stdout:?}"
    );
    let (_, stdout, _) = run_c("declare -p SRANDOM");
    assert!(
        stdout.starts_with("declare -i SRANDOM=\""),
        "stdout={stdout:?}"
    );
    let (_, stdout, _) = run_c("declare -p BASHPID");
    assert!(
        stdout.starts_with("declare -i BASHPID=\""),
        "stdout={stdout:?}"
    );
    let (_, stdout, _) = run_c("declare -p EPOCHSECONDS");
    assert!(
        stdout.starts_with("declare -- EPOCHSECONDS=\""),
        "stdout={stdout:?}"
    );
    let (_, stdout, _) = run_c("declare -p LINENO");
    assert!(
        stdout.starts_with("declare -- LINENO=\""),
        "stdout={stdout:?}"
    );
    let (_, stdout, _) = run_c("declare -p GROUPS");
    assert_eq!(stdout, "declare -a GROUPS=([0]=\"0\")\n");

    // A user-assigned RANDOM seeds the generator (GNU keeps the integer
    // view; the value is the next generator draw, not the literal 42).
    let (_, stdout, _) = run_c("RANDOM=42; declare -p RANDOM");
    assert!(
        stdout.starts_with("declare -i RANDOM=\"") && stdout.trim_end().ends_with('"'),
        "stdout={stdout:?}"
    );

    // `unset RANDOM` removes the cell for the shell's lifetime.
    let (code, _, stderr) = run_c("unset RANDOM; declare -p RANDOM");
    assert_eq!(code, Some(1));
    assert!(stderr.contains("RANDOM: not found"), "stderr={stderr:?}");
}

#[test]
fn declare_reads_advance_the_generator_like_gnu() {
    // WSL GNU bash 5.3.0 (exp.tests 242-247 shape): the declare -p getter
    // read advances the generator and `declare -i RANDOM=42` reseeds —
    // `RANDOM=42; declare -p RANDOM >/dev/null; echo ${#RANDOM}` -> 5
    // (draw 17772), `declare -i RANDOM=42; echo ${#RANDOM}` -> 5.
    let (_, stdout, _) = run_c("RANDOM=42\ndeclare -p RANDOM >/dev/null\necho ${#RANDOM}\n");
    assert_eq!(stdout, "5\n");
    let (_, stdout, _) = run_c("RANDOM=42\ndeclare -p RANDOM >/dev/null\necho $RANDOM\n");
    // the declare -p read consumed 17772; the next draw continues.
    assert_eq!(stdout, "26794\n");
    let (_, stdout, _) = run_c("declare -i RANDOM=42\necho ${#RANDOM}\n");
    assert_eq!(stdout, "5\n");
    let (_, stdout, _) = run_c("RANDOM=42\necho $RANDOM\necho $RANDOM\n");
    assert_eq!(stdout, "17772\n26794\n");
    // `export RANDOM=42` does NOT reseed (the sequence continues).
    let (_, stdout, _) =
        run_c("RANDOM=42\necho $RANDOM >/dev/null\nexport RANDOM=42\necho $RANDOM\n");
    assert_eq!(stdout, "26794\n");
}

#[test]
fn unset_funcname_is_not_rebuilt_by_function_entry() {
    // WSL GNU bash 5.3.0: `unset FUNCNAME; f(){ echo "[${FUNCNAME[0]}]"; }; f`
    // -> `[]` (the cell stays unbound; make_funcname_visible finds nothing).
    let (_, stdout, _) = run_c("unset FUNCNAME; f(){ echo \"[${FUNCNAME[0]}]\"; }; f");
    assert_eq!(stdout, "[]\n");

    // Without the unset the stack view still works.
    let (_, stdout, _) = run_c("f(){ echo \"[${FUNCNAME[0]}]\"; }; f");
    assert_eq!(stdout, "[f]\n");
}
