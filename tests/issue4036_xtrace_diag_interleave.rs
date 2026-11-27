use std::fs;
use std::path::PathBuf;
use std::process::Command;

// rubash#4036: under `set -x`, a command's xtrace line (`+ echo`) must be
// emitted before its failing redirection diagnostic (`3: Bad file descriptor`),
// matching GNU `execute_cmd.c` `execute_simple_command`. The `case.sh`
// expectations are byte-exact WSL GNU Bash 5.3.20 outputs for LF script
// files (probed 2026-11-27).

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("rubash-issue4036-{tag}-{}", std::process::id()));
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

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n"),
        output.status.code(),
    )
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

#[test]
fn xtrace_line_precedes_failing_redirect_diagnostic() {
    let (out, err, code) = run_script("set -x\necho $(echo in 1>&2) 1>&3\n");
    assert_eq!(code, Some(1));
    assert_eq!(out, "");
    let lines: Vec<&str> = err.lines().collect();
    assert!(lines.len() >= 4, "unexpected stderr: {err:?}");
    let xtrace_idx = lines
        .iter()
        .position(|line| line.starts_with("+ echo"))
        .expect("missing `+ echo` xtrace line");
    let bad_fd_idx = lines
        .iter()
        .position(|line| line.contains("3: Bad file descriptor"))
        .expect("missing `3: Bad file descriptor` diagnostic");
    assert!(
        xtrace_idx < bad_fd_idx,
        "expected `+ echo` before `3: Bad file descriptor`, got:\n{err}"
    );
}

#[test]
fn posix_function_bad_dup_trace_precedes_diagnostic_and_body_is_not_executed() {
    let script =
        "exec 9>&-\nset -o posix\nset -x\nf() { echo INFUNC; }\nf <&9\necho \"AFTER rc=$?\"\n";
    let (rc, stdout, stderr) = run_case("fn-posix-bad-dup", script);
    assert!(
        !stdout.contains("INFUNC"),
        "function body stdout ran: {stdout:?}"
    );
    assert!(
        !stderr.contains("+ echo INFUNC"),
        "function body xtrace ran: {stderr:?}"
    );
    assert_eq!(rc, 0, "rc\nstdout={stdout:?}\nstderr={stderr:?}");
    assert_eq!(stdout, "AFTER rc=1\n", "stdout\nstderr={stderr:?}");
    assert_eq!(
        stderr, "+ f\ncase.sh: line 5: 9: Bad file descriptor\n+ echo 'AFTER rc=1'\n",
        "stderr\nstdout={stdout:?}"
    );
}

#[test]
fn external_bad_dup_trace_precedes_diagnostic() {
    assert_case(
        "ext-bad-dup",
        "exec 9>&-\nset -x\ncat <&9\necho \"AFTER rc=$?\"\n",
        0,
        "AFTER rc=1\n",
        "+ cat\ncase.sh: line 3: 9: Bad file descriptor\n+ echo 'AFTER rc=1'\n",
    );
}

#[test]
fn function_nounset_fatal_trace_precedes_diagnostic() {
    assert_case(
        "fn-nounset",
        "unset nope\nset -u\nset -x\nf() { echo INFUNC; }\nf > $nope\necho \"AFTER rc=$?\"\n",
        1,
        "",
        "+ f\ncase.sh: line 5: nope: unbound variable\n",
    );
}

#[test]
fn external_nounset_trace_precedes_diagnostic_and_is_contained() {
    assert_case(
        "ext-nounset",
        "unset nope\nset -u\nset -x\ncat > $nope\necho \"AFTER rc=$?\"\n",
        0,
        "AFTER rc=1\n",
        "+ cat\ncase.sh: line 4: nope: unbound variable\n+ echo 'AFTER rc=1'\n",
    );
}

#[test]
fn function_ambiguous_redirect_trace_precedes_diagnostic() {
    assert_case(
        "fn-ambiguous",
        "amb=\nset -x\nf() { echo INFUNC; }\nf > $amb\necho \"AFTER rc=$?\"\n",
        0,
        "AFTER rc=1\n",
        "+ f\ncase.sh: line 4: $amb: ambiguous redirect\n+ echo 'AFTER rc=1'\n",
    );
}

#[test]
fn external_ambiguous_redirect_trace_precedes_diagnostic() {
    assert_case(
        "ext-ambiguous",
        "amb=\nset -x\ncat > $amb\necho \"AFTER rc=$?\"\n",
        0,
        "AFTER rc=1\n",
        "+ cat\ncase.sh: line 3: $amb: ambiguous redirect\n+ echo 'AFTER rc=1'\n",
    );
}

#[test]
fn tempenv_external_bad_subst_redirect_word_without_xtrace() {
    assert_case(
        "tempenv-ext-badsub",
        "a=9 cat > pre'sq'${-x}\necho AFTER rc=$?\n",
        0,
        "AFTER rc=1\n",
        "case.sh: line 1: pre'sq'${-x}: bad substitution\n",
    );
}

#[test]
fn tempenv_external_bad_subst_redirect_word_xtrace_precedes_diagnostic() {
    assert_case(
        "tempenv-ext-badsub-x",
        "set -x\na=9 cat > pre'sq'${-x}\necho AFTER rc=$?\n",
        0,
        "AFTER rc=1\n",
        "+ a=9\n+ cat\ncase.sh: line 2: pre'sq'${-x}: bad substitution\n+ echo AFTER rc=1\n",
    );
}

#[test]
fn tempenv_builtin_bad_subst_redirect_word_without_xtrace() {
    assert_case(
        "tempenv-bi-badsub",
        "a=9 echo hi > pre'sq'${-x}\necho AFTER rc=$?\n",
        0,
        "AFTER rc=1\n",
        "case.sh: line 1: pre'sq'${-x}: bad substitution\n",
    );
}

#[test]
fn tempenv_builtin_bad_subst_redirect_word_xtrace_precedes_diagnostic() {
    assert_case(
        "tempenv-bi-badsub-x",
        "set -x\na=9 echo hi > pre'sq'${-x}\necho AFTER rc=$?\n",
        0,
        "AFTER rc=1\n",
        "+ a=9\n+ echo hi\ncase.sh: line 2: pre'sq'${-x}: bad substitution\n+ echo AFTER rc=1\n",
    );
}
