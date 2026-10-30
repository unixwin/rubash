//! Issue rubash#2259: the invocation-classification family
//! (`__RUBASH_INTERACTIVE` and its siblings — login shape, `-c` shape,
//! debugger mode) is THIS process's own derivation from its argv/tty
//! shape (GNU shell.c:540-549 derives `interactive_shell` from the
//! process's own shape; the state lives in process-local globals,
//! never in the environment). Pre-fix the family was stored in the
//! variable table, mirrored into the real process environment, and
//! projected from there into every plain child's environment, so:
//!
//!   * a nested shell imported its parent's copy and classified
//!     ITSELF by the parent's shape (the discard misclassification
//!     rubash#2236 proved live), and
//!   * `env` / `printenv` in any child of an interactive shell showed
//!     internal stamps GNU never publishes (GNU ground: an interactive
//!     bash's `env` output contains no shell-internal markers at all).
//!
//! The fix enforces one adjudicated set
//! (executor/local_helpers.rs INVOCATION_STATE_ENV_KEYS) at all three
//! boundaries: the mirror gate never writes the family to the process
//! env, the child-environment composition never projects it, and
//! Executor construction strips ambient copies at import (the
//! rubash#2137 F7 adjudication, extended from the debugger pair to the
//! whole family). Host-stamped `__RUBASH_*` protocol markers outside
//! the family keep their deliberate mirror transport (ms_stopbleed
//! class 2) — the last test pins that this did not regress.
//!
//! Rust semantic owners: executor/local_helpers.rs
//! (INVOCATION_STATE_ENV_KEYS + shell_assignment_needs_process_env),
//! executor/init.rs (import strip), executor/readonly_functions.rs
//! (compose_child_environment projection),
//! executor/compound_exec.rs + executor/external_setup.rs (respawn
//! env_map base adjudication).

use std::process::{Command, Stdio};

