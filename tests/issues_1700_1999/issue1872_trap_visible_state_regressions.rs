//! rubash#1872: trap-visible state polluted by compound redirections.
//!
//! Two invariant breaks, one family:
//!
//! (A) $BASH_COMMAND — GNU stamps each command with the text of its
//! OWN node: print_cmd.c make_command_string prints the node's own
//! redirection list, and execute_cmd.c updates BASH_COMMAND from that
//! text before running traps. An enclosing compound's redirections
//! are runtime fd state, never text. Rubash splices a compound's
//! redirects into every body element node (marked as injected — the
//! mark's contract is "NOT the command's own redirect"), and the
//! text renderers ignored the mark, so inside `{ ...; } >>f 2>>f`
//! the DEBUG trap reported `echo one >> f 2>> f` instead of
//! `echo one`. The renderers now filter marked redirects at the
//! single funnel every node's text passes through.
//!
//! (B) $BASH_SOURCE — GNU keeps ONE live dynamic array
//! (variables.c:1892 INIT_DYNAMIC_VAR, get_self): element reads,
//! scalar reads (element 0) and declare -p all observe the same
//! stack, maintained by push_source on function entry
//! (execute_cmd.c:5310-5313) and evalfile push/pop for `source`.
//! Rubash's env mirror cell for the array only refreshed at
//! source-frame push/pop, while function entry/exit moved only the
//! live stack — so after any function call, the SCALAR read (the
//! form bats' DEBUG trap passes: 'bats_debug_trap "$BASH_SOURCE"')
//! and declare -p kept returning the last sourced file's frozen
//! frame, bats classified every test command as a bats-internal
//! path, and its failure stack traces came out empty. Scalar and
//! declaration reads now go through the same composed dynamic view
//! element reads use (the FUNCNAME precedent), for both BASH_SOURCE
//! and its BASH_LINENO sibling.
//!
//! Oracle: GNU bash 5.3.20, script files, captured verbatim.

#![cfg(unix)]

use std::process::{Command, Stdio};

/// Local stand-in for the `tempfile` crate (not a dev-dependency of
/// this repo): a unique scratch directory removed on drop.
struct ScratchDir(std::path::PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch_tempdir() -> ScratchDir {
    let dir = std::env::temp_dir().join(format!(
        "rubash-scratch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    ScratchDir(dir)
}

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

struct Run {
    stdout: String,
    stderr: String,
    ok: bool,
    dir: ScratchDir,
}

impl Run {
    fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.0.join(name))
            .unwrap_or_else(|_| panic!("expected output file {name} to exist"))
    }
}

/// Write `files` into a scratch dir, run `main.sh`, capture streams.
fn run_files(files: &[(&str, &str)]) -> Run {
    let dir = scratch_tempdir();
    for (name, content) in files {
        std::fs::write(dir.0.join(name), content).expect("write file");
    }
    let output = Command::new(rubash_bin())
        // Relative spelling, like an interactive invocation: the main
        // frame's BASH_SOURCE is the path AS GIVEN (./main.sh).
        .arg("./main.sh")
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", &dir.0)
        .current_dir(&dir.0)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        ok: output.status.success(),
        dir,
    }
}

const LOADER: &str = "loader_fn() { source ./defs.sh; }\n";
const DEFS_TF: &str = "tf() { echo in-tf; }\n";

#[test]
fn issue1872_group_redirect_debug_trap_bash_command_pure() {
    let run = run_files(&[(
        "main.sh",
        "set -T\n\
         trap 'echo \"DBG cmd=[$BASH_COMMAND]\"' DEBUG\n\
         { echo one; echo two; } >> out.txt 2>> out.txt\n\
         echo after\n\
         trap - DEBUG\n\
         echo done\n",
    )]);
    assert!(run.ok, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        "DBG cmd=[echo after]\nafter\nDBG cmd=[trap - DEBUG]\ndone\n"
    );
    // The element stamps land in the group's output file (the trap
    // runs in the element's redirected context): pure node text, no
    // baked-in `>> out.txt 2>> out.txt` suffix.
    assert_eq!(
        run.file("out.txt"),
        "DBG cmd=[echo one]\none\nDBG cmd=[echo two]\ntwo\n"
    );
}

