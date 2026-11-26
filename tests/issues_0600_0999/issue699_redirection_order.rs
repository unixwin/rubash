//! Issue rubash#699 regression: redirection order and file-descriptor
//! divergences from GNU bash.
//!
//! R1.1 `2>&1 >file` applied the redirections in the wrong order. GNU's
//! redir.c do_redirection_internal (:767-955) applies a compound command's
//! redirect list strictly left to right, each dup2 against the fd-table
//! state at that moment (redir.c do_redirections :246). rubash additionally
//! replayed an injected `2>&1` on every body leaf, resolving fd 1 *after*
//! the later `>file` had already rebound it — so both streams landed in the
//! file. The same inversion hit `1>&2 2>file`.
//!
//! R1.2 A builtin's write failure must be reported to fd 2 as the command's
//! own redirects bound it (builtins/common.c:320-334 sh_chkwrite ->
//! builtin_error), not to the shell's real stderr.
//!
//! R1.3 Closing fd 2 makes the diagnostic write fail; GNU drops it silently
//! and folds only stdout's ferror into the status (sh_chkwrite), so
//! `ls missing 2>&-` is quiet with status 2.
//!
//! R1.4 A directory-changing builtin's diagnostic inside a subshell escaped
//! the enclosing `2>e` / `2>&1` binding.
//!
//! Verified against WSL GNU Bash 5.3.0 (/usr/local/bin/bash) script-file
//! probes; the `bash` on PATH is the Niubash shim and is circular here.

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
    /// Contents of a file the probe wrote in its working directory.
    fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.join(name)).unwrap_or_else(|e| {
            panic!("read {}: {e}", self.dir.join(name).display());
        })
    }
}

fn run_rubash(slug: &str, script: &str) -> Outcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue699-{slug}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue699 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    // LF endings so the same probe runs under WSL GNU Bash for baselines.
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

/// `{ ...; } 2>&1 >file`: fd 2 keeps the original stdout (ERR prints),
/// fd 1 goes to the file (OUT). The safe order `>file 2>&1` is unchanged.
#[test]
fn compound_two_to_one_then_file_orders_left_to_right() {
    let out = run_rubash(
        "compound-order",
        "{ echo OUT; echo ERR >&2; } 2>&1 >f\n\
         { echo OUT; echo ERR >&2; } >g 2>&1\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "ERR\n"); // ERR on the original stdout
    assert_eq!(out.stderr, ""); // nothing leaked past the group
    assert_eq!(out.file("f"), "OUT\n"); // only OUT reached the file
    assert_eq!(out.file("g"), "OUT\nERR\n"); // safe order: both in the file
}

/// The common `cmd 2>&1 >/dev/null` idiom must still show the command's
/// stderr — the reason this bug changes whether users see errors at all.
#[test]
fn compound_two_to_one_devnull_keeps_stderr_visible() {
    let out = run_rubash(
        "compound-devnull",
        "{ echo OUT; echo ERR >&2; } 2>&1 >/dev/null\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "ERR\n");
    assert_eq!(out.stderr, "");
}

/// Function call-site redirections follow the same left-to-right rule.
#[test]
fn function_two_to_one_devnull_keeps_stderr_visible() {
    let out = run_rubash(
        "function-devnull",
        "f(){ echo OUT; echo ERR >&2; }\n\
         f 2>&1 >/dev/null\n\
         f >/dev/null 2>&1\n\
         echo done\n",
    );
    assert_eq!(out.code, Some(0));
    // First call: ERR visible, OUT discarded. Second: both discarded.
    assert_eq!(out.stdout, "ERR\ndone\n");
    assert_eq!(out.stderr, "");
}

/// `1>&2 2>file` is the mirror image: OUT goes to the original stderr and
/// ERR to the file.
#[test]
fn compound_one_to_two_then_file_stderr_split() {
    let out = run_rubash(
        "compound-one-to-two",
        "{ echo OUT; echo ERR >&2; } 1>&2 2>g\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "OUT\n"); // fd 1 followed fd 2 to the old stderr
    assert_eq!(out.file("g"), "ERR\n"); // fd 2 went to the file
}

/// R1.2: a builtin whose stdout write fails reports `write error` on fd 2
/// *as the command's own `2>err` bound it*.
#[test]
fn builtin_write_error_routes_to_command_stderr() {
    let out = run_rubash(
        "builtin-write-error",
        "printf 'ro\\n' > f\n\
         exec 4< f\n\
         echo X >&4 2>err\n\
         cat err\n",
    );
    assert_eq!(out.code, Some(0));
    // The diagnostic itself is on stdout here only because the probe cats
    // the redirect file; the shell's own stderr must stay untouched.
    assert_eq!(out.stderr, "");
    assert!(
        out.stdout.contains("write error: Bad file descriptor"),
        "diagnostic should land in err, got stdout={:?} err={:?}",
        out.stdout,
        out.file("err")
    );
    assert!(out.file("err").contains("write error: Bad file descriptor"));
}

/// R1.3: `2>&-` is silent and does not corrupt the command's status.
#[cfg(windows)]
#[test]
fn closed_stderr_is_silent_and_keeps_status() {
    let out = run_rubash(
        "closed-stderr",
        "d=\"$PWD/no_such_699\"\n\
         ls \"$d\" 2>&- ; echo \"ls=$?\"\n\
         cat \"$d\" 2>&- ; echo \"cat=$?\"\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "ls=2\ncat=1\n");
    assert_eq!(out.stderr, "");
}

/// R1.4: a subshell `cd` diagnostic honours an enclosing stderr redirect.
#[test]
fn subshell_cd_diagnostic_respects_stderr_redirect() {
    let out = run_rubash(
        "subshell-cd",
        "( cd \"$PWD/no_such_699\" ) 2>e\n\
         wc -c < e\n\
         ( cd \"$PWD/no_such_699\" ) 2>&1 | wc -l\n",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stderr, "");
    let mut lines = out.stdout.lines();
    let bytes: u64 = lines
        .next()
        .expect("byte count line")
        .trim()
        .parse()
        .expect("wc -c output");
    assert!(bytes > 0, "the cd diagnostic must be captured in e");
    assert_eq!(lines.next(), Some("1")); // pipeline captured one line
    assert!(out.file("e").contains("cd:"));
}
