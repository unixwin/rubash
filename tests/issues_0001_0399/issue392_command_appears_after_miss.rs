//! niubash#392 / rubash#392: a command that appears inside an already-walked
//! PATH directory must become findable in the SAME shell process.
//!
//! GNU bash keeps no negative command cache: findcmd.c:623
//! find_user_command_in_path re-walks $PATH for every unhashed lookup, so a
//! miss re-observes the filesystem. rubash memoized misses and per-directory
//! listings for the life of a command-lookup fingerprint, so a command link
//! replaced by `wpm` (unlink+create, WinuxCmd#1188/#1206) stayed invisible for
//! the rest of the process — observed live as `awk: command not found` in 5/5
//! iterations of one loop while the upgrade was running, healing only in the
//! next process.
//!
//! The fix drops miss memoization and validates a cached directory listing
//! against the directory mtime before a miss is concluded, then re-walks once.
//! Hit lookups (the hot path) are untouched.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-392-{tag}-{}-{}",
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

/// The reported shape: a miss for a name, then the file appears (the wpm
/// window), then the name must run — in the same process.
#[test]
fn command_appearing_after_a_miss_is_found() {
    let (stdout, stderr, rc) = run_script(
        "\
export PATH=\"$PWD/bin:$PATH\"
mkdir -p bin
newcmd --version >/dev/null 2>&1; echo \"miss=$?\"
printf '#!/bin/sh\\necho appeared\\n' > bin/newcmd
chmod +x bin/newcmd
newcmd; echo \"found=$?\"
",
    );
    assert_eq!(
        stdout, "miss=127\nappeared\nfound=0\n",
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// The listing-primed variant: the directory was already listed for a
/// *different* name before the new executable appeared.
#[test]
fn command_appearing_after_the_directory_was_listed_is_found() {
    let (stdout, stderr, rc) = run_script(
        "\
export PATH=\"$PWD/bin:$PATH\"
mkdir -p bin
other --version >/dev/null 2>&1; echo \"prime=$?\"
printf '#!/bin/sh\\necho appeared\\n' > bin/newcmd
chmod +x bin/newcmd
newcmd; echo \"found=$?\"
",
    );
    assert_eq!(
        stdout, "prime=127\nappeared\nfound=0\n",
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// Guard: a name that really is absent keeps failing (rc 127) — the fix must
/// not turn every miss into a slow hunt that succeeds.
#[test]
fn genuinely_absent_command_still_fails() {
    let (stdout, stderr, rc) = run_script(
        "\
export PATH=\"$PWD/bin:$PATH\"
mkdir -p bin
absent --version >/dev/null 2>&1; echo \"rc=$?\"
absent --version >/dev/null 2>&1; echo \"rc2=$?\"
",
    );
    assert_eq!(stdout, "rc=127\nrc2=127\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}