#[test]
fn issue1872_group_redirect_function_element_bash_command_pure() {
    let run = run_files(&[
        (
            "main.sh",
            "set -T\n\
             source ./handler.sh\n\
             trap handler DEBUG\n\
             source ./defs.sh\n\
             { body_fn; } >> out.txt 2>> out.txt\n\
             echo after\n\
             trap - DEBUG\n\
             echo done\n",
        ),
        (
            "handler.sh",
            "handler() { echo \"DBG cmd=[$BASH_COMMAND]\"; }\n",
        ),
        ("defs.sh", "body_fn() { echo body; }\n"),
    ]);
    assert!(run.ok, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        "DBG cmd=[source ./defs.sh]\nDBG cmd=[echo after]\nafter\n\
         DBG cmd=[trap - DEBUG]\ndone\n"
    );
    assert_eq!(
        run.file("out.txt"),
        "DBG cmd=[body_fn]\nDBG cmd=[body_fn]\nDBG cmd=[echo body]\nbody\n"
    );
}

#[test]
fn issue1872_group_redirect_keeps_element_own_redirect_text() {
    // Filtering removes only the INJECTED redirects: an element's own
    // redirect stays in its stamp (GNU prints the node's own list).
    let run = run_files(&[(
        "main.sh",
        "set -T\n\
         trap 'echo \"DBG cmd=[$BASH_COMMAND]\"' DEBUG\n\
         { echo own > own.txt; echo plain; } > grp.txt\n\
         trap - DEBUG\n\
         echo done\n",
    )]);
    assert!(run.ok, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "DBG cmd=[trap - DEBUG]\ndone\n");
    assert_eq!(
        run.file("grp.txt"),
        "DBG cmd=[echo own > own.txt]\nDBG cmd=[echo plain]\nplain\n"
    );
    assert_eq!(run.file("own.txt"), "own\n");
}

#[test]
fn issue1872_err_trap_bash_command_in_redirected_group() {
    let run = run_files(&[(
        "main.sh",
        "set -E\n\
         trap 'echo \"ERRTRAP cmd=[$BASH_COMMAND] rc=$?\"' ERR\n\
         { false; echo after; } > out.txt 2>&1\n\
         trap - ERR\n\
         echo done\n",
    )]);
    assert!(run.ok, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "done\n");
    assert_eq!(run.file("out.txt"), "ERRTRAP cmd=[false] rc=1\nafter\n");
}

#[test]
fn issue1872_function_call_redirect_body_stamps_pure() {
    // A function CALL's own redirect splices into the body elements
    // the same way a group's does; body stamps must stay pure. The
    // call's entry trap fire may land on either stream (its handler
    // fd placement is a separate surface), so the stamp SET across
    // both streams is what is pinned here.
    let run = run_files(&[(
        "main.sh",
        "set -T\n\
         f() { echo in-f; echo two; }\n\
         trap 'echo \"DBG [$BASH_COMMAND]\"' DEBUG\n\
         f > out.txt\n\
         trap - DEBUG\n\
         echo done\n",
    )]);
    assert!(run.ok, "stderr: {}", run.stderr);
    let out = run.file("out.txt");
    assert!(
        out.contains("DBG [echo in-f]\nin-f\nDBG [echo two]\ntwo\n"),
        "out.txt: {out}"
    );
    let mut stamps: Vec<&str> = run
        .stdout
        .lines()
        .chain(out.lines())
        .filter(|line| line.starts_with("DBG ["))
        .collect();
    stamps.sort_unstable();
    assert_eq!(
        stamps,
        [
            "DBG [echo in-f]",
            "DBG [echo two]",
            "DBG [f > out.txt]",
            "DBG [f > out.txt]",
            "DBG [trap - DEBUG]",
        ]
    );
}

