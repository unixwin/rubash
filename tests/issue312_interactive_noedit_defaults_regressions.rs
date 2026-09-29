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
    assert_eq!(out, "DASH=[himBHc]\nEMACS-ON\n");

    let (out, _) = rubash_args(&["--rcfile", "rc.sh", "-i"], true);
    assert_eq!(out, "DASH=[himBH]\nEMACS-ON\n");

    let (out, _) = rubash_args(&["-i", "rc.sh"], true);
    assert_eq!(out, "DASH=[himBH]\nEMACS-OFF\n");

    let (out, _) = rubash_args(
        &[
            "-c",
            "echo \"DASH=[$-]\"; [[ -o emacs ]] && echo EMACS-ON || echo EMACS-OFF",
        ],
        true,
    );
    assert_eq!(out, "DASH=[hBc]\nEMACS-OFF\n");
}

/// The `-i -s` pipe shape.
#[test]
fn interactive_stdin_pipe_defaults() {
    let (out, _) = rubash_args(&["-i", "-s"], false);
    assert!(out.contains("DASH=[himBHs]"), "out: {out}");
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
