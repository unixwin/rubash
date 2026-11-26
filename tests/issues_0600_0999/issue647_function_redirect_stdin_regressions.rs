//! rubash#647: a function call-site file redirect must keep the real fd 0
//! open across external children. GNU Bash 5.3 leaves the shared file offset
//! at the line consumed by `read`; the child does not drain the rest of the
//! function input into a text mirror.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-647-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Resolve an external `true` via PATH. The tests below need a REAL
/// external spawn between reads (a `true` builtin would not fork), and
/// the hardcoded Linux path does not exist on Android (Termux has no
/// /usr/bin; rubash#655). An absolute path also bypasses builtin lookup,
/// so the spawn stays external on every host.
fn external_true() -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in ["true", "true.exe"] {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn run_in_dir(script: &str, cwd: &Path) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(cwd)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn function_call_file_redirect_survives_external_spawn() {
    let Some(true_bin) = external_true() else {
        eprintln!("skipping: no external true on PATH");
        return;
    };
    let dir = scratch_dir();
    let script = format!(
        r#"
printf 'l1\nl2\nl3\nl4\n' > input
F () {{ while read l; do echo "iter[$l]"; {true_bin}; done; }}
F < input
"#
    );
    let (stdout, stderr, status) = run_in_dir(&script, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        stdout, "iter[l1]\niter[l2]\niter[l3]\niter[l4]\n",
        "stderr: {stderr:?}"
    );
    assert_eq!(status, Some(0), "stderr: {stderr:?}");
}

#[test]
fn function_call_file_redirect_keeps_next_read_at_shared_offset() {
    let Some(true_bin) = external_true() else {
        eprintln!("skipping: no external true on PATH");
        return;
    };
    let dir = scratch_dir();
    let script = format!(
        r#"
printf 'l1\nl2\nl3\nl4\n' > input
F () {{
  while read l; do
    echo "first[$l]"
    {true_bin}
    if read l2; then echo "second[$l2]"; else echo 'second=EOF'; fi
    return
  done
}}
F < input
"#
    );
    let (stdout, stderr, status) = run_in_dir(&script, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stdout, "first[l1]\nsecond[l2]\n", "stderr: {stderr:?}");
    assert_eq!(status, Some(0), "stderr: {stderr:?}");
}
