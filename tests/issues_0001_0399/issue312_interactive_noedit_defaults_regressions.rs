//! Issue rubash#312 regressions (wt13/sweepfix lane).
//!
//! Interactive no-tty startup defaults diverged: with stdin not a tty
//! (the normal `--rcfile` harness shape), rubash left the editing mode
//! OFF (`[[ -o emacs ]]` false) for `-i --rcfile`, `-i -s` and `-i -c`,
//! and `$-` lacked `H` in the `--rcfile` path. ble.sh aborted at its
//! noediting gate (`[[ ! -o emacs && ! -o vi ]]` true) for any PTY-less
//! load.
//!
//! GNU contract (vendored third_party/bash, re-probed 2026-09-29 with
//! script-file drivers — the issue's `-i -c` row misreported `hBc`; the
//! verified matrix is):
//!
//! | invocation (stdin /dev/null) | `$-` | `[[ -o emacs ]]` |
//! | --- | --- | --- |
//! | `-i -c '…'` | `himBHc` | ON |
//! | `-i --rcfile rc` | `himBH` | ON |
//! | `-i -s` (pipe) | `himBHs` | ON |
//! | `-i file.sh` | `himBH` | OFF |
//! | `-c '…'` | `hBc` | OFF |
//!
//! (The `$-` column was probed with a terminal the shell could
//! claim, so it carries `m`; without one GNU drops the letter —
//! rubash#2948. The asserts below factor `m` out; the editing-mode
//! column is terminal-independent.)
//!
//! - shell.c:540-549: `-i` forces init_interactive() IMMEDIATELY, before
//!   the startup files run.
//! - shell.c:1830-1842 init_interactive: history defaults on and
//!   histexp_flag set (the H) — and it never touches no_line_editing.
//! - shell.c:1863-1870 init_interactive_script (the `bash -i script`
//!   path, shell.c:1714-1718): runs init_noninteractive first, whose
//!   shell.c:1853 sets `no_line_editing = 1` — emacs OFF for that one
//!   shape.
//! - builtins/set.def:446-449 get_edit_mode: `[[ -o emacs ]]` is
//!   `no_line_editing == 0 && rl_editing_mode == 1`, and readline's
//!   default editing mode is emacs — ON for every interactive shell,
//!   tty or not. `--noediting` (no_line_editing=1) and an explicit
//!   `-o vi` keep it off.
//!
//! Root cause: the emacs option had no default-ON site at all, and the
//! history/histexpand options were set only AFTER the init file
//! (prepare_interactive_history), so an rcfile echoing `$-` saw no H.
//! Fix (main.rs): apply_init_interactive_defaults runs at the top of the
//! three interactive init paths (before run_init_file), setting
//! history/histexpand and emacs (guarded by `--noediting` and `-o vi`);
//! the `-i script` path deliberately keeps the interactive-script shape.

use std::process::Command;

