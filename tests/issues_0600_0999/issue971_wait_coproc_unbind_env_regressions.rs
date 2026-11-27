//! Issue rubash#971: the `wait -p` variable-capture unbind and the coproc
//! unbind paths (`executor/job_builtins.rs` `execute_wait` /
//! `coproc_unset_vars`) removed the variable-table entry but left the
//! inherited process-env copy — the same residue class fixed for `unset`
//! in #943. A name that entered the process env by inheritance predates
//! the write mirror, so the table-miss read fallbacks
//! (`exact_case_env_var` bare-`$NAME` reads, nounset boundness, the
//! child-spawn env base) resurrected it. The unbind must also drop the
//! typed cell and the attribute bits: GNU variables.c:3959 makunbound
//! disposes the whole SHELL_VAR and removes the name from the export
//! environment (variables.c:3839).
//!
//! GNU references:
//! - builtins/wait.def:166 -> builtins/common.c:1021 builtin_unbind_variable:
//!   find_variable (variables.c:2363) resolves the nameref chain, so the
//!   readonly / non-unsettable gates test the REFERENT while diagnostics
//!   name the operand; a dead chain leaves v == 0 and the operand itself
//!   is makunbound. A bracketed operand is a literal name at this layer —
//!   `wait -p 'A[0]'` never unbinds an element (unlike `unset 'A[0]'`).
//! - execute_cmd.c:2450 coproc_unsetvars: unbind_variable_noref on
//!   `<name>_PID` (literal), then check_unbind_variable on `<name>`
//!   (nameref-resolved, readonly-gated).
//!
//! Baselines below were verified against WSL GNU bash 5.3.20.

#![cfg(windows)]

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn run_script(script: &str, env: &[(&str, &str)]) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue971-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue971 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT");
    for (name, value) in env {
        command.env(name, value);
    }
    let output = command.output().expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// `wait -p NAME` unbinds NAME before any waiting happens (wait.def:139-166);
/// a name that never gets bound stays unset — including the inherited
/// process-env copy.
#[test]
fn wait_p_unbind_drops_inherited_process_env_copy() {
    // WSL GNU 5.3.20: after=[unset], printenv misses (rc=1).
    let (code, stdout, _) = run_script(
        "echo \"before=[${WP-unset}]\"\nwait -p WP\necho \"rc=$? after=[${WP-unset}]\"\necho \"bare=[$WP]\"\n",
        &[("WP", "parent")],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "before=[parent]\nrc=0 after=[unset]\nbare=[]\n");
}

/// The `wait -n` no-candidate path (wait.def:233-234 -> 127) must leave the
/// -p operand unbound, residue included.
#[test]
fn wait_n_p_unbind_drops_inherited_process_env_copy() {
    // WSL GNU 5.3.20: rc=127 after=[].
    let (_, stdout, _) = run_script(
        "echo \"before=[$WN]\"\nwait -n -p WN\necho \"rc=$? after=[$WN]\"\n",
        &[("WN", "parent")],
    );
    assert_eq!(stdout, "before=[parent]\nrc=127 after=[]\n");
}

/// Under `set -u`, a resurrected residue makes the name look bound; GNU
/// aborts on the unbound expansion instead.
#[test]
fn wait_p_unbind_survives_nounset() {
    // WSL GNU 5.3.20: `WU: unbound variable`, exit non-zero, no `after=`.
    let (code, stdout, stderr) = run_script(
        "set -u\nwait -p WU\necho \"after=[$WU]\"\n",
        &[("WU", "parent")],
    );
    assert_eq!(stdout, "");
    assert!(stderr.contains("WU: unbound variable"), "stderr: {stderr}");
    assert_ne!(code, Some(0));
}

/// makunbound disposes the whole SHELL_VAR: the exported attribute dies
/// with the binding.
#[test]
fn wait_p_unbind_drops_attributes() {
    // WSL GNU 5.3.20: `declare: WX: not found` (rc 1) — no `declare -x WX`
    // residue; `declare -i` behaves identically.
    let (_, _, stderr) = run_script("export WX=inherited\nwait -p WX\ndeclare -p WX\n", &[]);
    assert!(
        stderr.contains("declare: WX: not found"),
        "stderr: {stderr}"
    );

    let (_, _, stderr) = run_script("declare -i WI=42\nwait -p WI\ndeclare -p WI\n", &[]);
    assert!(
        stderr.contains("declare: WI: not found"),
        "stderr: {stderr}"
    );
}

