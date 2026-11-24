//! Issue rubash#4175 — after `exec > out` persists a stdout
//! redirection, the in-process emulated `cat` read an operand naming
//! that very file and wrote it back onto itself (the file doubled,
//! exit status 0, no diagnostic). A spawned cat never gets the
//! chance to make this mistake: GNU cat.c checks every regular-file
//! operand against the file fd 1 writes to and refuses the collision
//! with `cat: <operand>: input file is output file`, status 1,
//! skipping only that operand. The emulated serve carried that check
//! for `/dev/std*` / `/dev/fd/N` spellings (via
//! dev_fd_operand_bytes_for_command) but not for plain path operands.
//!
//! GNU envelope (verified byte-for-byte against the cloud GNU Bash
//! 5.3.20 oracle host's cat and a local GNU coreutils 9.4 cat, both
//! driving script files): the refusal is identity-based, so the
//! `./out`, absolute-path, symlink and hard-link spellings are all
//! refused with the operand rendered as spelled; other operands are
//! still served in order; an EMPTY target file is exempt (status 0,
//! silent — the same carve-out the /dev/fd face already encodes);
//! real external children (echo, wc, head) inherit the shared offset
//! and were never divergent.
//!
//! All runs use a null stdin; diagnostics are observed on the
//! script's stderr (fd 2 is never redirected by `exec > out`) and
//! per-command statuses in a status.txt the script writes through
//! its own redirect.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_SEQ: AtomicUsize = AtomicUsize::new(0);

fn tempdir(tag: &str) -> PathBuf {
    let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rubash4175-{tag}-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    files: HashMap<String, Option<String>>,
}

/// Run `script` as probe.sh in a fresh tempdir with a null stdin;
/// capture the process streams plus the contents of the named
/// artifacts the script may have created.
fn run_script(tag: &str, script: &str) -> Run {
    let dir = tempdir(tag);
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let mut files = HashMap::new();
    for name in ["out", "other", "status.txt"] {
        files.insert(
            name.to_string(),
            std::fs::read_to_string(dir.join(name)).ok(),
        );
    }
    let run = Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
        files,
    };
    let _ = std::fs::remove_dir_all(&dir);
    run
}

#[test]
fn exec_persisted_cat_same_file_refused() {
    let run = run_script(
        "same",
        "exec > out\necho hello\ncat out\necho \"cat_rc=$?\" > status.txt\n",
    );
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=1\n"));
    assert!(
        run.stderr.contains("cat: out: input file is output file"),
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.files["out"].as_deref(), Some("hello\n"));
    assert_eq!(run.code, Some(0));
}

#[test]
fn exec_persisted_cat_dot_slash_spelling_refused() {
    let run = run_script(
        "dotslash",
        "exec > out\necho hello\ncat ./out\necho \"cat_rc=$?\" > status.txt\n",
    );
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=1\n"));
    assert!(
        run.stderr.contains("cat: ./out: input file is output file"),
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.files["out"].as_deref(), Some("hello\n"));
}

#[test]
fn exec_persisted_cat_absolute_spelling_refused() {
    let run = run_script(
        "absolute",
        "exec > out\necho hello\ncat \"$PWD/out\"\necho \"cat_rc=$?\" > status.txt\n",
    );
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=1\n"));
    assert!(
        run.stderr.contains("/out: input file is output file"),
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.files["out"].as_deref(), Some("hello\n"));
}

#[cfg(unix)]
#[test]
fn exec_persisted_cat_hard_link_refused() {
    let run = run_script(
        "hardlink",
        "exec > out\necho hello\nln out outlink\ncat outlink\necho \"cat_rc=$?\" > status.txt\n",
    );
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=1\n"));
    assert!(
        run.stderr
            .contains("cat: outlink: input file is output file"),
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.files["out"].as_deref(), Some("hello\n"));
}