fn rubash_args(args: &[&str], stdin_null: bool) -> (String, Option<i32>) {
    use std::io::Write;
    use std::process::Stdio;
    let dir = std::env::temp_dir().join(format!(
        "rubash-i312-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(
        dir.join("rc.sh"),
        "echo \"DASH=[$-]\"\n[[ -o emacs ]] && echo EMACS-ON || echo EMACS-OFF\n",
    )
    .expect("write rc");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .args(args)
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin_null {
        command.stdin(Stdio::null());
    } else {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().expect("spawn rubash");
    if !stdin_null {
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(b"echo \"DASH=[$-]\"\n[[ -o emacs ]] && echo E-ON || echo E-OFF\nexit\n")
            .expect("write stdin");
    }
    let output = child.wait_with_output().expect("wait");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

/// Assert the two output lines with the `m` letter factored out.
/// The matrix in the header was probed with the shell's stderr on a
/// terminal, where GNU establishment grants job control and `$-`
/// carries `m`; without a claimable terminal GNU drops `m`
/// (rubash#2948: jobs.c:4869 makes the letter the establishment
/// result). This file's subject is the editing-mode defaults, which
/// do not depend on the terminal, so the DASH line is compared with
/// `m` removed and the rest byte-exact; the exact per-harness `m`
/// matrix is pinned by tests/issue2948_monitor_flag_regressions.rs.
fn assert_dash_and_edit_mode(out: &str, dash_without_m: &str, edit_line: &str) {
    let mut lines = out.lines();
    let dash = lines.next().unwrap_or("");
    let dash = dash.replacen("DASH=[him", "DASH=[hi", 1);
    assert_eq!(dash, format!("DASH=[{dash_without_m}]"), "out: {out}");
    assert_eq!(lines.next(), Some(edit_line), "out: {out}");
    assert_eq!(lines.next(), None, "out: {out}");
}

/// The verified GNU matrix.
#[test]
fn interactive_no_tty_defaults_match_gnu_matrix() {
    let (out, _) = rubash_args(
        &[
            "-i",
            "-c",
            "echo \"DASH=[$-]\"; [[ -o emacs ]] && echo EMACS-ON || echo EMACS-OFF",
        ],
        true,
    );
    assert_dash_and_edit_mode(&out, "hiBHc", "EMACS-ON");

    let (out, _) = rubash_args(&["--rcfile", "rc.sh", "-i"], true);
    assert_dash_and_edit_mode(&out, "hiBH", "EMACS-ON");

    let (out, _) = rubash_args(&["-i", "rc.sh"], true);
    assert_dash_and_edit_mode(&out, "hiBH", "EMACS-OFF");

    let (out, _) = rubash_args(
        &[
            "-c",
            "echo \"DASH=[$-]\"; [[ -o emacs ]] && echo EMACS-ON || echo EMACS-OFF",
        ],
        true,
    );
    assert_dash_and_edit_mode(&out, "hBc", "EMACS-OFF");
}

/// The `-i -s` pipe shape.
#[test]
fn interactive_stdin_pipe_defaults() {
    let (out, _) = rubash_args(&["-i", "-s"], false);
    assert!(
        out.contains("DASH=[himBHs]") || out.contains("DASH=[hiBHs]"),
        "out: {out}"
    );
    assert!(out.contains("E-ON"), "out: {out}");
}

/// The ble.sh noediting gate now passes for PTY-less interactive loads.
#[test]
fn interactive_noediting_gate_passes() {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i312g-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(
        dir.join("gate.sh"),
        "if [[ ! -o emacs && ! -o vi ]]; then echo NOEDITING-ABORT; exit 1; fi\necho GATE-PASSED\n",
    )
    .expect("write gate");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["--rcfile", "gate.sh", "-i"])
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "GATE-PASSED\n");
    assert_eq!(output.status.code(), Some(0));
}

/// `--noediting` and an explicit `-o vi` keep emacs off, like GNU.
#[test]
fn interactive_editing_guards() {
    let (out, _) = rubash_args(
        &[
            "--noediting",
            "-i",
            "-c",
            "[[ -o emacs ]] && echo NE-ON || echo NE-OFF; [[ -o vi ]] && echo VI-ON || echo VI-OFF",
        ],
        true,
    );
    assert_eq!(out, "NE-OFF\nVI-OFF\n");

    let (out, _) = rubash_args(
        &[
            "-i",
            "-o",
            "vi",
            "-c",
            "[[ -o emacs ]] && echo E-ON || echo E-OFF; [[ -o vi ]] && echo VI-ON || echo VI-OFF",
        ],
        true,
    );
    assert_eq!(out, "E-OFF\nVI-ON\n");
}

fn interactive_alias_output(args: &[&str], input: &str, rc: Option<&str>) -> std::process::Output {
    use std::io::Write;
    use std::process::Stdio;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    let dir = std::env::temp_dir().join(format!(
        "rubash-interactive-alias-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("probe.sh"), input).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.arg("--noprofile");
    if let Some(contents) = rc {
        std::fs::write(dir.join("alias-rc.sh"), contents).unwrap();
        command.args(["--rcfile", "alias-rc.sh"]);
    } else {
        command.arg("--norc");
    }
    command
        .args(args)
        .current_dir(&dir)
        .env("HOME", &dir)
        .env("USERPROFILE", &dir)
        .env("HISTFILE", dir.join("history"))
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("NIU_INVOKED_AS")
        .env_remove("BASH_ARGV0")
        .env_remove("__RUBASH_SHOPT_STATE")
        .env_remove("__RUBASH_ALIAS_STREAMED")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!("interactive alias timeout: {args:?}: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
    output
}

#[test]
fn interactive_alias_defaults_match_gnu_reader_command_and_script() {
    let source = "shopt -p expand_aliases\nalias ll='printf ALIAS_OK'\nll\nprintf '\\n'\n";
    for args in [
        vec!["-i"],
        vec!["-i", "-s"],
        vec!["-i", "-c", source],
        vec!["-ic", source],
        vec!["-i", "probe.sh"],
    ] {
        let output = interactive_alias_output(&args, source, None);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "shopt -s expand_aliases\nALIAS_OK\n",
            "{args:?}: {output:?}"
        );
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("command not found"),
            "{args:?}: {output:?}"
        );
    }
}

#[test]
fn interactive_alias_cli_overrides_survive_option_order() {
    let probe = "shopt -p expand_aliases; :";
    for (options, enabled) in [
        (vec!["-i", "+O", "expand_aliases"], false),
        (vec!["+O", "expand_aliases", "-i"], false),
        (vec!["-i", "-O", "expand_aliases"], true),
        (vec!["-O", "expand_aliases", "-i"], true),
        (
            vec!["-i", "-O", "expand_aliases", "+O", "expand_aliases"],
            false,
        ),
        (
            vec!["-i", "+O", "expand_aliases", "-O", "expand_aliases"],
            true,
        ),
    ] {
        for command_string in [false, true] {
            let mut args = options.clone();
            if command_string {
                args.extend(["-c", probe]);
            }
            let output = interactive_alias_output(&args, probe, None);
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                if enabled {
                    "shopt -s expand_aliases\n"
                } else {
                    "shopt -u expand_aliases\n"
                },
                "{args:?}: {output:?}"
            );
        }
    }
}

#[test]
fn interactive_alias_rc_can_disable_cli_defaults() {
    let rc = "shopt -p expand_aliases\nshopt -u expand_aliases\nalias ll='printf UNEXPECTED'\n";
    let probe = "shopt -p expand_aliases\nll\nprintf 'status=%s\\n' \"$?\"\n";
    for (options, initial) in [
        (vec!["-i"], "shopt -s expand_aliases\n"),
        (
            vec!["-i", "-O", "expand_aliases"],
            "shopt -s expand_aliases\n",
        ),
        (
            vec!["-i", "+O", "expand_aliases"],
            "shopt -u expand_aliases\n",
        ),
    ] {
        for command_string in [false, true] {
            let mut args = options.clone();
            if command_string {
                args.extend(["-c", probe]);
            }
            let output = interactive_alias_output(&args, probe, Some(rc));
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                format!("{initial}shopt -u expand_aliases\nstatus=127\n"),
                "{args:?}: {output:?}"
            );
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("ll: command not found"),
                "{args:?}: {output:?}"
            );
        }
    }
}