fn run_rubash(args: &[&str], stamps: &[(&str, &str)]) -> (Option<i32>, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT");
    for (name, value) in stamps {
        cmd.env(name, value);
    }
    let output = cmd.output().expect("run rubash");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn run_script(script: &str, stamps: &[(&str, &str)]) -> (Option<i32>, String, String) {
    // Unique scratch dir per call: the pid alone is shared by every test
    // in this binary, and the script tests run in parallel threads —
    // keyed only by pid they remove/rewrite each other's probe.sh.
    static CALL_SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seq = CALL_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rubash-issue2259-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue2259 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let result = run_rubash(&[script_path.to_string_lossy().as_ref()], stamps);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// The full classification family, stamped into the parent environment
/// the way a pre-fix interactive parent's mirror used to publish it.
const FAMILY_STAMPS: &[(&str, &str)] = &[
    ("__RUBASH_INTERACTIVE", "1"),
    ("__RUBASH_INTERACTIVE_FLAG_OFF", "1"),
    ("__RUBASH_IS_C", "1"),
    ("__RUBASH_LOGIN_SHELL", "1"),
    ("__RUBASH_DEBUGGER_MODE", "1"),
];

#[test]
fn ambient_invocation_stamps_classify_nothing_and_reach_no_child() {
    // A plain `-c` shell whose environ carries the whole family must
    // still classify by its own shape: `$-` carries no `i`, and none
    // of the family is visible to its children.
    let (_, stdout, _) = run_rubash(
        &[
            "-c",
            "echo \"flags=$-\"; \
             printenv __RUBASH_INTERACTIVE; echo \"pi=$?\"; \
             printenv __RUBASH_INTERACTIVE_FLAG_OFF; echo \"pf=$?\"; \
             printenv __RUBASH_IS_C; echo \"pc=$?\"; \
             printenv __RUBASH_LOGIN_SHELL; echo \"pl=$?\"; \
             printenv __RUBASH_DEBUGGER_MODE; echo \"pd=$?\"",
        ],
        FAMILY_STAMPS,
    );
    let flags_line = stdout
        .lines()
        .find(|line| line.starts_with("flags="))
        .expect("flags line present");
    assert!(
        !flags_line.contains('i'),
        "an ambient __RUBASH_INTERACTIVE must not make a -c shell \
         classify itself interactive; got: {flags_line}"
    );
    for marker in ["pi=1", "pf=1", "pc=1", "pl=1", "pd=1"] {
        assert!(
            stdout.lines().any(|line| line == marker),
            "child env must not carry the invocation family ({marker}); stdout:\n{stdout}"
        );
    }
}

#[test]
fn nested_shell_classifies_by_its_own_shape() {
    // The #2259 headline: an interactive-shaped parent environment must
    // not make a NESTED rubash process interactive. The outer script
    // shell runs under the stamps; the nested `-c` shell it spawns
    // must report its own (non-interactive) flags.
    let bin = env!("CARGO_BIN_EXE_rubash");
    let script = format!("\"{bin}\" -c 'echo \"nested=$-\"'\n");
    let (_, stdout, _) = run_script(&script, FAMILY_STAMPS);
    let nested_line = stdout
        .lines()
        .find(|line| line.starts_with("nested="))
        .expect("nested flags line present");
    assert!(
        !nested_line.contains('i'),
        "nested shell classified itself by the inherited stamp; got: {nested_line}"
    );
}

#[test]
fn own_interactive_shape_classifies_but_children_stay_clean() {
    // The session's own `-i` derivation must keep working (its own
    // `$-` shows `i`) while its plain children see no stamp at all —
    // GNU ground: an interactive bash's `env` shows no internal
    // interactive marker.
    let (_, stdout, _) = run_rubash(
        &[
            "-i",
            "-c",
            "echo \"flags=$-\"; \
             printenv __RUBASH_INTERACTIVE; echo \"pi=$?\"; \
             env | grep -c '^__RUBASH_INTERACTIVE='; true",
        ],
        &[],
    );
    let flags_line = stdout
        .lines()
        .find(|line| line.starts_with("flags="))
        .expect("flags line present");
    assert!(
        flags_line.contains('i'),
        "an -i shell must still classify itself interactive; got: {flags_line}"
    );
    assert!(
        stdout.lines().any(|line| line == "pi=1"),
        "an interactive shell's child env must not carry its stamp; stdout:\n{stdout}"
    );
    assert!(
        stdout.lines().any(|line| line == "0"),
        "`env` in an interactive shell's child must show no \
         __RUBASH_INTERACTIVE line; stdout:\n{stdout}"
    );
}

#[test]
fn ambient_frame_stamps_do_not_reach_script_children() {
    // The diagnostic frame pair joins the same adjudication at import
    // for non-respawn entries: a foreign parent's script name / line
    // must not surface in this script's children (the script's OWN
    // name, stamped by its own entry, is the only one allowed).
    let (_, stdout, _) = run_script(
        "printenv __RUBASH_CURRENT_LINE; echo \"pl=$?\"\n\
         printenv __RUBASH_SCRIPT_NAME; echo \"ps=$?\"\n",
        &[
            ("__RUBASH_CURRENT_LINE", "99"),
            ("__RUBASH_SCRIPT_NAME", "/foreign/parent.sh"),
        ],
    );
    assert!(
        stdout.lines().any(|line| line == "pl=1"),
        "a foreign __RUBASH_CURRENT_LINE must not reach children; stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("/foreign/parent.sh"),
        "a foreign __RUBASH_SCRIPT_NAME must not surface; stdout:\n{stdout}"
    );
    assert!(
        stdout.lines().any(|line| line == "ps=0"),
        "the script's own stamped name must still reach its children; stdout:\n{stdout}"
    );
}

#[test]
fn host_protocol_mirror_still_reaches_children() {
    // Not the classification family: a session's own mirrored
    // `__RUBASH_*` assignment is the deliberate host protocol
    // transport (ms_stopbleed class 2). It must keep reaching spawned
    // children exactly as before this fix.
    let (_, stdout, _) = run_script(
        "__RUBASH_ISSUE2259_PROBE=probe-2259\nprintenv __RUBASH_ISSUE2259_PROBE\n",
        &[],
    );
    assert_eq!(stdout, "probe-2259\n");
}

#[test]
fn respawn_shapes_still_work() {
    // Compatibility spot guards for the respawn families whose env_map
    // bases this change adjudicates (full verification of the
    // issue2236 / issue1582 / issue1654 pin families is the main
    // session's batch gate): command substitution and background
    // respawns still run, and the top-level depth stays 0.
    let (_, stdout, _) = run_rubash(
        &[
            "-c",
            "x=$(echo inner); echo \"comsub=$x\"; \
             echo bg & wait; \
             echo \"depth=$BASH_SUBSHELL\"",
        ],
        &[],
    );
    assert!(
        stdout.contains("comsub=inner"),
        "command substitution respawn broke; stdout:\n{stdout}"
    );
    assert!(
        stdout.lines().any(|line| line == "bg"),
        "background respawn broke; stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("depth=0"),
        "top-level BASH_SUBSHELL must stay 0; stdout:\n{stdout}"
    );
}
