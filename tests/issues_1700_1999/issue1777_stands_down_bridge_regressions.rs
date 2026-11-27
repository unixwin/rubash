//! rubash#1777 — the Windows suite bridge leaked into execution
//! decisions: `command_path` answers `type` / `command -v` for
//! mv/cat/ls while PATH still holds its imported value (rubash#588),
//! but `emulated_external_stands_down` decided by that same lookup —
//! so under an imported PATH holding none of them, `cat`'s emulation
//! arms (external file builtins, pipeline inline stages, comsub
//! shortcuts) ran while `head` / `wc` stood down and reported 127,
//! and a command's existence depended on the invocation shape (the
//! rubash#1545 invariant). The fix routes stands_down through a
//! bridge-free lookup; display keeps the bridge. Expectations below
//! are GNU's: under a PATH that resolves nothing, every one of
//! cat/head/wc/ls/mv reports `command not found`, 127, on every
//! surface.
//!
//! The imported-PATH shape is reproduced by starting the child with a
//! cleared environment and PATH = one empty scratch dir: the executor
//! records that PATH as its startup value, so the bridge is armed on
//! Windows exactly as in the filed lane, while the walk finds nothing.
//! On unix the bridge is cfg'd out, so these tests pin the same
//! uniformity contract there.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rubash-1777-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("empty")).expect("create scratch dirs");
        Scratch(dir)
    }

    fn rubash_file(&self, script: &str) -> (String, String, Option<i32>) {
        let file = self.0.join("case.sh");
        fs::write(&file, script).expect("write script");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&file)
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", self.0.join("empty"))
            .env("HOME", &self.0)
            .output()
            .expect("run rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------
// The filed form: `cat` executed in-process under an imported PATH
// while its unbridged siblings stood down.
// ---------------------------------------------------------------------

#[test]
fn issue1777_imported_path_simple_cat_stands_down() {
    let scratch = Scratch::new("simple-cat");
    let (stdout, stderr, code) =
        scratch.rubash_file("echo hello > f.txt\ncat f.txt; echo cat=$?\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "cat=127\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cat: command not found"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1777_imported_path_pipeline_stages_uniform() {
    let scratch = Scratch::new("pipe");
    let (stdout, stderr, code) = scratch.rubash_file(
        "echo hi | cat; echo cat=$?\necho hi | head -1; echo head=$?\necho hi | wc -l; echo wc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "cat=127\nhead=127\nwc=127\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cat: command not found"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("head: command not found"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("wc: command not found"), "stderr: {stderr}");
}

#[test]
fn issue1777_imported_path_comsub_cat_stands_down() {
    let scratch = Scratch::new("comsub");
    let (stdout, stderr, code) =
        scratch.rubash_file("echo hello > f.txt\nx=$(cat f.txt); echo \"[$x] rc=$?\"\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "[] rc=127\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cat: command not found"),
        "stderr: {stderr}"
    );
}

// ---------------------------------------------------------------------
// Controls: ls/mv have no emulation gated on stands_down, so they
// already stood down — they pin the uniformity claim end to end.
// ---------------------------------------------------------------------

#[test]
fn issue1777_imported_path_ls_mv_controls() {
    let scratch = Scratch::new("ls-mv");
    let (stdout, stderr, code) = scratch.rubash_file("ls; echo ls=$?\nmv a b; echo mv=$?\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "ls=127\nmv=127\n", "stderr: {stderr}");
    assert!(stderr.contains("ls: command not found"), "stderr: {stderr}");
    assert!(stderr.contains("mv: command not found"), "stderr: {stderr}");
}

// ---------------------------------------------------------------------
// Display keeps the bridge: the fix must not take `type` down with it.
// ---------------------------------------------------------------------

#[cfg(windows)]
#[test]
fn issue1777_imported_path_display_bridge_intact_windows() {
    let scratch = Scratch::new("display-win");
    let (stdout, stderr, code) =
        scratch.rubash_file("type cat; echo cat=$?\ntype ls; echo ls=$?\ntype mv; echo mv=$?\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("/bin/cat") && stdout.contains("cat=0"),
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stdout.contains("/bin/ls") && stdout.contains("ls=0"),
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stdout.contains("/usr/bin/mv") && stdout.contains("mv=0"),
        "stdout: {stdout} stderr: {stderr}"
    );
}

#[cfg(not(windows))]
#[test]
fn issue1777_imported_path_display_has_no_bridge_unix() {
    let scratch = Scratch::new("display-unix");
    let (stdout, stderr, code) = scratch.rubash_file("type cat; echo cat=$?\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "cat=1\n", "stderr: {stderr}");
}