#[test]
fn noninteractive_alias_default_stays_disabled() {
    let output = interactive_alias_output(&["-c", "shopt -p expand_aliases; :"], "", None);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "shopt -u expand_aliases\n"
    );
}

#[test]
fn interactive_alias_shared_invocation_defaults_and_overrides() {
    use rubash::executor::Executor;
    use rubash::invocation::ShellInvocation;

    for (options, enabled) in [
        (vec!["-i", "-c", ":"], true),
        (vec!["-ic", ":"], true),
        (vec!["-i", "+O", "expand_aliases", "-c", ":"], false),
        (vec!["+O", "expand_aliases", "-i", "-c", ":"], false),
        (vec!["-O", "expand_aliases", "-i", "-c", ":"], true),
        (
            vec![
                "-i",
                "+O",
                "expand_aliases",
                "-O",
                "expand_aliases",
                "-c",
                ":",
            ],
            true,
        ),
        (
            vec![
                "-i",
                "-O",
                "expand_aliases",
                "+O",
                "expand_aliases",
                "-c",
                ":",
            ],
            false,
        ),
        (vec!["-c", ":"], false),
    ] {
        let args: Vec<String> = options.iter().map(|arg| (*arg).to_string()).collect();
        let invocation = ShellInvocation::parse(&args).unwrap();
        let mut executor = Executor::new();
        executor.set_shopt_option("expand_aliases", false);
        invocation
            .apply_to_executor_before_startup(&mut executor)
            .unwrap();
        assert_eq!(executor.alias_expansion_enabled(), enabled, "{options:?}");
        executor.set_shopt_option("expand_aliases", false);
        invocation.finish_startup(&mut executor);
        assert!(
            !executor.alias_expansion_enabled(),
            "{options:?}: rc override"
        );
    }
}
