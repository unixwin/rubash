//! Windows product-form pins for rubash#2036 S2 (introspection
//! unification + disclosure ruling).
//!
//! The S1/S2 unix battery (`issue2036_precedence_introspection_regressions`)
//! drives fake bodies as PATH shell scripts, which Windows cannot
//! execute. The describe faces never execute the body they resolve,
//! so on Windows the product form — WinuxCmd-style applet files
//! (`stty.exe`, `cat.exe`, ...) sitting on PATH — can be pinned with
//! empty placeholder files: `type` only resolves them.
//!
//! What is pinned here, against a PATH containing only the fake
//! applet directory:
//! - the simple-command claims disclose: `type -t stty` -> `emulation`,
//!   `type stty` -> "stty is a shell emulation of <...stty.exe>",
//!   `command -v cat` -> the bare name;
//! - `tput`'s Windows-only claim is live on Windows (it is not live
//!   in the unix battery);
//! - with an empty PATH the claim stands down: no `emulation` answer;
//! - the unified builtin projection reaches the product faces:
//!   `compgen -b` and `enable -a` both list `env` (KD-1), and
//!   `help env` has a topic (env was the one builtin without one).

#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_bash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join("bash.exe")
}

struct Output {
    stdout: String,
    stderr: String,
}

fn run(dir: &Path, path: &str, script: &str) -> Output {
    let output = Command::new(shell_bin())
        .args(["-c", script])
        .current_dir(dir)
        .env("PATH", path)
        .env_remove("BASH_ENV")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Output {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).replace('\r', ""),
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2036s2-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn issue2036_s2_windows_applet_path_disclosure() {
    let root = scratch("win");
    let fakebin = root.join("fakebin");
    fs::create_dir_all(&fakebin).expect("fakebin");
    for name in ["stty.exe", "cat.exe", "tput.exe", "sleep.exe"] {
        fs::write(fakebin.join(name), b"").expect("fake applet");
    }
    let path = fakebin.to_str().unwrap().to_string();
    let out = run(
        &root,
        &path,
        "echo stty=$(type -t stty); type stty; echo cat=$(command -v cat); \
         echo tput=$(type -t tput); echo sleep=$(type -t sleep)",
    );
    assert!(out.stdout.contains("stty=emulation"), "{}", out.stdout);
    assert!(
        out.stdout.contains("stty is a shell emulation of ") && out.stdout.contains("stty.exe"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("cat=cat"), "{}", out.stdout);
    assert!(out.stdout.contains("tput=emulation"), "{}", out.stdout);
    assert!(out.stdout.contains("sleep=emulation"), "{}", out.stdout);

    // Stand-down honesty on the product face too: empty PATH, no claim.
    let empty = scratch("winempty");
    let out = run(&root, empty.to_str().unwrap(), "type -t stty; echo rc=$?");
    assert!(!out.stdout.contains("emulation"), "{}", out.stdout);
    assert!(out.stdout.contains("rc=1"), "{}", out.stdout);
}

#[test]
fn issue2036_s2_windows_builtin_projection_faces() {
    let root = scratch("winproj");
    let path = std::env::var("PATH").unwrap_or_default();
    let out = run(
        &root,
        &path,
        "compgen -b | sort; echo ---; enable -a | wc -l; enable -s | wc -l; help env",
    );
    let lines: Vec<&str> = out.stdout.lines().collect();
    let sep = lines.iter().position(|l| *l == "---").expect("separator");
    let compgen_names = &lines[..sep];
    assert_eq!(compgen_names.len(), 62, "{:?}", compgen_names);
    assert!(compgen_names.contains(&"env"), "{:?}", compgen_names);
    assert!(
        out.stdout
            .contains("Run a command in a modified environment"),
        "{}",
        out.stdout
    );
    // enable -a lists the same 62; enable -s lists the 16 specials.
    let counts: Vec<&str> = lines[sep + 1..sep + 3].to_vec();
    assert_eq!(counts[0].trim(), "62", "{}", out.stdout);
    assert_eq!(counts[1].trim(), "16", "{}", out.stdout);
}
