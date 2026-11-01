//! Regression pins for rubash#2514: the lookup/reporting faces
//! (`command -v` / `command -V`, `type`, `type -a`, `type -t`,
//! `type -p` / `-P`, and the `hash` listing) must describe a real
//! PATH file exactly as GNU bash 5.3 does, even when the name is
//! registered in rubash's emulation layer.
//!
//! GNU oracle: `describe_command` (builtins/type.def:211), shared by
//! `command` (builtins/command.def:113) and `type`
//! (builtins/type.def:180). Its kind words are only alias / keyword /
//! function / builtin / file; a resolved disk file reports the full
//! path for `-v`, "X is <path>" (or "X is hashed (<path>)") for the
//! verbose forms, and `file` for `-t`. There is no emulation kind.
//!
//! Before the fix, the describe faces consulted the registry claim
//! first (#2036 S2 disclosure): for every resolvable registry name
//! `command -v sed` printed the bare name, `type sed` printed
//! "sed is a shell emulation of /usr/bin/sed", `type -a` prepended
//! that claim line, and `type -t sed` printed `emulation` — while
//! execution itself resolved the real file. That broke reporting
//! consumers, modernish's `sys/base/rev` probe being the field case:
//! it requires an absolute path out of `$(PATH=$DEFPATH; command -v
//! sed)` and aborted with "Can't find a functioning 'sed'".
//!
//! #2514 adjudication: the S2 disclosure survives only on the
//! Windows product face (pinned by
//! `issue2036_s2_introspection_windows`); off Windows the describe
//! faces are GNU-shaped. This battery is therefore unix-only.
//! Dispatch routing is untouched by the fix and not pinned here.

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
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2514-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn write_fake(dir: &Path, name: &str) {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\necho FAKE-{name}\n")).expect("write fake");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod fake");
}

