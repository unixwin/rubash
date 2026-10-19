//! #800A regression: the inline `tput` arm (Windows execution-routing stand-in
//! in `src/executor/external_file_builtins.rs` +
//! `src/executor/tput_emulation.rs` + the pipeline-stage and command-substitution
//! fast-path arms). The harvest runner paid ~300ms per `$(tput …)` spawn —
//! kiddae/dotfiles hermine `.bashrc` sourced in 9.7s across ~32 calls — while
//! rubash's own spawn floor sits at parity (~0.5ms/spawn, benchmarks/29); the
//! delta lives inside the tput child, so the arm answers admitted shapes
//! in-process.
//!
//! Admission is a WHITELIST whose every row was verified byte-for-byte against
//! the GNU oracle (WSL ncurses tput, Ubuntu 24.04 ncurses 6.4, od probes
//! 2026-10-19; the full 269-row differential ran clean under
//! `wsl bash tput_matrix.sh`, raw dump in the #800 PR). The rows below pin
//! that contract permanently: because admitted answers are produced
//! in-process, they are host-independent — no tput binary needs to exist for
//! the admitted rows to hold. Fall-through rows only assert rc != 0 /
//! empty stdout so they stay valid on hosts whose PATH tput (winuxcmd,
//! msys ncurses) answers with its own vendor-specific text.

use std::path::PathBuf;
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
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

struct Output {
    status: i32,
    stdout: String,
    #[allow(dead_code)]
    stderr: String,
}

