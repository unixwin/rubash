//! rubash#2984 regressions (the `$(host)` face): a function was
//! invisible inside `$(...)` when the comsub was a double-quoted
//! whole-word simple-command argument — the external program ran
//! instead (GNU dispatch order is function > builtin > PATH:
//! execute_cmd.c execute_simple_command -> find_function before the
//! PATH search; subst.c:7143 command_substitute -> parse_and_execute
//! uses the same dispatch). The `&self` comsub evaluator ran its
//! trivial-body shortcut family (echo/printf/cat/basename/pwd/type/
//! kill/mktemp, the hardcoded `true`/`false`/`:` arms) and its
//! external dispatch — which never consults the function table —
//! before the real-executor fallback that would have found the
//! function. Unquoted, embedded, and assignment comsubs took the
//! typed path, whose function-call shortcut runs first, and worked.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2984-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn check(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "script failed: {script}\nstderr: {stderr}");
    assert_eq!(stderr, "", "script {script} wrote to stderr");
    assert_eq!(stdout, expected_stdout, "script: {script}");
}

#[test]
fn issue2984_double_quoted_function_comsub_prefers_function() {
    check(
        r#"host() { echo HOSTFN; }
date() { echo DATEFN; }
echo "$(host)"
echo "$(date)"
"#,
        "HOSTFN\nDATEFN\n",
    );
}

#[test]
fn issue2984_double_quoted_function_comsub_shadowing_builtin() {
    check(
        r#"true() { echo TRUEFN; }
pwd() { echo PWDFN; }
echo "$(true)"
echo "$(pwd)"
"#,
        "TRUEFN\nPWDFN\n",
    );
}

#[test]
fn issue2984_unquoted_and_assignment_comsubs_still_work() {
    check(
        r#"host() { echo HOSTFN; }
echo $(host)
v="$(host)"
echo "$v"
"#,
        "HOSTFN\nHOSTFN\n",
    );
}
