//! Regression pins for rubash issue #3429: `set -u` with an unbound
//! variable as a redirect target must abort the script when the target
//! command runs in the main shell — including an arithmetic command
//! and a brace group, whose compound redirections GNU applies before
//! dispatch (execute_cmd.c:854 `do_redirections(command->redirects,
//! RX_ACTIVE|RX_UNDOABLE)` in the compound prologue shared by
//! cm_arith / cm_group, :479-495; err_unboundvar, error.c:447).
//!
//! rubash's reject gate classified by the node's synthetic words:
//! `(( ... ))` records words `["((", "<expr>", "))"]` and a brace
//! group records no words at all, so both read as a forked-child
//! (external) target and the fatal jump was contained — the walker
//! reported `name: unbound variable` and set status 1, yet the script
//! kept running. The fix teaches
//! `heredoc_error_command_runs_in_main_shell` that an arithmetic or
//! brace-group node runs in the main shell (unless backgrounded or
//! pipelined, which still fork first).
//!
//! Expectations are the GNU bash 5.3.20 oracle outputs for the lane
//! probe matrix (probe-3429-gnu.sh, 18 shapes), compared byte-exact
//! on rc / stdout / stderr with the script file invoked as
//! `rubash case.sh`, so diagnostics carry the bare `case.sh` prefix.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("rubash-issue3429-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_case(tag: &str, script: &str) -> (i32, String, String) {
    let scratch = Scratch::new(tag);
    let case_path = scratch.0.join("case.sh");
    fs::write(&case_path, script).expect("write case script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&scratch.0)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn assert_case(tag: &str, script: &str, want_rc: i32, want_out: &str, want_err: &str) {
    let (rc, out, err) = run_case(tag, script);
    assert_eq!(rc, want_rc, "{tag}: rc\nstdout={out:?}\nstderr={err:?}");
    assert_eq!(out, want_out, "{tag}: stdout\nstderr={err:?}");
    assert_eq!(err, want_err, "{tag}: stderr\nstdout={out:?}");
}

// ---- Fail-first cells: the gate contained the fatal jump. ----

#[test]
fn arith_multiline_output_redirect_word_is_fatal() {
    assert_case(
        "x05",
        "set -u\n(( x = 1 \\\n)) > $undefined_target_xyz\necho after=$?\n",
        1,
        "",
        "case.sh: line 3: undefined_target_xyz: unbound variable\n",
    );
}

#[test]
fn arith_input_redirect_word_is_fatal() {
    assert_case(
        "a-in",
        "set -u\n(( x = 1 )) < $undefined_target_xyz\necho after=$?\n",
        1,
        "",
        "case.sh: line 2: undefined_target_xyz: unbound variable\n",
    );
}

#[test]
fn arith_stderr_redirect_word_is_fatal() {
    assert_case(
        "a-err",
        "set -u\n(( x = 1 )) 2> $undefined_target_xyz\necho after=$?\n",
        1,
        "",
        "case.sh: line 2: undefined_target_xyz: unbound variable\n",
    );
}

#[test]
fn brace_group_input_redirect_word_is_fatal() {
    assert_case(
        "g-in",
        "set -u\n{ echo hi; } < $undefined_target_xyz\necho after=$?\n",
        1,
        "",
        "case.sh: line 2: undefined_target_xyz: unbound variable\n",
    );
}

#[test]
fn inverted_arith_output_redirect_word_is_fatal() {
    assert_case(
        "a-not",
        "set -u\n! (( x = 1 )) > $undefined_target_xyz\necho after=$?\n",
        1,
        "",
        "case.sh: line 2: undefined_target_xyz: unbound variable\n",
    );
}

// ---- Controls: faces that already agreed with GNU. ----

#[test]
fn brace_group_output_redirect_word_stays_fatal() {
    assert_case(
        "g-out",
        "set -u\n{ echo hi; } > $undefined_target_xyz\necho after=$?\n",
        1,
        "",
        "case.sh: line 2: undefined_target_xyz: unbound variable\n",
    );
}

#[test]
fn arith_output_redirect_word_background_stays_contained() {
    assert_case(
        "a-bg",
        "set -u\n(( x = 1 )) > $undefined_target_xyz &\nwait\necho after=$?\n",
        0,
        "after=0\n",
        "case.sh: line 2: undefined_target_xyz: unbound variable\n",
    );
}

#[test]
fn arith_output_redirect_word_pipeline_stays_contained() {
    assert_case(
        "a-pipe",
        "set -u\n(( x = 1 )) > $undefined_target_xyz | cat\necho after=$?\n",
        0,
        "after=0\n",
        "case.sh: line 2: undefined_target_xyz: unbound variable\n",
    );
}

#[test]
fn arith_output_redirect_word_subshell_stays_contained() {
    assert_case(
        "a-sub",
        "set -u\n( (( x = 1 )) > $undefined_target_xyz; echo inner )\necho after=$?\n",
        0,
        "after=1\n",
        "case.sh: line 2: undefined_target_xyz: unbound variable\n",
    );
}

#[test]
fn simple_builtin_output_redirect_word_stays_fatal() {
    assert_case(
        "b-out",
        "set -u\necho hi > $undefined_target_xyz\necho after=$?\n",
        1,
        "",
        "case.sh: line 2: undefined_target_xyz: unbound variable\n",
    );
}

#[test]
fn null_command_output_redirect_word_stays_fatal() {
    assert_case(
        "null",
        "set -u\n> $undefined_target_xyz\necho after=$?\n",
        1,
        "",
        "case.sh: line 2: undefined_target_xyz: unbound variable\n",
    );
}

#[test]
fn external_output_redirect_word_stays_contained() {
    assert_case(
        "ext",
        "set -u\ncat > $undefined_target_xyz\necho after=$?\n",
        0,
        "after=1\n",
        "case.sh: line 2: undefined_target_xyz: unbound variable\n",
    );
}

#[test]
fn arith_output_redirect_word_quoted_stays_literal() {
    assert_case(
        "a-q",
        "set -u\n(( x = 1 )) > '$undefined_target_xyz'\necho after=$?\n",
        0,
        "after=0\n",
        "",
    );
}

#[test]
fn function_output_redirect_word_stays_fatal() {
    assert_case(
        "fn",
        "set -u\nf() { echo hi; }\nf > $undefined_target_xyz\necho after=$?\n",
        1,
        "",
        "case.sh: line 3: undefined_target_xyz: unbound variable\n",
    );
}
