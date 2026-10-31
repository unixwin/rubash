//! rubash#2308 — ambient Text (heredoc) endpoints on the CONCURRENT
//! pipeline spawners.
//!
//! #2292 (PR #2303) taught the sequential spawn faces to materialize
//! ambient Text endpoints into carrier files: GNU materializes a
//! heredoc body once (redir.c:423 here_document_to_fd) and every
//! forked child inherits the descriptor (redir.c:237
//! do_redirections; execute_cmd.c:2620 execute_pipeline). The
//! concurrent pipeline spawn loops in pipeline_exec.rs only ever
//! registered the handle-backed ambient pins
//! (attach_ambient_fd_pins, rubash#1688) — a Text endpoint has no
//! handle to pin, no env stamp was made, and no carrier was
//! materialized, so a stage spawned in the concurrent burst read
//! EBADF on the ambient heredoc descriptor while the same pipeline
//! run sequentially delivered the body.
//!
//! The fix runs the SAME materialization pass the sequential faces
//! use (deliver_ambient_text_endpoints) in each concurrent spawn
//! loop (attach_ambient_text_delivery), registering its pins as a
//! second pre_exec closure and closing the parent-side duplicates
//! once the stage's spawn resolves — the apply_external_fd_delivery
//! lifetime contract. Carrier files live in procsub_adhoc_carriers
//! and die at finish_process_substitutions, after every stage is
//! reaped.
//!
//! Observers are nested rubash processes (a real spawn — never an
//! ENOEXEC script child, which would share the parent's fd table
//! and pass vacuously), mirroring the issue2292 pins. Expectations
//! byte-verified against GNU bash 5.3.20 with the probe cells in
//! the cluster workdir (probes/p1-p7).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2308-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    fs::write(dir.join("seed.txt"), "seed-line\n").expect("write seed");
    dir
}

/// Run `script` as a `-c` string in `dir` with stdin closed (null).
fn rubash_in(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(RUBASH)
        .arg("-c")
        .arg(script)
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

/// Foreign observer: a nested rubash whose own external `cat`
/// reads fd 3 — a real spawn between the binding and the read.
fn obs3() -> String {
    format!("\"{RUBASH}\" -c 'cat <&3'")
}

/// Foreign observer reading its stdin (fd 0, the pipeline stream).
fn obs0() -> String {
    format!("\"{RUBASH}\" -c 'cat'")
}

#[test]
fn issue2308_exec_ambient_reaches_concurrent_stage() {
    let dir = scratch("exec-one");
    let script = format!("exec 3<<EOF\nambient-body\nEOF\n{} | {}\n", obs3(), obs0());
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "ambient-body\n");
}

#[test]
fn issue2308_group_wrapped_pipeline_stage_reads_ambient_heredoc() {
    let dir = scratch("group");
    let script = format!("{{ {} | {}; }} 3<<EOF\ngroup-body\nEOF\n", obs3(), obs0());
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "group-body\n");
}

#[test]
fn issue2308_two_stages_read_the_same_ambient_heredoc() {
    let dir = scratch("two-stages");
    // Two stages of one concurrent pipeline read the SAME ambient
    // heredoc. Each stage redirects its read to a file so both
    // deliveries are independently observable. Per the standing
    // #2272/#2292 delivery model each spawn receives its own
    // snapshot carrier of the body (GNU shares one description,
    // whose concurrent readers race on the offset — no deterministic
    // GNU expectation exists for this shape; the sequential face's
    // per-spawn snapshot is the established rubash semantic).
    //
    // NOT pinned: two DIFFERENT ambient heredocs read through a
    // nested rubash observer — the parent's delivery is correct
    // there (a foreign python3 witness reads body-three at fd 3 and
    // body-four at fd 4), but a rubash CHILD's startup restore
    // crosses two or more inherited Text carriers (its slot 3
    // serves the last carrier's body). That crossing reproduces on
    // the pure sequential face with no pipeline at all, predates
    // this fix, and is reported as a separate discovery in the
    // cluster REPORT.md (residual R1).
    let script = format!(
        "exec 3<<EOF\nshared-body\nEOF\n\"{RUBASH}\" -c 'cat <&3 > out_a.txt' | \"{RUBASH}\" -c 'cat <&3 > out_b.txt'\n"
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert_eq!(
        fs::read_to_string(dir.join("out_a.txt")).expect("out_a written"),
        "shared-body\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("out_b.txt")).expect("out_b written"),
        "shared-body\n"
    );
}

#[test]
fn issue2308_prefix_stages_feed_brace_group_tail() {
    let dir = scratch("brace-tail");
    // External prefix stages feeding a compound tail: the prefix
    // spawners (prefix-concurrently / streaming-tail loops) are
    // concurrent spawners too, and the reader stage sits in the
    // prefix. The tail must consume the stream inside itself — a
    // bare `... | read line; echo $line` reads the variable in the
    // OUTER shell, where GNU's pipeline-subshell semantics leave it
    // unset (verified against GNU 5.3.20, probes/p8).
    let script = format!(
        "exec 3<<EOF\ntail-body\nEOF\n{} | {} | {{ read line; echo \"got:[$line]\"; }}\n",
        obs3(),
        obs0()
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "got:[tail-body]\n");
}

#[test]
fn issue2308_stage_and_grandchild_both_receive_ambient_heredoc() {
    let dir = scratch("grandchild");
    // The outer concurrent spawn must stamp the nested shell's fd
    // table (env channel); the nested shell's OWN concurrent
    // pipeline must then deliver it again to its stage — the
    // grandchild cat.
    let script = format!(
        "exec 3<<EOF\nnested-body\nEOF\n\"{RUBASH}\" -c 'cat <&3 | cat' | {}\n",
        obs0()
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "nested-body\n");
}

#[test]
fn issue2308_writer_pipeline_consumer_reads_ambient_heredoc() {
    let dir = scratch("writer");
    // Shell-writer pipeline: in-process producer, external consumer
    // spawned by run_shell_writer_pipeline under the group's heredoc.
    let script = format!(
        "prod() {{ echo from-prod; }}\n{{ prod | {}; }} 3<<EOF\nwriter-body\nEOF\n",
        obs3()
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "writer-body\n");
}

#[test]
fn issue2308_mixed_file_and_text_ambient_reach_same_stage() {
    let dir = scratch("mixed");
    // fd 4 is a File endpoint (rides the #1688 pin closure), fd 3 a
    // Text endpoint (rides the #2308 materialization closure). Both
    // pre_exec closures must run for the one stage spawn.
    let script = format!(
        "exec 3<<EOF\ntext-body\nEOF\nexec 4<seed.txt\n\"{RUBASH}\" -c 'cat <&3; cat <&4' | {}\n",
        obs0()
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "text-body\nseed-line\n");
}

#[test]
fn issue2308_second_stage_also_reads_ambient_heredoc() {
    let dir = scratch("second-stage");
    // The reader is the LAST stage: delivery is per stage, not a
    // stage-0 special case.
    let script = format!("exec 3<<EOF\nlast-body\nEOF\n{} | {}\n", obs0(), obs3());
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "last-body\n");
}
