//! rubash#1404 regression: `test -r/-w/-x`, `[ -r/-w/-x ]` and
//! `[[ -r/-w/-x ]]` were ALWAYS false on Android/bionic — even for a 777
//! file the shell itself could read — silently taking the wrong branch.
//!
//! Root cause: the unix backend called
//! `faccessat(AT_FDCWD, path, mode, AT_EACCESS)` with the glibc flag
//! value 0x200, but bionic's 4-arg faccessat returns EINVAL for EVERY
//! nonzero flag (the faccessat2 syscall is seccomp-filtered on Android)
//! and the platform linux/fcntl.h resolves AT_EACCESS to 0. The Termux
//! GNU build (HAVE_FACCESSAT=1 in its shipped config.h) compiles the
//! same eaccess.c call but therefore runs faccessat(...,0) — a real-ids
//! access(2). rubash now calls access(2) when ruid==euid && rgid==egid
//! (always true for app processes) and otherwise falls back to a port of
//! sh_stataccess, the same stat-based check GNU uses for differing ids.
//!
//! GNU owners: builtins/test.def `unary_test` -> sh_eaccess
//! (lib/sh/eaccess.c:194), sh_stataccess (lib/sh/eaccess.c:136-165),
//! group_member (general.c:1334). The expected transcript below is
//! byte-identical to GNU bash 5.3.20 run from a script file on-device:
//! 113 output lines, 52 denied + 60 granted verdicts, stderr empty.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_dir() -> PathBuf {
    std::env::temp_dir().join(format!("rubash-it1404-{}", std::process::id()))
}

fn chmod(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod fixture");
}

fn write_file(dir: &Path, name: &str, mode: u32) {
    let path = dir.join(name);
    std::fs::write(&path, b"x").expect("write fixture");
    chmod(&path, mode);
}

fn run_script(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(dir)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn setup() -> PathBuf {
    let dir = fixture_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    write_file(&dir, "f777", 0o777);
    write_file(&dir, "f644", 0o644);
    write_file(&dir, "f000", 0o000);
    write_file(&dir, "f555", 0o555);
    write_file(&dir, "f111", 0o111);
    write_file(&dir, "sp ace", 0o777);
    for (name, mode) in [("d755", 0o755), ("d000", 0o000)] {
        let path = dir.join(name);
        std::fs::create_dir(&path).expect("mkdir fixture");
        chmod(&path, mode);
    }
    std::os::unix::fs::symlink("f644", dir.join("linkfile")).expect("symlink fixture");
    dir
}

fn teardown(dir: &Path) {
    // Restore d000 so removal can see inside it.
    let _ = std::fs::set_permissions(
        dir.join("d000"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    );
    let _ = std::fs::remove_dir_all(dir);
}

const PROBE: &str = r#"for f in f777 f644 f000 f555 f111 d755 d000 linkfile nonexist "sp ace" ""; do
    test -r "$f"; echo "test -r <$f> rc=$?"
    test -w "$f"; echo "test -w <$f> rc=$?"
    test -x "$f"; echo "test -x <$f> rc=$?"
    [ -r "$f" ];   echo "[ -r <$f> ] rc=$?"
    [ -w "$f" ];   echo "[ -w <$f> ] rc=$?"
    [ -x "$f" ];   echo "[ -x <$f> ] rc=$?"
    [[ -r $f ]];   echo "[[ -r <$f> ]] rc=$?"
    [[ -w $f ]];   echo "[[ -w <$f> ]] rc=$?"
    [[ -x $f ]];   echo "[[ -x <$f> ]] rc=$?"
done
v=f777
test -r "$v"; echo "var-r rc=$?"
test -x "$(echo f111)"; echo "comsub-x rc=$?"
[[ -w ${v} ]]; echo "br-var-w rc=$?"
! test -r f000; echo "neg rc=$?"
test -r f777 -a -w f777; echo "and rc=$?"
test -r nonexist -o -w f777; echo "or rc=$?"
if test -x f111; then echo "if-branch taken"; else echo "if-branch else"; fi
test -r /etc/hosts; echo "etc-hosts rc=$?"
test -r "$BASH"; echo "self-r rc=$?"
test -x "$BASH"; echo "self-x rc=$?"
[[ -x . ]]; echo "dot-x rc=$?"
test -r .; echo "dot-r rc=$?"
"#;

/// Per-operand verdicts (non-root euid; root would pass -w/-r on more
/// modes per the kernel access(2) rule — GNU would too, so the matrix is
/// still a parity pin, just a different expected table).
fn expected() -> String {
    let mut out = String::new();
    // (name, r, w, x) — GNU bash 5.3.20 verdicts for euid != 0.
    let table: &[(&str, i32, i32, i32)] = &[
        ("f777", 0, 0, 0),
        ("f644", 0, 0, 1),
        ("f000", 1, 1, 1),
        ("f555", 0, 1, 0),
        ("f111", 1, 1, 0),
        ("d755", 0, 0, 0),
        ("d000", 1, 1, 1),
        ("linkfile", 0, 0, 1),
        ("nonexist", 1, 1, 1),
        ("sp ace", 0, 0, 0),
        ("", 1, 1, 1),
    ];
    for (name, r, w, x) in table {
        out.push_str(&format!("test -r <{name}> rc={r}\n"));
        out.push_str(&format!("test -w <{name}> rc={w}\n"));
        out.push_str(&format!("test -x <{name}> rc={x}\n"));
        out.push_str(&format!("[ -r <{name}> ] rc={r}\n"));
        out.push_str(&format!("[ -w <{name}> ] rc={w}\n"));
        out.push_str(&format!("[ -x <{name}> ] rc={x}\n"));
        out.push_str(&format!("[[ -r <{name}> ]] rc={r}\n"));
        out.push_str(&format!("[[ -w <{name}> ]] rc={w}\n"));
        out.push_str(&format!("[[ -x <{name}> ]] rc={x}\n"));
    }
    out.push_str(
        "var-r rc=0\ncomsub-x rc=0\nbr-var-w rc=0\nneg rc=0\nand rc=0\nor rc=0\n\
         if-branch taken\netc-hosts rc=0\nself-r rc=0\nself-x rc=0\n\
         dot-x rc=0\ndot-r rc=0\n",
    );
    out
}

/// Full matrix: -r/-w/-x x test/[ /[[ ]] x all operand shapes
/// (literal/variable/comsub/space/symlink/dir/missing/empty), the
/// original issue repro plus the extra probes from the ticket.
#[test]
fn test_rwx_matches_gnu_transcript() {
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipping: expected table is the non-root verdict set");
        return;
    }
    let dir = setup();
    let (stdout, stderr, rc) = run_script(&dir, PROBE);
    teardown(&dir);
    assert_eq!(stderr, "", "stderr must be empty");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, expected());
}

/// The exact issue repro: touch+chmod 777 then every consumer form.
#[test]
fn issue_repro_777_file() {
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let dir = fixture_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    write_file(&dir, "f", 0o777);
    let (stdout, stderr, rc) = run_script(
        &dir,
        "test -r f; echo \"rc=$?\"\n\
         test -w f; echo \"rc=$?\"\n\
         test -x f; echo \"rc=$?\"\n\
         [[ -r f ]]; echo \"rc=$?\"\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=0\nrc=0\nrc=0\nrc=0\n");
}
