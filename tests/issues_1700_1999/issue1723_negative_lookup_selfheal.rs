//! rubash#1723: a memoized NEGATIVE command lookup must self-heal — GNU
//! never remembers a failed search. findcmd.c:348-380 search_for_command
//! re-stats even positively hashed entries on every lookup, and
//! findcmd.c:623-697 find_user_command_in_path walks $PATH live for every
//! unhashed name; a failed lookup leaves no trace. Rubash keeps miss
//! memoization for the `set +h` / tempenv-PATH bypass (rubash#159 — a
//! Windows miss costs tens of ms) but gates every memoized-miss serve on
//! the rubash#392 candidate probe grid, so install-then-invoke resolves
//! with zero user action instead of demanding `hash -r`.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1723-{tag}-{}-{}",
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

fn run_script(script: &str) -> (String, String, Option<i32>) {
    // The scripts below create files with relative names (`bin/...`); run the
    // child in its own sandbox so nothing lands in the repository root.
    let dir = scratch("run");
    let path = dir.join("probe.sh");
    fs::write(&path, script).expect("write probe script");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash script");
    let _ = fs::remove_dir_all(&dir);
    let mut stdout = String::new();
    let mut stderr = String::new();
    let _ = out.stdout.as_slice().read_to_string(&mut stdout);
    let _ = out.stderr.as_slice().read_to_string(&mut stderr);
    (stdout, stderr, out.status.code())
}

/// The reported shape: `eza` misses, `wpm install eza` drops eza.exe into
/// an already-listed PATH dir (the unlink+create window), and the next
/// lookup resolves WITHOUT `hash -r` and WITHOUT a PATH change. The test
/// uses a guaranteed-absent name (`eza` itself is installed on the dev
/// host). `command -v` is the lookup probe — the created file is not
/// executable content, only its existence under a PATH dir is under test.
#[test]
fn command_appearing_after_memoized_miss_is_found() {
    let (stdout, stderr, rc) = run_script(
        "\
export PATH=\"$PWD/bin:$PATH\"
mkdir -p bin
command -v zz_eza_1723 >/dev/null 2>&1; echo \"miss=$?\"
printf 'x' > bin/zz_eza_1723.exe
printf '#!/bin/sh\\nexit 0\\n' > bin/zz_eza_1723
chmod +x bin/zz_eza_1723
command -v zz_eza_1723
echo \"found=$?\"
",
    );
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("miss=1"), "stderr: {stderr:?}");
    let resolved = lines.next().unwrap_or_default();
    assert!(
        resolved.ends_with("bin/zz_eza_1723.exe") || resolved.ends_with("bin/zz_eza_1723"),
        "command -v zz_eza_1723 should print the new PATH entry, got stdout={stdout:?} stderr={stderr:?}"
    );
    assert_eq!(lines.next(), Some("found=0"), "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// Same scenario through the `set +h` bypass — GNU skips phash there and
/// re-walks $PATH per lookup; rubash's memoized miss must still heal.
#[test]
fn memoized_miss_self_heals_with_hashall_off() {
    let (stdout, stderr, rc) = run_script(
        "\
export PATH=\"$PWD/bin:$PATH\"
mkdir -p bin
set +h
command -v zz_eza_1723 >/dev/null 2>&1; echo \"miss=$?\"
printf 'x' > bin/zz_eza_1723.exe
printf '#!/bin/sh\\nexit 0\\n' > bin/zz_eza_1723
chmod +x bin/zz_eza_1723
command -v zz_eza_1723 >/dev/null; echo \"found=$?\"
command -v zz_eza_1723 >/dev/null; echo \"again=$?\"
",
    );
    assert_eq!(stdout, "miss=1\nfound=0\nagain=0\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// Same scenario through the tempenv-PATH bypass (`PATH=dir cmd`) — the
/// second serve hits the memoized miss and must re-probe, not replay it.
#[test]
fn memoized_miss_self_heals_under_tempenv_path() {
    let (stdout, stderr, rc) = run_script(
        "\
mkdir -p bin
PATH=\"$PWD/bin\" command -v zz_eza_1723 >/dev/null 2>&1; echo \"miss=$?\"
printf 'x' > bin/zz_eza_1723.exe
printf '#!/bin/sh\\nexit 0\\n' > bin/zz_eza_1723
chmod +x bin/zz_eza_1723
PATH=\"$PWD/bin\" command -v zz_eza_1723 >/dev/null; echo \"found=$?\"
",
    );
    assert_eq!(stdout, "miss=1\nfound=0\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// Guard: a name that really is absent keeps failing — the memoized-miss
/// serve must stay bounded (the probe grid runs; the full PATH x PATHEXT
/// listing scan does not) and must not turn misses into hits.
#[test]
fn genuinely_absent_command_still_fails_repeatedly() {
    let (stdout, stderr, rc) = run_script(
        "\
export PATH=\"$PWD/bin:$PATH\"
mkdir -p bin
set +h
command -v absent_1723 >/dev/null 2>&1; echo \"a=$?\"
command -v absent_1723 >/dev/null 2>&1; echo \"b=$?\"
command -v absent_1723 >/dev/null 2>&1; echo \"c=$?\"
set -h
command -v absent_1723 >/dev/null 2>&1; echo \"d=$?\"
",
    );
    assert_eq!(stdout, "a=1\nb=1\nc=1\nd=1\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// `hash -r` still works as a manual flush: a memoized miss followed by
/// `hash -r` then the install resolves — and the memoized-miss path does
/// not break the existing hash/-r/-p surface.
#[test]
fn hash_r_still_flushes_and_hash_p_still_binds() {
    let (stdout, stderr, rc) = run_script(
        "\
export PATH=\"$PWD/bin:$PATH\"
mkdir -p bin
command -v zz_eza_1723 >/dev/null 2>&1; echo \"miss=$?\"
hash -r
printf 'x' > bin/zz_eza_1723.exe
printf '#!/bin/sh\\nexit 0\\n' > bin/zz_eza_1723
chmod +x bin/zz_eza_1723
command -v zz_eza_1723 >/dev/null; echo \"found=$?\"
hash -p \"$PWD/bin/pinned_tool\" pinned_tool
command -v pinned_tool
echo \"pinned=$?\"
",
    );
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("miss=1"), "stderr: {stderr:?}");
    assert_eq!(lines.next(), Some("found=0"), "stderr: {stderr:?}");
    let pinned = lines.next().unwrap_or_default();
    assert!(
        pinned.ends_with("bin/pinned_tool"),
        "hash -p should bind the name, got stdout={stdout:?} stderr={stderr:?}"
    );
    assert_eq!(lines.next(), Some("pinned=0"), "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}
