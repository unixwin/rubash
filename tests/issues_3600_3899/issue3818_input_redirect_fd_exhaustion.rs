//! rubash#3818: input-redirect failures under RLIMIT_NOFILE fd exhaustion.
//!
//! Face 1: `exec N<file` / `exec N>file` / `exec N>>file` when NO fd is free
//! below the RLIMIT_NOFILE soft limit must fail on the open(2) error naming
//! the target (`/dev/null: Too many open files`), NOT `N: Bad file descriptor`.
//! When a free fd DOES exist below the limit, open succeeds and dup2 fails
//! with `N: Bad file descriptor`.
//!
//! Face 2: `< /dev/null` on an external command (including emulated `cat`)
//! must fail with `/dev/null: Too many open files` (rc 1) instead of being
//! silently dropped, and `< /dev/null` on a builtin (`read x < /dev/null`)
//! must include the filename in the diagnostic (`/dev/null: Too many open files`).

#[cfg(unix)]
mod unix_tests {
    use std::process::{Command, Stdio};

    fn run_script(script: &str) -> (String, String, Option<i32>) {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(script)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("spawn rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }

    #[test]
    fn exec_4_lt_dev_null_under_exhaustion_names_file() {
        let (stdout, stderr, rc) =
            run_script("ulimit -n 4; exec 3</dev/null; exec 4</dev/null; echo rc=$?");
        assert_eq!(rc, Some(0));
        assert_eq!(stdout.trim(), "rc=1");
        assert!(
            stderr.contains("/dev/null: Too many open files"),
            "stderr: {stderr}"
        );
    }

    #[test]
    fn exec_4_gt_and_append_dev_null_under_exhaustion_name_file() {
        let (stdout, stderr, rc) =
            run_script("ulimit -n 4; exec 3</dev/null; exec 4>/dev/null; echo rc=$?");
        assert_eq!(rc, Some(0));
        assert_eq!(stdout.trim(), "rc=1");
        assert!(
            stderr.contains("/dev/null: Too many open files"),
            "stderr: {stderr}"
        );

        let (stdout, stderr, rc) =
            run_script("ulimit -n 4; exec 3</dev/null; exec 4>>/dev/null; echo rc=$?");
        assert_eq!(rc, Some(0));
        assert_eq!(stdout.trim(), "rc=1");
        assert!(
            stderr.contains("/dev/null: Too many open files"),
            "stderr: {stderr}"
        );
    }

    #[test]
    fn exec_3_lt_under_limit_3_names_file() {
        let (stdout, stderr, rc) = run_script("ulimit -n 3; exec 3</dev/null; echo rc=$?");
        assert_eq!(rc, Some(0));
        assert_eq!(stdout.trim(), "rc=1");
        assert!(
            stderr.contains("/dev/null: Too many open files"),
            "stderr: {stderr}"
        );
    }

    #[test]
    fn ladder_7_names_file_on_failing_rung() {
        let (stdout, stderr, rc) = run_script(
            "ulimit -n 7; exec 3</dev/null; exec 4</dev/null; exec 5</dev/null; exec 6</dev/null; exec 7</dev/null; echo rc=$?",
        );
        assert_eq!(rc, Some(0));
        assert_eq!(stdout.trim(), "rc=1");
        assert!(
            stderr.contains("/dev/null: Too many open files"),
            "stderr: {stderr}"
        );
    }

    #[test]
    fn external_cat_lt_dev_null_under_exhaustion_refused() {
        let (stdout, stderr, rc) =
            run_script("ulimit -n 4; exec 3</dev/null; cat < /dev/null; echo rc=$?");
        assert_eq!(rc, Some(0));
        assert_eq!(stdout.trim(), "rc=1");
        assert!(
            stderr.contains("/dev/null: Too many open files"),
            "stderr: {stderr}"
        );
    }

    #[test]
    fn external_command_lt_and_numbered_dev_null_under_exhaustion_name_file() {
        let (stdout, stderr, rc) =
            run_script("ulimit -n 4; exec 3</dev/null; /bin/true < /dev/null; echo rc=$?");
        assert_eq!(rc, Some(0));
        assert_eq!(stdout.trim(), "rc=1");
        assert!(
            stderr.contains("/dev/null: Too many open files"),
            "stderr: {stderr}"
        );

        let (stdout, stderr, rc) =
            run_script("ulimit -n 4; exec 3</dev/null; /bin/true 4</dev/null; echo rc=$?");
        assert_eq!(rc, Some(0));
        assert_eq!(stdout.trim(), "rc=1");
        assert!(
            stderr.contains("/dev/null: Too many open files"),
            "stderr: {stderr}"
        );

        let (stdout, stderr, rc) =
            run_script("ulimit -n 4; exec 3</dev/null; /bin/true 4>/dev/null; echo rc=$?");
        assert_eq!(rc, Some(0));
        assert_eq!(stdout.trim(), "rc=1");
        assert!(
            stderr.contains("/dev/null: Too many open files"),
            "stderr: {stderr}"
        );
    }

    #[test]
    fn builtin_read_lt_dev_null_under_exhaustion_names_file() {
        let (stdout, stderr, rc) =
            run_script("ulimit -n 4; exec 3</dev/null; read x < /dev/null; echo \"rc=$? x=[$x]\"");
        assert_eq!(rc, Some(0));
        assert_eq!(stdout.trim(), "rc=1 x=[]");
        assert!(
            stderr.contains("/dev/null: Too many open files"),
            "stderr: {stderr}"
        );
    }

    #[test]
    fn controls_free_fd_missing_file_reopen_and_no_limit() {
        // Control 7: free fd below limit -> open succeeds, dup2 fails EBADF.
        let (stdout, stderr, _) = run_script("ulimit -n 4; exec 4</dev/null; echo rc=$?");
        assert_eq!(stdout.trim(), "rc=1");
        assert!(
            stderr.contains("4: Bad file descriptor"),
            "stderr: {stderr}"
        );

        // Control 8: missing file -> ENOENT.
        let (stdout, stderr, _) = run_script("ulimit -n 4; exec 4</nonexistent; echo rc=$?");
        assert_eq!(stdout.trim(), "rc=1");
        assert!(
            stderr.contains("No such file or directory"),
            "stderr: {stderr}"
        );

        // Control 10: exec 9< with no limit works.
        let (stdout, stderr, _) = run_script("exec 9</dev/null; echo rc=$?");
        assert_eq!(stdout.trim(), "rc=0");
        assert_eq!(stderr, "");

        // Control 11: re-opening already-open fd under exhaustion fails EMFILE.
        let (stdout, stderr, _) =
            run_script("ulimit -n 4; exec 3</dev/null; exec 3</dev/null; echo rc=$?");
        assert_eq!(stdout.trim(), "rc=1");
        assert!(
            stderr.contains("/dev/null: Too many open files"),
            "stderr: {stderr}"
        );
    }
}
