//! rubash#2103 — a persistent fd-0 binding must not swallow a
//! subsequent pipeline stage's stdin.
//!
//! GNU execute_cmd.c execute_pipeline forks every element and do_piping
//! (execute_cmd.c:6373, `if (pipe_in) dup2 (pipe_in, 0)`) replaces the
//! forked element's fd 0 with the inter-stage pipe; an element whose fd
//! 0 is a staged payload (heredoc, here-string, its own `< file`) gets
//! the same replacement from its own do_redirections. A descriptor the
//! shell bound BEFORE the pipeline — `exec 0</dev/null` (redir.c
//! RX_ACTIVE persistence) or an enclosing `while read; done < file`
//! stream bind — must therefore be invisible to every stage that does
//! not rebind fd 0 itself, and must survive the pipeline untouched
//! (the dup2 runs in the forked child, the parent keeps its open file
//! description and offset).
//!
//! Rubash judged stage fd 0 from the fd table first: the read ladder's
//! fd-0 File arm (read_io.rs) and the external spawner's inherit arm
//! (external_setup.rs apply_external_stdin_redirect) served the
//! binding instead of the stage payload staged in FUNCTION_STDIN —
//! after `exec 0</dev/null` every later pipeline consumer read EOF,
//! and after `exec 0<file` it read the file. The stage funnels now
//! clear fd-table slot 0 for the stage's duration (the same rebind the
//! streaming tail performs) and restore the snapshot after.
//!
//! Oracle: WSL GNU bash 5.3.20, script-file form. Every expectation
//! below is byte-verified against it.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[track_caller]
fn assert_gnu(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(
        stdout, expected_stdout,
        "stdout diverged from GNU 5.3.20 for: {script}"
    );
    assert_eq!(stderr, "", "unexpected stderr for: {script}");
    assert_eq!(code, Some(0), "exit status diverged for: {script}");
}

#[test]
fn exec_devnull_compound_read_stage_reads_pipe() {
    // The reported face: `exec 0</dev/null`, then a brace-group stage
    // whose `read` must draw the inter-stage pipe, not the null device.
    assert_gnu(
        "exec 0</dev/null; printf y | { read l; echo \"stage-read=[$l]\"; }",
        "stage-read=[y]\n",
    );
}

#[test]
fn exec_devnull_external_stage_reads_pipe() {
    assert_gnu("exec 0</dev/null; printf y | cat", "y");
}

#[test]
fn exec_devnull_group_cat_stage_reads_pipe() {
    assert_gnu("exec 0</dev/null; printf y | { cat; }", "y");
}

#[test]
fn exec_devnull_command_substitution_pipeline_reads_pipe() {
    assert_gnu(
        "exec 0</dev/null; v=$(printf y | cat); echo \"comsub=[$v]\"",
        "comsub=[y]\n",
    );
}

#[test]
fn exec_devnull_multi_stage_and_loop_faces() {
    assert_gnu(
        "exec 0</dev/null; printf y | { tr a-z A-Z; } | { read l; echo \"two=[$l]\"; }",
        "two=[Y]\n",
    );
    assert_gnu(
        "exec 0</dev/null; printf 'a\nb\n' | { while read -r l; do echo \"loop=[$l]\"; done; }",
        "loop=[a]\nloop=[b]\n",
    );
    assert_gnu("exec 0</dev/null; echo y | cat | cat", "y\n");
}

#[test]
fn exec_file_compound_read_stage_still_reads_pipe() {
    // GNU do_piping replaces fd 0 for elements after the first even when
    // the shell holds a REAL persistent file binding: the stage reads
    // the pipe, not the file.
    assert_gnu(
        "printf 'seed\n' > rubash2103sf.txt; exec 0<rubash2103sf.txt; \
         printf y | { read l; echo \"stage-read=[$l]\"; }; rm -f rubash2103sf.txt",
        "stage-read=[y]\n",
    );
    assert_gnu(
        "printf 'seedline\n' > rubash2103sf2.txt; exec 0<rubash2103sf2.txt; \
         printf y | cat; rm -f rubash2103sf2.txt",
        "y",
    );
}

#[test]
fn loop_file_binding_does_not_swallow_nested_pipeline() {
    // The enclosing `done < file` stream bind (bind_command_fd0_input)
    // holds fd 0 on the fd table while the body runs; the body's
    // pipeline stage must still read its pipe.
    assert_gnu(
        "printf 'go\n' > rubash2103loop.txt; \
         while read -r cmd; do printf y | { read l; echo \"inner=[$l]\"; }; done < rubash2103loop.txt; \
         rm -f rubash2103loop.txt",
        "inner=[y]\n",
    );
}

#[test]
fn parent_binding_survives_pipeline_with_offset_intact() {
    // The dup2 runs in GNU's forked element: the parent's open file
    // description and offset survive the pipeline, so the top-level
    // `read` after it continues the file's second line.
    assert_gnu(
        "printf 'l1\nl2\n' > rubash2103off.txt; exec 0<rubash2103off.txt; \
         read first; echo \"first=[$first]\"; \
         printf y | { read l; echo \"stage=[$l]\"; }; \
         read third; echo \"third=[$third]\"; \
         rm -f rubash2103off.txt",
        "first=[l1]\nstage=[y]\nthird=[l2]\n",
    );
}

#[test]
fn scoped_compound_devnull_redirect_stays_inert() {
    // A SCOPED `{ :; } </dev/null` never binds the persistent slot; the
    // later pipeline worked before this fix and must keep working.
    assert_gnu(
        "{ :; } </dev/null; printf y | { read l; echo \"stage-read=[$l]\"; }",
        "stage-read=[y]\n",
    );
}

#[test]
fn exec_inside_stage_still_rebinds_the_stage() {
    // rubash#1102's class: an `exec 0<file` INSIDE the stage body rebinds
    // the stage's fd 0; the stage's own readers draw the file, not the
    // pipe bytes staged before the exec.
    assert_gnu(
        "printf 'seed\n' > rubash2103in.txt; \
         printf y | { exec 0<rubash2103in.txt; read l; echo \"inside=[$l]\"; }; \
         rm -f rubash2103in.txt",
        "inside=[seed]\n",
    );
}

#[test]
fn lastpipe_element_reads_pipe_over_persistent_binding() {
    // GNU runs the lastpipe element in the current shell with the pipe
    // dup2'd onto fd 0 (execute_cmd.c:2758); the persistent binding must
    // not shadow it there either.
    assert_gnu(
        "shopt -s lastpipe; exec 0</dev/null; \
         printf 'a\nb\n' | { while read -r l; do echo \"lp=[$l]\"; done; }",
        "lp=[a]\nlp=[b]\n",
    );
}
