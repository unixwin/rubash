//! rubash#588: after a PATH-limiting ("starving") assignment, command lookup
//! fell back to out-of-band Windows namespaces — the embedded /bin display
//! paths and the winuxcmd dispatcher — instead of reporting GNU's 127.
//!
//! Repro (affected hosts, e.g. the winuxcmd install where WINUXCMD_PATH is
//! exported): `PATH=/nonexistent; hash -r; ls; echo $?` printed a real
//! directory listing with rc 0, and `command -v ls` reported `/bin/ls`
//! (or the dispatcher executable), where GNU bash reports
//! `ls: command not found` rc 127 and `command -v ls` rc 1.
//!
//! Root cause: `find_user_command_uncached` (src/executor/path.rs) consulted
//! the winuxcmd dispatcher and `lookup_paths.rs command_path`/`command_paths`
//! injected the /bin-namespace display paths REGARDLESS of PATH state, so any
//! explicit PATH assignment — plain, `export`, or a tempenv prefix — lost to
//! the fixture whenever its directories resolved nothing.
//!
//! GNU contract (vendored bash, third_party/bash): search_for_command feeds
//! the PATH variable verbatim to the walk (findcmd.c:383-386
//! `path_list = value_cell(path)`, empty -> "."); find_user_command_in_path
//! (findcmd.c:623-697) consults nothing beyond those elements and yields no
//! command when every element misses; a PATH-less shell gets a real startup
//! assignment instead (variables.c:550 set_if_not("PATH",
//! DEFAULT_PATH_VALUE)). The fixture namespaces stand in for that startup
//! default only: they stay legitimate while PATH still equals the value the
//! process imported (init.rs __RUBASH_INITIAL_PATH), and stand down once a
//! shell-level assignment moves resolution under the script's control.
//! Prepending/shadowing (`PATH=$dir:$PATH`) keeps working through the walk
//! itself — only the starvation direction was broken.
//!
//! Baselines: WSL GNU bash 5.3.0 script-file probes, 2026-10-13
//! (target/probe587/g3.sh, g4.sh): tempenv 127, export 127, plain 127,
//! `command -v ls`/`type -t ls` rc 1 after starvation, `command -v echo`
//! still finds the builtin, `PATH=` normalizes to "." and misses.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(20);

fn run_rubash_script(script: &str) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                use std::io::Read;
                child
                    .stdout
                    .take()
                    .expect("stdout pipe")
                    .read_to_end(&mut stdout)
                    .expect("read stdout");
                child
                    .stderr
                    .take()
                    .expect("stderr pipe")
                    .read_to_end(&mut stderr)
                    .expect("read stderr");
                return (stdout, stderr, status.code());
            }
            Ok(None) => {
                if start.elapsed() > DEADLINE {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rubash did not exit within {DEADLINE:?}: {script}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("wait rubash: {error}"),
        }
    }
}

fn stdout_string(script: &str) -> String {
    let (stdout, _, _) = run_rubash_script(script);
    String::from_utf8_lossy(&stdout).into_owned()
}

/// The exact issue repro: a starving plain assignment must be respected —
/// the external is not found, rc 127, GNU's diagnostic on stderr.
#[test]
fn issue_repro_starved_path_external_is_127() {
    let (stdout, stderr, code) =
        run_rubash_script(r"PATH=/nonexistent-dir-for-probe; hash -r; ls / >/dev/null; echo rc=$?");
    assert_eq!(code, Some(0), "the script itself completes");
    assert_eq!(stdout, b"rc=127\n");
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(
        stderr.contains("ls") && stderr.contains("command not found"),
        "unexpected stderr: {stderr}"
    );
}

