//! rubash#2871 — `bind 'set <var> <bogus>'` silently accepted invalid
//! values for the enumerated readline variables `bell-style` and
//! `keymap`: no diagnostic, and `bell-style` even stored the bogus
//! value. GNU ground: rl_variable_bind (lib/readline/bind.c:2056)
//! calls the variable's set_func and reports
//! ``readline: NAME: could not set value to `VALUE'`` (bind.c:2073)
//! when it returns nonzero, keeping the old value. The validating
//! setters are sv_bell_style (bind.c:2227: none/off, audible/on,
//! visible, empty → audible, case-insensitive) and sv_keymap
//! (bind.c:2185: rl_get_keymap_by_name over the eight names at
//! bind.c:2404, matched case-insensitively at bind.c:2424). Variable
//! NAMES are matched case-insensitively too (find_string_var /
//! find_boolean_var use _rl_stricmp) and the diagnostic echoes the
//! name as typed. Valid bell-style values read back canonically
//! (`on` → audible, `off` → none) because `bind -v` prints the enum,
//! not the input spelling.

use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

struct Run {
    stdout: String,
    stderr: String,
}

fn run(script: &str) -> Run {
    #[cfg(unix)]
    const EMPTY_INPUTRC: &str = "/dev/null";
    #[cfg(windows)]
    const EMPTY_INPUTRC: &str = "NUL";
    let out = Command::new(RUBASH)
        .args(["-c", script])
        .env("INPUTRC", EMPTY_INPUTRC)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    // The `bind: warning: line editing not enabled` line is invocation
    // noise both shells print outside interactive mode; the readline
    // diagnostics under test are what remains.
    let stderr = String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|line| !line.contains("line editing not enabled"))
        .map(|line| format!("{line}\n"))
        .collect();
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr,
    }
}

#[test]
fn issue2871_bell_style_bogus_is_rejected_loudly() {
    let r = run("bind 'set bell-style bogus'; echo rc=$?; bind -v | grep bell-style");
    assert_eq!(r.stdout, "rc=0\nset bell-style audible\n");
    assert_eq!(
        r.stderr,
        "readline: bell-style: could not set value to `bogus'\n"
    );
}

#[test]
fn issue2871_keymap_bogus_is_rejected_loudly() {
    let r = run("bind 'set keymap bogus'; echo rc=$?; bind -v | grep '^set keymap'");
    assert_eq!(r.stdout, "rc=0\nset keymap emacs\n");
    assert_eq!(
        r.stderr,
        "readline: keymap: could not set value to `bogus'\n"
    );
}

#[test]
fn issue2871_bell_style_values_read_back_canonically() {
    // sv_bell_style stores an enum; the dump prints its name, so the
    // synonyms collapse (GNU 5.3.20 probe).
    let r = run("bind 'set bell-style on'; bind -v | grep bell-style");
    assert_eq!(r.stdout, "set bell-style audible\n");
    let r = run("bind 'set bell-style off'; bind -v | grep bell-style");
    assert_eq!(r.stdout, "set bell-style none\n");
    let r = run("bind 'set bell-style VISIBLE'; bind -v | grep bell-style");
    assert_eq!(r.stdout, "set bell-style visible\n");
    assert_eq!(r.stderr, "");
    // Empty value resets to audible (sv_bell_style's NULL/empty arm).
    let r = run("bind 'set bell-style visible'; bind 'set bell-style'; bind -v | grep bell-style");
    assert_eq!(r.stdout, "set bell-style audible\n");
}

#[test]
fn issue2871_keymap_names_match_case_insensitively() {
    let r = run("bind 'set keymap VI'; bind -v | grep '^set keymap'");
    assert_eq!(r.stdout, "set keymap vi\n");
    assert_eq!(r.stderr, "");
    let r = run("bind 'set keymap Emacs-Ctlx'; bind -v | grep '^set keymap'");
    assert_eq!(r.stdout, "set keymap emacs-ctlx\n");
    // The full GNU name set stays accepted.
    let r = run("bind 'set keymap vi-command'; bind -v | grep '^set keymap'");
    assert_eq!(r.stdout, "set keymap vi\n");
}

#[test]
fn issue2871_variable_names_match_case_insensitively() {
    // find_string_var matches with _rl_stricmp; the diagnostic echoes
    // the name as typed, storage lands under the canonical name.
    let r = run("bind 'set BELL-STYLE bogus'; bind -v | grep bell-style");
    assert_eq!(r.stdout, "set bell-style audible\n");
    assert_eq!(
        r.stderr,
        "readline: BELL-STYLE: could not set value to `bogus'\n"
    );
    let r = run("bind 'set Keymap vi-insert'; bind -v | grep '^set keymap'");
    assert_eq!(r.stdout, "set keymap vi-insert\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2871_valid_values_still_apply() {
    let r = run("bind 'set bell-style visible'; bind 'set keymap vi'; bind -v | grep -E 'bell-style|keymap'");
    assert_eq!(r.stdout, "set bell-style visible\nset keymap vi\n");
    assert_eq!(r.stderr, "");
}
