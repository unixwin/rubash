//! rubash#2314 — a dup on the in-process serve path (`read -u` /
//! `mapfile -u`) resolved its SOURCE fd against the source's FINAL
//! own-list binding instead of its binding at the dup's position
//! (residual D2 of the rubash#2305 cluster).
//!
//! GNU applies a command's redirect list as ONE ordered walk:
//! do_redirections (redir.c:237) loops `for (temp = list; temp;
//! temp = temp->next)` (redir.c:252) and each dup copies the
//! descriptor as bound at its own position — later entries that
//! rebind the source cannot retroactively change what an earlier
//! dup copied, and a cyclic pair (`3<&4 4<&3`) is not a loop: the
//! second dup sees the first's result, the first saw the pre-list
//! ambient state. The #2305 serve resolved a dup's source with a
//! whole-list last-wins scan and served the terminal file/heredoc
//! by rev-find (last entry for the fd), so `3<&4 4<inner` read
//! inner through fd 3, and the cyclic pair chased 3→4→3 forever
//! and reported the fd invalid.
//!
//! The fix threads positions through the serve: a positioned
//! binding scan stops before a given list index and reports the
//! winner's index; the dup chase (serve and availability, read
//! and mapfile) resolves each source against the entries before
//! the current bound, and a list-sourced winner is served
//! entry-exactly (heredocs via the ordinal correlation
//! materialize_async_stdin_body uses). `redirect_in` has no
//! recoverable list position: it is consulted only when the
//! bounded scan yields no binding at all, reports position None,
//! and a dup sourced from it resolves its source against the
//! full list (positions strictly descend from there).
//!
//! Expectations byte-verified against GNU bash 5.3.20 with
//! script-file probes (cluster workdir probes/: d2_headline,
//! cyc_a, cyc_b, rebound_doc, doc_source, unrebound, mf_headline,
//! mf_cyclic, chain, pre_chain, close_after_dup, dup_after_close).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2314-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    fs::write(dir.join("inner.txt"), "inner-body\n").expect("write inner");
    fs::write(dir.join("outer2.txt"), "outer-line1\nouter-line2\n").expect("write outer2");
    fs::write(dir.join("seed.txt"), "seed-line\n").expect("write seed");
    dir
}

fn write_script(dir: &Path, name: &str, body: &str) {
    fs::write(dir.join(name), body).expect("write fixture");
}

/// Run a script FILE in `dir` with stdin closed (null).
fn run_script(dir: &Path, name: &str) -> (String, String, Option<i32>) {
    let output = Command::new(RUBASH)
        .arg(dir.join(name))
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_serves(tag: &str, script: &str, expected_stdout: &str) {
    let dir = scratch(tag);
    write_script(&dir, "case.sh", script);
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, expected_stdout, "stderr: {stderr}");
}

// ---- read -u: source rebound after the dup ----------------------

#[test]
fn issue2314_read_dup_source_rebound_to_file() {
    assert_serves(
        "headline",
        "exec 4<outer2.txt\nread -u 3 a 3<&4 4<inner.txt; echo \"A:$a\"\n",
        "A:outer-line1\n",
    );
}

#[test]
fn issue2314_read_dup_source_rebound_to_heredoc() {
    assert_serves(
        "rebound-doc",
        "exec 4<outer2.txt\nread -u 3 a 3<&4 4<<RUBASH_EOF\ndoc-body\nRUBASH_EOF\necho \"A:$a\"\n",
        "A:outer-line1\n",
    );
}

#[test]
fn issue2314_read_dup_source_is_heredoc_before_rebind() {
    assert_serves(
        "doc-source",
        "exec 4<outer2.txt\nread -u 3 a 4<<RUBASH_EOF 3<&4 4<inner.txt\ndoc-body\nRUBASH_EOF\necho \"A:$a\"\n",
        "A:doc-body\n",
    );
}

#[test]
fn issue2314_read_dup_source_closed_after_dup() {
    assert_serves(
        "close-after-dup",
        "exec 4<outer2.txt\nread -u 3 a 3<&4 4<&-; echo \"A:$a\"\n",
        "A:outer-line1\n",
    );
}

// ---- read -u: cyclic dups ---------------------------------------

#[test]
fn issue2314_read_cyclic_dups_serve_first_fd() {
    assert_serves(
        "cyc-a",
        "exec 3<outer2.txt 4<inner.txt\nread -u 3 a 3<&4 4<&3; echo \"A:$a\"\n",
        "A:inner-body\n",
    );
}

#[test]
fn issue2314_read_cyclic_dups_serve_second_fd() {
    assert_serves(
        "cyc-b",
        "exec 3<outer2.txt 4<inner.txt\nread -u 4 b 3<&4 4<&3; echo \"B:$b\"\n",
        "B:inner-body\n",
    );
}

// ---- read -u: chained dups --------------------------------------

#[test]
fn issue2314_read_dup_chain_ignores_later_rebinds() {
    assert_serves(
        "chain",
        "exec 4<seed.txt 5<outer2.txt\nread -u 3 a 3<&4 4<&5 5<inner.txt; echo \"A:$a\"\n",
        "A:seed-line\n",
    );
}

#[test]
fn issue2314_read_dup_chain_through_earlier_dup() {
    assert_serves(
        "pre-chain",
        "exec 5<seed.txt 4<outer2.txt\nread -u 3 a 4<&5 3<&4 5<inner.txt; echo \"A:$a\"\n",
        "A:seed-line\n",
    );
}

// ---- controls ----------------------------------------------------

#[test]
fn issue2314_read_dup_unrebound_source_control() {
    assert_serves(
        "unrebound",
        "exec 4<outer2.txt\nread -u 3 a 3<&4; echo \"A:$a\"\n",
        "A:outer-line1\n",
    );
}

#[test]
fn issue2314_read_dup_of_closed_source_stays_invalid() {
    // The dup's source is closed BEFORE the dup in the same list:
    // GNU fails the redirection (`4: Bad file descriptor`) and the
    // read never runs — stdout stays at the bare echo either way.
    let dir = scratch("dup-after-close");
    write_script(
        &dir,
        "case.sh",
        "exec 4<outer2.txt\nread -u 3 a 4<&- 3<&4; echo \"A:$a\"\n",
    );
    let (stdout, _stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "A:\n");
}

// ---- mapfile -u ---------------------------------------------------

#[test]
fn issue2314_mapfile_dup_source_rebound_to_file() {
    assert_serves(
        "mf-headline",
        "exec 4<outer2.txt\nmapfile -u 3 -t arr 3<&4 4<inner.txt; echo \"A:${arr[0]}\"\n",
        "A:outer-line1\n",
    );
}

#[test]
fn issue2314_mapfile_cyclic_dups() {
    assert_serves(
        "mf-cyclic",
        "exec 3<outer2.txt 4<inner.txt\nmapfile -u 3 -t arr 3<&4 4<&3; echo \"A:${arr[0]}\"\n",
        "A:inner-body\n",
    );
}