/// Resolve `name` the way the host tools do, under an explicit PATH,
/// so expectations name the same file GNU would report.
fn which(path: &str, name: &str) -> String {
    let out = Command::new("which")
        .arg(name)
        .env("PATH", path)
        .output()
        .expect("which");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// sed (EmulatedExternal) and cat (EmulatedExternal) describe as
/// their PATH file on every face, with the resolution under the
/// test's control: a fakebin script shadows the system tools, and
/// the reported path must be the fake's — the describe faces never
/// execute the body they resolve, so the script form is safe and
/// makes the expected string exact.
#[test]
fn issue2514_registry_names_describe_as_path_file() {
    let root = scratch("file");
    let fakebin = root.join("fakebin");
    fs::create_dir_all(&fakebin).expect("fakebin");
    write_fake(&fakebin, "sed");
    write_fake(&fakebin, "cat");
    let path = format!("{}:/usr/bin:/bin", fakebin.display());
    let fake_sed = format!("{}/sed", fakebin.display());
    let fake_cat = format!("{}/cat", fakebin.display());

    let out = run(
        &root,
        &path,
        "echo v_sed=$(command -v sed); echo V_sed=\"$(command -V sed)\"; \
         echo t_sed=$(type -t sed); echo plain_sed=\"$(type sed)\"; \
         echo p_sed=$(type -p sed); echo P_sed=$(type -P sed); \
         echo v_cat=$(command -v cat); echo t_cat=$(type -t cat); \
         echo plain_cat=\"$(type cat)\"",
    );
    assert!(
        out.stdout.contains(&format!("v_sed={fake_sed}")),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains(&format!("V_sed=sed is {fake_sed}")),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("t_sed=file"), "{}", out.stdout);
    assert!(
        out.stdout.contains(&format!("plain_sed=sed is {fake_sed}")),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains(&format!("p_sed={fake_sed}")),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains(&format!("P_sed={fake_sed}")),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains(&format!("v_cat={fake_cat}")),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("t_cat=file"), "{}", out.stdout);
    assert!(
        out.stdout.contains(&format!("plain_cat=cat is {fake_cat}")),
        "{}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("emulation"),
        "no describe face may disclose the claim off Windows (#2514): {}",
        out.stdout
    );

    // type -a enumerates the PATH hits and nothing else: the first
    // line is the fakebin hit, every line is a file line, and the
    // system hit follows (GNU user_command_matches order).
    let out = run(&root, &path, "type -a sed");
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert!(lines.len() >= 2, "{}", out.stdout);
    assert_eq!(lines[0], format!("sed is {fake_sed}"), "{}", out.stdout);
    assert!(
        lines.iter().all(|l| l.starts_with("sed is /")),
        "{}",
        out.stdout
    );
    assert!(
        lines.iter().any(|l| *l == "sed is /usr/bin/sed"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("emulation"), "{}", out.stdout);
}

/// uname (HiddenFastpath, ScriptYield dispatch): when PATH resolves
/// the name to a user script, the reporting must name that script —
/// GNU has no uname builtin to outrank the file, and the script is
/// what dispatch runs.
#[test]
fn issue2514_uname_script_resolution_reports_script() {
    let root = scratch("uname");
    let fakebin = root.join("fakebin");
    fs::create_dir_all(&fakebin).expect("fakebin");
    write_fake(&fakebin, "uname");
    let path = format!("{}:/usr/bin:/bin", fakebin.display());
    let fake_uname = format!("{}/uname", fakebin.display());
    let out = run(
        &root,
        &path,
        "echo v=$(command -v uname); echo t=$(type -t uname); echo plain=\"$(type uname)\"",
    );
    assert!(
        out.stdout.contains(&format!("v={fake_uname}")),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("t=file"), "{}", out.stdout);
    assert!(
        out.stdout.contains(&format!("plain=uname is {fake_uname}")),
        "{}",
        out.stdout
    );
}

/// The modernish shape: a temporary PATH assignment inside the
/// command substitution, then an absolute-path check on the result
/// (vendor/modernish sys/base/rev). GNU yields the resolved absolute
/// path; the bare name aborts modernish's probe.
#[test]
fn issue2514_modernish_command_v_shape_yields_absolute_path() {
    let root = scratch("msh");
    let ambient = std::env::var("PATH").unwrap_or_default();
    let sed_path = which("/usr/bin:/bin", "sed");
    assert!(sed_path.starts_with('/'), "oracle sed must resolve");
    let out = run(
        &root,
        &ambient,
        "x=$(PATH=/usr/bin:/bin; command -v sed); echo \"v=$x\"; \
         case $x in /*) echo abs=yes;; *) echo abs=no;; esac; \
         y=$(PATH=/usr/bin:/bin; command -v cat); echo \"vcat=$y\"",
    );
    assert!(
        out.stdout.contains(&format!("v={sed_path}")),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("abs=yes"), "{}", out.stdout);
    assert!(out.stdout.contains("vcat=/"), "{}", out.stdout);
}

/// After a real execution records the resolution, GNU's verbose
/// forms switch to the hashed wording (builtins/type.def phash
/// branch) and `command -v` still prints the path; the `hash`
/// listing names the resolved file. All three must hold for a
/// registry name too.
#[test]
fn issue2514_hashed_resolution_reports_gnu_forms() {
    let root = scratch("hash");
    let ambient = std::env::var("PATH").unwrap_or_default();
    let sed_path = which(&ambient, "sed");
    assert!(sed_path.starts_with('/'), "oracle sed must resolve");
    let out = run(
        &root,
        &ambient,
        "sed --version >/dev/null; echo plain=\"$(type sed)\"; \
         echo V=\"$(command -V sed)\"; echo v=$(command -v sed); hash",
    );
    assert!(
        out.stdout
            .contains(&format!("plain=sed is hashed ({sed_path})")),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains(&format!("V=sed is hashed ({sed_path})")),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains(&format!("v={sed_path}")),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains(&sed_path), "{}", out.stdout);
    assert!(!out.stdout.contains("emulation"), "{}", out.stdout);
}

/// Control: a name with no registry row (ls) keeps the file answer
/// it always had — the fix must not move non-registry names.
#[test]
fn issue2514_ls_non_registry_control() {
    let root = scratch("ls");
    let ambient = std::env::var("PATH").unwrap_or_default();
    let ls_path = which(&ambient, "ls");
    assert!(ls_path.starts_with('/'), "oracle ls must resolve");
    let out = run(
        &root,
        &ambient,
        "echo v=$(command -v ls); echo t=$(type -t ls); echo plain=\"$(type ls)\"",
    );
    assert!(
        out.stdout.contains(&format!("v={ls_path}")),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("t=file"), "{}", out.stdout);
    assert!(
        out.stdout.contains(&format!("plain=ls is {ls_path}")),
        "{}",
        out.stdout
    );
}

/// Empty-PATH control: when the name does not resolve, both shells
/// agree it is not found (rc 1, no answer) — the #2514 change must
/// not manufacture an answer for unresolvable registry names.
#[test]
fn issue2514_empty_path_not_found_control() {
    let root = scratch("empty");
    let empty = root.join("emptybin");
    fs::create_dir_all(&empty).expect("emptybin");
    let out = run(
        &root,
        empty.to_str().unwrap(),
        "command -v sed; echo v_rc=$?; type -t sed; echo t_rc=$?; \
         type sed 2>/dev/null; echo plain_rc=$?",
    );
    assert!(out.stdout.contains("v_rc=1"), "{}", out.stdout);
    assert!(out.stdout.contains("t_rc=1"), "{}", out.stdout);
    assert!(out.stdout.contains("plain_rc=1"), "{}", out.stdout);
    assert!(!out.stdout.contains("emulation"), "{}", out.stdout);
    assert!(!out.stdout.contains("sed is"), "{}", out.stdout);
}
