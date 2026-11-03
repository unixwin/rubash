//! Deep-audit lane A3 finding F4: `mapfile -C` ignored callback
//! modifications to the target array, where GNU re-looks-up the array
//! after every callback (builtins/mapfile.def:142-218: "Bad things can
//! happen if the callback modifies ENTRY... so we look it up again every
//! time") and binds the current record on top of the callback's version.
//!
//! GNU bash 5.3.20 oracle behavior (script-file probes, twice confirmed):
//! - `cb() { arr=(MOD); }; mapfile -C cb -c 1 -t arr` on a/b/c lines
//!   -> n=2, arr=(MOD c): each callback resets the array, then the
//!   current line binds.
//! - `cb() { unset arr; }; mapfile -C cb -c 1 -t arr` on a/b lines
//!   -> n=1, arr=([1]="b").
//! The pre-fix build printed n=3 [a b c] and n=2 respectively.

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

#[test]
fn mapfile_callback_reset_survives() {
    let (stdout, _, code) = rubash(
        "cb() { arr=(MOD); }; printf 'a\\nb\\nc\\n' | { mapfile -C cb -c 1 -t arr; echo \"n=${#arr[@]} arr=[${arr[*]}]\"; }",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=2 arr=[MOD c]\n");
}

#[test]
fn mapfile_callback_unset_survives() {
    let (stdout, _, code) = rubash(
        "cb() { unset arr; }; printf 'a\\nb\\n' | { mapfile -C cb -c 1 -t arr; echo \"n=${#arr[@]} arr=[${arr[*]}]\"; }",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=1 arr=[b]\n");
}

#[test]
fn mapfile_callback_noop_keeps_normal_path() {
    let (stdout, _, code) = rubash(
        "cb() { :; }; printf 'a\\nb\\nc\\n' | { mapfile -C cb -c 1 -t arr; echo \"n=${#arr[@]} arr=[${arr[*]}]\"; }",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=3 arr=[a b c]\n");
}
