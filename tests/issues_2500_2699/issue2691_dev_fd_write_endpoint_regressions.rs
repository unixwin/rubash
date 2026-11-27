//! rubash#2691 — `>&/dev/fd/N` and `>&/dev/stderr` failed with
//! ENOENT across the command faces: the output-target opener's
//! dev-stdio ladder handled file and process-substitution endpoints,
//! but any other write endpoint already bound to the source fd (a
//! pipe, the process stdio channels, a coproc) fell into the ENOENT
//! arm.
//!
//! GNU opens the magic link (redir.c:725 redir_open): the open
//! resolves to whatever the named descriptor is bound to. The fix
//! completes the ladder's final match in open_command_output_target
//! by mapping every remaining FdWriteEndpoint to its walk-level
//! OutputTarget twin — the same mapping command_output_fd_state
//! seeds for dup sources. (A `>&word` whose word is an fd-alias
//! spelling keeps r_err_and_out OPEN semantics, redir.c:832-838 —
//! the discriminator shapes below pin that the bytes follow the
//! source fd's CURRENT binding, not a dup snapshot.)
//!
//! Every expectation below was cross-checked byte-for-byte against
//! GNU bash 5.3.20 (script-file probes, stdout captured through a
//! pipe — with a regular-file stdout the pre-fix shape was masked).

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
    let dir = std::env::temp_dir().join(format!("issue2691_{}_{}", std::process::id(), seq));
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
fn combined_redirect_to_dev_fd_spellings_succeeds() {
    let run = run_script(
        "echo x >&/dev/fd/1\necho \"rc=$?\"\necho y >&/dev/stderr\necho \"rc=$?\"\necho z >&/dev/stdout\necho \"rc=$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("x\n"), "stdout: {}", run.stdout);
    assert!(run.stdout.contains("z\n"), "stdout: {}", run.stdout);
    assert_eq!(
        run.stdout.matches("rc=0").count(),
        3,
        "stdout: {}",
        run.stdout
    );
    assert!(run.stderr.contains("y\n"), "stderr: {}", run.stderr);
}

#[test]
fn explicit_fd_combined_redirect_to_dev_spelling() {
    let run = run_script("echo w 1>&/dev/fd/1\necho \"rc=$?\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("w\n"), "stdout: {}", run.stdout);
    assert!(run.stdout.contains("rc=0"), "stdout: {}", run.stdout);
}

#[test]
fn combined_alias_redirect_opens_both_streams_onto_the_target() {
    // `>&word` with an fd-alias spelling is r_err_and_out OPEN
    // semantics (redir.c:832-838): BOTH streams open onto the named
    // descriptor's current target. With fd 2 bound to err.txt by
    // exec, the function's `>&/dev/fd/1` rebinds both its streams
    // onto fd 1's target (the pipe): O and E both land on stdout
    // and err.txt stays empty. (Pre-fix this face failed ENOENT;
    // a dup implementation would have left E in err.txt.)
    let run = run_script(
        "f() { echo O; echo E >&2; }\nexec 2>err.txt\nf >&/dev/fd/1\nexec 2>&-\necho \"errfile:[$(cat err.txt)]\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "O\nE\nerrfile:[]\n", "stdout: {}", run.stdout);
    assert_eq!(read_file(&run, "err.txt"), "");
}

#[test]
fn combined_alias_redirect_on_a_group() {
    // The same combined open on a brace group: OUT and ERR both
    // follow the open onto fd 1's target.
    let run = run_script(
        "exec 2>err.txt\n{ echo OUT; echo ERR >&2; } >&/dev/fd/1\nexec 2>&-\necho \"errfile:[$(cat err.txt)]\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "OUT\nERR\nerrfile:[]\n",
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn plain_redirect_still_splits_the_streams() {
    // Control: with a plain `>plain.txt` the streams split as
    // usual — stdout to the file, stderr keeps its exec binding.
    let run = run_script(
        "f() { echo O; echo E >&2; }\nexec 2>err.txt\nf >plain.txt\nexec 2>&-\necho \"errfile:[$(cat err.txt)] plain:[$(cat plain.txt)]\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(read_file(&run, "err.txt"), "E\n");
    assert_eq!(read_file(&run, "plain.txt"), "O\n");
}
