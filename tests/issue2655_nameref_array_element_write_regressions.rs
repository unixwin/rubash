//! rubash#2655 — assignment through a nameref to an array element
//! desynced the typed cell from the mirror store and the debug-only
//! command-boundary verifier panicked (`variable_state.rs:1095`
//! indexed / `:1109` assoc: "typed cell disagrees with its mirror").
//!
//! Two one-sided write paths, one mechanism family:
//!
//! * `declare`/`local`/`typeset` through a nameref wrote the resolved
//!   target's MIRROR only — the post-declare sync (`sync_typed_
//!   assignments`) synced the operand name (`b`), never the resolved
//!   target (`a` for `b -> a[0]`). Fixed by syncing the resolved
//!   target's base as well (nameref18/21/22/23.sub).
//! * a write through a CIRCULAR nameref (`assign_circular_fallback`)
//!   wrote the global namesake's array/assoc MIRROR snapshot only,
//!   never the typed snapshot beside it (nameref15.sub: `a=X`
//!   through `a -> a` moved the mirror to "X" while the cell kept
//!   "0"). Fixed by rebuilding the saved typed cell from the same
//!   render.
//!
//! The plain-assignment funnel already synced both sides, so the
//! issue's minimal snippet (`f 'a[0]'` failing sh_invalidid) never
//! panicked on its own — the panic needs the through-nameref write
//! above followed by a command boundary. Each pin below is the
//! shortest script that panicked before the fix.
//!
//! Expectations measured against GNU bash 5.2.21 (box /usr/bin/bash;
//! #2655 oracle-confirmed the panic class against 5.3.20). Pins with
//! circular-nameref warnings assert stdout + exit code + no panic
//! only: rubash's third warning reads "maximum nameref depth (8)
//! exceeded" where GNU repeats "circular name reference" (a
//! pre-existing diagnostic divergence, not this issue).

