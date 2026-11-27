//! rubash#799 → relaxed by rubash#1283 (owner ruling 2026-10-22): external
//! command-name resolution folds the ASCII case of BOTH the stem and the
//! appended PATHEXT-style extension, matching the volume's own namespace —
//! System32 ships `PING.EXE` / `NETSTAT.EXE` / `TRACERT.EXE` /
//! `HOSTNAME.EXE`, and a lowercase `ping` must reach them (CMD, PowerShell
//! and msys bash all fold). A byte-equal listing hit still wins within a
//! directory, and the RESOLVED spelling is always the on-disk one.
//!
//! Oracle evidence:
//! * 2026-10-13 (#799 era) — WSL GNU bash 5.3.0 on a case-sensitive fs:
//!   byte-exact stems; that strictness cannot be transplanted onto Windows
//!   (`ping`/`netstat`/`tracert` were unreachable, first reported live by
//!   the owner on 1.4.1/1.4.3).
//! * form oracle — msys bash 5.2.37/5.3: bare names resolve extension
//!   candidates with the whole name case-FOLDED by the NTFS stat layer.
//! * The #799 harvest hang (a gitconfig alias `r = …` launching GNU R's
//!   interpreter) is accepted again as the price of Windows parity — the
//!   same launch happens under Git Bash; the sourcing test below keeps the
//!   original crash shape (`source` must RETURN so the session survives a
//!   stub R.exe that exits).

#![cfg(windows)]

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-799-{tag}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run `script` with PATH = fixture dir + the system dirs, and return
/// (stdout, rc). System dirs keep the shell's own startup needs (`uname`,
/// consoles) working without carrying a toolchain that owns `r`.
fn run_with_fixture(fixture: &Path, script: &str) -> (String, i32) {
    let script_path = scratch("run").join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let path_var = format!(
        "{};C:\\Windows\\System32;C:\\Windows",
        fixture.to_string_lossy().replace('/', "\\")
    );
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .env("PATH", &path_var)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash script");
    let _ = fs::remove_dir_all(script_path.parent().unwrap());
    let mut stdout = String::new();
    let _ = out.stdout.as_slice().read_to_string(&mut stdout);
    (stdout, out.status.code().unwrap_or(-1))
}

fn write_stub(dir: &Path, name: &str) {
    fs::write(dir.join(name), b"stub").unwrap();
}

/// The System32 shape (rubash#1283): on-disk names carry UPPERCASE stems
/// (`PING.EXE`, `NETSTAT.EXE`); the lowercase invocation must resolve and
/// report the on-disk spelling, with or without a typed extension.
#[test]
fn lowercase_invocation_reaches_uppercase_disk_name() {
    let dir = scratch("upper-only");
    // A real PE so an actual launch would succeed, not just a listing hit.
    fs::copy("C:\\Windows\\System32\\where.exe", dir.join("PING.EXE")).expect("seed PING.EXE PE");
    write_stub(&dir, "NETSTAT.EXE");

    let (out, _rc) = run_with_fixture(&dir, "a=$(command -v ping); echo \"ping=${a##*/}\"\n");
    assert!(
        out.contains("ping=PING.EXE"),
        "lowercase `ping` must resolve on-disk PING.EXE; got: {out}"
    );

    let (out, _rc) = run_with_fixture(&dir, "a=$(command -v ping.exe); echo \"ext=${a##*/}\"\n");
    assert!(
        out.contains("ext=PING.EXE"),
        "`ping.exe` (typed extension) must resolve PING.EXE too; got: {out}"
    );

    let (out, _rc) = run_with_fixture(&dir, "command -v netstat >/dev/null; echo \"net=$?\"\n");
    assert!(
        out.contains("net=0"),
        "lowercase `netstat` must reach NETSTAT.EXE; got: {out}"
    );
}

