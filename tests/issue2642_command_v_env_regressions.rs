//! Regression pins for rubash#2642: the describe faces (`command -v` /
//! `command -V`, `type`, `type -a`, `type -t`, `type -p` / `-P`) must
//! describe `env` as its PATH file on unix, exactly as GNU bash 5.3
//! does — `command -v env` prints the full path, `command -V env`
//! prints `env is <path>`, `type -t env` prints `file`.
//!
//! GNU oracle: `describe_command` (builtins/type.def:211), shared by
//! `command` (builtins/command.def:113) and `type`
//! (builtins/type.def:180). GNU has no `env` builtin (there is no
//! builtins/env.def; env is a coreutils program), so describe resolves
//! it as a disk file. rubash's `env` row exists only because Windows
//! has no /usr/bin/env (docs/builtins.md §2: the one extension
//! builtin); #2514 adjudicated the emulation-registered names
//! (sed, cat, cp, rm, mkdir, uname) to the same GNU file answer off
//! Windows, but `env` was missed because it is a builtin-table row,
//! not an emulation-registry claim — the #2514 gate in
//! `describe_emulated_claim_with_io` never saw it, and the describe
//! faces short-circuited on `is_enabled_shell_builtin_name("env")`.
//!
//! #2642 completes #2514 for that one row, describe-only: off
//! Windows the describe faces fall through to the PATH branch.
//! Dispatch routing, `enable -n env`, `compgen -b`, `builtin env`
//! and the `help env` topic keep the unified projection and are
//! pinned as unchanged below. This battery is therefore unix-only.
//!
//! Before the fix, `command -v env` printed the bare name `env` and
//! `command -V env` printed `env is a shell builtin`.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
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
        .join("rubash")
}

struct Output {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

/// Run one `-c` script in `dir` with an explicit PATH (never the
/// ambient one, so hashed/fake entries cannot leak between shapes).
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
        code: output.status.code(),
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2642-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn write_fake(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    fs::write(&path, body).expect("write fake");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod fake");
}

/// The first executable `env` on `path`, exactly as a PATH walk
/// finds it — the oracle for the ambient-PATH shape.
fn find_env_on_path(path: &str) -> Option<PathBuf> {
    for dir in path.split(':') {
        if dir.is_empty() {
            continue;
        }
        let candidate = Path::new(dir).join("env");
        if let Ok(meta) = fs::metadata(&candidate) {
            if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
                return Some(candidate);
            }
        }
    }
    None
}

/// The issue's exact seed against a controlled PATH: every describe
/// face reports the fake PATH file, never the builtin answer.
#[test]
fn issue2642_command_v_env_reports_path_file() {
    let root = scratch("fakebin");
    let fakebin = root.join("fakebin");
    fs::create_dir_all(&fakebin).expect("fakebin dir");
    write_fake(&fakebin, "env", "#!/bin/sh\necho FAKE-env\n");
    let fake_path = fakebin.to_str().unwrap();
    let expected = format!("{fake_path}/env");

    let out = run(
        &root,
        fake_path,
        "command -v env; command -V env; type env; echo t=$(type -t env); type -p env; type -P env",
    );
    assert_eq!(
        out.stdout,
        format!(
            "{expected}\nenv is {expected}\nenv is {expected}\nt=file\n{expected}\n{expected}\n"
        ),
        "stdout: {}, stderr: {}",
        out.stdout,
        out.stderr
    );

    // `type -a` prints the file line only — no builtin line (#2642),
    // no emulation claim line (#2514).
    let out = run(&root, fake_path, "type -a env");
    assert_eq!(
        out.stdout,
        format!("env is {expected}\n"),
        "stdout: {}, stderr: {}",
        out.stdout,
        out.stderr
    );
    assert!(
        !out.stdout.contains("builtin") && !out.stdout.contains("emulation"),
        "no builtin/emulation line: {}",
        out.stdout
    );
}

