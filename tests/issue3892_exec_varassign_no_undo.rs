//! rubash#3892: GNU `execute_builtin_or_function` runs
//! `do_redirections(RX_ACTIVE|RX_UNDOABLE)` (`execute_cmd.c:5606`), and
//! `redir.c:972/1091/1153` registers NO undo record for `REDIR_VARASSIGN`
//! (unless `varredir_close` is enabled). Consequently, `{var}` entries
//! positioned before a failing redirect entry survive, `{var}` entries
//! positioned after the failing entry never run, and bare redirect-only
//! commands without words do not keep `{var}`.

use std::path::PathBuf;
use std::process::Command;

fn run_script(name: &str, script: &str) -> (String, String, Option<i32>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash-issue3892-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create probe directory");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        dir,
    )
}

#[test]
fn issue3892_exec_varassign_heredoc_before_limit_failure_survives() {
    let (stdout, stderr, code, _dir) = run_script(
        "exec-heredoc",
        "ulimit -n 16\n\
         exec {v}<<A 16<<B\n\
         body-a\n\
         A\n\
         body-b\n\
         B\n\
         rc=$?\n\
         read -u \"$v\" line\n\
         echo \"rc=$rc v=$v line=$line\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 v=10 line=body-a\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "stderr={stderr:?}"
    );
}

#[test]
fn issue3892_exec_varassign_output_before_limit_failure_survives() {
    let (stdout, stderr, code, dir) = run_script(
        "exec-output-limit",
        "ulimit -n 16\n\
         exec {v}>fu 16<<B\n\
         body-b\n\
         B\n\
         rc=$?\n\
         echo written >&\"$v\"\n\
         exec {v}>&-\n\
         echo \"rc=$rc v=$v content=$(cat fu)\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 v=10 content=written\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "stderr={stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue3892_exec_varassign_output_before_missing_input_survives() {
    let (stdout, stderr, code, dir) = run_script(
        "exec-output-missing",
        "exec {v}>fu < /nonexistent_issue3892_missing\n\
         rc=$?\n\
         echo written >&\"$v\"\n\
         exec {v}>&-\n\
         echo \"rc=$rc v=$v content=$(cat fu)\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 v=10 content=written\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("/nonexistent_issue3892_missing: No such file or directory"),
        "stderr={stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue3892_exec_numbered_undone_and_varassign_between_survives() {
    let (stdout, stderr, code, _dir) = run_script(
        "exec-mixed",
        "ulimit -n 16\n\
         exec 15<<A {w}<<B 16<<C\n\
         body-a\n\
         A\n\
         body-b\n\
         B\n\
         body-c\n\
         C\n\
         rc=$?\n\
         read -u 15 a_line 2>/dev/null; a_rc=$?\n\
         read -u \"$w\" b_line\n\
         echo \"rc=$rc a_rc=$a_rc w=$w b_line=$b_line\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(
        stdout, "rc=1 a_rc=1 w=10 b_line=body-b\n",
        "stdout={stdout:?}"
    );
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "stderr={stderr:?}"
    );
}

#[test]
fn issue3892_exec_varassign_after_failure_stays_unset() {
    let (stdout, stderr, code, dir) = run_script(
        "exec-after-fail",
        "ulimit -n 16\n\
         exec 16<<B {v}<<A\n\
         body-b\n\
         B\n\
         body-a\n\
         A\n\
         rc1=$?\n\
         exec </nonexistent_issue3892_missing {w}>fu\n\
         rc2=$?\n\
         echo \"rc1=$rc1 v=${v-UNSET} rc2=$rc2 w=${w-UNSET}\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc1=1 v=UNSET rc2=1 w=UNSET\n", "stdout={stdout:?}");
    assert!(
        !dir.join("fu").exists(),
        "fu must not be created after failing </missing"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue3892_exec_with_operand_varassign_before_failure_survives() {
    let (stdout, stderr, code, _dir) = run_script(
        "exec-operand",
        "ulimit -n 16\n\
         exec {v}<<A 16<<B /bin/true\n\
         body-a\n\
         A\n\
         body-b\n\
         B\n\
         rc=$?\n\
         read -u \"$v\" line\n\
         echo \"rc=$rc v=$v line=$line\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 v=10 line=body-a\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "stderr={stderr:?}"
    );
}

#[test]
fn issue3892_non_exec_builtin_varassign_before_and_after_failure() {
    let (stdout, stderr, code, dir) = run_script(
        "non-exec-builtin",
        "ulimit -n 16\n\
         echo notreached1 {v}<<A 16<<B\n\
         body-a\n\
         A\n\
         body-b\n\
         B\n\
         rc1=$?\n\
         read -u \"$v\" line\n\
         echo notreached2 16<<B {w}<<A\n\
         body-b\n\
         B\n\
         body-a\n\
         A\n\
         rc2=$?\n\
         echo notreached3 </nonexistent_issue3892_missing {z}>fu\n\
         rc3=$?\n\
         echo \"rc1=$rc1 v=$v line=$line rc2=$rc2 w=${w-UNSET} rc3=$rc3 z=${z-UNSET}\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(
        stdout, "rc1=1 v=10 line=body-a rc2=1 w=UNSET rc3=1 z=UNSET\n",
        "stdout={stdout:?}"
    );
    assert!(
        !dir.join("fu").exists(),
        "fu must not be created after failing </missing"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue3892_function_varassign_before_and_after_failure() {
    let (stdout, stderr, code, _dir) = run_script(
        "function",
        "ulimit -n 16\n\
         f() { echo body-ran; }\n\
         f {v}<<A 16<<B\n\
         body-a\n\
         A\n\
         body-b\n\
         B\n\
         rc1=$?\n\
         read -u \"$v\" line\n\
         f 16<<B {w}<<A\n\
         body-b\n\
         B\n\
         body-a\n\
         A\n\
         rc2=$?\n\
         echo \"rc1=$rc1 v=$v line=$line rc2=$rc2 w=${w-UNSET}\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(
        stdout, "rc1=1 v=10 line=body-a rc2=1 w=UNSET\n",
        "stdout={stdout:?}"
    );
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "stderr={stderr:?}"
    );
}

#[test]
fn issue3892_bare_redirect_only_without_words_does_not_keep_var() {
    let (stdout, stderr, code, _dir) = run_script(
        "bare-no-words",
        "ulimit -n 16\n\
         {v}<<A 16<<B\n\
         body-a\n\
         A\n\
         body-b\n\
         B\n\
         rc=$?\n\
         echo \"rc=$rc v=${v-UNSET}\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 v=UNSET\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "stderr={stderr:?}"
    );
}

#[test]
fn issue3892_varredir_close_closes_surviving_varassign_on_failure() {
    let (stdout, stderr, code, _dir) = run_script(
        "varredir-close",
        "ulimit -n 16\n\
         shopt -s varredir_close\n\
         echo notreached {v}<<A 16<<B\n\
         body-a\n\
         A\n\
         body-b\n\
         B\n\
         rc=$?\n\
         read -u \"$v\" line 2>/dev/null; read_rc=$?\n\
         echo \"rc=$rc v=$v read_rc=$read_rc\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 v=10 read_rc=1\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "stderr={stderr:?}"
    );
}