#[cfg(unix)]
#[test]
fn exec_persisted_cat_symlink_refused() {
    let run = run_script(
        "symlink",
        "exec > out\necho hello\nln -s out slink\ncat slink\necho \"cat_rc=$?\" > status.txt\n",
    );
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=1\n"));
    assert!(
        run.stderr.contains("cat: slink: input file is output file"),
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.files["out"].as_deref(), Some("hello\n"));
}

#[test]
fn exec_persisted_cat_empty_target_silent() {
    let run = run_script(
        "empty",
        "exec > out\ncat out\necho \"cat_rc=$?\" > status.txt\n",
    );
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=0\n"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.files["out"].as_deref(), Some(""));
}

#[test]
fn exec_persisted_cat_multi_operand_serves_others_in_order() {
    for (tag, script) in [
        (
            "multi-first",
            "exec > out\necho hello\necho OTHER > other\ncat out other\necho \"cat_rc=$?\" > status.txt\n",
        ),
        (
            "multi-last",
            "exec > out\necho hello\necho OTHER > other\ncat other out\necho \"cat_rc=$?\" > status.txt\n",
        ),
    ] {
        let run = run_script(tag, script);
        assert_eq!(
            run.files["status.txt"].as_deref(),
            Some("cat_rc=1\n"),
            "{tag}"
        );
        assert!(
            run.stderr
                .contains("cat: out: input file is output file"),
            "{tag} stderr: {:?}",
            run.stderr
        );
        // Only the OTHER operand's bytes land in the target, in
        // operand order, appended at the shared offset.
        assert_eq!(run.files["out"].as_deref(), Some("hello\nOTHER\n"), "{tag}");
    }
}

#[test]
fn exec_persisted_cat_stdin_operand_then_same_file() {
    let run = run_script(
        "stdin-first",
        "exec > out\necho hello\nprintf 'seed\\n' > seed\ncat - out < seed\necho \"cat_rc=$?\" > status.txt\n",
    );
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=1\n"));
    assert!(
        run.stderr.contains("cat: out: input file is output file"),
        "stderr: {:?}",
        run.stderr
    );
    // The `-` operand's bytes are served; the file operand's are not.
    assert_eq!(run.files["out"].as_deref(), Some("hello\nseed\n"));
}

#[test]
fn exec_persisted_cat_different_redirect_target_serves() {
    let run = run_script(
        "different-target",
        "exec > out\necho hello\ncat out > other\necho \"cat_rc=$?\" > status.txt\n",
    );
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=0\n"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.files["out"].as_deref(), Some("hello\n"));
    assert_eq!(run.files["other"].as_deref(), Some("hello\n"));
}

// Deliberately NOT pinned here: an operand naming the command's OWN
// redirect target (`cat out >> out`, `cat out > out`). On pristine
// master those shapes never reach this serve at all — they deadlock
// in the own-redirect materialization face (worker thread blocked on
// a stdin read; the `>` form still truncates the file first), only
// intermittently completing under some process plumbing. GNU answers
// them deterministically (append: refuse rc 1; truncate: the file is
// already empty, serve rc 0). That deadlock is a separate residual
// recorded in the #4175 lane dossier; pinning either form here would
// hang the suite.

#[test]
fn pipeline_last_stage_cat_same_file_refused() {
    let run = run_script(
        "pipeline-last",
        "exec > out\necho hello\ntrue | cat out\necho \"cat_rc=$?\" > status.txt\n",
    );
    assert_eq!(run.files["status.txt"].as_deref(), Some("cat_rc=1\n"));
    assert!(
        run.stderr.contains("cat: out: input file is output file"),
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.files["out"].as_deref(), Some("hello\n"));
}

#[test]
fn pipeline_middle_stage_cat_serves() {
    // A middle stage's stdout is the pipe, not the persisted file:
    // no refusal, the byte count lands in the target via wc.
    let run = run_script(
        "pipeline-middle",
        "exec > out\necho hello\ncat out | wc -c\necho \"wc_rc=$?\" > status.txt\n",
    );
    assert_eq!(run.files["status.txt"].as_deref(), Some("wc_rc=0\n"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.files["out"].as_deref(), Some("hello\n6\n"));
}
