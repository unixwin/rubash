//! rubash#2281 — `{var}` pending closes carried no position in the
//! redirect list, and the two consumers applied them at the wrong
//! end of the walk:
//!
//! 1. The spawn delivery performed every recorded close up front,
//!    before the ordered walk — `cmd 3<file {v}<&-` (v=3) therefore
//!    delivered the just-bound file at fd 3 instead of the close
//!    (GNU: the close at its position revokes the earlier bind).
//! 2. The redirect gate never saw the closes at all (the applier
//!    only records them, rubash#2272), so `cmd {v}<&- {w}<&9` (v=9)
//!    validated the dup against the still-open ambient entry and
//!    delivered it — GNU's ordered walk (redir.c:237 do_redirections)
//!    fails the dup with EBADF and the command does not run.
//!
//! Fix: records are (position, fd) pairs stamped with the applier
//! run's generation. The gate folds each record into its `opened`
//! overlay exactly once, at the first candidate past its position
//! (a same-list rebind after the close still wins). The spawn
//! delivery applies each close when the walk passes its position,
//! revoking the slot's stamps/pins/binding records
//! (apply_pending_close_at + Executor::apply_fd_var_pending_close).
//!
//! Expectations byte-verified against GNU bash 5.3.20 via
//! probes/p2281.sh and probes/p2281-extra.sh (oracle runs in the
//! cluster workdir). Observers are nested rubash spawns (never
//! ENOEXEC script children, rubash#2279's harness lesson).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2281-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    fs::write(dir.join("seed"), "SEED\n").expect("write seed");
    fs::write(dir.join("f9"), "F9\n").expect("write f9");
    dir
}

fn rubash_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(RUBASH)
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// One shell line reporting descriptor `fd` in the observing child:
/// `fdN=<content>` when open, `fdN=CLOSED` when not.
fn fd_report(fd: u32) -> String {
    format!(
        "if : <&{fd}; then printf \"fd{fd}=%s\\n\" \"$(cat <&{fd})\"; else echo \"fd{fd}=CLOSED\"; fi"
    )
}

/// The observing child: a nested rubash reporting each listed fd.
fn observer(fds: &[u32]) -> String {
    let inner = fds
        .iter()
        .map(|fd| fd_report(*fd))
        .collect::<Vec<_>>()
        .join("; ");
    format!("\"{RUBASH}\" -c '{inner}'")
}

#[test]
fn issue2281_close_after_same_list_bind_revokes_the_bind() {
    let dir = scratch("shape1");
    let script = format!(
        "exec 3<seed; v=3; {} 3<f9 {{v}}<&-; echo \"parent3=$(cat <&3)\"",
        observer(&[3])
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "fd3=CLOSED\nparent3=SEED\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2281_dup_of_just_closed_fd_is_rejected() {
    let dir = scratch("shape2");
    let script = format!(
        "exec 9<seed; v=9; {} {{v}}<&- {{w}}<&9; echo \"rc=$?\"; echo \"parent9=$(cat <&9)\"",
        observer(&[9, 10])
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    // The observer never ran: no fd report lines at all.
    assert_eq!(stdout, "rc=1\nparent9=SEED\n", "stderr: {stderr}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "stderr should carry the dup EBADF diagnostic: {stderr}"
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2281_dup_before_close_is_delivered_then_close_lands() {
    let dir = scratch("control");
    let script = format!("exec 9<seed; v=9; {} {{w}}<&9 {{v}}<&-", observer(&[9, 10]));
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "fd9=CLOSED\nfd10=SEED\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2281_rebind_after_close_wins_for_later_dup() {
    let dir = scratch("shape4");
    let script = format!(
        "exec 9<seed; v=9; {} {{v}}<&- 9<f9 {{w}}<&9",
        observer(&[9, 10])
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "fd9=F9\nfd10=F9\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2281_move_form_close_poisons_later_dup() {
    let dir = scratch("shape5");
    let script = format!(
        "exec 9<seed; {} {{w}}<&9- {{x}}<&9; echo \"rc=$?\"",
        observer(&[9, 10])
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "stderr should carry the dup EBADF diagnostic: {stderr}"
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2281_function_face_close_after_bind_applies_during_call() {
    let dir = scratch("shape6");
    let script = "exec 3<seed; v=3; \
                  f() { if : <&3; then echo \"body3=OPEN\"; else echo \"body3=CLOSED\"; fi; }; \
                  f 3<f9 {v}<&-; echo \"parent3=$(cat <&3)\"";
    let (stdout, _stderr, code) = rubash_in(&dir, script);
    assert_eq!(stdout, "body3=CLOSED\nparent3=SEED\n");
    assert_eq!(code, Some(0));
}
