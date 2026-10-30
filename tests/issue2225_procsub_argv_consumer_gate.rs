//! rubash#2225: the #1959 fifo-marker rendezvous (rubash PR #2156) was
//! applied to EVERY procsub argv word, so a consumer that is not
//! marker-aware — one whose file layer never bridges a `!<fifo>` marker
//! to the named pipe behind it — opened the marker path plainly and
//! read the 7 marker bytes instead of the producer's stream
//! (niu#720/#723: Windows' own findstr.exe printing `!<fifo>` for
//! `findstr . <(hostname)`). GNU subst.c:6362 process_substitute names
//! a live pipe; any consumer opening the name must receive the
//! producer's bytes.
//!
//! The fix gates the marker delivery on the consumer's class: the
//! resolved program must BE the WinuxCmd runtime (the dispatcher, or
//! an applet hardlink of it — `path::winuxcmd_fifo_marker_aware`).
//! Every other consumer takes the pre-#2156 drain-to-temp delivery.
//! These pins run only on Windows (the marker face does not exist
//! elsewhere):
//!
//! * plain readers (findstr, cmd's type) receive producer bytes;
//! * an applet-hardlink consumer still receives the marker operand
//!   (the #1959 streaming route is chosen for its class — the route's
//!   streaming behavior itself stays pinned in
//!   issue1959_procsub_argv_streaming.rs);
//! * a consumer outside any WinuxCmd tree, and a plain COPY of an
//!   applet inside the tree (tree membership is not runtime identity),
//!   receive the drained temp operand.
//!
//! Baseline note: on master (pre-fix) the two plain-read pins print
//! `!<fifo>` and fail, and the two drain word-shape pins see a
//! `rubash-procsub-*` operand and fail; the applet pin passes on both
//! (the marker route was unconditional). The word-shape pins use an
//! argv-echo consumer (a hardlink/copy of the rubash binary itself
//! printing $1) so the route the gate chose is directly observable.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-2225-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    ScratchDir(dir)
}

fn system32() -> PathBuf {
    PathBuf::from(r"C:\Windows\System32")
}

fn path_string(dirs: &[PathBuf]) -> String {
    std::env::join_paths(dirs)
        .expect("join path")
        .to_string_lossy()
        .into_owned()
}

