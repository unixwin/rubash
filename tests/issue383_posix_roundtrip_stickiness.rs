//! Issue rubash#383 regression: the posix round-trip never flipped
//! `inherit_errexit`. GNU 5.3 `set -o posix` enables it (and the rest of
//! the posix_vars walk), and a later `set +o posix` does NOT reset it —
//! the flag is sticky. rubash never flipped it at all, so after a posix
//! round-trip the comsub-errexit behavior diverged.
//!
//! GNU spec: builtins/set.def:403-418 set_posix_mode (no-op flip guard,
//! POSIXLY_CORRECT bind/unbind) -> variables.c:6257-6268 sv_strict_posix
//! -> general.c:103-128 posix_initialize over the posix_vars table
//! (general.c:85-92): enable turns on interactive_comments,
//! source_uses_path/sourcepath, expand_aliases, inherit_errexit and
//! print_shift_error/shift_verbose; the disable branch (general.c:122-128,
//! no saved bitmap) restores only expand_aliases (to the interactive
//! default), print_shift_error and source_searches_cwd — the other three
//! stay sticky. The `-` local bitmap additionally snapshots/restores the
//! posix_vars (set.def:330-352 get_current_options appends
//! general.c:138 get_posix_options; set.def:386 set_posix_options).
//!
//! Verified byte-for-byte against WSL GNU Bash 5.3.0 script-file probes
//! (target/optfix/probe383{a..h}.sh matrices in the lane artifacts).

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue383-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue383 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    // LF line endings: .gitattributes pins *.sh eol=lf and the probes run
    // identically under WSL GNU Bash for baseline comparison.
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash probe");
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn run_rubash_args(args: &[&str]) -> RunOutcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash args");
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// `set -o posix` runs the posix_vars walk: inherit_errexit,
/// expand_aliases and shift_verbose report on (general.c:108-114).
#[test]
fn posix_enable_turns_on_posix_vars() {
    let out =
        run_rubash_script("set -o posix\nshopt inherit_errexit expand_aliases shift_verbose\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(
        out.stdout,
        "inherit_errexit     \ton\nexpand_aliases      \ton\nshift_verbose       \ton\n"
    );
}

/// general.c:122-128: `set +o posix` resets expand_aliases and
/// shift_verbose but leaves inherit_errexit, interactive_comments and
/// sourcepath sticky (rubash#383 core shape).
#[test]
fn posix_roundtrip_is_sticky_for_inherit_errexit() {
    let out = run_rubash_script(concat!(
        "shopt -u interactive_comments\n",
        "shopt -u sourcepath\n",
        "set -o posix\n",
        "set +o posix\n",
        "shopt inherit_errexit expand_aliases shift_verbose interactive_comments sourcepath\n",
    ));
    // `shopt` query reports failure (1) because two of the five are off.
    assert_eq!(out.code, Some(1));
    assert_eq!(
        out.stdout,
        concat!(
            "inherit_errexit     \ton\n",
            "expand_aliases      \toff\n",
            "shift_verbose       \toff\n",
            "interactive_comments\ton\n",
            "sourcepath          \ton\n",
        )
    );
}

/// The sticky flag is armed: after the round-trip, `set -e` plus a comsub
/// whose interior fails aborts the session (GNU probe383h e_roundtrip).
#[test]
fn sticky_inherit_errexit_arms_comsub_errexit() {
    let out = run_rubash_script(concat!(
        "set -o posix\n",
        "set +o posix\n",
        "set -e\n",
        "v=$(false; echo hi)\n",
        "echo \"UNREACHABLE [$v]\"\n",
    ));
    // GNU: the assignment aborts the shell before the echo; rc 1.
    assert_eq!(out.code, Some(1));
    assert!(
        !out.stdout.contains("UNREACHABLE"),
        "stdout: {}",
        out.stdout
    );
}

