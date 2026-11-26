//! Issue rubash#4001 — redirect-word parameter expansion errors and
//! `:=` bindings across all 4 classes (`$undef`, `${x:?msg}`, `${-x}`,
//! `${x:=val}`) × external, builtin, compound, and `2>/dev/null` routing.
//!
//! GNU anchors (`redir.c:298 redirection_expand` -> `expand_words_no_vars`):
//! - Redirect words expand in `do_redirections` order, so an earlier
//!   `2>/dev/null` silences diagnostics from a later redirect word.
//! - `$undef` (under `set -u`) and `${x:?msg}` are fatal word-expansion
//!   errors whose containment follows where `do_redirections` runs:
//!   contained by the forked child for external commands (script continues,
//!   status 0 after next command), fatal to the script for builtins and
//!   compound commands (status 1).
//! - `${-x}` (`bad substitution`) is a non-fatal word-expansion error
//!   (status 1, script continues) citing the raw word.
//! - `${x:=val}` binds `x` at the redirect's evaluation point (persists in
//!   the shell for builtins/compounds).
//!
//! Verified against WSL GNU Bash 5.3.20 via script file.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_SEQ: AtomicUsize = AtomicUsize::new(0);

fn tempdir(tag: &str) -> PathBuf {
    let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rubash4001-{tag}-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let code = output.status.code();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, code)
}

#[test]
fn issue4001_undef_redirect_word_classes() {
    let (out, err, code) = run_script("undef_ext", "set -u\ncat > \"$undef\"\necho AFTER\n");
    assert_eq!(out, "AFTER\n");
    assert_eq!(err, "probe.sh: line 2: undef: unbound variable\n");
    assert_eq!(code, Some(0));

    let (out, err, code) = run_script("undef_bi", "set -u\necho hi > \"$undef\"\necho AFTER\n");
    assert_eq!(out, "");
    assert_eq!(err, "probe.sh: line 2: undef: unbound variable\n");
    assert_eq!(code, Some(1));

    let (out, err, code) = run_script(
        "undef_cmp",
        "set -u\n{ echo hi; } > \"$undef\"\necho AFTER\n",
    );
    assert_eq!(out, "");
    assert_eq!(err, "probe.sh: line 2: undef: unbound variable\n");
    assert_eq!(code, Some(1));

    let (out, err, code) = run_script(
        "undef_dn_bi",
        "set -u\necho hi 2>/dev/null > \"$undef\"\necho AFTER\n",
    );
    assert_eq!(out, "");
    assert_eq!(err, "");
    assert_eq!(code, Some(1));

    let (out, err, code) = run_script(
        "undef_dn_ext",
        "set -u\ncat 2>/dev/null > \"$undef\"\necho AFTER\n",
    );
    assert_eq!(out, "AFTER\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue4001_qmark_redirect_word_classes() {
    let (out, err, code) = run_script("qmark_ext", "cat > \"${x:?missing_x}\"\necho AFTER\n");
    assert_eq!(out, "AFTER\n");
    assert_eq!(err, "probe.sh: line 1: x: missing_x\n");
    assert_eq!(code, Some(0));

    let (out, err, code) = run_script("qmark_bi", "echo hi > \"${x:?missing_x}\"\necho AFTER\n");
    assert_eq!(out, "");
    assert_eq!(err, "probe.sh: line 1: x: missing_x\n");
    assert_eq!(code, Some(1));

    let (out, err, code) = run_script(
        "qmark_cmp",
        "{ echo hi; } > \"${x:?missing_x}\"\necho AFTER\n",
    );
    assert_eq!(out, "");
    assert_eq!(err, "probe.sh: line 1: x: missing_x\n");
    assert_eq!(code, Some(1));

    let (out, err, code) = run_script(
        "qmark_dn_bi",
        "echo hi 2>/dev/null > \"${x:?missing_x}\"\necho AFTER\n",
    );
    assert_eq!(out, "");
    assert_eq!(err, "");
    assert_eq!(code, Some(1));

    let (out, err, code) = run_script(
        "qmark_dn_ext",
        "cat 2>/dev/null > \"${x:?missing_x}\"\necho AFTER\n",
    );
    assert_eq!(out, "AFTER\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue4001_bad_subst_redirect_word_classes() {
    let (out, err, code) = run_script("badsub_ext", "cat > pre'sq'${-x}\necho AFTER rc=$?\n");
    assert_eq!(out, "AFTER rc=1\n");
    assert_eq!(err, "probe.sh: line 1: pre'sq'${-x}: bad substitution\n");
    assert_eq!(code, Some(0));

    let (out, err, code) = run_script("badsub_bi", "echo hi > pre'sq'${-x}\necho AFTER rc=$?\n");
    assert_eq!(out, "AFTER rc=1\n");
    assert_eq!(err, "probe.sh: line 1: pre'sq'${-x}: bad substitution\n");
    assert_eq!(code, Some(0));

    let (out, err, code) = run_script(
        "badsub_cmp",
        "{ echo hi; } > pre'sq'${-x}\necho AFTER rc=$?\n",
    );
    assert_eq!(out, "AFTER rc=1\n");
    assert_eq!(err, "probe.sh: line 1: pre'sq'${-x}: bad substitution\n");
    assert_eq!(code, Some(0));

    let (out, err, code) = run_script(
        "badsub_dn_bi",
        "echo hi 2>/dev/null > pre'sq'${-x}\necho AFTER rc=$?\n",
    );
    assert_eq!(out, "AFTER rc=1\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));

    let (out, err, code) = run_script(
        "badsub_dn_ext",
        "cat 2>/dev/null > pre'sq'${-x}\necho AFTER rc=$?\n",
    );
    assert_eq!(out, "AFTER rc=1\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue4001_assign_redirect_word_classes() {
    let (out, err, code) = run_script(
        "assign_bi",
        "unset x\necho hi > \"${x:=out_bi.txt}\"\necho \"x=$x\" content=\"$(cat out_bi.txt)\"\n",
    );
    assert_eq!(out, "x=out_bi.txt content=hi\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));

    let (out, err, code) = run_script(
        "assign_cmp",
        "unset x\n{ echo hi; } > \"${x:=out_cmp.txt}\"\necho \"x=$x\" content=\"$(cat out_cmp.txt)\"\n",
    );
    assert_eq!(out, "x=out_cmp.txt content=hi\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));

    let (out, err, code) = run_script(
        "assign_dn_bi",
        "unset x\necho hi 2>/dev/null > \"${x:=out_dn.txt}\"\necho \"x=$x\" content=\"$(cat out_dn.txt)\"\n",
    );
    assert_eq!(out, "x=out_dn.txt content=hi\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}
