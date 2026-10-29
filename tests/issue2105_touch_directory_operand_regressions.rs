//! Regression pins for rubash#2105: the emulated `touch` refused
//! DIRECTORY operands (`Is a directory`, rc=1) because it stamped
//! through a write open. GNU touch stamps an existing operand
//! through utimensat — no open at all — so directories take the
//! stamp with rc=0 (nvm's `acquire_install_lock` ages a lock
//! directory with `touch -t ... "$lock"`), and the same holds for
//! the sibling shapes the write open mishandled: a fifo operand
//! blocked on the open forever. Expectations measured against GNU
//! coreutils 9.4 under GNU bash 5.3.20.
//!
//! Semantic rows run with the real system PATH (the emulation
//! answers in-process). `-t` rows pin TZ=UTC0 so the stamped epoch
//! is exact; mtime/atime are asserted from Rust metadata.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2105-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run_in_dir(dir: &Path, script: &str) -> String {
    let output = Command::new(shell_bin())
        .args(["-c", script])
        .current_dir(dir)
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("TZ", "UTC0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    let mut text = String::from_utf8_lossy(&output.stdout).replace('\r', "");
    let stderr = String::from_utf8_lossy(&output.stderr).replace('\r', "");
    if !stderr.is_empty() {
        text.push_str("STDERR:");
        text.push_str(&stderr);
    }
    text
}

fn epoch_secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .expect("post-epoch time")
        .as_secs()
}

const EPOCH_2020: u64 = 1_577_836_800; // 2020-01-01 00:00:00 UTC
const EPOCH_2022: u64 = 1_640_995_200; // 2022-01-01 00:00:00 UTC

#[test]
fn issue2105_touch_stamps_a_directory_with_t() {
    let dir = scratch("dir-t");
    fs::create_dir(dir.join("lock")).expect("mkdir lock");
    let out = run_in_dir(&dir, "touch -t 202001010000 lock; echo rc=$?");
    assert_eq!(out, "rc=0\n");
    let mtime = fs::metadata(dir.join("lock"))
        .expect("stat lock")
        .modified()
        .expect("lock mtime");
    assert_eq!(epoch_secs(mtime), EPOCH_2020);
}

#[test]
fn issue2105_touch_stamps_a_directory_without_options() {
    let dir = scratch("dir-now");
    fs::create_dir(dir.join("lock")).expect("mkdir lock");
    // Age the directory first (the -t shape above), then a bare
    // touch must move the stamp back to ~now with rc=0.
    let out = run_in_dir(&dir, "touch -t 202001010000 lock; echo age-rc=$?");
    assert_eq!(out, "age-rc=0\n");
    let before = SystemTime::now();
    let out = run_in_dir(&dir, "touch lock; echo rc=$?");
    assert_eq!(out, "rc=0\n");
    let mtime = fs::metadata(dir.join("lock"))
        .expect("stat lock")
        .modified()
        .expect("lock mtime");
    assert!(
        mtime >= before - Duration::from_secs(5)
            && mtime <= SystemTime::now() + Duration::from_secs(5),
        "directory stamp moved to now: {mtime:?}"
    );
}

#[test]
fn issue2105_touch_no_create_still_stamps_an_existing_directory() {
    let dir = scratch("dir-c");
    fs::create_dir(dir.join("lock")).expect("mkdir lock");
    let out = run_in_dir(
        &dir,
        "touch -c -t 202001010000 lock; echo dir-rc=$?; \
         touch -c missing; echo missing-rc=$?; test -e missing && echo BAD-CREATED || echo NOT-CREATED",
    );
    assert_eq!(out, "dir-rc=0\nmissing-rc=0\nNOT-CREATED\n");
    let mtime = fs::metadata(dir.join("lock"))
        .expect("stat lock")
        .modified()
        .expect("lock mtime");
    assert_eq!(epoch_secs(mtime), EPOCH_2020);
}

#[test]
fn issue2105_touch_m_only_moves_mtime_on_a_directory() {
    let dir = scratch("dir-m");
    fs::create_dir(dir.join("lock")).expect("mkdir lock");
    let out = run_in_dir(
        &dir,
        "touch -t 202001010000 lock; echo full-rc=$?; \
         touch -m -t 202201010000 lock; echo m-rc=$?",
    );
    assert_eq!(out, "full-rc=0\nm-rc=0\n");
    let meta = fs::metadata(dir.join("lock")).expect("stat lock");
    assert_eq!(epoch_secs(meta.modified().expect("mtime")), EPOCH_2022);
    assert_eq!(epoch_secs(meta.accessed().expect("atime")), EPOCH_2020);
}

#[test]
fn issue2105_touch_regular_file_faces_unchanged() {
    let dir = scratch("file");
    let out = run_in_dir(
        &dir,
        "touch -t 202001010000 afile; echo create-rc=$?; test -f afile && echo IS-FILE; \
         touch afile; echo bare-rc=$?",
    );
    assert_eq!(out, "create-rc=0\nIS-FILE\nbare-rc=0\n");
}

#[cfg(unix)]
#[test]
fn issue2105_touch_fifo_operand_does_not_block() {
    use std::sync::mpsc;
    let dir = scratch("fifo");
    let status = Command::new("mkfifo")
        .arg(dir.join("p"))
        .status()
        .expect("mkfifo");
    assert!(status.success(), "mkfifo p");
    // The pre-fix emulation blocked forever opening the fifo for
    // writing; bound the run so the pin fails instead of hanging.
    let (tx, rx) = mpsc::channel();
    let dir_clone = dir.clone();
    std::thread::spawn(move || {
        let out = run_in_dir(&dir_clone, "touch p; echo rc=$?");
        let _ = tx.send(out);
    });
    let out = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("touch on a fifo operand must return, not block on open");
    assert_eq!(out, "rc=0\n");
}
