//! Issue rubash#537 regression: Windows-native `$PWD` is ALWAYS the POSIX
//! `/<drive>/…` form (msys persona), even when a host exports
//! `WINUXSH_SHELL_PATH_STYLE=native` / `NIU_SHELL_PATH_STYLE=native` —
//! which niu/winuxsh do for the entire shell lifetime.
//!
//! The pre-#537 escape hatch rendered `$PWD` as `D:/…` under those hosts
//! and `nvm.sh` v0.40.8's `nvm_find_up` death-looped on it: the walk's only
//! terminations are `""` / `"."` / a found `.nvmrc`, and `${path_%/*}`
//! never shortens `D:` (GNU bash given the same PWD form hangs identically;
//! real msys bash never *produces* that form — `D:/Git/usr/bin/bash.exe -c
//! 'echo $PWD'` prints `/d/…`). This file is perf probe 24
//! (benchmarks/24-nvm-load.sh, nvm auto-load) upgraded to a regression
//! tripwire that also runs on Windows CI.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const STYLE_VARS: [&str; 3] = [
    "WINUXSH_SHELL_PATH_STYLE",
    "NIU_SHELL_PATH_STYLE",
    "__RUBASH_PATH_STYLE",
];

/// Spawn rubash with a hard wall-clock budget: on expiry the child is
/// killed and the test fails naming the issue. A pre-fix binary HANGS the
/// nvm cell (rc would never arrive), so a plain `output()` would stall the
/// suite — the watchdog turns the silent hang into a named failure.
fn run_bounded(
    args: &[&str],
    native_style: bool,
    budget: Duration,
) -> (Option<i32>, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in STYLE_VARS {
        command.env_remove(name);
    }
    if native_style {
        // The niu/winuxsh host contract: native display spelling. Post-#537
        // this must be inert for PWD semantics.
        command.env("WINUXSH_SHELL_PATH_STYLE", "native");
        command.env("NIU_SHELL_PATH_STYLE", "native");
    }
    let mut child = command.spawn().expect("spawn rubash");
    let started = Instant::now();
    loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => {
                // Drain output after exit (pipes hold it all).
                use std::io::Read;
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = pipe.read_to_string(&mut stdout);
                }
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                return (status.code(), stdout, stderr);
            }
            None => {
                if started.elapsed() > budget {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "rubash did not exit within {budget:?} (unixwin/rubash#537 hang shape: \
                         drive-form PWD death-loops nvm_find_up / native PWD rendered)"
                    );
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// `$PWD`, `$(pwd)`, `pwd`, and `pwd -P` stay in the POSIX `/<drive>/…`
/// domain — leading `/`, no drive colon — after startup AND after a `cd`,
/// with and without the host native-style variable.
#[test]
fn pwd_family_stays_posix_with_host_native_style() {
    if !cfg!(windows) {
        // On POSIX platforms the form is native POSIX regardless; the
        // style variables are a Windows host contract.
        return;
    }
    for native_style in [true, false] {
        let script =
            "echo PWD=$PWD; echo SUB=$(pwd); echo BUILTIN=$(pwd -P); cd /; echo AFTER=$PWD";
        let (code, stdout, stderr) =
            run_bounded(&["-c", script], native_style, Duration::from_secs(30));
        assert_eq!(code, Some(0), "stderr: {stderr}");
        for (tag, value) in [
            ("PWD", stdout.lines().find(|l| l.starts_with("PWD="))),
            ("SUB", stdout.lines().find(|l| l.starts_with("SUB="))),
            (
                "BUILTIN",
                stdout.lines().find(|l| l.starts_with("BUILTIN=")),
            ),
            ("AFTER", stdout.lines().find(|l| l.starts_with("AFTER="))),
        ] {
            let value = value.unwrap_or_else(|| panic!("missing {tag} in {stdout:?}"));
            let value = value.split_once('=').unwrap().1;
            assert!(
                value.starts_with('/') && !value.contains(':') && !value.contains('\\'),
                "native_style={native_style}: {tag}={value:?} is not POSIX form (rubash#537)"
            );
        }
    }
}

fn nvm_corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/regression/corpus/nvm/nvm.sh")
}

/// Perf probe 24 (benchmarks/24-nvm-load.sh) as a hard regression: source
/// the real nvm.sh v0.40.8 into a scratch NVM_DIR from a drive-letter cwd
/// and let its auto-use entry (`nvm_process_parameters` -> `nvm_auto use`
/// -> `nvm_rc_version` -> `nvm_find_up('.nvmrc')`) run to completion —
/// including the empty-NVM_DIR/no-default-alias/no-.nvmrc canonical form
/// the perf suite uses. Must finish far inside the 60s probe guard, with
/// the host native-style variable present (the pre-#537 hang trigger).
#[test]
fn nvm_auto_load_terminates_within_probe_budget() {
    let corpus = nvm_corpus();
    assert!(corpus.is_file(), "missing vendored nvm.sh at {corpus:?}");
    let scratch = std::env::temp_dir().join(format!("rubash-537-nvm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let script_path = scratch.join("nvm-auto-537.sh");
    let mut script = std::fs::File::create(&script_path).unwrap();
    let corpus_display = corpus.to_string_lossy().replace('\\', "/");
    // Probe-24 shape: NVM_DIR pinned under a scratch tree, source runs the
    // auto-use entry; then the explicit entry the issue bisect hung on.
    writeln!(
        script,
        "NVM_DIR=\"{}/nvmdir\"",
        scratch.to_string_lossy().replace('\\', "/")
    )
    .unwrap();
    writeln!(script, "mkdir -p \"$NVM_DIR\"").unwrap();
    writeln!(script, ". \"{corpus_display}\"").unwrap();
    writeln!(script, "nvm_auto use").unwrap();
    writeln!(script, "echo NVM537_DONE").unwrap();
    drop(script);

    let budget = Duration::from_secs(60);
    for native_style in [true, false] {
        let (code, stdout, stderr) =
            run_bounded(&[&script_path.to_string_lossy()], native_style, budget);
        assert_eq!(
            code,
            Some(0),
            "native_style={native_style}: nvm.sh auto-load rc={code:?}, stderr: {stderr}"
        );
        assert!(
            stdout.contains("NVM537_DONE"),
            "native_style={native_style}: auto-load did not complete; stdout: {stdout:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);
}