/// builtin_unbind_variable looks the operand up literally: a bracketed
/// operand names no stored cell, so nothing is unbound — even for a
/// scalar or a readonly array (the readonly gate sees find_variable's
/// NULL and is skipped).
#[test]
fn wait_p_bracket_operand_is_a_literal_name() {
    // WSL GNU 5.3.20: scalar survives, still exported.
    let (_, stdout, _) = run_script(
        "wait -p 'AA[0]'\necho \"after=[$AA]\"\ndeclare -p AA\n",
        &[("AA", "parent")],
    );
    assert_eq!(stdout, "after=[parent]\ndeclare -x AA=\"parent\"\n");

    // WSL GNU 5.3.20: element 0 stays bound; `unset 'A[0]'` (the real
    // element path) still removes it — the two spellings differ upstream.
    let (_, stdout, _) = run_script(
        "A=(x y)\nwait -p 'A[0]'\ndeclare -p A\nunset 'A[0]'\ndeclare -p A\n",
        &[],
    );
    assert_eq!(
        stdout,
        "declare -a A=([0]=\"x\" [1]=\"y\")\ndeclare -a A=([1]=\"y\")\n"
    );

    // WSL GNU 5.3.20: readonly array + bracketed operand -> rc 0, intact.
    let (_, stdout, _) = run_script(
        "readonly -a RA=(x y)\nwait -p 'RA[0]'\necho \"rc=$?\"\ndeclare -p RA\n",
        &[],
    );
    assert_eq!(stdout, "rc=0\ndeclare -ar RA=([0]=\"x\" [1]=\"y\")\n");
}

/// The nameref resolution of unbind_variable (variables.c:3794-3801):
/// `wait -p ref` unbinds the referent and keeps the nameref; a chain that
/// resolves to nothing unbinds the operand itself.
#[test]
fn wait_p_unbind_resolves_namerefs() {
    // WSL GNU 5.3.20: `declare -n ref="target"` survives, target is gone.
    let (_, stdout, stderr) = run_script(
        "target=old\ndeclare -n ref=target\nwait -p ref\ndeclare -p ref target\n",
        &[],
    );
    assert_eq!(stdout, "declare -n ref=\"target\"\n");
    assert!(
        stderr.contains("declare: target: not found"),
        "stderr: {stderr}"
    );

    // Dead chain (ref -> missing): the operand itself is unbound.
    let (_, _, stderr) = run_script(
        "declare -n dref=missing\nwait -p dref\ndeclare -p dref\n",
        &[],
    );
    assert!(
        stderr.contains("declare: dref: not found"),
        "stderr: {stderr}"
    );

    // Element-cell nameref (e -> arr[0]): the operand is unbound, the
    // array is untouched.
    let (_, stdout, stderr) = run_script(
        "arr=(x y)\ndeclare -n el=arr[0]\nwait -p el\ndeclare -p el arr\n",
        &[],
    );
    assert_eq!(stdout, "declare -a arr=([0]=\"x\" [1]=\"y\")\n");
    assert!(
        stderr.contains("declare: el: not found"),
        "stderr: {stderr}"
    );
}

/// The gates test the resolved referent while the diagnostic names the
/// operand (wait.def:166 -> common.c:1021-1036).
#[test]
fn wait_p_gates_apply_to_the_resolved_referent() {
    // WSL GNU 5.3.20: `wait: ref: cannot unset: readonly variable`, rc 1,
    // and neither the nameref nor its readonly target is unbound. (The
    // script's own exit status comes from the last command; the wait
    // failure is observed through `$?`.)
    let (_, stdout, stderr) = run_script(
        "readonly target=ro\ndeclare -n ref=target\nwait -p ref\necho \"rc=$?\"\ndeclare -p ref target\n",
        &[],
    );
    assert_eq!(
        stdout,
        "rc=1\ndeclare -n ref=\"target\"\ndeclare -r target=\"ro\"\n"
    );
    assert!(
        stderr.contains("wait: ref: cannot unset: readonly variable"),
        "stderr: {stderr}"
    );

    // att_nounset dynamic variables (variables.c:1889-1893) refuse the
    // unbind outright.
    let (_, stdout, stderr) = run_script("wait -p BASH_SOURCE\necho \"rc=$?\"\n", &[]);
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("wait: BASH_SOURCE: cannot unset"),
        "stderr: {stderr}"
    );
}

