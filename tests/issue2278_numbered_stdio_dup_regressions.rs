//! rubash#2278 — numbered stdio dups (`3<&0`, `3>&1`) on the
//! simple-command spawn face were never delivered: the numbered
//! redirect walk skipped dup spellings outright, so the external
//! child saw EBADF on the target slot in BOTH list orders.
//!
//! GNU binds a command's whole redirect list in the forked child
//! with ONE ordered walk (redir.c:237 do_redirections), and a dup
//! entry services `dup2(source, target)` with the source as bound
//! AT THAT WALK POSITION (redir.c:1138 do_redirection_internal,
//! the dup2 at :1193): `cmd 3<&0 <seed` therefore delivers the
//! AMBIENT stdin at fd 3 (the dup precedes the `<seed` binding),
//! while `cmd <seed 3<&0` delivers the seed (the dup copies the
//! binding the earlier entry installed).
//!
//! Expectations byte-verified against GNU bash 5.3.20 with the
//! probe cells in the cluster workdir (probes/cells): every cell
//! was run under the oracle with a foreign observer (dd) reading
//! the raw descriptor, plus the nested-shell seed shapes.
//!
//! OBSERVER NOTE: the observers are dd (a foreign binary reading
//! /dev/fd/N, i.e. the descriptor the spawn actually delivered)
//! and the rubash binary itself re-invoked — never a `./reader.sh`
//! script: rubash runs ENOEXEC scripts in-process, where the
//! child shares the parent's fd table and the pins would pass
//! vacuously.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2278-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    fs::write(dir.join("seed.txt"), "seed-line-1\nseed-line-2\n").expect("write seed");
    fs::write(dir.join("amb.txt"), "amb-line-1\n").expect("write amb");
    fs::write(dir.join("payload.txt"), "payload-line\n").expect("write payload");
    dir
}

fn rubash_in(dir: &Path, script: &str) -> (String, String, Option<i32>) {
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

fn rubash_in_stdin_file(
    dir: &Path,
    script: &str,
    stdin_file: &Path,
) -> (String, String, Option<i32>) {
    let file = fs::File::open(stdin_file).expect("open stdin file");
    let output = Command::new(RUBASH)
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .stdin(Stdio::from(file))
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The spawned child: a nested rubash whose own external `cat`
/// reads the descriptor — two real spawns between the binding and
/// the read, so nothing passes through the in-process face.
fn child(inner: &str) -> String {
    format!("\"{RUBASH}\" -c '{inner}'")
}

#[cfg(unix)]
#[test]
fn issue2278_dup_before_bind_reads_ambient_stdin() {
    let dir = scratch("dupbefore");
    // GNU A: the dup precedes the `<seed.txt` binding, so fd 3
    // copies the ambient stdin, NOT the seed.
    let amb = dir.join("amb.txt");
    let (stdout, stderr, code) =
        rubash_in_stdin_file(&dir, "dd if=/dev/fd/3 status=none 3<&0 <seed.txt\n", &amb);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "amb-line-1\n");
}

#[cfg(unix)]
#[test]
fn issue2278_bind_before_dup_reads_own_stdin_binding() {
    let dir = scratch("bindbefore");
    // GNU B: `<seed.txt` binds fd 0 first; the dup copies THAT
    // binding (the ordered walk), not the ambient stdin.
    let amb = dir.join("amb.txt");
    let (stdout, stderr, code) =
        rubash_in_stdin_file(&dir, "dd if=/dev/fd/3 status=none <seed.txt 3<&0\n", &amb);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "seed-line-1\nseed-line-2\n");
}

#[cfg(unix)]
#[test]
fn issue2278_nested_dup_before_bind_reads_ambient_stdin() {
    let dir = scratch("nestbefore");
    // The issue's regression seed shape, dup-first order, with a
    // nested rubash as the spawned child reading fd 3.
    let amb = dir.join("amb.txt");
    let script = format!("{} 3<&0 <seed.txt\n", child("cat <&3"));
    let (stdout, stderr, code) = rubash_in_stdin_file(&dir, &script, &amb);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "amb-line-1\n");
}

#[test]
fn issue2278_nested_bind_before_dup_reads_seed() {
    let dir = scratch("nestafter");
    // The issue's regression seed shape, bind-first order.
    let script = format!("{} <seed.txt 3<&0\n", child("cat <&3"));
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "seed-line-1\nseed-line-2\n");
}