/// set.def:406-409: a no-op flip returns before sv_strict_posix, so
/// `set +o posix` on an already-off shell must not arm anything.
#[test]
fn noop_disable_does_not_walk() {
    let out = run_rubash_script(concat!(
        "shopt -u inherit_errexit\n",
        "set +o posix\n",
        "shopt inherit_errexit\n",
        "set -e\n",
        "v=$(false; echo hi)\n",
        "echo \"reached [$v]\"\n",
    ));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "inherit_errexit     \toff\nreached [hi]\n");
}

/// set.def:413/415-416: set_posix_mode binds POSIXLY_CORRECT="y" (plain,
/// non-exported) on enable and unbinds it on disable.
#[test]
fn posix_bind_unbinds_posixly_correct() {
    let out = run_rubash_script(concat!(
        "set -o posix\n",
        "declare -p POSIXLY_CORRECT\n",
        "echo \"env-count=$(env | grep -c POSIXLY)\"\n",
        "set +o posix\n",
        "declare -p POSIXLY_CORRECT 2>&1\n",
        "echo \"rc=$?\"\n",
    ));
    // The unbind diagnostic carries the full script path; assert by parts.
    assert_eq!(out.code, Some(0));
    assert!(out
        .stdout
        .starts_with("declare -- POSIXLY_CORRECT=\"y\"\nenv-count=0\n"));
    assert!(out
        .stdout
        .ends_with(": declare: POSIXLY_CORRECT: not found\nrc=1\n"));
    assert!(out
        .stdout
        .contains("probe.sh: line 5: declare: POSIXLY_CORRECT: not found"));
}

/// `shopt -o -s posix` funnels through the same set_posix_mode walk
/// (shopt.def's -o mode drives the set -o option table).
#[test]
fn shopt_o_route_walks_posix_vars() {
    let out = run_rubash_script(
        "shopt -o -s posix\nshopt inherit_errexit\nshopt -o -u posix\nshopt inherit_errexit\n",
    );
    assert_eq!(out.code, Some(0));
    // Disable is sticky for inherit_errexit (general.c:122-128).
    assert_eq!(
        out.stdout,
        "inherit_errexit     \ton\ninherit_errexit     \ton\n"
    );
}

/// CLI `--posix` runs sv_strict_posix at startup (shell.c:566-572), so the
/// walk must arm inherit_errexit before the first command runs.
#[test]
fn cli_posix_flag_arms_inherit_errexit() {
    let out = run_rubash_args(&["--posix", "-c", "shopt inherit_errexit"]);
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "inherit_errexit     \ton\n");

    let out = run_rubash_args(&["+o", "posix", "-c", "shopt inherit_errexit"]);
    assert_eq!(out.code, Some(1));
    assert_eq!(out.stdout, "inherit_errexit     \toff\n");
}

/// set.def:330-352/358-386: the `-` local snapshots the posix_vars with
/// the option bitmap and restores them verbatim on scope pop, which undoes
/// a posix enable performed inside the frame.
#[test]
fn local_dash_restores_posix_vars_snapshot() {
    let out = run_rubash_script(concat!(
        "f() { local -; set -o posix; }\n",
        "f\n",
        "shopt inherit_errexit\n",
        "echo \"posix-on=$(set -o | grep -c '^posix.*on')\"\n",
        "g() { local -; }\n",
        "set -o posix\n",
        "g\n",
        "shopt inherit_errexit\n",
    ));
    assert_eq!(out.code, Some(0));
    assert_eq!(
        out.stdout,
        "inherit_errexit     \toff\nposix-on=0\ninherit_errexit     \ton\n"
    );
}

/// general.c:127: on disable expand_aliases falls to the interactive-shell
/// default, NOT to a pre-enable saved value (`shopt -s expand_aliases`
/// before the round-trip still ends off in a script).
#[test]
fn roundtrip_resets_expand_aliases_to_interactive_default() {
    let out = run_rubash_script(
        "shopt -s expand_aliases\nset -o posix\nset +o posix\nshopt expand_aliases\n",
    );
    // Query of an off option reports failure (1).
    assert_eq!(out.code, Some(1));
    assert_eq!(out.stdout, "expand_aliases      \toff\n");
}
