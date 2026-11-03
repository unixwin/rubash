//! Issue unixwin/rubash#2602: `/dev/fd/N` operand materializations
//! were finished by hand at every call site, and every `?`
//! early-return between materialize and finish leaked the finish —
//! a donor entry a child had consumed survived the aborted pipeline
//! and served its bytes a second time to a later command.
//!
//! The fix is the mechanism, not the sites: the materialization
//! record now carries the deferred finish in its `Drop`
//! (`finish_on_drop`), an explicit finish disarms it, and the call
//! sites place/disarm the record so "armed" means exactly "a child
//! ran against this record".
//!
//! GNU has no materialization layer to pin against: a forked child
//! shares the donor's open file description, so whatever the child
//! read is simply gone from the stream (the single ordered
//! redirection walk, redir.c:237; pipeline children forked per
//! execute_cmd.c). These pins therefore assert the OBSERVABLE
//! equivalence — the bytes a command serves and the donor state a
//! later command observes — with expectations taken from GNU Bash
//! 5.3.20 running the same script files (probes A–D, run side by
//! side during the fix; GNU rc=0 for every script).
//!
//! The census's heredoc-feed window is deliberately NOT pinned:
//! `feed_child_stdin` swallows a broken pipe by design (rubash#1822 —
//! a member that already exited must not fail the pipeline), and the
//! remaining feed failure modes need an OS-level write error a
//! script cannot induce deterministically. The spawn-error window
//! below is the same `?`-between-materialize-and-finish class at the
//! same call site.
//!
//! UNVERIFIED-BY-CAPTAIN: written code-only under the 2026-11-02
//! pace directive; the captain runs these stems centrally.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2602-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn collect(child: std::process::Child) -> (String, String, Option<i32>) {
    let mut child = child;
    let mut out = child.stdout.take().expect("stdout pipe");
    let mut err = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut err, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let status = child.wait().expect("wait rubash");
    (
        out_thread.join().unwrap_or_default(),
        err_thread.join().unwrap_or_default(),
        status.code(),
    )
}

/// Run `script` as rubash's stdin inside `dir`. The directory holds
/// `badinterp.sh`, an executable script whose interpreter does not
/// exist, so spawning it fails at exec time (the stage-1 spawn error
/// probe A needs).
fn run_script(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bad = dir.join("badinterp.sh");
        std::fs::write(&bad, "#!/nonexistent/rubash2602/interp\necho never\n")
            .expect("write badinterp.sh");
        std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o755))
            .expect("chmod badinterp.sh");
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write script");
    drop(child.stdin.take());
    collect(child)
}

/// Probe A — the originally filed shape: a concurrent pipeline whose
/// second stage fails to spawn AFTER the first stage materialized
/// (and its child consumed) the fd 6 donor. The pipeline aborts; GNU
/// leaves the donor drained because the first stage's child really
/// read the shared stream. Pre-fix rubash skipped the first stage's
/// finish on the error return, so the trailing read served the bytes
/// a second time. (The shells' diagnostics for the failed stage
/// differ in wording, so only stdout and the final status are
/// pinned.)
#[test]
#[cfg(unix)]
fn issue2602_spawn_error_abort_still_consumes_donor() {
    let dir = test_dir("spawnerr");
    let (out, _err, code) = run_script(
        &dir,
        "exec 6< <(printf 'AAA\\nBBB\\n')\n\
         cat /dev/fd/6 | ./badinterp.sh\n\
         echo CONTINUED\n\
         cat <&6\n\
         echo END\n",
    );
    assert_eq!(out, "CONTINUED\nEND\n");
    assert_eq!(code, Some(0));
}

/// Probe B — success-path control for the concurrent pipeline: the
/// donor is consumed exactly once (the RAII drop replacing the old
/// hand-called finish loop must not double-consume or skip), the
/// served bytes are the donor's, and the trailing read sees EOF.
#[test]
fn issue2602_successful_pipeline_consumes_donor_exactly_once() {
    let dir = test_dir("pipeok");
    let (out, err, code) = run_script(
        &dir,
        "exec 6< <(printf 'AAA\\nBBB\\n')\n\
         cat /dev/fd/6 | wc -c\n\
         cat <&6\n\
         echo END\n",
    );
    assert_eq!(out, "8\nEND\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Probe C — success-path control for the single-command spawn
/// (`spawn_external_process`, whose finish used to ride the
/// wait-success arm only): the operand serves the donor's bytes and
/// the donor is left consumed exactly once.
#[test]
fn issue2602_single_command_consumes_donor_exactly_once() {
    let dir = test_dir("singleok");
    let (out, err, code) = run_script(
        &dir,
        "exec 6< <(printf 'AAA\\nBBB\\n')\n\
         wc -c /dev/fd/6\n\
         cat <&6\n\
         echo END\n",
    );
    assert_eq!(out, "8 /dev/fd/6\nEND\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Probe D — the mirror guard: when NO child runs against a record,
/// the finish must not fire. With fd 2 rebound, the command-
/// substitution fast path declines after materializing and the full
/// execution path runs the command for real; the capture must hold
/// the donor's full bytes (a decline-time consume would starve the
/// real run) and the real run must leave the donor consumed exactly
/// once (the trailing read sees EOF).
#[test]
fn issue2602_comsub_decline_leaves_donor_for_real_run() {
    let dir = test_dir("comsubdecline");
    let (out, err, code) = run_script(
        &dir,
        "exec 6< <(printf 'AAA\\nBBB\\n')\n\
         exec 2>cap.txt\n\
         x=$(cat /dev/fd/6)\n\
         echo \"[$x]\"\n\
         cat <&6\n\
         echo END\n",
    );
    assert_eq!(out, "[AAA\nBBB]\nEND\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}