#[test]
fn issue1872_source_redirect_sourced_command_stamps_pure() {
    // The `source` face of the same splice: commands of a file sourced
    // under a redirect inherit it for fd routing only.
    let run = run_files(&[
        (
            "main.sh",
            "set -T\n\
             trap 'echo \"DBG cmd=[$BASH_COMMAND]\"' DEBUG\n\
             source ./sourced.sh > out.txt\n\
             trap - DEBUG\n\
             echo done\n",
        ),
        ("sourced.sh", "echo s-one\necho s-two\n"),
    ]);
    assert!(run.ok, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        "DBG cmd=[source ./sourced.sh > out.txt]\nDBG cmd=[trap - DEBUG]\ndone\n"
    );
    assert_eq!(
        run.file("out.txt"),
        "DBG cmd=[echo s-one]\ns-one\nDBG cmd=[echo s-two]\ns-two\n"
    );
}

#[test]
fn issue1872_bash_source_trap_argument_tracks_current_file() {
    // The bats shape in minimal form: the trap string expands
    // $BASH_SOURCE when the trap fires; the value must be the file
    // of the command about to run at EVERY depth — across a function
    // that sources a definition file (the loader pattern) and into
    // the function defined there.
    let run = run_files(&[
        (
            "main.sh",
            "set -T\n\
             source ./handler.sh\n\
             trap 'thandler \"$BASH_SOURCE\" \"$_\"' DEBUG\n\
             source ./lib-loader.sh\n\
             loader_fn\n\
             tf\n\
             trap - DEBUG\n\
             echo done\n",
        ),
        ("handler.sh", "thandler() { echo \"TRAP src=[$1]\"; }\n"),
        ("lib-loader.sh", LOADER),
        ("defs.sh", DEFS_TF),
    ]);
    assert!(run.ok, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        "TRAP src=[./main.sh]\n\
         TRAP src=[./main.sh]\n\
         TRAP src=[./lib-loader.sh]\n\
         TRAP src=[./lib-loader.sh]\n\
         TRAP src=[./main.sh]\n\
         TRAP src=[./defs.sh]\n\
         TRAP src=[./defs.sh]\n\
         in-tf\n\
         TRAP src=[./main.sh]\n\
         done\n"
    );
}

#[test]
fn issue1872_bash_source_scalar_matches_element_after_returns() {
    let run = run_files(&[
        (
            "main.sh",
            "source ./lib-loader.sh\n\
             loader_fn\n\
             echo \"A scalar=[$BASH_SOURCE] elem=[${BASH_SOURCE[0]}]\"\n\
             f2() { echo in-f2; }\n\
             f2\n\
             echo \"B scalar=[$BASH_SOURCE] elem=[${BASH_SOURCE[0]}]\"\n",
        ),
        ("lib-loader.sh", LOADER),
        ("defs.sh", DEFS_TF),
    ]);
    assert!(run.ok, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        "A scalar=[./main.sh] elem=[./main.sh]\n\
         in-f2\n\
         B scalar=[./main.sh] elem=[./main.sh]\n"
    );
}

#[test]
fn issue1872_declare_p_stacks_after_function_return() {
    let run = run_files(&[
        (
            "main.sh",
            "show() { declare -p BASH_SOURCE BASH_LINENO FUNCNAME; }\n\
             declare -p BASH_SOURCE BASH_LINENO FUNCNAME\n\
             source ./lib-loader.sh\n\
             loader_fn\n\
             show\n\
             declare -p BASH_SOURCE BASH_LINENO FUNCNAME\n",
        ),
        ("lib-loader.sh", LOADER),
        ("defs.sh", DEFS_TF),
    ]);
    assert!(run.ok, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        "declare -a BASH_SOURCE=([0]=\"./main.sh\")\n\
         declare -a BASH_LINENO=([0]=\"0\")\n\
         declare -a FUNCNAME\n\
         declare -a BASH_SOURCE=([0]=\"./main.sh\" [1]=\"./main.sh\")\n\
         declare -a BASH_LINENO=([0]=\"5\" [1]=\"0\")\n\
         declare -a FUNCNAME=([0]=\"show\" [1]=\"main\")\n\
         declare -a BASH_SOURCE=([0]=\"./main.sh\")\n\
         declare -a BASH_LINENO=([0]=\"0\")\n\
         declare -a FUNCNAME\n"
    );
}