/// An unbound dynamic variable stays unbound for the shell's lifetime.
#[test]
fn wait_p_unbind_kills_dynamic_variables() {
    // WSL GNU 5.3.20: LINENO reads empty after `wait -p LINENO`.
    let (code, stdout, _) = run_script("wait -p LINENO\necho \"rc=$? LINENO=[$LINENO]\"\n", &[]);
    assert_eq!(stdout, "rc=0 LINENO=[]\n");
    assert_eq!(code, Some(0));
}

/// makunbound inside a function context takes the invisible-mark branch
/// (variables.c:3984-4005): the local disappears from reads but keeps its
/// attribute word, and the outer binding is not disturbed.
#[test]
fn wait_p_unbind_marks_locals_invisible() {
    // WSL GNU 5.3.20: `declare -- WP` inside the function, outer value of
    // G restored at scope pop, WP absent globally afterwards.
    let (_, stdout, _) = run_script(
        "G=outer\nf() { local G=inner WP=wl; wait -p WP; declare -p WP; wait -p G; echo \"G=[$G]\"; declare -p G; }\nf\necho \"outerG=$G\"\ndeclare -p WP\n",
        &[],
    );
    assert_eq!(stdout, "declare -- WP\nG=[]\ndeclare -- G\nouterG=outer\n");
}

/// `wait -p` then bind through a real reap still assigns the pid.
#[test]
fn wait_p_still_binds_the_reaped_pid() {
    // WSL GNU 5.3.20: WP holds the waited pid (printed value differs per
    // platform — assert the match, not the digits).
    let (_, stdout, _) = run_script(
        "sleep 0.2 &\nP=$!\nwait -p WP \"$P\"\necho \"match=$([ \"$WP\" = \"$P\" ] && echo yes || echo no)\"\n",
        &[],
    );
    assert_eq!(stdout, "match=yes\n");
}

/// coproc_unsetvars (execute_cmd.c:2450) unbinds <name> and <name>_PID
/// when the coprocess is reaped — including inherited process-env copies
/// and every attribute bit.
#[test]
fn coproc_unbind_drops_inherited_process_env_copies() {
    // WSL GNU 5.3.20: after=[][], arith=0, both names report not found.
    let (_, stdout, _) = run_script(
        "echo \"before=[$CP][$CP_PID]\"\ncoproc CP { printf hi; }\necho \"bound=[$CP][$CP_PID]\"\nwait\necho \"after=[$CP][$CP_PID]\"\necho \"arith=$((CP_PID + 0))\"\n",
        &[("CP", "parent"), ("CP_PID", "999")],
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "before=[parent][999]");
    assert_eq!(lines[2], "after=[][]");
    assert_eq!(lines[3], "arith=0");

    let (_, _, stderr) = run_script(
        "coproc CP { printf hi; }\nwait\ndeclare -p CP\ndeclare -p CP_PID\n",
        &[("CP", "parent"), ("CP_PID", "999")],
    );
    assert!(
        stderr.contains("declare: CP: not found"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("declare: CP_PID: not found"),
        "stderr: {stderr}"
    );
}

/// The unnamed coproc uses COPROC/COPROC_PID — same unbind channel.
#[test]
fn unnamed_coproc_unbind_drops_inherited_copies() {
    // WSL GNU 5.3.20: after unbind both names are gone.
    let (_, stdout, _) = run_script(
        "coproc { printf hi; }\nwait\necho \"after=[$COPROC][$COPROC_PID]\"\n",
        &[("COPROC", "pc"), ("COPROC_PID", "5")],
    );
    assert_eq!(stdout, "after=[][]\n");
}