/// Byte-equal spellings still win within a directory: with both spellings
/// on disk (case-sensitive media only; on NTFS this fixture simulates the
/// directory that holds one of them — here we keep the exact-hit arm
/// covered by the *lowercase* disk name resolving as itself).
#[test]
fn exact_spelling_still_answers_with_itself() {
    let dir = scratch("exact-arm");
    write_stub(&dir, "r2.exe");
    let (out, _rc) = run_with_fixture(
        &dir,
        "a=$(command -v r2); echo \"low=${a##*/}\"\nb=$(command -v R2); echo \"up=${b##*/}\"\n",
    );
    assert!(out.contains("low=r2.exe"), "own case hits; got: {out}");
    assert!(
        out.contains("up=r2.exe"),
        "`R2` folds onto r2.exe and reports the on-disk spelling; got: {out}"
    );
}

/// Extensionless on-disk names have no extension arm and still answer only
/// their literal spelling (the fold needs a `.` to split).
#[test]
fn extensionless_name_stays_byte_exact() {
    let dir = scratch("noext");
    write_stub(&dir, "weird");
    let (out, _rc) = run_with_fixture(
        &dir,
        "command -v weird >/dev/null; echo \"low=$?\"\ncommand -v WEIRD >/dev/null; echo \"up=$?\"\n",
    );
    assert!(out.contains("low=0"), "verbatim lowercase hits; got: {out}");
    assert!(
        out.contains("up=1"),
        "uppercase query must miss an extensionless on-disk name; got: {out}"
    );
}

/// The fold never crosses the stem boundary: `r` matches `R.EXE` but never
/// a name whose stem merely STARTS with r (`rx.exe` stays invisible to
/// `r`), and a stem fold never spans a second dot into the extension.
#[test]
fn fold_is_bound_to_the_full_stem() {
    let dir = scratch("stem-bound");
    write_stub(&dir, "rx.exe");
    let (out, _rc) = run_with_fixture(&dir, "command -v r >/dev/null; echo \"r=$?\"\n");
    assert!(
        out.contains("r=1"),
        "`r` must not lift `rx.exe` (fold binds to the full stem); got: {out}"
    );
}

/// The original #799 asset shape, kept as a liveness contract: sourcing a
/// pure gitconfig-alias file launches the folded `R.exe` (now accepted,
/// same as Git Bash) but `source` still RETURNS — the harness's next probe
/// line must reach the shell, never a hanging interpreter.
#[test]
fn sourcing_gitconfig_aliases_returns_after_the_fold() {
    let dir = scratch("asset");
    // A real PE so the folded launch genuinely runs — and exits.
    let real = dir.join("R.exe");
    fs::copy("C:\\Windows\\System32\\where.exe", &real).expect("seed R.exe PE");
    let script = "source './aliases.gitconfig'\ncommand echo PROBE_$((42))_OK\n";
    let script_path = scratch("asset-run").join("probe.sh");
    let alias_path = script_path.parent().unwrap().join("aliases.gitconfig");
    fs::write(&script_path, script).expect("write probe script");
    fs::write(
        &alias_path,
        "[alias]\n\ta = add\n\tr = reflog --date=iso8601 --pretty=reflog-default\n\tst = status\n",
    )
    .expect("write gitconfig");
    let path_var = format!(
        "{};C:\\Windows\\System32;C:\\Windows",
        dir.to_string_lossy().replace('/', "\\")
    );
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .env("PATH", &path_var)
        .current_dir(script_path.parent().unwrap())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash script");
    let mut stdout = String::new();
    let mut stderr = String::new();
    let _ = out.stdout.as_slice().read_to_string(&mut stdout);
    let _ = out.stderr.as_slice().read_to_string(&mut stderr);
    let _ = fs::remove_dir_all(script_path.parent().unwrap());
    assert!(
        stdout.contains("PROBE_42_OK"),
        "source must return so the next probe runs; stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !stderr.contains("r: command not found"),
        "`r` now folds onto R.exe (rubash#1283); stderr={stderr:?}"
    );
}
