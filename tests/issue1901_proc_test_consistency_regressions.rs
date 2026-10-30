//! Issue rubash#1901 (Windows synthetic /proc): `cat /proc/stat`
//! reads generated content while `test -e /proc/stat` answered false
//! — the read face consulted the proc registry and the test face did
//! not. The A-phase convergence (rubash#1922) put the test face on
//! `proc_vfs::proc_unary_facts`; this pin is the issue's own
//! regression seed: every synthetic name the content face serves must
//! answer -e/-r true, so the two faces can never drift apart again
//! silently.
//!
//! GNU 5.3.20 oracle: on real procfs each of these names exists and
//! is readable, and `test -e` / `test -r` agree with a successful
//! read of the same name.
//!
//! Windows-gated: off Windows the real procfs serves both faces.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1901-{tag}-{}", std::process::id()));
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
fn issue1901_test_faces_agree_with_content_face() {
    let dir = tempdir("consistency");
    let script = "\
for name in \
  /proc/cpuinfo /proc/loadavg /proc/meminfo /proc/stat /proc/uptime /proc/version \
  /proc/self/status /proc/self/cmdline /proc/self/exe /proc/self/cwd \
  /proc/self /proc \
; do
  ok=ok
  test -e \"$name\" || ok=\"FAIL-e $name\"
  test -r \"$name\" || ok=\"FAIL-r $name\"
  echo \"$ok $name\"
done
# The consistency the issue reported: a name cat can read is a name
# test must call existing.
if cat /proc/stat >/dev/null && test -e /proc/stat; then echo agree-stat; fi
if cat /proc/self/status >/dev/null && test -e /proc/self/status; then echo agree-status; fi
";
    let run = rubash_c(script, &dir);
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert!(
        !run.stdout.contains("FAIL"),
        "a test face disagreed with the content face: stdout={}",
        run.stdout
    );
    assert!(run.stdout.contains("agree-stat"), "stdout={}", run.stdout);
    assert!(
        run.stdout.contains("agree-status"),
        "stdout={}",
        run.stdout
    );
}
