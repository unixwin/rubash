//! rubash#2718 — a redirect-only (null) command expanded its redirect
//! words BEFORE binding its permanent assignments, so `a=new >
//! ${a}.txt` created the file named by the OLD value.
//!
//! GNU expands a command's words in order during expand_words: the
//! assignment words of a null command bind as they are expanded, and
//! only afterwards does execute_null_command run do_redirections
//! (execute_cmd.c:4626-4635 execute_null_command ->
//! execute_cmd.c:4268-4286; the redirect word itself was expanded at
//! redir.c:298 redirection_expand during that same in-order pass —
//! with the NEW value already bound for the null command, and with
//! the OLD value for a named command, whose assignments are tempenv
//! and bind only after the redirects). The fix moves the
//! reject_invalid_redirects gate behind the assignment bind on both
//! null-command paths (execute_empty_words_command and the standalone
//! assignment branch of the dispatch funnel).
//!
//! Every expectation below was cross-checked byte-for-byte against
//! GNU bash 5.3.20 (script-file probes).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2718_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

fn read_file(run: &Run, name: &str) -> String {
    std::fs::read_to_string(run.dir.join(name)).unwrap_or_default()
}

fn exists(run: &Run, name: &str) -> bool {
    run.dir.join(name).exists()
}

#[test]
fn null_command_binds_assignment_before_expanding_redirect_word() {
    let run = run_script("a=oldname\na=newname > ${a}.txt\necho \"a=$a\"\nls\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("a=newname"), "stdout: {}", run.stdout);
    assert!(
        exists(&run, "newname.txt"),
        "the redirect target must use the NEW value; dir listing via stdout: {}",
        run.stdout
    );
    assert!(
        !exists(&run, "oldname.txt"),
        "the OLD value's file must not be created"
    );
}

#[test]
fn named_command_tempenv_still_expands_redirect_with_old_value() {
    // The opposite order is correct for a named command: its
    // assignments are temporary environment, bound only after
    // do_redirections, so the redirect word sees the old value.
    let run = run_script("a=oldname\na=newname /usr/bin/true > ${a}.txt\nls\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(exists(&run, "oldname.txt"));
    assert!(!exists(&run, "newname.txt"));
}

#[test]
fn null_command_assignment_survives_a_failing_redirect() {
    // The bind happens during expansion, before do_redirections runs
    // (and fails): the variable keeps its new value.
    let run = run_script("a=oldname\na=newname >/no-such-dir/f\necho \"rc=$? a=$a\"\n");
    assert!(run.stdout.contains("a=newname"), "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("/no-such-dir/f"),
        "the redirect failure must still be diagnosed; stderr: {}",
        run.stderr
    );
    let _ = read_file(&run, "unused");
}
