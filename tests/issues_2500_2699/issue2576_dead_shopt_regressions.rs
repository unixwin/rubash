//! rubash#2576 — dead shopt options: settable, reported on, gating
//! nothing. This pin covers the items implemented from GNU source:
//!
//! * progcomp (GNU bashline.c:1681 `prog_completion_enabled` gates the
//!   programmable-completion attempt): `shopt -u progcomp` must turn
//!   compspecs AND their -F hooks off, leaving plain completion.
//! * restricted_shell (GNU builtins/shopt.def:750-757
//!   set_restricted_shell): the value is the invocation state; shopt
//!   attempts to change it are silently forced back, status 0.
//! * no_empty_cmd_completion (bashline.c:1879-1884): completing an
//!   empty command word yields no matches when on.
//! * force_fignore / FIGNORE (bashline.c:3065-3091
//!   filename_completion_ignore): FIGNORE suffixes are excluded from
//!   readline filename completion; the exclusion empties the list only
//!   under force_fignore (default on). GNU's compgen does NOT apply
//!   FIGNORE (oracle-verified), so compgen output is pinned unchanged.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn run_script(executor: &mut Executor, script: &str) {
    let tokens = tokenize(script);
    let ast = parse(&tokens);
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok(), "script failed: {script:?} -> {result:?}");
}

fn rubash(script: &str, cwd: &std::path::Path) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(cwd)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue_2576_progcomp_off_disables_compspecs_and_hooks() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "_f() { COMPREPLY=(hooked); }\ncomplete -F _f foo\ncomplete -W 'bar baz' qux",
    );
    assert_eq!(
        executor.complete_line("qux ba", 6),
        vec!["bar".to_string(), "baz".to_string()]
    );
    assert_eq!(
        executor.complete_line("foo ", 4),
        vec!["hooked".to_string()]
    );
    run_script(&mut executor, "shopt -u progcomp");
    // Compspec ignored: `qux ba` falls back to file completion (the
    // wordlist no longer applies) and the -F hook never runs.
    let fallback = executor.complete_line("qux ba", 6);
    assert!(
        !fallback.iter().any(|c| c == "bar" || c == "baz"),
        "wordlist must be gone with progcomp off: {fallback:?}"
    );
    let fallback = executor.complete_line("foo ", 4);
    assert!(
        !fallback.iter().any(|c| c == "hooked"),
        "the -F hook must not run with progcomp off: {fallback:?}"
    );
}

#[test]
fn issue_2576_no_empty_cmd_completion() {
    let mut executor = Executor::new();
    // Default off: an empty command word completes command names.
    assert!(!executor.complete_line("", 0).is_empty());
    run_script(&mut executor, "shopt -s no_empty_cmd_completion");
    assert!(executor.complete_line("", 0).is_empty());
}

#[test]
fn issue_2576_restricted_shell_is_invocation_state() {
    let dir = std::env::temp_dir();
    // Not invoked restricted: -s is a silent no-op (rc 0), value stays
    // off, and $BASHOPTS must not claim it.
    let (stdout, stderr, code) = rubash(
        "shopt -s restricted_shell; echo rc=$?; shopt -q restricted_shell; echo q=$?; case :$BASHOPTS: in *:restricted_shell:*) echo bashopts=BAD;; *) echo bashopts=ok;; esac",
        &dir,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\nq=1\nbashopts=ok\n", "stderr: {stderr}");
    let (stdout, stderr, code) = rubash(
        "shopt -u restricted_shell; shopt -q restricted_shell; echo q=$?",
        &dir,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "q=1\n", "stderr: {stderr}");
}

#[test]
fn issue_2576_restricted_shell_on_under_restricted_invocation() {
    // Invoked with -r: the value is on, an -u attempt is forced back,
    // and $BASHOPTS reports it (it is genuinely in effect).
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-r", "-c"])
        .arg("shopt -q restricted_shell; echo q0=$?; shopt -u restricted_shell; shopt -q restricted_shell; echo q1=$?; case :$BASHOPTS: in *:restricted_shell:*) echo bashopts=on;; *) echo bashopts=BAD;; esac")
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash -r");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "q0=0\nq1=0\nbashopts=on\n", "stderr: {stderr}");
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(files: &[&str]) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-2576-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    for file in files {
        std::fs::write(dir.join(file), "x").expect("write file");
    }
    dir
}

#[test]
fn issue_2576_fignore_filters_readline_completion_not_compgen() {
    let dir = scratch(&["a.o", "a.c", "b.o"]);
    let mut executor = Executor::new();
    run_script(&mut executor, "FIGNORE='.o'");
    // Absolute directory prefix in the word: no process-cwd dependence.
    let line = format!("ls {}/", dir.to_str().expect("utf8 path"));
    let listed = executor.complete_line(&line, line.len());
    assert!(
        listed.iter().any(|c| c.ends_with("a.c")),
        "kept: {listed:?}"
    );
    assert!(
        !listed.iter().any(|c| c.ends_with(".o")),
        "filtered: {listed:?}"
    );
    // compgen does NOT apply FIGNORE (GNU oracle): all files listed.
    let (stdout, stderr, code) = rubash("FIGNORE='.o' compgen -A file | sort", &dir);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "a.c\na.o\nb.o\n", "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue_2576_force_fignore_controls_emptying() {
    let dir = scratch(&["x.o", "y.o"]);
    let mut executor = Executor::new();
    run_script(&mut executor, "FIGNORE='.o'");
    let line = format!("ls {}/", dir.to_str().expect("utf8 path"));
    // force_fignore defaults on: the exclusion may empty the list.
    assert!(executor.complete_line(&line, line.len()).is_empty());
    run_script(&mut executor, "shopt -u force_fignore");
    // Off: a list that would be emptied survives whole.
    let listed = executor.complete_line(&line, line.len());
    assert_eq!(listed.len(), 2, "surviving list: {listed:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
