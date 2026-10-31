//! rubash#2282 — the dynamic-fd applier's `{var}` allocation ignored
//! the command's own numbered redirect slots: `cmd 10<f2 {v}<f1`
//! allocated v=10 in the parent table, squatting the slot the ordered
//! walk was about to bind to f2, so the child's fd 10 collided and the
//! {var} binding landed on the wrong descriptor.
//!
//! GNU's do_redirections is one ordered walk over one descriptor
//! space (redir.c:237): the REDIR_VARASSIGN allocation (redir.c:940,
//! fcntl F_DUPFD with SHELL_FD_BASE) searches the same space, so a
//! numbered entry BEFORE the `{var}` entry already holds its slot
//! (`10<f2 {v}<f1` -> v=11), while a numbered entry AFTER it does not
//! count yet (`{v}<f1 10<f2` -> v=10, and the later bind wins the
//! slot). The spawn-delivery helper (fd_var_delivery_fd) already
//! avoided the command's own slots when computing the child's
//! descriptor; the applier now counts the walk's already-bound slots
//! as occupied in its allocator (FdTable::allocate_dynamic_with_limit_avoiding).
//!
//! Expectations byte-verified against GNU bash 5.3.20 via
//! probes/p2282.sh and probes/p2282-heredoc-min.sh (oracle runs in the
//! cluster workdir). The observing child is a nested rubash (a real
//! spawn) — never an ENOEXEC script child (rubash#2279's harness
//! lesson).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2282-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    fs::write(dir.join("f1"), "F1\n").expect("write f1");
    fs::write(dir.join("f2"), "F2\n").expect("write f2");
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
/// `fdN=<content>` when the descriptor is open (content = first read),
/// `fdN=CLOSED` when it is not.
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
fn issue2282_spawn_numbered_before_var_allocates_above() {
    let dir = scratch("spawn");
    let script = format!(
        "{} 10<f2 {{v}}<f1; echo \"parent-v=${{v-UNSET}}\"",
        observer(&[10, 11])
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(
        stdout, "fd10=F2\nfd11=F1\nparent-v=UNSET\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2282_spawn_no_own_slot_control_allocates_10() {
    let dir = scratch("control10");
    let script = format!("{} {{v}}<f1", observer(&[10, 11]));
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "fd10=F1\nfd11=CLOSED\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2282_spawn_two_vars_keep_increasing_order() {
    let dir = scratch("twovars");
    let script = format!("{} {{a}}<f1 {{b}}<f2", observer(&[10, 11]));
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "fd10=F1\nfd11=F2\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2282_spawn_two_own_slots_allocate_12() {
    let dir = scratch("twoown");
    let script = format!("{} 10<f2 11<f1 {{v}}<f2", observer(&[10, 11, 12]));
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "fd10=F2\nfd11=F1\nfd12=F2\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2282_function_face_sees_var_11_and_both_contents() {
    let dir = scratch("func");
    let script = "g() { echo \"v=$v\"; echo \"f10=$(cat <&10)\"; echo \"f11=$(cat <&11)\"; }; \
                  g 10<f2 {v}<f1";
    let (stdout, stderr, code) = rubash_in(&dir, script);
    assert_eq!(stdout, "v=11\nf10=F2\nf11=F1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2282_var_before_numbered_keeps_10_and_later_bind_wins() {
    // GNU position rule: a numbered entry AFTER the {var} entry does
    // not count at allocation time — v=10, and the slot's final
    // binding is the numbered entry's file.
    let dir = scratch("varfirst");
    let script = format!("{} {{v}}<f1 10<f2", observer(&[10, 11]));
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "fd10=F2\nfd11=CLOSED\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");

    let script = "g() { echo \"v=$v f10=$(cat <&10)\"; }; g {v}<f1 10<f2";
    let (stdout, stderr, code) = rubash_in(&dir, script);
    assert_eq!(stdout, "v=10 f10=F2\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2282_heredoc_var_avoids_own_slot() {
    let dir = scratch("heredoc");
    let script = format!("{} 10<f2 {{v}}<<EOF\nBODY\nEOF", observer(&[10, 11]));
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(stdout, "fd10=F2\nfd11=BODY\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

#[test]
fn issue2282_exec_face_allocates_above_own_slot() {
    let dir = scratch("exec");
    let script =
        "exec 10<f2 {v}<f1; echo \"v=$v\"; echo \"f10=$(cat <&10)\"; echo \"f11=$(cat <&11)\"";
    let (stdout, stderr, code) = rubash_in(&dir, script);
    assert_eq!(stdout, "v=11\nf10=F2\nf11=F1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}
