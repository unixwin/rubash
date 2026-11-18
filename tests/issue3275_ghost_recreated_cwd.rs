//! rubash#3275 — a deleted cwd recreated at the same path is a GHOST:
//! the stationary session must not cross into it.
//!
//! GNU anchors a session's cwd in the INODE, not the path string:
//! cd moves the process cwd itself (builtins/cd.def cd_builtin ->
//! chdir), children inherit that descriptor-level position
//! (execute_cmd.c execute_disk_command), and in-process reads walk
//! from it. When the directory is unlinked and a NEW directory is
//! later created at the same path, the session still sits in the dead
//! inode: a file that exists only in the recreated directory is
//! ENOENT for it, and only an explicit `cd` (back) into the recreated
//! directory adopts the new inode.
//!
//! The #3198 two-mode rule asked the anchor refresh whether the cwd
//! PATH resolves — and the refresh, seeing the path resolve to a
//! different (dev, ino), RE-OPENED the anchor onto the recreated
//! directory. A session that never moved thereby crossed into the
//! ghost at the next spawn stamp / in-process read: `cat marker`
//! printed the ghost's content rc 0 where GNU reports ENOENT rc 1
//! (measured on the base tree, probe-3268-ghost2.sh in the #3268
//! design volume).
//!
//! The fix splits the refresh intents: the cd funnels (the session
//! HAS moved — adopting the inode the path now names is correct)
//! keep the adopt refresh, while the spawn stamp and the anchored
//! read (the session has NOT moved) ask an identity question instead
//! — the path resolves for the session only while it still names the
//! very inode the anchor holds; a mismatch is the dead-directory
//! case and the anchor is spent, never re-opened.
//!
//! Oracle: GNU bash 5.3.20 (probe-3268-ghost2.sh, design volume):
//!   pre-recreation `cat ../f` -> `hello` rc 0; `cat marker` -> rc 1;
//!   post-recreation `cat marker` -> ENOENT rc 1, bare `ls` -> empty;
//!   after `cd "$HOME/a"`, `cat marker` -> `ghost` rc 0.

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
    let dir = std::env::temp_dir().join(format!("rubash-3275-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    ScratchDir(dir)
}

fn tool_path() -> String {
    if cfg!(windows) {
        std::env::var("PATH").unwrap_or_default()
    } else {
        "/usr/bin:/bin".to_string()
    }
}

/// Spawn rubash on `script` inside `dir` (the harness cwd is the
/// scratch root D); returns (stdout, stderr, status code).
fn run_rubash(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg(script)
        .current_dir(dir)
        .env_clear()
        .env("PATH", tool_path())
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if cfg!(windows) {
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
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("rubash did not exit within 30s");
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    use std::io::Read;
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    (stdout, stderr, status.code())
}

/// The deleted-cwd stage in `dir`: D/f holds `hello`, the shell cd's
/// into D/a and a subshell removes it. The session is stationary in
/// the dead inode from here on unless the script cd's again.
fn prelude(dir: &Path) -> String {
    std::fs::write(dir.join("f"), "hello\n").expect("write f");
    std::fs::create_dir_all(dir.join("a")).expect("create a");
    "cd a || exit 99\n(cd ..; rmdir a) || exit 98\n".to_string()
}

/// Recreate D/a from OUTSIDE the stationary session (a subshell that
/// walks by the absolute $HOME path) and stock it with a file that
/// exists only in the recreated directory.
const RECREATE: &str = "(cd \"$HOME\" && mkdir a && printf 'ghost\\n' > a/marker) || exit 97\n";

#[test]
fn issue3275_ghost_in_process_read_stays_on_dead_inode() {
    let dir = scratch("ghost-read");
    let script = format!(
        "{}cat ../f; echo up=$?\ncat marker 2>/dev/null; echo before=$?\n{}cat marker 2>/dev/null; echo after=$?\n",
        prelude(&dir.0),
        RECREATE
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    // Before recreation the anchor read still serves the real parent
    // (`../f` -> hello), and `marker` does not exist in the dead
    // directory. After recreation the SAME read must still miss:
    // the marker lives only in the ghost, and the stationary
    // session never moved. On the base tree the last line reads
    // `ghost` / after=0 — the anchor refresh crossed over.
    assert_eq!(
        stdout, "hello\nup=0\nbefore=1\nafter=1\n",
        "stderr:\n{stderr}"
    );
}

#[test]
fn issue3275_ghost_spawn_child_lands_on_dead_inode() {
    let dir = scratch("ghost-spawn");
    let script = format!(
        "{}ls; echo before=$?\n{}ls; echo after=$?\ngrep -q ghost marker 2>/dev/null; echo grep=$?\n",
        prelude(&dir.0),
        RECREATE
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    // A spawned child inherits the session's cwd INODE: bare `ls`
    // lists the dead (empty) directory both before and after the
    // recreation, and no child can see the ghost's marker. The grep
    // verdict is 2, not 1: GNU grep exits 2 when an operand cannot
    // be opened (measured with the GNU 5.3.20 oracle in the dead
    // inode, and for a missing file in a live one), and the child in
    // the dead inode gets exactly that ENOENT. On the base tree the
    // stamp crossed: `ls` listed `marker` and grep FOUND the ghost
    // content (grep=0).
    assert_eq!(stdout, "before=0\nafter=0\ngrep=2\n", "stderr:\n{stderr}");
}

#[test]
fn issue3275_cd_back_into_recreated_dir_adopts_it() {
    let dir = scratch("ghost-adopt");
    let script = format!(
        "{}cat marker 2>/dev/null; echo crossed=$?\n{}cd \"$HOME/a\" || exit 96\ncat marker; echo adopted=$?\n",
        prelude(&dir.0),
        RECREATE
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    // The one legitimate crossing is the session's own cd: after
    // cd'ing into the recreated directory BY PATH, its marker is
    // the session's own file and reads fine. (The first read pins
    // the stationary verdict in this script too: crossed=1.)
    assert_eq!(stdout, "crossed=1\nghost\nadopted=0\n", "stderr:\n{stderr}");
}

#[test]
fn issue3275_deleted_cwd_without_recreation_keeps_anchor_semantics() {
    // Control: with no recreation at all, the #3198 anchor semantics
    // are untouched by the ghost gate — the anchored parent read
    // resolves, a name absent from the dead directory still misses.
    let dir = scratch("control");
    let script = format!(
        "{}cat ../f; echo up=$?\ncat f 2>/dev/null; echo missing=$?\n",
        prelude(&dir.0)
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "hello\nup=0\nmissing=1\n", "stderr:\n{stderr}");
}