use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_c(script: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn assert_no_panic(run: &Run, script: &str) {
    assert!(
        !run.stderr.contains("panicked"),
        "script panicked: {script}\nstderr: {}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("disagrees with its mirror"),
        "script hit the cell/mirror assert: {script}\nstderr: {}",
        run.stderr
    );
}

/// nameref15.sub: `a=X` through the circular nameref `a -> a` must
/// land on the global array's element 0 — mirror AND typed cell —
/// so the next command boundary does not panic.
#[test]
fn issue2655_circular_nameref_indexed_write_syncs_cell() {
    let script = "f() { local -n a=$1; a=X; }; a=(0); f 'a'; declare -p a";
    let run = run_c(script);
    assert_no_panic(&run, script);
    assert_eq!(run.stdout, "declare -a a=([0]=\"X\")\n", "script: {script}");
    assert_eq!(run.code, Some(0), "script: {script}");
}

/// nameref15.sub, assoc arm of the same fallback: `a=X` through
/// circular `a -> a` writes key "0" of the global assoc array.
#[test]
fn issue2655_circular_nameref_assoc_write_syncs_cell() {
    let script = "f() { local -n a=$1; a=X; }; declare -A a=([k]=v); f 'a'; declare -p a";
    let run = run_c(script);
    assert_no_panic(&run, script);
    assert_eq!(
        run.stdout, "declare -A a=([0]=\"X\" [k]=\"v\" )\n",
        "script: {script}"
    );
    assert_eq!(run.code, Some(0), "script: {script}");
}

/// nameref15.sub, append through the circular nameref: `a+=Y`
/// appends to element 0 of the global array, both representations.
#[test]
fn issue2655_circular_nameref_indexed_append_syncs_cell() {
    let script = "f() { local -n a=$1; a+=Y; }; a=(X); f 'a'; declare -p a";
    let run = run_c(script);
    assert_no_panic(&run, script);
    assert_eq!(
        run.stdout, "declare -a a=([0]=\"XY\")\n",
        "script: {script}"
    );
    assert_eq!(run.code, Some(0), "script: {script}");
}

/// nameref23.sub: `declare b+=1` through `b -> a[0]` must bump the
/// integer element in the typed cell as well as the mirror — before
/// the fix the mirror reached 6 while the cell still held 5.
#[test]
fn issue2655_declare_append_through_nameref_to_indexed_element() {
    let script = "declare -ai a; a[0]=4; declare -n b='a[0]'; b+=1; declare b+=1; declare -p a";
    let run = run_c(script);
    assert_no_panic(&run, script);
    assert_eq!(
        run.stdout, "declare -ai a=([0]=\"6\")\n",
        "script: {script}"
    );
    assert_eq!(run.stderr, "", "script: {script}");
    assert_eq!(run.code, Some(0), "script: {script}");
}

/// nameref18.sub: `declare ref=` through `ref -> var[123]` binds
/// element 123 in both representations.
#[test]
fn issue2655_declare_bind_through_nameref_to_indexed_element() {
    let script = "declare -n ref=var[123]; unset ref; declare ref=; declare -p var";
    let run = run_c(script);
    assert_no_panic(&run, script);
    assert_eq!(
        run.stdout, "declare -a var=([123]=\"\")\n",
        "script: {script}"
    );
    assert_eq!(run.stderr, "", "script: {script}");
    assert_eq!(run.code, Some(0), "script: {script}");
}

/// nameref22.sub: `declare foo+=...` through `foo -> bar[0]`
/// appends to element 0 in both representations.
#[test]
fn issue2655_declare_append_through_nameref_to_indexed_element_spaces() {
    let script = "declare -a bar; declare -n foo='bar[0]'; declare foo+=' still more spaces'; declare -p bar";
    let run = run_c(script);
    assert_no_panic(&run, script);
    assert_eq!(
        run.stdout, "declare -a bar=([0]=\" still more spaces\")\n",
        "script: {script}"
    );
    assert_eq!(run.stderr, "", "script: {script}");
    assert_eq!(run.code, Some(0), "script: {script}");
}

/// nameref21.sub (the `:1109` assoc leg): `local -A ref=([1]=)`
/// through `ref -> var` replaces the assoc mirror AND the typed
/// cell, so the following plain `ref=([2]=)` and the second call
/// stay coherent across the boundary.
#[test]
fn issue2655_local_assoc_compound_through_nameref_syncs_cell() {
    let script = "f() { local -n ref=var; local -A ref=([1]=); ref=([2]=); declare -p ref var; }; unset ref var; f; unset ref; var=0; f";
    let run = run_c(script);
    assert_no_panic(&run, script);
    assert_eq!(
        run.stdout,
        "declare -n ref=\"var\"\ndeclare -A var=([2]=\"\" )\ndeclare -n ref=\"var\"\ndeclare -A var=([2]=\"\" )\n",
        "script: {script}"
    );
    assert_eq!(run.stderr, "", "script: {script}");
    assert_eq!(run.code, Some(0), "script: {script}");
}

/// Guard: the issue's minimal snippet — `f 'a[0]'` where the
/// nameref points at an element of the shadowed array itself —
/// still fails sh_invalidid exactly as GNU does (exit 1, no
/// `declare -p` output under `-c`), and never panics.
#[test]
fn issue2655_nameref_to_own_array_element_invalid_id_unchanged() {
    let script = "f() { local -n a=$1; a=X; }; a=(0); f 'a[0]'; declare -p a";
    let run = run_c(script);
    assert_no_panic(&run, script);
    assert_eq!(run.stdout, "", "script: {script}");
    assert_eq!(run.code, Some(1), "script: {script}");
    assert!(
        run.stderr.contains("warning: a: circular name reference"),
        "script: {script}\nstderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("`a[0]': not a valid identifier"),
        "script: {script}\nstderr: {}",
        run.stderr
    );
}
