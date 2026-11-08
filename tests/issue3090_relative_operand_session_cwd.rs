//! Regression pins for rubash#3090 — the file-operand builtins
//! opened a relative filename against the PROCESS cwd instead of
//! the session cwd, so after `cd` the file sitting in the current
//! directory was reported missing (`bind -f` rc 1 "cannot read";
//! `history -r` rc 1 silent) or written to the wrong directory
//! (`history -w` created the file in the process cwd).
//!
//! GNU ownership: GNU has one cwd, and it is the session's.
//! builtins/bind.def:274 hands the `-f` operand to
//! rl_read_init_file, which opens it verbatim (lib/readline/
//! bind.c:1079 _rl_read_init_file; `$include` names go through the
//! same open at bind.c:1502 parser_include). builtins/history.def:273
//! resolves the filename (operand, else $HISTFILE) and hands it
//! verbatim to maybe_append_history (history.def:295),
//! write_history (:297), read_history (:300) and
//! read_history_range (:316).
//!
//! The fix routes both faces through the one G-02 anchor
//! (executor::path::anchored_file_operand → anchor_at_session_cwd)
//! for the opens only; diagnostics keep the as-typed spelling.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probe scripts (~/workspace/fix-3090/probes) under LC_ALL=C.UTF-8.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-3090-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Fixture root with `sub/pbind` (inputrc) and `sub/phist` (one
/// history line), mirroring the probe fixture.
fn fixture(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).expect("fixture subdir");
    std::fs::write(sub.join("pbind"), "set editing-mode emacs\n").expect("pbind");
    std::fs::write(sub.join("phist"), "echo HISTPLAIN\n").expect("phist");
    root
}

/// Run `script` (written into the fixture root) with the shell's
/// process cwd at the fixture root — the session must `cd` to reach
/// `sub`, which is exactly the process/session cwd split.
fn run(root: &Path, script: &str) -> (String, String) {
    let script_path = root.join("probe.sh");
    std::fs::write(&script_path, script).expect("probe script");
    let output = Command::new(BIN)
        .arg(&script_path)
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .env("FIX", root)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn bind_f_relative_operand_reads_session_cwd() {
    let root = fixture("bind-rel");
    let (stdout, stderr) = run(
        &root,
        "cd \"$FIX/sub\" || exit 99\nbind -f pbind; echo \"bind rc=$?\"\n",
    );
    assert!(stdout.contains("bind rc=0"), "stdout: {stdout}");
    assert!(
        !stderr.contains("cannot read"),
        "stderr must not report the operand unreadable: {stderr}"
    );
}

#[test]
fn bind_f_include_relative_to_session_cwd() {
    let root = fixture("bind-inc");
    std::fs::write(root.join("sub").join("pbind"), "$include pinc\n").expect("pbind include");
    std::fs::write(root.join("sub").join("pinc"), "set bell-style none\n").expect("pinc");
    let (stdout, _stderr) = run(
        &root,
        "cd \"$FIX/sub\" || exit 99\nbind -f pbind; echo \"bind rc=$?\"\nbind -v | grep bell-style\n",
    );
    assert!(stdout.contains("bind rc=0"), "stdout: {stdout}");
    // The included file's `set` only lands if the include read was
    // anchored at the session cwd too (GNU default is bell-style
    // audible; `none` proves the include was read).
    assert!(
        stdout.contains("set bell-style none"),
        "stdout: {stdout}"
    );
}

#[test]
fn bind_f_absolute_operand_still_works() {
    let root = fixture("bind-abs");
    let (stdout, _stderr) = run(&root, "bind -f \"$FIX/sub/pbind\"; echo \"bind rc=$?\"\n");
    assert!(stdout.contains("bind rc=0"), "stdout: {stdout}");
}

#[test]
fn bind_f_missing_relative_operand_diagnostic_keeps_typed_name() {
    let root = fixture("bind-missing");
    let (stdout, stderr) = run(
        &root,
        "cd \"$FIX/sub\" || exit 99\nbind -f nosuchfile; echo \"bind rc=$?\"\n",
    );
    assert!(stdout.contains("bind rc=1"), "stdout: {stdout}");
    // GNU prints the operand as typed, never the anchored path.
    assert!(
        stderr.contains("bind: nosuchfile: cannot read: No such file or directory"),
        "stderr: {stderr}"
    );
}

#[test]
fn history_read_relative_operand_reads_session_cwd() {
    let root = fixture("hist-r");
    let (stdout, _stderr) = run(
        &root,
        "cd \"$FIX/sub\" || exit 99\nset -o history\nhistory -r phist; echo \"hist rc=$?\"\nhistory\n",
    );
    assert!(stdout.contains("hist rc=0"), "stdout: {stdout}");
    assert!(stdout.contains("echo HISTPLAIN"), "stdout: {stdout}");
}

#[test]
fn history_read_new_relative_histfile_default() {
    let root = fixture("hist-n");
    let (stdout, _stderr) = run(
        &root,
        "export HISTFILE=phist\nset -o history\ncd \"$FIX/sub\" || exit 99\nhistory -n; echo \"n rc=$?\"\nhistory\n",
    );
    assert!(stdout.contains("n rc=0"), "stdout: {stdout}");
    assert!(stdout.contains("echo HISTPLAIN"), "stdout: {stdout}");
}

#[test]
fn history_write_and_append_relative_operand_use_session_cwd() {
    let root = fixture("hist-w");
    let (stdout, _stderr) = run(
        &root,
        "set -o history\nhistory -s cmd-one\nhistory -s cmd-two\ncd \"$FIX/sub\" || exit 99\nhistory -w whist; echo \"w rc=$?\"\nhistory -s cmd-three\nhistory -a whist; echo \"a rc=$?\"\n",
    );
    assert!(stdout.contains("w rc=0"), "stdout: {stdout}");
    assert!(stdout.contains("a rc=0"), "stdout: {stdout}");
    let written = std::fs::read_to_string(root.join("sub").join("whist"))
        .expect("whist must land in the session cwd (sub/)");
    assert!(written.contains("cmd-one"), "whist: {written}");
    assert!(written.contains("cmd-two"), "whist: {written}");
    assert!(written.contains("cmd-three"), "whist: {written}");
    assert!(
        !root.join("whist").exists(),
        "whist must not be created in the process cwd"
    );
}

#[test]
fn history_read_absolute_operand_still_works() {
    let root = fixture("hist-abs");
    let (stdout, _stderr) = run(
        &root,
        "set -o history\nhistory -r \"$FIX/sub/phist\"; echo \"hist rc=$?\"\nhistory\n",
    );
    assert!(stdout.contains("hist rc=0"), "stdout: {stdout}");
    assert!(stdout.contains("echo HISTPLAIN"), "stdout: {stdout}");
}

#[test]
fn history_append_missing_dir_diagnostic_keeps_typed_name() {
    let root = fixture("hist-missing");
    let (stdout, stderr) = run(
        &root,
        "set -o history\ncd \"$FIX/sub\" || exit 99\nhistory -a nodir/whist; echo \"a rc=$?\"\n",
    );
    assert!(stdout.contains("a rc=1"), "stdout: {stdout}");
    // GNU prints the operand as typed, never the anchored path.
    assert!(
        stderr.contains("history: nodir/whist: cannot create: No such file or directory"),
        "stderr: {stderr}"
    );
}