fn run_script(script: &str, tag: &str) -> Output {
    let dir = std::env::temp_dir().join(format!("rubash-issue800-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let child = Command::new(shell_bin())
        .args(["-c", script])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let output = child.wait_with_output().expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Output {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// One admitted (TERM, invocation) row: stdout bytes must equal the GNU
/// oracle exactly and the status must be 0. `script` runs with `-c`; the
/// TERM export lives inside the script so every row exercises the
/// child-visible TERM resolution.
fn assert_admitted(tag: &str, script: &str, expected: &str) {
    let output = run_script(script, tag);
    assert_eq!(
        output.status, 0,
        "[{tag}] expected rc 0, got {} (stderr: {})",
        output.status, output.stderr
    );
    assert_eq!(output.stdout, expected, "[{tag}] stdout bytes differ");
}

#[test]
fn issue800_admitted_no_param_caps_match_gnu_bytes() {
    // Oracle (WSL ncurses tput, 2026-10-19):
    //   xterm/xterm-256color: sgr0 = \e(B\e[m, clear = \e[H\e[2J\e[3J
    //   screen*/tmux-256color/linux/vt100: sgr0 = \e[m\x0f
    //   tmux-256color clear carries E3; screen's does not.
    let rows: &[(&str, &str, &str)] = &[
        ("xterm", "bold", "\x1b[1m"),
        ("xterm", "sgr0", "\x1b(B\x1b[m"),
        ("xterm", "clear", "\x1b[H\x1b[2J\x1b[3J"),
        ("xterm", "civis", "\x1b[?25l"),
        ("xterm", "cnorm", "\x1b[?12l\x1b[?25h"),
        ("xterm", "el", "\x1b[K"),
        ("xterm", "ed", "\x1b[J"),
        ("xterm", "smcup", "\x1b[?1049h\x1b[22;0;0t"),
        ("xterm", "rmcup", "\x1b[?1049l\x1b[23;0;0t"),
        ("xterm-256color", "clear", "\x1b[H\x1b[2J\x1b[3J"),
        ("screen", "sgr0", "\x1b[m\x0f"),
        ("screen", "clear", "\x1b[H\x1b[J"),
        ("screen-256color", "clear", "\x1b[H\x1b[J"),
        ("tmux-256color", "clear", "\x1b[H\x1b[J\x1b[3J"),
        ("tmux-256color", "smcup", "\x1b[?1049h"),
        ("linux", "clear", "\x1b[H\x1b[J\x1b[3J"),
        ("linux", "civis", "\x1b[?25l\x1b[?1c"),
        ("linux", "cnorm", "\x1b[?25h\x1b[?0c"),
        ("vt100", "bold", "\x1b[1m"),
        ("vt100", "el", "\x1b[K"),
    ];
    for (index, (term, cap, expected)) in rows.iter().enumerate() {
        assert_admitted(
            &format!("noparam-{index}"),
            &format!("export TERM={term}; tput {cap}"),
            expected,
        );
    }
}

#[test]
fn issue800_admitted_absent_caps_are_silent_rc1() {
    // ncurses: a capability the terminal lacks prints NOTHING, exits 1
    // (verified `TERM=vt100 tput setaf 1`, `TERM=linux tput smcup`).
    for (index, script) in [
        "export TERM=vt100; tput setaf 1",
        "export TERM=vt100; tput civis",
        "export TERM=linux; tput smcup",
    ]
    .iter()
    .enumerate()
    {
        let output = run_script(script, &format!("absent-{index}"));
        assert_eq!(output.status, 1, "[absent-{index}] rc");
        assert!(output.stdout.is_empty(), "[absent-{index}] stdout");
    }
}

#[test]
fn issue800_admitted_setaf_setab_boundaries() {
    let rows: &[(&str, &str, &str)] = &[
        // 8-color entries format N verbatim into \e[3Nm / \e[4Nm.
        ("xterm", "setaf 0", "\x1b[30m"),
        ("xterm", "setaf 7", "\x1b[37m"),
        ("xterm", "setaf 8", "\x1b[38m"),
        ("xterm", "setaf 10", "\x1b[310m"),
        ("xterm", "setaf 255", "\x1b[3255m"),
        ("xterm", "setab 255", "\x1b[4255m"),
        ("screen", "setaf 9", "\x1b[39m"),
        ("linux", "setaf 9", "\x1b[39m"),
        // 256-color entries: base 0-7, bright 8-15 (NO ;5; segment),
        // palette 16+.
        ("xterm-256color", "setaf 9", "\x1b[91m"),
        ("xterm-256color", "setab 15", "\x1b[107m"),
        ("xterm-256color", "setaf 16", "\x1b[38;5;16m"),
        ("xterm-256color", "setaf 255", "\x1b[38;5;255m"),
        ("xterm-256color", "setab 255", "\x1b[48;5;255m"),
        ("screen-256color", "setaf 8", "\x1b[90m"),
        ("tmux-256color", "setaf 16", "\x1b[38;5;16m"),
    ];
    for (index, (term, invocation, expected)) in rows.iter().enumerate() {
        assert_admitted(
            &format!("color-{index}"),
            &format!("export TERM={term}; tput {invocation}"),
            expected,
        );
    }
}

#[test]
fn issue800_admitted_cup_shapes() {
    let rows: &[(&str, &str)] = &[
        ("tput cup 0 0", "\x1b[1;1H"),
        ("tput cup 5 10", "\x1b[6;11H"),
        ("tput cup 99 123", "\x1b[100;124H"),
    ];
    for (index, (invocation, expected)) in rows.iter().enumerate() {
        assert_admitted(
            &format!("cup-{index}"),
            &format!("export TERM=xterm; {invocation}"),
            expected,
        );
    }
}

#[test]
fn issue800_child_visible_term_sources() {
    // tempenv prefix override wins (apply_external_environment order)…
    assert_admitted(
        "tempenv",
        "export TERM=xterm; TERM=xterm-256color tput setaf 9",
        "\x1b[91m",
    );
    // …an unexported shell TERM never reaches a spawned child, so the arm
    // stands down and the real binary keeps answering (rc is host-owned:
    // winuxcmd 1, msys 4, missing 127 — all non-zero).
    let output = run_script("TERM=xterm; tput setaf 300", "unexported");
    assert_ne!(output.status, 0, "unexported TERM must not be admitted");
    // TERM unset/empty: GNU errors (rc 2); the arm stands down and the
    // host binary answers. setaf 300 errors on every vendor (winuxcmd
    // unknown-capability rc 1, msys no-TERM rc 2, missing 127), so the
    // non-zero rc holds host-tolerantly. (`tput bold` would NOT: the
    // winuxcmd binary ignores TERM and prints \e[1m rc 0 — the exact
    // pre-arm behavior the stand-down preserves.)
    let output = run_script("tput setaf 300", "no-term");
    assert_ne!(output.status, 0, "missing TERM must not be admitted");
}

#[test]
#[cfg(windows)]
fn issue800_strtol_base0_operands_never_answer_from_the_table() {
    // CR #884 B1: GNU parses numeric operands with strtol base-0 — `010`
    // is octal 8 (`\e[38m`), `+010` signed octal, `009`/`08` truncate at
    // the first illegal octal digit (`\e[30m`), `000000000010` is octal
    // 10. The arm must NOT read them as decimal (`\e[310m`, `\e[39m`,
    // `\e[38m`): the narrowed admission rejects every non-canonical
    // decimal spelling and the real binary answers instead. Each row
    // pins the invariant "stdout is never the decimal misread" — the one
    // shape that holds on every host (winuxcmd errors with empty stdout,
    // msys answers its own strtol-faithful bytes, missing 127).
    let rows: &[(&str, &str)] = &[
        ("tput setaf 010", "\x1b[310m"),
        ("tput setaf 017", "\x1b[317m"),
        ("tput setaf 009", "\x1b[39m"),
        ("tput setaf 08", "\x1b[38m"),
        ("tput setaf +010", "\x1b[310m"),
        ("tput setaf +7", "\x1b[37m"),
        ("tput setaf 000000000010", "\x1b[310m"),
        ("tput setab 010", "\x1b[410m"),
        ("TERM=tmux-256color tput setaf 010", "\x1b[38;5;10m"),
    ];
    for (index, (invocation, decimal_misread)) in rows.iter().enumerate() {
        let output = run_script(
            &format!("export TERM=xterm; {invocation}"),
            &format!("strtol-{index}"),
        );
        assert_ne!(
            output.stdout, *decimal_misread,
            "[{invocation}] answered the DECIMAL misread from the table \
             (strtol base-0 operands must fall through)"
        );
    }
    // cup with octal-shaped operands: GNU `cup 010 010` → \e[9;9H; the
    // decimal misread would be \e[11;11H.
    let output = run_script("export TERM=xterm; tput cup 010 010", "strtol-cup");
    assert_ne!(
        output.stdout, "\x1b[11;11H",
        "[cup 010 010] answered the DECIMAL misread from the table"
    );
    let output = run_script("export TERM=xterm; tput cup 008 010", "strtol-cup2");
    assert_ne!(
        output.stdout, "\x1b[9;11H",
        "[cup 008 010] answered the DECIMAL misread from the table"
    );
}

#[test]
fn issue800_capture_and_redirect_channels() {
    // Command substitution (the harvest asset shape) captures the bytes.
    assert_admitted(
        "comsub",
        r#"export TERM=xterm; B="$(tput bold)"; printf "%s" "$B""#,
        "\x1b[1m",
    );
    // The pipeline-stage arm (single-command comsub bodies are pipelines)
    // and plain pipelines answer identically.
    assert_admitted("pipeline", "export TERM=xterm; tput bold | cat", "\x1b[1m");
    // `> file` writes the same bytes.
    let output = run_script(
        "export TERM=xterm; tput bold > out.bin; od -An -c out.bin",
        "redirect",
    );
    assert_eq!(output.status, 0);
    assert_eq!(output.stdout, " 033   [   1   m\n");
}

#[test]
fn issue800_hash_introspection_records_the_path_binary() {
    // The admitted invocation stands in for the PATH binary and records the
    // same resolution the emulated family does — but only when a PATH tput
    // actually exists (hosts without one keep no entry; find_user_command
    // decides, like every other wrapper in the family).
    let script = "\
if command -v tput >/dev/null 2>&1; then
  export TERM=xterm
  tput bold >/dev/null
  hash -t tput
fi";
    let output = run_script(script, "hash");
    if output.status == 0 {
        assert!(
            output.stdout.contains("tput"),
            "[hash] expected the PATH binary path, got {:?}",
            output.stdout
        );
    }
    // Introspection must keep classifying tput as a file (hidden-builtin
    // contract, docs/builtins.md §3 — same face as uname/arch).
    let output = run_script("type -t tput", "type");
    if output.status == 0 {
        assert_eq!(output.stdout.trim(), "file");
    }
}

#[test]
#[cfg(windows)]
fn issue800_non_admitted_shapes_fall_through() {
    // Everything the whitelist does not admit keeps the real binary's own
    // behavior — the arm may only remove spawns, never change answers.
    // Only host-tolerant invariants are asserted: these shapes error on
    // every vendor (winuxcmd rc 1, msys ncurses rc 2/4, missing 127) with
    // empty stdout, so `rc != 0` holds everywhere. Shapes where some vendor
    // answers rc 0 (tput bold extra, cup 3, setaf with no operand) are
    // pinned at the pure-logic layer instead
    // (tput_emulation::resolve_admitted_tput unit tests).
    let rows: &[(&str, &str)] = &[
        ("export TERM=xterm; tput zzz", "unknown-cap"),
        ("export TERM=xterm; tput -T xterm zzz", "t-flag"),
        ("export TERM=xterm; tput setaf -1", "setaf-negative"),
        ("export TERM=dumb; tput zzz", "dumb"),
        ("export TERM=Xterm; tput zzz", "case-sensitive"),
    ];
    for (index, (script, tag)) in rows.iter().enumerate() {
        let output = run_script(script, &format!("fallthrough-{tag}-{index}"));
        assert_ne!(
            output.status, 0,
            "[{tag}] non-admitted shape answered with rc 0 (stdout: {:?})",
            output.stdout
        );
    }
}
