//! Issue rubash#726 regression: a body-level `exec` must not escape a
//! compound command's ordered redirects.
//!
//! GNU applies a compound command's redirect list once to its real
//! descriptors before the body runs (redir.c:767-955 do_redirection_internal,
//! unwound at redir.c:949-955), so `{ ...; } 2>&1` dup2's fd 2 to the stdout
//! object at group entry; a later `exec > f` changes fd 1 only. rubash
//! additionally spliced an injected `2>&1` onto every body leaf, and the
//! replay resolved fd 1 at leaf time — after the exec — so
//! `{ exec > f; echo x >&2; } 2>&1` wrote x into f. The splice is redundant:
//! the fd-table binding from open_compound_output_redirects is authoritative
//! for builtins and spawned children (injected_redirect_fd_is_bound), so the
//! DuplicateOutput replay was removed. The open/append legs are unchanged.
//!
//! Guards assert the shapes rubash#725 already fixed stay correct: the safe
//! `>f 2>&1` order, `&>`/`&>>`, nested groups, `case`/`if`/`while`
//! compounds, and fd-3 swaps.
//!
//! Byte-checked against WSL GNU Bash 5.3.0 script-file probes; the `bash`
//! on PATH is the Niubash shim (circular here).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Outcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

impl Outcome {
    fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.join(name)).unwrap_or_default()
    }
}

fn run_rubash(slug: &str, script: &str) -> Outcome {
    let dir =
        std::env::temp_dir().join(format!("rubash-issue726-cmp-{slug}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue726 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    file.write_all(script.as_bytes()).expect("write probe");
    drop(file);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash probe");
    Outcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

/// `exec > f` inside a group rebinds fd 1 only; the group's `2>&1` had
/// already snapshotted fd 2 to the original stdout, so `x` stays visible.
#[test]
fn body_exec_rebinding_does_not_capture_group_dup() {
    let out = run_rubash(
        "exec-source",
        "rm -f f\n\
         ( { exec > f; echo x >&2; } 2>&1 )\n\
         printf 'tail\\n'\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "x\ntail\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.file("f"), "");
}

/// The same body-level `exec` inside a pipeline stage keeps `2>&1` pointed at
/// the pipe, not the later fd-1 target.
#[test]
fn body_exec_rebinding_in_pipeline_keeps_pipe() {
    let out = run_rubash(
        "exec-pipeline",
        "rm -f h\n\
         { exec > h; echo y >&2; } 2>&1 | cat\n\
         printf 'tail\\n'\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "y\ntail\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.file("h"), "");
}

/// `exec 2>g` inside the body rebinds the dup destination; the group's
/// `2>&1` must not replay over it (GNU keeps fd 2 on g).
#[test]
fn body_exec_rebinding_dest_wins() {
    let out = run_rubash(
        "exec-dest",
        "rm -f g\n\
         ( { exec 2> g; echo x1 >&2; } 2>&1 )\n\
         printf 'out3\\n'\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "out3\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.file("g"), "x1\n");
}

/// Guard: the static `2>&1 >file` group order rubash#725 fixed still holds.
#[test]
fn static_group_dup_order_unchanged() {
    let out = run_rubash("static-group", "{ echo OUT; echo ERR >&2; } 2>&1 >f\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "ERR\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.file("f"), "OUT\n");
}

/// Guard: the safe `>file 2>&1` order still sends both streams to the file.
#[test]
fn safe_file_then_dup_order_unchanged() {
    let out = run_rubash(
        "safe-order",
        "{ echo OUT; echo ERR >&2; } >f 2>&1\n\
         printf 'tail\\n'\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "tail\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.file("f"), "OUT\nERR\n");
}

/// Guard: `&>` / `&>>` still route both streams, appending on `&>>`.
#[test]
fn combined_redirects_unchanged() {
    let out = run_rubash(
        "combined",
        "{ echo A; echo B >&2; } &>f\n\
         { echo C; echo D >&2; } &>>f\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "");
    assert_eq!(out.file("f"), "A\nB\nC\nD\n");
}

/// Guard: nested groups keep the inner group's own redirect authoritative.
#[test]
fn nested_group_redirects_unchanged() {
    let out = run_rubash(
        "nested-group",
        "{ { echo A; echo B >&2; } >a; echo C; } >b 2>&1\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "");
    assert_eq!(out.file("a"), "A\n");
    assert_eq!(out.file("b"), "B\nC\n");
}

/// Guard: `case` / `if` / `while` compounds follow the same ordered list as a
/// brace group (`2>&1 >file` splits the streams).
#[test]
fn compound_redirects_unchanged() {
    let out = run_rubash(
        "compounds",
        "case x in x) echo C1; echo C2 >&2;; esac 2>&1 >fc\n\
         if true; then echo I1; echo I2 >&2; fi 2>&1 >fi\n\
         i=0; while [ $i -lt 1 ]; do echo W1; echo W2 >&2; i=1; done 2>&1 >fw\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "C2\nI2\nW2\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.file("fc"), "C1\n");
    assert_eq!(out.file("fi"), "I1\n");
    assert_eq!(out.file("fw"), "W1\n");
}

/// Guard: an fd-3 swap (`3>&1 2>&1 >file`) keeps fd 3 and fd 2 on the
/// original stdout while fd 1 goes to the file.
#[test]
fn fd3_swap_unchanged() {
    let out = run_rubash("fd3-swap", "{ echo O3 >&3; echo E3 >&2; } 3>&1 2>&1 >f\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "O3\nE3\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.file("f"), "");
}
