//! rubash#655 (W5 residual / characterization pin): two
//! function-redirect-stdin tests from rubash#653 fail deterministically
//! on Android/Termux hosts while passing on desktop hosts:
//! `issue647_function_redirect_stdin_regressions::
//! function_call_file_redirect_survives_external_spawn` and
//! `executor_tests::command_chaining::part_054::
//! test_function_definition_stdin_redirect_overrides_call_stdin`.
//! Both are fd-redirection + external-spawn shapes; the recorded
//! disposition is that this is an Android host-environment family
//! difference (external process / fd-table behavior on that host, cf.
//! rubash#615), to be confirmed per-host rather than "fixed" in the
//! engine blind.
//!
//! This file pins the two shapes in script form on non-Android hosts,
//! where the GNU-matching behavior holds: if they ever fail here, that
//! is a real engine regression, not host noise. The file compiles to
//! nothing on Android so the suite there does not re-litigate the host
//! residual; the Android failure itself remains recorded on #655.
#![cfg(not(target_os = "android"))]

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-655-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

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
fn issue655_function_call_file_redirect_survives_external_spawn() {
    // Same shape as the #653 fix's own pin in
    // tests/issue647_function_redirect_stdin_regressions.rs, re-asserted
    // under #655: the call-site file redirect keeps the shared offset
    // across a real external spawn between reads.
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
fn issue655_function_definition_stdin_redirect_shape() {
    // Script form of executor_tests part_054's
    // test_function_definition_stdin_redirect_overrides_call_stdin: a
    // definition-site stdin redirect supplies the function's input.
    let dir = scratch_dir();
    let script = r#"
printf 'definition\n' > def.txt
printf 'call\n' > call.txt
f() { read line; echo "$line"; } < def.txt
f < call.txt
"#;
    let (stdout, stderr, status) = run_in_dir(script, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stdout, "definition\n", "stderr: {stderr:?}");
    assert_eq!(status, Some(0), "stderr: {stderr:?}");
}