/// `command -v ls` under starvation fails like GNU (rc 1, no output) —
/// no /bin display path, no dispatcher hit.
#[test]
fn starved_command_v_reports_nothing() {
    let (stdout, _, code) =
        run_rubash_script(r"PATH=/nonexistent-dir-for-probe; hash -r; command -v ls; echo rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"rc=1\n");
}

/// `type -t ls` under starvation reports no file/builtin (rc 1).
#[test]
fn starved_type_t_reports_nothing() {
    let (stdout, _, code) =
        run_rubash_script(r"PATH=/nonexistent-dir-for-probe; hash -r; type -t ls; echo rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"rc=1\n");
}

/// A tempenv PATH prefix is a command-scoped assignment: GNU feeds the
/// tempenv cell to the walk (findcmd.c:356-365, 383-386), so a starving
/// one is 127 too.
#[test]
fn starved_tempenv_path_prefix_is_127() {
    let (stdout, _, code) =
        run_rubash_script(r"PATH=/nonexistent-dir-for-probe ls / >/dev/null; echo rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"rc=127\n");
}

/// `export PATH=...` is an explicit assignment with the same contract.
#[test]
fn starved_export_path_is_127() {
    let (stdout, _, code) = run_rubash_script(
        r"export PATH=/nonexistent-dir-for-probe; hash -r; ls / >/dev/null; echo rc=$?",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"rc=127\n");
}

/// An explicitly EMPTY PATH normalizes to "." in GNU (path_value,
/// findcmd.c:273-276): the walk visits the cwd, finds nothing, rc 1 for
/// `command -v`.
#[test]
fn explicitly_empty_path_misses_command_v() {
    let (stdout, _, code) = run_rubash_script(r"PATH=; hash -r; command -v ls; echo rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"rc=1\n");
}

/// `unset PATH` removes the walk's input entirely (GNU path_value returns
/// NULL, findcmd.c:279-286) — the fixture namespaces stand down with it.
#[test]
fn unset_path_misses_command_v() {
    let (stdout, _, code) = run_rubash_script(r"unset PATH; command -v ls; echo rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"rc=1\n");
}

/// Starvation is per-mode: posix mode keeps the same contract.
#[test]
fn starved_path_contract_holds_in_posix_mode() {
    let (stdout, _, code) = run_rubash_script(
        r"set -o posix; PATH=/nonexistent-dir-for-probe; hash -r; ls / >/dev/null; echo rc=$?",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"rc=127\n");
}

/// Builtins and functions are PATH-independent: starvation must not take
/// them down (GNU: `command -v echo` still reports the builtin,
/// g3.sh `cmdv-echo rc=0`).
#[test]
fn starved_path_keeps_builtins_and_functions() {
    let (stdout, _, code) = run_rubash_script(
        r#"PATH=/nonexistent-dir-for-probe; hash -r; echo hi; command -v echo; echo "cmdv=$?"; f588(){ echo FUNC; }; f588"#,
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"hi\necho\ncmdv=0\nFUNC\n");
}

/// The prefix/shadow direction keeps working: a directory prepended by a
/// PATH assignment is consulted by the walk itself (here asserted through
/// `command -v` on a plain file, which needs no exec bit — GNU
/// find_in_path_element's existence tier).
#[test]
fn prepended_directory_is_searched_by_the_walk() {
    let dir = std::env::temp_dir().join(format!("rubash588-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create shadow dir");
    std::fs::write(dir.join("mytool588"), "#!/bin/sh\necho hi\n").expect("write shadow tool");
    // Shell-form the directory for the `:`-separated PATH (rubash#175): on
    // Windows the temp dir arrives as `C:\...` and PATH entries are
    // `/c/...`.
    let mut dir_text = dir.to_string_lossy().replace('\\', "/");
    if let Some(rest) = dir_text.strip_prefix("//?/") {
        dir_text = rest.to_string();
    }
    let shell_form = if cfg!(windows) {
        let bytes = dir_text.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' {
            format!(
                "/{}{}",
                (bytes[0] as char).to_ascii_lowercase(),
                &dir_text[2..]
            )
        } else {
            dir_text.clone()
        }
    } else {
        dir_text.clone()
    };
    let script = format!("PATH={shell_form}:$PATH; command -v mytool588; echo rc=$?");
    let (stdout, _, code) = run_rubash_script(&script);
    assert_eq!(code, Some(0));
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    assert!(
        stdout.contains("mytool588") && stdout.ends_with("rc=0\n"),
        "prepended directory not searched: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Control: with no assignment (the startup fixture state) lookup keeps
/// resolving, and an identity re-assignment does not change that.
#[test]
fn unassigned_and_identity_path_still_resolve() {
    let out = stdout_string(r"command -v ls >/dev/null; echo rc=$?");
    assert_eq!(out, "rc=0\n");
    let out = stdout_string(r"PATH=$PATH; command -v ls >/dev/null; echo rc=$?");
    assert_eq!(out, "rc=0\n");
}

/// Rejoining the inherited PATH keeps resolving through the walk — the
/// directories that physically hold the tools still answer after an
/// assignment (GNU: the walk answers from the variable's elements).
#[test]
fn rejoined_path_still_resolves_via_the_walk() {
    let out = stdout_string(
        r"PATH=$PATH:/nonexistent-dir-for-probe; command -v ls >/dev/null; echo rc=$?",
    );
    assert_eq!(out, "rc=0\n");
}