/// The issue's exact seed on the ambient PATH, against the GNU
/// oracle (`command -v env` -> /usr/bin/env, `command -V env` ->
/// `env is /usr/bin/env`).
#[test]
fn issue2642_command_v_env_ambient_path_matches_gnu() {
    let dir = scratch("ambient");
    let path = std::env::var("PATH").unwrap_or_default();
    let expected = find_env_on_path(&path).expect("ambient PATH must carry an env");
    let expected = expected.to_str().unwrap().to_string();
    let out = run(&dir, &path, "command -v env; command -V env");
    assert_eq!(
        out.stdout,
        format!("{expected}\nenv is {expected}\n"),
        "stdout: {}, stderr: {}",
        out.stdout,
        out.stderr
    );
}

/// Stand-down honesty: when `env` does not resolve, no builtin
/// answer is reported — the not-found answer returns, exactly as
/// GNU's describe_command does for an unresolvable name.
#[test]
fn issue2642_command_v_env_not_found_on_empty_path() {
    let root = scratch("emptybin");
    let emptybin = root.join("emptybin");
    fs::create_dir_all(&emptybin).expect("emptybin dir");
    let empty_path = emptybin.to_str().unwrap();
    let out = run(
        &root,
        empty_path,
        "echo v=\"$(command -v env)\"; echo t=\"$(type -t env)\"; type env; echo rc=$?",
    );
    assert!(out.stdout.contains("v=\n"), "{}", out.stdout);
    assert!(out.stdout.contains("t=\n"), "{}", out.stdout);
    assert!(out.stdout.contains("rc=1"), "{}", out.stdout);
    assert!(
        !out.stdout.contains("builtin"),
        "unresolvable env must not report builtin: {}",
        out.stdout
    );
}

/// Dispatch is untouched by the fix: the bare name still runs the
/// in-process env (print mode, assignment form, `builtin env`),
/// while the `command` prefix still reaches the disk file.
#[test]
fn issue2642_env_dispatch_still_in_process() {
    let root = scratch("dispatch");
    let fakebin = root.join("fakebin");
    fs::create_dir_all(&fakebin).expect("fakebin dir");
    write_fake(&fakebin, "env", "#!/bin/sh\necho FAKE-env\n");
    let fake_path = fakebin.to_str().unwrap();

    let out = run(
        &root,
        fake_path,
        "export MARKER2642=present; out=$(env); case \"$out\" in *MARKER2642=present*) echo print-ok;; *) echo print-miss;; esac; \
         out=$(env FOO2642=zzz env); case \"$out\" in *FOO2642=zzz*) echo assign-ok;; *) echo assign-miss;; esac; \
         out=$(builtin env); case \"$out\" in *MARKER2642=present*) echo builtin-ok;; *) echo builtin-miss;; esac; \
         echo disk=$(command env)",
    );
    assert!(out.stdout.contains("print-ok"), "{}", out.stdout);
    assert!(out.stdout.contains("assign-ok"), "{}", out.stdout);
    assert!(out.stdout.contains("builtin-ok"), "{}", out.stdout);
    assert!(
        out.stdout.contains("disk=FAKE-env"),
        "`command env` must reach the disk file: {}",
        out.stdout
    );
}

/// `enable -n env` still disables the in-process implementation to
/// the PATH body (pinned by issue2036 §3.1; guarded here so the
/// describe gate cannot silently break the disable path).
#[test]
fn issue2642_enable_n_env_still_disables_to_path() {
    let root = scratch("disable");
    let fakebin = root.join("fakebin");
    fs::create_dir_all(&fakebin).expect("fakebin dir");
    write_fake(&fakebin, "env", "#!/bin/sh\necho FAKE-env\n");
    let fake_path = fakebin.to_str().unwrap();

    let out = run(
        &root,
        fake_path,
        "enable -n env; echo envn_rc=$?; echo t=$(type -t env); echo off=$(env)",
    );
    assert!(out.stdout.contains("envn_rc=0"), "{}", out.stdout);
    assert!(out.stdout.contains("t=file"), "{}", out.stdout);
    assert!(out.stdout.contains("off=FAKE-env"), "{}", out.stdout);
}
