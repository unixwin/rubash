//! Issue rubash#1936 (Windows synthetic /proc): `test -w` / `test -x`
//! never consulted the proc registry facts — they fell through to a
//! filesystem that has no such names and answered false for every
//! synthetic entry, while GNU answers from the entries' procfs modes
//! (test.c:556/559 via sh_eaccess, lib/sh/eaccess.c:194).
//!
//! GNU 5.3.20 oracle matrix (real procfs): the top-level generated
//! files are 0444 (no write for anyone), per-pid status/cmdline are
//! 0644 (owner-writable), the directories are 0555 (traverse, no
//! write), /proc/self/exe resolves to the r-x image (owner-writable),
//! /proc/self/cwd to the owner's rwx directory. ProcUnaryFacts now
//! carries those definitional writable/executable facts and
//! proc_unary_test answers -w/-x from them.
//!
//! Windows-gated: the synthetic faces exist only in the Windows
//! simulation; off Windows the real procfs answers through stat.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1936-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_c(script: &str, dir: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

#[test]
fn issue1936_proc_unary_wx_matrix() {
    let dir = tempdir("matrix");
    // One verdict line per (op, path): T when the test holds.
    let script = "\
for spec in \
  'w /proc' 'x /proc' \
  'w /proc/version' 'x /proc/version' \
  'w /proc/stat' 'x /proc/stat' \
  'w /proc/meminfo' \
  'w /proc/self' 'x /proc/self' \
  'w /proc/self/status' 'x /proc/self/status' \
  'w /proc/self/cmdline' 'x /proc/self/cmdline' \
  'w /proc/self/exe' 'x /proc/self/exe' \
  'w /proc/self/cwd' 'x /proc/self/cwd' \
; do
  op=${spec%% *}; path=${spec#* }
  if test -$op \"$path\"; then echo \"T -$op $path\"; else echo \"F -$op $path\"; fi
done
";
    let run = rubash_c(script, &dir);
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    let expected = "\
T -x /proc
F -w /proc
F -w /proc/version
F -x /proc/version
F -w /proc/stat
F -x /proc/stat
F -w /proc/meminfo
F -w /proc/self
T -x /proc/self
T -w /proc/self/status
F -x /proc/self/status
T -w /proc/self/cmdline
F -x /proc/self/cmdline
T -w /proc/self/exe
T -x /proc/self/exe
T -w /proc/self/cwd
T -x /proc/self/cwd
";
    // Verdicts are emitted in sorted shell order? No — the loop order
    // is the spec order above; compare as sorted sets to stay robust.
    let mut got: Vec<&str> = run.stdout.lines().collect();
    let mut want: Vec<&str> = expected.lines().collect();
    got.sort_unstable();
    want.sort_unstable();
    assert_eq!(got, want, "stdout={}", run.stdout);
}

#[test]
fn issue1936_proc_unary_wx_bracket_form_agrees() {
    let dir = tempdir("bracket");
    // [[ ]] delegates its file unary tests to the same face.
    let run = rubash_c(
        "[[ -w /proc/self/status ]] && echo bracket-w-ok; \
         [[ -x /proc/self/exe ]] && echo bracket-x-ok; \
         [[ -w /proc/version ]] || echo bracket-w-ro-ok",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert!(run.stdout.contains("bracket-w-ok"), "stdout={}", run.stdout);
    assert!(run.stdout.contains("bracket-x-ok"), "stdout={}", run.stdout);
    assert!(
        run.stdout.contains("bracket-w-ro-ok"),
        "stdout={}",
        run.stdout
    );
}
