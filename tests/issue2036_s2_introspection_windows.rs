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
//! rubash#2514 (2026-11-02) adjudicated the S2 describe disclosure
//! to this Windows product face only: off Windows the describe faces
//! answer the GNU file form (see the #2514 update in
//! `issue2036_precedence_introspection_regressions`).
//!
//! niubash#993 §4 (2026-11-16) then retired the disclosure on the
//! Windows face too: GNU `describe_command` (builtins/type.def:211)
//! knows no `emulation` type word and never answers a resolvable
//! external name with the bare name — every describe face prints the
//! real resolved file, exactly as it does off Windows.
//!
//! What is pinned here, against a PATH containing only the fake
//! applet directory:
//! - the describe faces report the GNU file answer: `type -t stty`
//!   -> `file`, `type stty` -> "stty is <...stty.exe>",
//!   `command -v cat` -> the resolved <...cat.exe> path;
//! - `tput`'s Windows-only claim resolves to its applet file;
//! - no face ever prints the retired `emulation` type word;
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
    _stderr: String,
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
        _stderr: String::from_utf8_lossy(&output.stderr).replace('\r', ""),
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
    // niubash#993 §4: the GNU file answer on every describe face —
    // `file` for -t, the real resolved path for `type`/`command -v`,
    // and never the retired `emulation` word nor a bare name.
    assert!(out.stdout.contains("stty=file"), "{}", out.stdout);
    assert!(out.stdout.contains("stty is "), "{}", out.stdout);
    // niubash#993 S7: the displayed join drops the .exe probe suffix.
    assert!(!out.stdout.contains("stty.exe"), "{}", out.stdout);
    assert!(out.stdout.contains("cat=/"), "{}", out.stdout);
    assert!(!out.stdout.contains("cat.exe"), "{}", out.stdout);
    assert!(!out.stdout.contains("cat=cat"), "{}", out.stdout);
    assert!(out.stdout.contains("tput=file"), "{}", out.stdout);
    assert!(out.stdout.contains("sleep=file"), "{}", out.stdout);
    assert!(!out.stdout.contains("emulation"), "{}", out.stdout);

    // No describe face may print the retired type word, whatever PATH
    // resolves — an empty dir may still report `file` when the applet
    // bundle supplies the body (dispatch would execute it), but GNU's
    // vocabulary has no `emulation` anywhere.
    let empty = scratch("winempty");
    let out = run(&root, empty.to_str().unwrap(), "type -t stty; echo rc=$?");
    assert!(!out.stdout.contains("emulation"), "{}", out.stdout);
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
