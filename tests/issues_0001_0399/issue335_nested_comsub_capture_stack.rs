//! Issue rubash#335 (residual) — nested function-body command-substitution
//! captures must not resolve to the enclosing capture.
//!
//! The resid21 partial fix (dup2-snapshot layer, wt21 e8a48f4c) cleared
//! fd 1's alias-generation record on the FULL comsub path
//! (command_substitution.rs command_list_substitution_output_typed), but the
//! `$(funcname args)` single-command FAST PATH (embedded_mutations.rs
//! run_function_command_substitution) rebinds fd 1's table entry to the live
//! Stdout endpoint WITHOUT dropping the record. Inside nvm's
//! `{ provided="$(nvm_rc_version 3>&1 1>&4)"; } 4>&1` (nvm.sh:3938), fd 1
//! carries the `1>&4` dup's record (the fd-4 snapshot of the OUTER capture):
//! every nested `$(func)` fast-path capture then wrote its body's stdout to
//! that snapshot's generation — one capture level too high — and read back
//! EMPTY. On the real f26 (nvm "use should respect system in .nvmrc") the
//! CWD line from nvm_find_up landed in the OUTERMOST capture while
//! `$(nvm_find_nvmrc)` came back empty.
//!
//! Fix: the fast path's fd-1 rebind now clears the record, mirroring the
//! subshell rebind (GNU subst.c:7306-7313 command_substitute — the child's
//! fd 1 is dup2'd onto the pipe: a NEW open file description that inherits
//! none of the parent fd 1's dup2 aliases).
//!
//! Expected outputs byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-30).

use std::io::Write;
use std::process::Command;

fn run_rubash_script(body: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-issue335-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("p{}.sh", body.len()));
    let mut file = std::fs::File::create(&path).unwrap();
    // LF line endings: the repo's .sh contract (CRLF breaks the WSL-side
    // baseline the expectations were byte-verified against).
    let _ = file.write_all(body.replace("\r\n", "\n").as_bytes());
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Minimal trigger: a `$(inner)` FAST-PATH capture running inside a function
/// that was itself invoked through the `{ p="$(mid 3>&1 1>&4)"; } 4>&1`
/// idiom. GNU: inner's write goes to the L2 capture (d=[INNER]), mid's own
/// stdout follows the fd-4 snapshot to the real stdout, p reads empty.
#[test]
fn function_fast_path_capture_survives_fd4_snapshot_scope() {
    let (stdout, stderr, code) = run_rubash_script(
        "inner() { echo INNER; }\nmid() { local d; d=\"$(inner)\"; echo \"d=[$d]\"; }\n{ p=\"$(mid 3>&1 1>&4)\"; } 4>&1\necho \"p=[$p]\"\n",
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "d=[INNER]\np=[]\n");
}

/// The full nvm shape (nvm.sh nvm_find_up / nvm_find_nvmrc / nvm_rc_version
/// / the `{ provided="$(... 3>&1 1>&4)"; } 4>&1` caller, captured with
/// `2>&1` like the test's `OUTPUT="$(nvm use 2>&1)"`): every layer's capture
/// holds exactly its own writes.
#[test]
fn nvm_shaped_nested_function_captures_stay_at_their_depth() {
    let (stdout, stderr, code) = run_rubash_script(
        r#"find_up() { echo UP-WRITE; }
find_nvmrc() {
  local dir
  dir="$(find_up)"
  echo "dir=[$dir]" >&2
  if [ -n "${dir}" ]; then echo NVMRC-WRITE; fi
}
rc_version() {
  local p
  p="$(find_nvmrc)"
  echo "p=[$p]" >&2
  echo "FOUND:${p}"
}
use_cmd() {
  local provided
  { provided="$(rc_version 3>&1 1>&4)"; } 4>&1
  echo "PROVIDED=[$provided]" >&2
}
OUT="$(use_cmd 2>&1)"
echo "OUT=[$OUT]"
"#,
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "OUT=[dir=[UP-WRITE]\np=[NVMRC-WRITE]\nFOUND:NVMRC-WRITE\nPROVIDED=[]]\n"
    );
}

/// Regression guard: plain nested function-body comsubs (no fd-4 idiom) keep
/// capturing at their own depth.
#[test]
fn plain_nested_function_comsubs_capture_own_writes() {
    let (stdout, stderr, code) = run_rubash_script(
        r#"l3() { echo L3A; local x; x=$(echo L3COM); echo L3B; }
l2() { echo L2A; local v; v="$(l3)"; echo "L2B[$v]"; }
l1() { echo L1A; local w; w="$(l2)"; echo "L1B[$w]"; }
echo "TOP[$(l1)]"
"#,
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "TOP[L1A\nL1B[L2A\nL2B[L3A\nL3B]]]\n");
}

/// The writes AFTER a nested comsub in the same body resolve to the CURRENT
/// capture, not the outer one (the issue's phrasing of the residue), with a
/// second fast-path call in the same body.
#[test]
fn writes_after_nested_fast_path_comsub_stay_in_current_capture() {
    let (stdout, stderr, code) = run_rubash_script(
        r#"one() { echo ONE; }
two() { echo TWO; }
body() {
  local a b
  a="$(one)"
  b="$(two)"
  echo "after[$a/$b]"
}
{ v="$(body 3>&1 1>&4)"; } 4>&1
echo "v=[$v]"
"#,
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after[ONE/TWO]\nv=[]\n");
}
