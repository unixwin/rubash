//! rubash#4138 — `set -C` (noclobber) with a simple command carrying
//! BOTH a heredoc and a `>` redirect: when the target does not exist,
//! GNU creates it and writes the heredoc body (rc=0). rubash's cat
//! emulation heredoc arm re-opened the target by path after the
//! emulation layer's retained walk had already performed the entry's
//! one open, so the re-open's O_EXCL judged the file that very open
//! created: rc=1 "cannot overwrite existing file" and the walk's
//! empty file left behind (create-before-check signature).
//!
//! GNU basis: redir.c do_redirections opens each redirect entry
//! exactly once; the noclobber verdict rides that single open.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash4138-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run(dir: &PathBuf, script_text: &str) -> (String, String) {
    let script = dir.join("probe.sh");
    std::fs::write(&script, script_text).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn noclobber_heredoc_to_new_target_writes_body() {
    let dir = tempdir("new");
    let (stdout, stderr) = run(
        &dir,
        "set -C\nrm -f a.txt\ncat <<EOF > a.txt\n114514\nEOF\necho \"rc=$? size=$(wc -c < a.txt)\"\n",
    );
    assert_eq!(stdout, "rc=0 size=7\n", "stderr: {stderr}");
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).expect("a.txt"),
        "114514\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn noclobber_heredoc_to_existing_target_refused_and_preserved() {
    let dir = tempdir("existing");
    let (stdout, stderr) = run(
        &dir,
        "set -C\necho keep > c.txt\ncat <<EOF > c.txt\nforced?\nEOF\necho \"rc=$?\"\ncat c.txt\n",
    );
    assert_eq!(stdout, "rc=1\nkeep\n", "stderr: {stderr}");
    assert!(
        stderr.contains("c.txt: cannot overwrite existing file"),
        "stderr: {stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("c.txt")).expect("c.txt"),
        "keep\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn noclobber_heredoc_clobber_form_forces_write() {
    let dir = tempdir("clobber");
    let (stdout, stderr) = run(
        &dir,
        "set -C\necho keep > c.txt\ncat <<EOF >| c.txt\nforced\nEOF\necho \"rc=$?\"\ncat c.txt\n",
    );
    assert_eq!(stdout, "rc=0\nforced\n", "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn noclobber_plain_and_compound_new_targets_still_create() {
    // Guard cells: the plain simple-command create (#1025's simple
    // face) and the compound create (#1025's fixed face) must keep
    // working — the fix must not turn "check before create" into a
    // blanket refusal.
    let dir = tempdir("guards");
    let (stdout, stderr) = run(
        &dir,
        "set -C\necho hi > b.txt\necho \"plain rc=$? size=$(wc -c < b.txt)\"\n{ echo hi; } > g.txt\necho \"compound rc=$? size=$(wc -c < g.txt)\"\n",
    );
    assert_eq!(
        stdout, "plain rc=0 size=3\ncompound rc=0 size=3\n",
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
