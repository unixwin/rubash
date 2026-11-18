//! Regression pins for rubash issue #3406: a redirect open failure
//! found by the executor's fd-state validation walk must carry the
//! same class prefix as the post-dispatch redirect paths
//! (rubash#3183's compound_redirect_error_prefix convention).
//!
//! GNU bash 5.3 runs do_redirections before the dispatch stamps any
//! line (execute_cmd.c:854), and error_prolog reads the global
//! line_number — except cm_arith / cm_cond, whose
//! executing_line_number substitutes the node's own line, which for a
//! command completed by a continuation is its COMPLETION line
//! (execute_cmd.c:412-428). The walk used to render with the plain
//! funnel prefix, so a multi-line `(( ... ))` command's failing
//! fd-0 open reported its first line where GNU reports line 2.
//!
//! Expectations are the GNU bash 5.3.20 oracle outputs for the lane
//! probe matrix (fix-diag-residuals probes.py, D-series), compared
//! byte-exact on rc / stdout / stderr with the script file invoked as
//! `rubash case.sh`, so diagnostics carry the bare `case.sh` prefix.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("rubash-issue3406-{tag}-{}", std::process::id()));
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

// ---- Fail-first cells: the walk reported the first line. ----

#[test]
fn arith_multiline_input_redirect_reports_completion_line() {
    assert_case(
        "d01",
        "(( x = 1 \\\n)) < /nonexistent-xyz/file\necho after=$?\n",
        0,
        "after=1\n",
        "case.sh: line 2: /nonexistent-xyz/file: No such file or directory\n",
    );
}

#[test]
fn arith_multiline_input_redirect_in_function_reports_completion_line() {
    assert_case(
        "d08",
        "f() {\n  (( x = 1 \\\n)) < /nonexistent-xyz/file\n}\nf\necho after=$?\n",
        0,
        "after=1\n",
        "case.sh: line 3: /nonexistent-xyz/file: No such file or directory\n",
    );
}

#[test]
fn arith_multiline_input_redirect_in_subshell_reports_completion_line() {
    assert_case(
        "d10",
        "( (( x = 1 \\\n)) < /nonexistent-xyz/file )\necho after=$?\n",
        0,
        "after=1\n",
        "case.sh: line 2: /nonexistent-xyz/file: No such file or directory\n",
    );
}

#[test]
fn arith_multiline_input_redirect_after_assignment_sibling_reports_completion_line() {
    assert_case(
        "d11",
        "x=1\n(( y = 2 \\\n)) < /nonexistent-xyz/file\necho after=$?\n",
        0,
        "after=1\n",
        "case.sh: line 3: /nonexistent-xyz/file: No such file or directory\n",
    );
}

// ---- Controls: faces that already agreed with GNU. ----

#[test]
fn arith_multiline_output_redirect_stays_completion_line() {
    assert_case(
        "d02",
        "(( x = 1 \\\n)) > /nonexistent-xyz/file\necho after=$?\n",
        0,
        "after=1\n",
        "case.sh: line 2: /nonexistent-xyz/file: No such file or directory\n",
    );
}

#[test]
fn if_compound_input_redirect_stays_ambient_line() {
    assert_case(
        "d04",
        "if true; then echo hi; fi < /nonexistent-xyz/file\necho after=$?\n",
        0,
        "after=1\n",
        "case.sh: line 1: /nonexistent-xyz/file: No such file or directory\n",
    );
}

#[test]
fn brace_group_input_redirect_stays_first_line() {
    assert_case(
        "d05",
        "{ echo hi; } < /nonexistent-xyz/file\necho after=$?\n",
        0,
        "after=1\n",
        "case.sh: line 1: /nonexistent-xyz/file: No such file or directory\n",
    );
}

#[test]
fn arith_one_line_input_redirect_stays_line_one() {
    assert_case(
        "d06",
        "(( x = 1 )) < /nonexistent-xyz/file\necho after=$?\n",
        0,
        "after=1\n",
        "case.sh: line 1: /nonexistent-xyz/file: No such file or directory\n",
    );
}

#[test]
fn simple_multiline_output_redirect_stays_first_line() {
    assert_case(
        "d07",
        "echo hi > \\\n/nonexistent-xyz/file\necho after=$?\n",
        0,
        "after=1\n",
        "case.sh: line 1: /nonexistent-xyz/file: No such file or directory\n",
    );
}
