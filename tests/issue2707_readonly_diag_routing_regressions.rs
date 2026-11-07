//! rubash#2707 (A3-R1, Stage-2 ④ Phase 4) — readonly-variable
//! diagnostics must be written through the command's own stderr
//! redirection, not escape to the shell's real stderr.
//!
//! GNU ground: the bind layer's readonly verdict is
//! `err_readonly` (error.c:453, report_error family) for
//! read/arithmetic/let binds and `sh_readonly`
//! (builtins/common.c:240, builtin_error family) for the
//! declare-family builtins; both write to fd 2 AS BOUND by the
//! command's redirections (execute_cmd.c do_redirections runs
//! before the builtin/evaluator). So `(( a = 6 )) 2>&1` lands the
//! diagnostic on stdout, and `read v 2>f` lands it in f with the
//! shell's real stderr empty.
//!
//! On the Phase 4 base (c095a5e2) the arithmetic/let cells fail:
//! `report_arithmetic_readonly_error` printed with a raw
//! `eprintln!`, bypassing the fd-2 routing (`arith_diag` =
//! `write_diagnostic_fd2`) every sibling arithmetic diagnostic
//! uses. The read/declare/export/plain-assignment cells are the
//! control half — they already route through the buffered
//! builtin IO and must not regress.

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
    let dir = std::env::temp_dir().join(format!("issue2707_{}_{}", std::process::id(), seq));
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

fn file(run: &Run, name: &str) -> String {
    std::fs::read_to_string(run.dir.join(name)).unwrap_or_else(|_| "<missing>".to_string())
}

#[test]
fn arith_readonly_diag_follows_dup_to_stdout() {
    let run = run_script("readonly a=5\n(( a = 6 )) 2>&1\necho \"rc=$? a=$a\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "case.sh: line 2: a: readonly variable\nrc=1 a=5\n",
        "stdout: {}",
        run.stdout
    );
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
}

#[test]
fn let_readonly_diag_follows_dup_to_stdout() {
    let run = run_script("readonly a=5\nlet 'a = 6' 2>&1\necho \"rc=$? a=$a\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "case.sh: line 2: a: readonly variable\nrc=1 a=5\n",
        "stdout: {}",
        run.stdout
    );
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
}

#[test]
fn arith_readonly_diag_follows_file_redirect() {
    let run = run_script("readonly a=5\n(( a = 6 )) 2>err.txt\necho \"rc=$? a=$a\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=1 a=5\n", "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(
        file(&run, "err.txt"),
        "case.sh: line 2: a: readonly variable\n"
    );
}

#[test]
fn let_readonly_diag_follows_file_redirect() {
    let run = run_script("readonly a=5\nlet 'a = 6' 2>err.txt\necho \"rc=$? a=$a\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=1 a=5\n", "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(
        file(&run, "err.txt"),
        "case.sh: line 2: a: readonly variable\n"
    );
}

#[test]
fn arith_readonly_diag_without_redirect_still_reported() {
    // Control: no redirection — the diagnostic reaches the shell's
    // stderr exactly once, status 1, value unchanged.
    let run = run_script("readonly a=5\n(( a = 6 ))\necho \"rc=$? a=$a\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=1 a=5\n", "stdout: {}", run.stdout);
    assert_eq!(
        run.stderr, "case.sh: line 2: a: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn read_scalar_readonly_diag_lands_in_redirect_file() {
    let run =
        run_script("readonly v=old\nprintf 'new\\n' | read v 2>err.txt\necho \"rc=$? v=$v\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=1 v=old\n", "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(
        file(&run, "err.txt"),
        "case.sh: line 2: v: readonly variable\n"
    );
}

#[test]
fn read_array_readonly_diag_lands_in_redirect_file() {
    let run = run_script(
        "declare -a arr=(x y)\nreadonly arr\nprintf 'p q\\n' | read -a arr 2>err.txt\necho \"rc=$? arr=${arr[*]}\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=1 arr=x y\n", "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(
        file(&run, "err.txt"),
        "case.sh: line 3: arr: readonly variable\n"
    );
}

#[test]
fn declare_assign_readonly_diag_lands_in_redirect_file() {
    let run = run_script("readonly v=old\ndeclare v=new 2>err.txt\necho \"rc=$? v=$v\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=1 v=old\n", "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(
        file(&run, "err.txt"),
        "case.sh: line 2: declare: v: readonly variable\n"
    );
}

#[test]
fn export_assign_readonly_diag_lands_in_redirect_file() {
    // GNU: export's bind goes through bind_variable, so the
    // diagnostic is the err_readonly shape — no builtin name —
    // but it DOES honor the builtin's `2>` (unlike a bare
    // assignment statement, next cell).
    let run = run_script("readonly v=old\nexport v=new 2>err.txt\necho \"rc=$? v=$v\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=1 v=old\n", "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(
        file(&run, "err.txt"),
        "case.sh: line 2: v: readonly variable\n"
    );
}

#[test]
fn plain_assignment_readonly_diag_ignores_statement_redirect() {
    // GNU ground (oracle-verified): a bare assignment statement's
    // redirections are never even opened for the assignment's own
    // diagnostic — the file is not created and the diagnostic
    // reaches the shell's real stderr. Control cell: this shape
    // must NOT start honoring the redirect.
    let run = run_script("readonly v=old\nv=new 2>err.txt\necho \"rc=$? v=$v\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=1 v=old\n", "stdout: {}", run.stdout);
    assert_eq!(
        run.stderr, "case.sh: line 2: v: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    assert!(
        !run.dir.join("err.txt").exists(),
        "err.txt must not be created"
    );
}

#[test]
fn local_readonly_assign_diag_ignores_statement_redirect() {
    // Same assignment-statement shape inside a function: GNU
    // aborts the function body at the failed bind, the redirect
    // file is never created, the diagnostic hits the shell's
    // stderr at the assignment's own line.
    let run = run_script(
        "f() { local lv=1; readonly lv; lv=2 2>err.txt; echo \"in rc=$? lv=$lv\"; }\nf\necho \"after f\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "after f\n", "stdout: {}", run.stdout);
    assert_eq!(
        run.stderr, "case.sh: line 1: lv: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    assert!(
        !run.dir.join("err.txt").exists(),
        "err.txt must not be created"
    );
}
