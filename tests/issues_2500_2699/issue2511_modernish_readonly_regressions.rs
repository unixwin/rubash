//! rubash#2511 — a function body that was alias-expanded by the reader
//! that produced it was expanded a SECOND time at the call whenever the
//! caller's ambient input stream did not carry rubash's internal
//! "already streamed" marker.
//!
//! GNU expands aliases only while reading (parse.y read_token_word ->
//! alias_expand_token); execute_cmd.c execute_function (execute_cmd.c:
//! 5181) copies the stored body tree and executes it verbatim, so a
//! definition-time expansion is never re-applied at the call. modernish
//! relies on this: bin/modernish sets `alias let='let --'`, modules
//! sourced afterwards bake `let --` into their function bodies at
//! definition, and a caller whose own text never mentions
//! `expand_aliases` (so the shell takes the whole-file fast path for it,
//! with no per-group marker) re-expanded the baked word —
//! `let "$#"` ran as `let -- -- 1` and died with
//! `let: --: arithmetic syntax error: operand expected`.
//!
//! The shapes below are the minimal divergence: a sourced library
//! (always read group-by-group, hence streamed) defines the alias and
//! the function; a main script whose text contains neither
//! `expand_aliases` nor a parse-gate shopt (hence batch-read, hence no
//! ambient marker) calls it. Expectations verified against GNU bash
//! 5.3.20 with the same files.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2511-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn write(dir: &PathBuf, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).expect("write script");
    path
}

fn run(script: &PathBuf) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const LIB: &str = "shopt -s expand_aliases\nalias let='let --'\nprobe_fn() { let \"$#\"; }\n";

/// Core shape: the streamed body must execute exactly as stored —
/// `let -- "$#"` — not be alias-expanded again at the call.
#[test]
fn streamed_body_is_not_reexpanded_at_call_from_batch_caller() {
    let dir = tempdir("core");
    let lib = write(&dir, "lib.sh", LIB);
    let main = write(
        &dir,
        "main.sh",
        &format!(". \"{}\"\nprobe_fn 1\necho \"rc=$?\"\n", lib.display()),
    );
    let (stdout, stderr, code) = run(&main);
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The protection composes through nested calls: a streamed outer body
/// calling the streamed inner function keeps the inner body verbatim
/// even though the outermost caller is a batch-read script.
#[test]
fn streamed_body_protection_composes_through_nested_calls() {
    let dir = tempdir("nested");
    let lib = write(
        &dir,
        "lib.sh",
        "shopt -s expand_aliases\nalias let='let --'\ninner() { let \"$#\"; }\nouter() { inner 1; }\n",
    );
    let main = write(
        &dir,
        "main.sh",
        &format!(". \"{}\"\nouter\necho \"rc=$?\"\n", lib.display()),
    );
    let (stdout, stderr, code) = run(&main);
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Definition-time baking is final: redefining the alias after the
/// definition must not reach the stored body (GNU disc E shape — the
/// stored tree names the builtin, later alias table state is invisible
/// to it). Here the post-definition alias would turn the call into
/// `let -- -- -- 1` if the body were re-expanded at the call.
#[test]
fn later_alias_redefinition_does_not_reach_stored_body() {
    let dir = tempdir("baked");
    let lib = write(
        &dir,
        "lib.sh",
        "shopt -s expand_aliases\nalias let='let --'\npf() { let \"$#\"; }\nalias let='let -- --'\n",
    );
    let main = write(
        &dir,
        "main.sh",
        &format!(". \"{}\"\npf 1\necho \"rc=$?\"\n", lib.display()),
    );
    let (stdout, stderr, code) = run(&main);
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