/// Spawn rubash on `script` inside `dir` with `path` as its whole PATH
/// plus the given extra environment. Bounded like the #1959 harness: a
/// hung run is killed and fails the pin instead of stalling the suite.
fn run_rubash(
    dir: &Path,
    path: &str,
    extra_env: &[(&str, String)],
    script: &str,
) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg(script)
        .current_dir(dir)
        .env_clear()
        .env("PATH", path)
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in [
        "SystemRoot",
        "SystemDrive",
        "COMSPEC",
        "PATHEXT",
        "OS",
        "TEMP",
        "TMP",
    ] {
        if let Ok(value) = std::env::var(name) {
            cmd.env(name, value);
        }
    }
    for (name, value) in extra_env {
        cmd.env(name, value);
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => {
                let mut out = child.stdout.take().unwrap();
                let mut err = child.stderr.take().unwrap();
                use std::io::Read;
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let _ = out.read_to_end(&mut stdout);
                let _ = err.read_to_end(&mut stderr);
                return (
                    String::from_utf8_lossy(&stdout).into_owned(),
                    String::from_utf8_lossy(&stderr).into_owned(),
                    status.code(),
                );
            }
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("rubash did not finish within 30s");
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// The niu#720/#723 shape with a deterministic producer: findstr is a
/// plain CreateFile reader and never bridges markers, so it must
/// receive the producer's bytes (drained), never the marker's.
#[test]
fn issue2225_findstr_plain_read_gets_producer_bytes() {
    let dir = scratch("findstr");
    let path = path_string(&[system32()]);
    let (stdout, stderr, code) = run_rubash(
        &dir.0,
        &path,
        &[],
        "C:/Windows/System32/findstr.exe . <(printf 'issue2225-producer-bytes\\n')\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert!(
        stdout.contains("issue2225-producer-bytes"),
        "findstr must print the producer's line, got stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("!<fifo>"),
        "findstr must never see marker bytes, got stdout:\n{stdout}"
    );
}

/// cmd.exe's `type` is the characterization cluster's plain reader:
/// same contract, direct and inside a command substitution (the
/// comsub spawn site must thread the consumer the same way).
#[test]
fn issue2225_cmd_type_plain_read_gets_producer_bytes() {
    let dir = scratch("cmdtype");
    let path = path_string(&[system32()]);
    let (stdout, stderr, code) = run_rubash(
        &dir.0,
        &path,
        &[],
        "cmd.exe /c type <(printf 'issue2225-plain-bytes\\n')\n\
         x=$(cmd.exe /c type <(printf 'issue2225-comsub-bytes\\n'))\n\
         echo \"got:[$x]\"\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert!(
        stdout.contains("issue2225-plain-bytes"),
        "type must print the producer's bytes, got stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("got:[issue2225-comsub-bytes]"),
        "comsub delivery must carry the producer's bytes, got stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("!<fifo>"),
        "no marker bytes may leak to a plain reader, got stdout:\n{stdout}"
    );
}

/// Build a fake WinuxCmd payload tree in `dir`: the rubash test binary
/// hardlinked in as the tree's `winuxcmd.exe` marker, plus a consumer
/// under `usr/bin`. `link_consumer` decides whether the consumer is a
/// hardlink of the marker (the runtime binary itself — marker-aware)
/// or a plain copy (a distinct file — not the runtime).
fn fake_payload(dir: &Path, consumer_name: &str, link_consumer: bool) -> (PathBuf, PathBuf) {
    let bin = dir.join("payload").join("usr").join("bin");
    std::fs::create_dir_all(&bin).expect("create payload bin");
    let dispatcher = bin.join("winuxcmd.exe");
    std::fs::hard_link(env!("CARGO_BIN_EXE_rubash"), &dispatcher).expect("link dispatcher");
    let consumer = bin.join(consumer_name);
    if link_consumer {
        std::fs::hard_link(env!("CARGO_BIN_EXE_rubash"), &consumer).expect("link consumer");
    } else {
        std::fs::copy(env!("CARGO_BIN_EXE_rubash"), &consumer).expect("copy consumer");
    }
    (dir.join("payload"), bin)
}

/// Ask a rubash-binary consumer to echo the operand word it received
/// for `<(printf 'x\n')` — the word's file name names the delivery
/// route the gate chose (`rubash-procsub-*` marker vs `rubash-devfd-*`
/// drained temp).
fn echoed_operand(dir: &Path, path: &str, extra_env: &[(&str, String)], consumer: &str) -> String {
    let script = format!("{consumer} -c 'printf \"%s\\n\" \"$1\"' {consumer} <(printf 'x\\n')\n");
    let (stdout, stderr, code) = run_rubash(dir, path, extra_env, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    stdout
}

/// A consumer that IS the WinuxCmd runtime binary (an applet hardlink
/// of the tree's dispatcher) keeps the #1959 streaming route: its
/// operand word is the fifo-marker path.
#[test]
fn issue2225_aware_applet_consumer_gets_marker_operand() {
    let dir = scratch("applet");
    let (payload, bin) = fake_payload(&dir.0, "showop.exe", true);
    let path = path_string(&[bin, system32()]);
    let home = payload.to_string_lossy().into_owned();
    let stdout = echoed_operand(&dir.0, &path, &[("WINUXCMD_HOME", home)], "showop");
    assert!(
        stdout.contains("rubash-procsub-"),
        "applet consumer must receive the marker operand, got:\n{stdout}"
    );
}

/// A consumer outside any WinuxCmd tree is not marker-aware: its
/// operand word is the drained temp file (the pre-#2156 shape).
#[test]
fn issue2225_plain_consumer_gets_drained_temp_operand() {
    let dir = scratch("plain");
    let plain_bin = dir.0.join("plain");
    std::fs::create_dir_all(&plain_bin).expect("create plain dir");
    std::fs::hard_link(env!("CARGO_BIN_EXE_rubash"), plain_bin.join("showop.exe"))
        .expect("link consumer");
    let path = path_string(&[plain_bin, system32()]);
    let stdout = echoed_operand(&dir.0, &path, &[], "showop");
    assert!(
        stdout.contains("rubash-devfd-"),
        "plain consumer must receive the drained temp operand, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("rubash-procsub-"),
        "plain consumer must never receive a marker operand, got:\n{stdout}"
    );
}

/// A plain COPY of an applet sitting inside the tree is a distinct
/// file, not the runtime binary: tree membership alone is not
/// awareness (the niubash#346 D6 class), so it takes the drain too.
#[test]
fn issue2225_tree_member_copy_is_not_marker_aware() {
    let dir = scratch("copy");
    let (payload, bin) = fake_payload(&dir.0, "showop.exe", false);
    let path = path_string(&[bin, system32()]);
    let home = payload.to_string_lossy().into_owned();
    let stdout = echoed_operand(&dir.0, &path, &[("WINUXCMD_HOME", home)], "showop");
    assert!(
        stdout.contains("rubash-devfd-"),
        "copied tree member must receive the drained temp operand, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("rubash-procsub-"),
        "copied tree member must never receive a marker operand, got:\n{stdout}"
    );
}
