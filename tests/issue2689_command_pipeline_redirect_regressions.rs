//! rubash#2689 — `command <external>` in a pipeline stage dropped
//! the stage's redirections: `command ls /bad 2>err.txt | cat`
//! created err.txt EMPTY and let the diagnostic escape to the real
//! stderr.
//!
//! GNU applies a pipeline element's redirections once, in the
//! element's fork (execute_cmd.c:2620 execute_pipeline ->
//! redir.c do_redirections), and the command run there inherits
//! those descriptors. The builtin stage strips the stdio redirects
//! from the clone's node so the driver's routing walk can apply
//! them to the captured streams — but the `command` builtin
//! re-dispatches to the external spawn path, which plans its stdio
//! from the clone's fd table, where nothing was bound. Function
//! stages never hit the hole because execute_function applies the
//! call's redirects onto the clone's fd table and the stage claims
//! the pipeline_stage_fds_pre_wired latch. The fix mirrors that
//! composition in execute_builtin_pipeline_stage: bind the
//! element's output redirects into the stage clone with
//! open_compound_output_redirects (list order, dup targets at their
//! position) and claim the latch, so in-process writes and
//! respawned children alike land on the opened targets and no
//! routing pass re-applies (or truncates) them.
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
    let dir = std::env::temp_dir().join(format!("issue2689_{}_{}", std::process::id(), seq));
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

#[test]
fn command_external_stage_stderr_redirect_reaches_the_spawn() {
    let run = run_script("command ls /no-such-xyz 2>err.txt | cat\necho end\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(
        read_file(&run, "err.txt").contains("cannot access '/no-such-xyz'"),
        "err.txt: {}",
        read_file(&run, "err.txt")
    );
    assert_eq!(
        run.stderr, "",
        "nothing may escape to the real stderr; stderr: {}",
        run.stderr
    );
    assert_eq!(run.stdout, "end\n", "stdout: {}", run.stdout);
}

#[test]
fn command_external_stage_stdout_redirect_reaches_the_spawn() {
    let run = run_script("command ls /no-such-xyz 2>/dev/null >out.txt | cat\necho end\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "end\n", "stdout: {}", run.stdout);
    assert_eq!(run.stderr, "", "stderr: {}", run.stderr);
}

#[test]
fn command_external_stage_dup_merges_into_the_pipe() {
    // `2>&1` on the element merges the child's stderr into the
    // stage's stdout — the pipe — in list order.
    let run = run_script("command ls /no-such-xyz 2>&1 | cat\necho end\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(
        run.stdout.contains("cannot access '/no-such-xyz'"),
        "the diagnostic must travel down the pipe; stdout: {}",
        run.stdout
    );
    assert_eq!(run.stderr, "", "stderr: {}", run.stderr);
}

#[test]
fn command_builtin_stage_output_still_routes() {
    // The builtin's own output under the same binding composition:
    // `command echo` writes through the element's fd-1 binding.
    let run = run_script("command echo hello >out.txt | cat\necho end\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(read_file(&run, "out.txt"), "hello\n");
    assert_eq!(run.stdout, "end\n", "stdout: {}", run.stdout);
}

#[test]
fn plain_builtin_stage_redirect_control() {
    // Control shape that already worked through capture routing and
    // must not regress under the binding composition.
    let run = run_script("echo hi 2>err.txt | cat\necho end\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "hi\nend\n", "stdout: {}", run.stdout);
    assert_eq!(read_file(&run, "err.txt"), "");
}