#[test]
fn issue1872_declare_p_listing_uses_live_stacks() {
    let run = run_files(&[
        (
            "main.sh",
            "source ./lib-loader.sh\n\
             loader_fn\n\
             declare -p | grep -E \"BASH_SOURCE|BASH_LINENO|FUNCNAME\"\n",
        ),
        ("lib-loader.sh", LOADER),
        ("defs.sh", DEFS_TF),
    ]);
    assert!(run.ok, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        "declare -a BASH_LINENO=([0]=\"0\")\n\
         declare -a BASH_SOURCE=([0]=\"./main.sh\")\n\
         declare -a FUNCNAME\n"
    );
}

#[test]
fn issue1872_bash_source_bats_geometry_fd4_trace() {
    // The full bats-exec-test geometry: handler writes its capture
    // to fd 4; a function defined by a loader-sourced file runs
    // inside a brace group redirected to the output file with 4>&1.
    // Every fire's expanded trap argument and $BASH_COMMAND must
    // track the command's own file and text.
    let run = run_files(&[
        (
            "main.sh",
            "set -T\n\
             source ./handler.sh\n\
             source ./lib-loader.sh\n\
             loader_fn\n\
             exec 4>trace.txt\n\
             trap 'handler \"$BASH_SOURCE\" \"$_\"' DEBUG\n\
             { tf; echo plain; } >>out.txt 2>&1 4>&1\n\
             trap - DEBUG\n\
             exec 4>&-\n\
             echo done\n",
        ),
        (
            "handler.sh",
            "handler() { printf 'H arg1=%s cmd=[%s] src0=%s srcall=[%s] funcs=[%s] lineno=[%s]\\n' \
             \"$1\" \"$BASH_COMMAND\" \"${BASH_SOURCE[0]}\" \"${BASH_SOURCE[*]}\" \
             \"${FUNCNAME[*]}\" \"${BASH_LINENO[*]}\" >&4; }\n",
        ),
        ("lib-loader.sh", LOADER),
        (
            "defs.sh",
            "tf() { echo \"in-tf src0=${BASH_SOURCE[0]}\"; echo second; }\n",
        ),
    ]);
    assert!(run.ok, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "done\n");
    assert_eq!(
        run.file("out.txt"),
        "H arg1=./main.sh cmd=[tf] src0=./handler.sh \
         srcall=[./handler.sh ./main.sh] funcs=[handler main] lineno=[7 0]\n\
         H arg1=./defs.sh cmd=[tf] src0=./handler.sh \
         srcall=[./handler.sh ./defs.sh ./main.sh] funcs=[handler tf main] lineno=[1 7 0]\n\
         H arg1=./defs.sh cmd=[echo \"in-tf src0=${BASH_SOURCE[0]}\"] src0=./handler.sh \
         srcall=[./handler.sh ./defs.sh ./main.sh] funcs=[handler tf main] lineno=[1 7 0]\n\
         in-tf src0=./defs.sh\n\
         H arg1=./defs.sh cmd=[echo second] src0=./handler.sh \
         srcall=[./handler.sh ./defs.sh ./main.sh] funcs=[handler tf main] lineno=[1 7 0]\n\
         second\n\
         H arg1=./main.sh cmd=[echo plain] src0=./handler.sh \
         srcall=[./handler.sh ./main.sh] funcs=[handler main] lineno=[7 0]\n\
         plain\n"
    );
    assert_eq!(
        run.file("trace.txt"),
        "H arg1=./main.sh cmd=[trap - DEBUG] src0=./handler.sh \
         srcall=[./handler.sh ./main.sh] funcs=[handler main] lineno=[8 0]\n"
    );
}