#[cfg(unix)]
#[test]
fn issue2278_plain_stdio_dup_reads_ambient_stdin() {
    let dir = scratch("plain");
    // No other redirect: fd 3 is a plain dup of the ambient stdin.
    let amb = dir.join("amb.txt");
    let (stdout, stderr, code) =
        rubash_in_stdin_file(&dir, "dd if=/dev/fd/3 status=none 3<&0\n", &amb);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "amb-line-1\n");
}

#[test]
fn issue2278_numbered_file_control_still_delivers() {
    let dir = scratch("control");
    // Control (green before this fix, via the #2272-era walk):
    // a numbered FILE binding reaches the child unchanged.
    let script = format!("{} 3<seed.txt\n", child("cat <&3"));
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "seed-line-1\nseed-line-2\n");
}

#[cfg(unix)]
#[test]
fn issue2278_chain_through_own_numbered_binding() {
    let dir = scratch("chainnum");
    // A dup sourcing a slot bound earlier in the SAME list copies
    // that binding (GNU's one ordered walk).
    let (stdout, stderr, code) = rubash_in(&dir, "dd if=/dev/fd/4 status=none 3<seed.txt 4<&3\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "seed-line-1\nseed-line-2\n");
}

#[cfg(unix)]
#[test]
fn issue2278_dup_of_ambient_numbered_fd() {
    let dir = scratch("ambnum");
    // A dup sourcing an AMBIENT numbered binding (an `exec`
    // binding in the parent shell) copies that descriptor.
    let (stdout, stderr, code) =
        rubash_in(&dir, "exec 3<seed.txt\ndd if=/dev/fd/4 status=none 4<&3\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "seed-line-1\nseed-line-2\n");
}

#[cfg(unix)]
#[test]
fn issue2278_chain_through_stdio_dup() {
    let dir = scratch("chainstdio");
    // `3<&0` binds fd 3 to the ambient stdin at its position
    // (before `<seed.txt` rebinds fd 0); `4<&3` then copies fd
    // 3's binding — the ambient stream, not the seed.
    let amb = dir.join("amb.txt");
    let (stdout, stderr, code) = rubash_in_stdin_file(
        &dir,
        "dd if=/dev/fd/4 status=none 3<&0 4<&3 <seed.txt\n",
        &amb,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "amb-line-1\n");
}

#[cfg(unix)]
#[test]
fn issue2278_output_dup_reaches_stdout() {
    let dir = scratch("outplain");
    // `3>&1` with no rebinding: fd 3 is the child's stdout.
    let (stdout, stderr, code) =
        rubash_in(&dir, "dd if=payload.txt of=/dev/fd/3 status=none 3>&1\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "payload-line\n");
}

#[cfg(unix)]
#[test]
fn issue2278_output_dup_before_bind_keeps_ambient_stdout() {
    let dir = scratch("outbefore");
    // GNU: the dup precedes the `>sink.txt` binding, so fd 3
    // copies the AMBIENT stdout; the sink is created but receives
    // nothing from fd 3.
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "dd if=payload.txt of=/dev/fd/3 status=none 3>&1 >sink.txt\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "payload-line\n");
    assert_eq!(
        fs::read_to_string(dir.join("sink.txt")).expect("read sink"),
        ""
    );
}

#[cfg(unix)]
#[test]
fn issue2278_output_bind_before_dup_shares_the_file() {
    let dir = scratch("outafter");
    // GNU: `>sink.txt` binds fd 1 first; the dup copies THAT
    // binding, so fd 3 writes land in the sink, not on stdout.
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "dd if=payload.txt of=/dev/fd/3 status=none >sink.txt 3>&1\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert_eq!(
        fs::read_to_string(dir.join("sink.txt")).expect("read sink"),
        "payload-line\n"
    );
}

#[cfg(unix)]
#[test]
fn issue2278_fdvar_stdio_dup_control_still_delivers() {
    let dir = scratch("fdvarctl");
    // Control for the shared walk: the #2277 {var} stdio-dup
    // delivery (fd-0 replay step) must not regress — `{v}<&0`
    // after the command's own `<seed.txt` reads the seed through
    // the var's allocated slot (10).
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "unset v\ndd if=/dev/fd/10 status=none <seed.txt {v}<&0\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "seed-line-1\nseed-line-2\n");
}
