//! Issue rubash#1719: re-declaring an array local with `local` after an
//! intervening plain assignment (or append / element write) installed the
//! typed cell left the cell stale: the `local` builtin wrote the new
//! binding into the rendered mirror only, and `execute_local`'s post-bind
//! pass skipped array/assoc operands without re-syncing the typed owner
//! (execute_declare performs that sync; execute_local never did). Debug
//! builds panicked in `verify_typed_cells` (`variable_state.rs` — "typed
//! indexed cell disagrees with its mirror"); in release builds the stale
//! cell silently shadowed the new binding, so `local -a a=()` did not
//! clear the array.
//!
//! The field trigger is oh-my-bash's module loader: `_omb_module_require`
//! runs `local -a locations=()` per module inside its `while (($#))` loop,
//! after the previous iteration's case-branch plain assignment
//! `locations=({...}/lib/"$name".{bash,sh})` had installed the cell — the
//! second module's load panicked (ecosuite DIV-C).
//!
//! GNU model: ONE variable object per name. builtins/declare.def:261
//! `declare_internal` (the single entry for declare/typeset/local) binds
//! through variables.c:2579 `make_local_variable` and variables.c:3225
//! `bind_variable`; compound values land via arrayfunc.c:700
//! `assign_compound_array_list` on that same object, and every read
//! consults it. The typed/mirror split is rubash-internal, so a mirror
//! write must sync or drop the structured cell (the storage.rs contract).
//!
//! Expected outputs verified against GNU bash 5.3.20 (script files and
//! `-c` strings agree).

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

fn assert_gnu(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
    assert_eq!(stdout, expected_stdout);
}

#[test]
fn issue1719_local_redeclare_empty_after_plain_assignment() {
    // The minimized repro: declare empty, plain-assign values (installs
    // the typed cell), re-declare empty — the array must read empty.
    assert_gnu(
        "f() { local -a a=(); a=(1 2 3 4); local -a a=(); echo \"count=${#a[@]}\"; }; f; echo \"rc=$?\"",
        "count=0\nrc=0\n",
    );
}

#[test]
fn issue1719_local_redeclare_new_values_after_plain_assignment() {
    // Re-declare with a replacement value: the cell must hold the NEW
    // element, not the plain assignment's stale ones.
    assert_gnu(
        "f() { local -a a=(); a=(1 2 3 4); local -a a=(9); echo \"count=${#a[@]} first=${a[0]}\"; }; f; echo \"rc=$?\"",
        "count=1 first=9\nrc=0\n",
    );
}

#[test]
fn issue1719_local_redeclare_after_append_and_element_write() {
    // The cell-installing write may also be `+=` or an element store;
    // the re-declare must clear whatever they built.
    assert_gnu(
        "f() { local -a a=(); a+=(1 2); a[2]=z; local -a a=(); echo \"count=${#a[@]}\"; }; f; echo \"rc=$?\"",
        "count=0\nrc=0\n",
    );
}

#[test]
fn issue1719_local_assoc_redeclare_after_plain_assignment() {
    // Assoc leg: same missing sync, assoc cell this time.
    assert_gnu(
        "f() { local -A A=(); A=([k]=v [k2]=v2); local -A A=(); echo \"count=${#A[@]}\"; }; f; echo \"rc=$?\"",
        "count=0\nrc=0\n",
    );
}

#[test]
fn issue1719_local_redeclare_loop_loader_shape() {
    // The oh-my-bash loader shape: per-iteration `local -a locations=()`
    // inside a `while (($#))` loop, case-branch plain assignment from a
    // brace expansion. The second iteration must come up with a fresh,
    // correctly filled array.
    assert_gnu(
        "f() { local name; while (($#)); do name=$1; shift; local -a locations=(); case $name in cli) locations=({\"/cust\",\"/osh\"}/lib/\"$name\".{bash,sh}) ;; esac; echo \"name=$name count=${#locations[@]}\"; done; }; f cli cli; echo \"rc=$?\"",
        "name=cli count=4\nname=cli count=4\nrc=0\n",
    );
}

#[test]
fn issue1719_local_redeclare_without_dash_a_on_array() {
    // `local a=()` on a variable already marked as an array local: the
    // scalar-shadow pass skips array-marked names, so this shape relied
    // on the same post-bind sync.
    assert_gnu(
        "f() { local -a a=(); a=(1 2 3 4); local a=(); echo \"count=${#a[@]}\"; }; f; echo \"rc=$?\"",
        "count=0\nrc=0\n",
    );
}

#[test]
fn issue1719_local_valueless_redeclare_keeps_values() {
    // Guard: a valueless re-declaration is not a bind — GNU keeps the
    // existing array (declare.def:659-668 same-context rule). This shape
    // already agreed before the fix and must not regress.
    assert_gnu(
        "f() { local -a a=(); a=(1 2 3 4); local -a a; echo \"count=${#a[@]} first=${a[0]}\"; }; f; echo \"rc=$?\"",
        "count=4 first=1\nrc=0\n",
    );
}
