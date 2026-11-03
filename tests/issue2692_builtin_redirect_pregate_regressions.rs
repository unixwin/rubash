//! rubash#2692 — state-mutating builtins EXECUTED despite a failed
//! redirection: the dispatch arms applied their effects first and
//! surfaced the redirect failure afterwards through the buffered
//! writer, so `export ZZ=before >/no-such-dir/f` bound ZZ even though
//! GNU never runs the builtin.
//!
//! GNU execute_builtin_or_function runs do_redirections
//! (execute_cmd.c:5606-5614, RX_ACTIVE|RX_UNDOABLE) BEFORE the builtin
//! body; a failed open undoes the partial redirects and returns
//! EX_REDIRFAIL, and execute_simple_command converts that to a shell
//! exit at EXECUTION_FAILURE for a special builtin in a
//! posixly_correct non-interactive shell (execute_cmd.c:4865-4875).
//! The fix pregates every in-process builtin dispatch with the ordered
//! open walk at the execute_materialized_command funnel — the one
//! point `command`/`builtin` re-dispatch and pipeline-stage
//! re-dispatch also pass — instead of gating per builtin arm.
//!
//! Every expectation below was cross-checked byte-for-byte against
//! GNU bash 5.3.20 (script-file probes, stdout captured through a
//! pipe).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    run_argv(&[], body)
}

fn run_argv(args: &[&str], body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2692_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
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

#[test]
fn export_does_not_bind_when_redirect_open_fails() {
    let run = run_script("export ZZ=before >/no-such-dir/f\necho \"rc=$? ZZ=${ZZ-unset}\"\n");
    assert_eq!(run.stdout, "rc=1 ZZ=unset\n");
    assert_eq!(
        run.stderr,
        "case.sh: line 1: /no-such-dir/f: No such file or directory\n"
    );
}

#[test]
fn readonly_declare_typeset_do_not_bind_when_redirect_open_fails() {
    let run = run_script(
        "readonly RO=before >/no-such-dir/f\necho \"RO=${RO-unset}\"\n\
         declare DC=before >/no-such-dir/f\necho \"DC=${DC-unset}\"\n\
         typeset TS=before >/no-such-dir/f\necho \"TS=${TS-unset}\"\n",
    );
    assert_eq!(run.stdout, "RO=unset\nDC=unset\nTS=unset\n");
}

#[test]
fn local_does_not_bind_when_redirect_open_fails() {
    let run = run_script(
        "f() { local LC=before >/no-such-dir/f; echo \"in rc=$? LC=${LC-unset}\"; }\nf\n",
    );
    assert_eq!(run.stdout, "in rc=1 LC=unset\n");
}

#[test]
fn umask_alias_trap_shopt_ulimit_do_not_mutate_when_redirect_open_fails() {
    let run = run_script(
        "umask 022\numask 077 >/no-such-dir/f\necho \"mask=$(umask)\"\n\
         alias alx='echo hi' >/no-such-dir/f\nalias alx 2>&1 || echo noalias\n\
         trap 'echo TRAPPED' EXIT >/no-such-dir/f\necho body\n\
         shopt -s nullglob >/no-such-dir/f\nshopt -q nullglob && echo ng=on || echo ng=off\n\
         before=$(ulimit -n)\nulimit -n 256 >/no-such-dir/f\n\
         [ \"$(ulimit -n)\" = \"$before\" ] && echo limit=same || echo limit=changed\n",
    );
    // GNU: mask unchanged, alias absent (the query reports
    // `alias: alx: not found` on stdout), no TRAPPED line at exit,
    // nullglob still off, fd limit unchanged.
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines[0], "mask=0022");
    assert!(lines[1].ends_with("alias: alx: not found"), "{}", lines[1]);
    assert_eq!(lines[2], "noalias");
    assert_eq!(lines[3], "body");
    assert_eq!(lines[4], "ng=off");
    assert_eq!(lines[5], "limit=same");
    assert!(!run.stdout.contains("TRAPPED"));
}

#[test]
fn cd_reports_the_failure_once_and_does_not_move() {
    let run = run_script(
        "start=$PWD\ncd /tmp >/no-such-dir/f\necho \"rc=$?\"\n\
         [ \"$PWD\" = \"$start\" ] && echo same=yes || echo same=no\n",
    );
    // Exactly one diagnostic line (the arm used to print its own plus
    // the writer's), the failure status is 1, and the working
    // directory is unchanged.
    assert_eq!(
        run.stderr,
        "case.sh: line 2: /no-such-dir/f: No such file or directory\n"
    );
    assert_eq!(run.stdout, "rc=1\nsame=yes\n");
}

#[test]
fn command_redispatch_is_gated_by_the_outer_redirects() {
    let run = run_script("command export ZZ=before >/no-such-dir/f\necho \"ZZ=${ZZ-unset}\"\n");
    assert_eq!(run.stdout, "ZZ=unset\n");
}

#[test]
fn diagnostic_routes_through_bindings_applied_so_far() {
    // `2>err.txt` precedes the failing entry: GNU's redirection_error
    // is written through the fd-2 binding already in effect.
    let run = run_script("export ZZ=before 2>err.txt >/no-such-dir/f\necho \"ZZ=${ZZ-unset}\"\n");
    assert_eq!(run.stdout, "ZZ=unset\n");
    assert_eq!(run.stderr, "");
    assert_eq!(
        read_file(&run, "err.txt"),
        "case.sh: line 1: /no-such-dir/f: No such file or directory\n"
    );
    // The failing entry precedes `2>err.txt`: the diagnostic stays on
    // the shell's own stderr and err.txt is created empty.
    let run = run_script("export ZZ=before >/no-such-dir/f 2>err.txt\necho \"ZZ=${ZZ-unset}\"\n");
    assert_eq!(run.stdout, "ZZ=unset\n");
    assert_eq!(
        run.stderr,
        "case.sh: line 1: /no-such-dir/f: No such file or directory\n"
    );
    assert_eq!(read_file(&run, "err.txt"), "");
}

#[test]
fn posix_mode_special_builtin_redirect_failure_exits_the_shell() {
    let run = run_argv(
        &["--posix"],
        "export ZZ=before >/no-such-dir/f\necho after\n",
    );
    assert_eq!(run.status, 1);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        "case.sh: line 1: /no-such-dir/f: No such file or directory\n"
    );
}

#[test]
fn successful_redirect_still_runs_the_builtin() {
    let run = run_script("export ZZ=ok >good.txt\necho \"ZZ=$ZZ\"\n");
    assert_eq!(run.stdout, "ZZ=ok\n");
    assert_eq!(read_file(&run, "good.txt"), "");
}
