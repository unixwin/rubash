//! rubash#1107/#1108 — compat plumbing convergence.
//!
//! GNU owners:
//! - `builtins/shopt.def:654-708 set_compatibility_level()` — shopt compat
//!   flags are mutually exclusive and rebind `BASH_COMPAT`.
//! - `builtins/shopt.def:710-741 set_compatibility_opts()` — `BASH_COMPAT`
//!   assignment syncs the flags back (50/51/52/53 leave all off).
//! - `variables.c:6460-6508 sv_shcompat()` — strict `NN`/`N.N` spellings,
//!   `compatibility value out of range` + default reset otherwise, raw
//!   value kept, assignment still status 0.
//! - `builtins/set.def:974` + `arrayfunc.c:1152-1200 unbind_array_element()`
//!   — `shell_compatibility_level <= 51` unbinds the whole variable for
//!   `unset 'a[@]'`/`'a[*]'` (any compat flag, or `BASH_COMPAT<=51`).
//! - `test.c:664-681 unary_operator -v` — default adds `AV_ATSTARKEYS`
//!   (literal `@`/`*` assoc key); compat is any-element.
//!
//! Every expectation below is byte-verified against bash 5.3.20
//! (`/data/data/com.termux/files/usr/bin/bash`, script-file probes).

use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    status: i32,
}

fn run(script: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code().unwrap_or(-1),
    }
}

// --- #1107: shopt -s/-u rebinds BASH_COMPAT ---

#[test]
fn shopt_set_compat_binds_bash_compat() {
    let out = run("shopt -s compat44; echo $BASH_COMPAT; shopt compat44");
    assert_eq!(out.stdout, "44\ncompat44            \ton\n");
    assert_eq!(out.status, 0);
}

#[test]
fn shopt_compat_flags_are_mutually_exclusive() {
    let out = run(
        "shopt -s compat44; shopt -s compat43; echo $BASH_COMPAT; shopt compat44; shopt compat43",
    );
    assert_eq!(
        out.stdout,
        "43\ncompat44            \toff\ncompat43            \ton\n"
    );
    assert_eq!(out.status, 0);
}

#[test]
fn shopt_unset_last_compat_restores_default_binding() {
    let out = run("shopt -s compat44; shopt -u compat44; echo ${BASH_COMPAT-unset}");
    assert_eq!(out.stdout, "53\n");
    assert_eq!(out.status, 0);
}

#[test]
fn shopt_unset_keeps_flagless_mid_level() {
    // GNU shopt.def oldval guard: BASH_COMPAT=51 has no flag, so unsetting
    // an (already off) compat flag keeps 51 instead of resetting to 53.
    let out = run("BASH_COMPAT=51; shopt -u compat44; echo $BASH_COMPAT");
    assert_eq!(out.stdout, "51\n");
    assert_eq!(out.status, 0);
}

// --- #1107: BASH_COMPAT assignment syncs the flags back ---

#[test]
fn bash_compat_assignment_shows_in_shopt() {
    let out = run("BASH_COMPAT=44; shopt -p compat44");
    assert_eq!(out.stdout, "shopt -s compat44\n");
    assert_eq!(out.stderr, "");
}

#[test]
fn bash_compat_dotted_assignment_shows_in_shopt() {
    let out = run("BASH_COMPAT=4.4; echo $BASH_COMPAT; shopt -p compat44");
    assert_eq!(out.stdout, "4.4\nshopt -s compat44\n");
    assert_eq!(out.stderr, "");
}

#[test]
fn bash_compat_flagless_level_leaves_shopt_off() {
    let out = run("BASH_COMPAT=51; shopt -p compat44; shopt -p compat43");
    assert_eq!(out.stdout, "shopt -u compat44\nshopt -u compat43\n");
    // Named -p listing with every flag off reports failure (GNU list_shopts).
    assert_eq!(out.status, 1);
}

#[test]
fn bash_compat_invalid_reports_and_resets_level() {
    let out =
        run("BASH_COMPAT=99; echo \"val=$BASH_COMPAT\"; a=(p q); unset \"a[@]\"; declare -p a");
    assert_eq!(out.stdout, "val=99\ndeclare -a a=()\n");
    assert!(
        out.stderr
            .contains("BASH_COMPAT: 99: compatibility value out of range"),
        "stderr: {}",
        out.stderr
    );
    assert_eq!(out.status, 0);
}

#[test]
fn bash_compat_padded_spelling_is_invalid() {
    let out = run("BASH_COMPAT=' 44'; echo \"val=[$BASH_COMPAT]\"; shopt -p compat44");
    assert_eq!(out.stdout, "val=[ 44]\nshopt -u compat44\n");
    assert!(
        out.stderr
            .contains("BASH_COMPAT:  44: compatibility value out of range"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn unset_bash_compat_clears_flags() {
    let out = run("shopt -s compat44; unset BASH_COMPAT; echo \"val=${BASH_COMPAT-unset}\"; shopt -p compat44");
    assert_eq!(out.stdout, "val=unset\nshopt -u compat44\n");
    assert_eq!(out.status, 1);
}

#[test]
fn export_bash_compat_syncs_flags() {
    let out = run("export BASH_COMPAT=44; shopt -p compat44");
    assert_eq!(out.stdout, "shopt -s compat44\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, 0);
}

#[test]
fn export_bash_compat_invalid_reports() {
    let out = run("export BASH_COMPAT=97; echo \"val=$BASH_COMPAT\"; shopt -p compat44; echo done");
    assert_eq!(out.stdout, "val=97\nshopt -u compat44\ndone\n");
    assert!(
        out.stderr
            .contains("BASH_COMPAT: 97: compatibility value out of range"),
        "stderr: {}",
        out.stderr
    );
    assert_eq!(out.status, 0);
}

#[test]
fn readonly_bash_compat_syncs_flags() {
    let out = run("readonly BASH_COMPAT=43; shopt -p compat43");
    assert_eq!(out.stdout, "shopt -s compat43\n");
    assert_eq!(out.status, 0);
}

#[test]
fn local_bash_compat_restores_flags_on_return() {
    let out = run("shopt -s compat43; f() { local BASH_COMPAT=42; shopt -p compat42; }; f; shopt -p compat42; shopt -p compat43");
    assert_eq!(
        out.stdout,
        "shopt -s compat42\nshopt -u compat42\nshopt -s compat43\n"
    );
    assert_eq!(out.status, 0);
}

#[test]
fn temp_prefix_restore_reruns_hook() {
    // GNU reinstalls the saved binding through sv_shcompat: value and
    // flags both come back, and a restored invalid value re-reports.
    let out = run("BASH_COMPAT=44; BASH_COMPAT=42 true; echo \"val=$BASH_COMPAT\"; shopt -p compat42; shopt -p compat44");
    assert_eq!(out.stdout, "val=44\nshopt -u compat42\nshopt -s compat44\n");
    assert_eq!(out.stderr, "");
}

#[test]
fn temp_prefix_restore_absent_clears_flags() {
    let out =
        run("BASH_COMPAT=42 true; echo \"val=${BASH_COMPAT-unset}\"; shopt -p compat42; echo done");
    assert_eq!(out.stdout, "val=unset\nshopt -u compat42\ndone\n");
    assert_eq!(out.stderr, "");
}

#[test]
fn readonly_bash_compat_blocks_rebind_but_sets_flag() {
    // GNU bind_variable refuses the rebind (sh_readonly, no `shopt:`
    // segment) but the flag change stands and the status stays 0.
    let out = run("readonly BASH_COMPAT; shopt -s compat44; echo \"rc=$? bc=${BASH_COMPAT-unset}\"; shopt -p compat44");
    assert_eq!(out.stdout, "rc=0 bc=unset\nshopt -s compat44\n");
    assert!(
        out.stderr.contains("BASH_COMPAT: readonly variable"),
        "stderr: {}",
        out.stderr
    );
    assert_eq!(out.status, 0);
}

#[test]
fn shopt_rebind_updates_expansion_after_invalid_bind() {
    // Dual-store: the mirror rebind must reach the typed owner or `echo`
    // shadows it with the stale cell.
    let out =
        run("BASH_COMPAT=99; shopt -s compat44; echo \"val=$BASH_COMPAT\"; declare -p BASH_COMPAT");
    assert_eq!(out.stdout, "val=44\ndeclare -- BASH_COMPAT=\"44\"\n");
    assert!(
        out.stderr.contains("compatibility value out of range"),
        "stderr: {}",
        out.stderr
    );
    assert_eq!(out.status, 0);
}

// --- #1108: compat unset 'a[@]' unbinds the whole variable ---

#[test]
fn compat_shopt_unset_at_unbinds_indexed() {
    let out = run("a=(p q); shopt -s compat44; unset \"a[@]\"; declare -p a");
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr.contains("declare: a: not found"),
        "stderr: {}",
        out.stderr
    );
    assert_eq!(out.status, 1);
}

#[test]
fn compat_env_unset_at_unbinds_indexed() {
    let out = run("a=(p q); BASH_COMPAT=51; unset \"a[@]\"; declare -p a");
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr.contains("declare: a: not found"),
        "stderr: {}",
        out.stderr
    );
    assert_eq!(out.status, 1);
}

#[test]
fn compat_shopt_unset_at_unbinds_assoc() {
    let out = run("declare -A m=([a]=1); shopt -s compat44; unset \"m[@]\"; declare -p m");
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr.contains("declare: m: not found"),
        "stderr: {}",
        out.stderr
    );
    assert_eq!(out.status, 1);
}

#[test]
fn default_unset_at_flushes_indexed() {
    let out = run("a=(p q); unset \"a[@]\"; echo \"${#a[@]}\"; declare -p a");
    assert_eq!(out.stdout, "0\ndeclare -a a=()\n");
    assert_eq!(out.status, 0);
}

#[test]
fn default_unset_at_assoc_is_noop() {
    let out = run("declare -A m=([a]=1 [b]=2); unset \"m[@]\"; echo \"${#m[@]}\"");
    assert_eq!(out.stdout, "2\n");
    assert_eq!(out.status, 0);
}

#[test]
fn default_unset_star_removes_literal_at_key() {
    let out = run("declare -A m=([@]=1 [b]=2); unset \"m[@]\"; echo \"${#m[@]}\"");
    assert_eq!(out.stdout, "1\n");
    assert_eq!(out.status, 0);
}

// --- #1108: test -v 'K[@]' is any-element under compat ---

#[test]
fn compat_test_v_assoc_at_is_any_element() {
    let out =
        run("declare -A m=([a]=1); shopt -s compat44; test -v \"m[@]\" && echo true || echo false");
    assert_eq!(out.stdout, "true\n");
    assert_eq!(out.status, 0);
}

#[test]
fn compat_test_v_assoc_at_empty_is_false() {
    let out =
        run("declare -A e=(); shopt -s compat44; test -v \"e[@]\" && echo true || echo false");
    assert_eq!(out.stdout, "false\n");
    assert_eq!(out.status, 0);
}

#[test]
fn default_test_v_assoc_at_is_literal_key() {
    let out = run("declare -A m=([a]=1); test -v \"m[@]\" && echo true || echo false");
    assert_eq!(out.stdout, "false\n");
    assert_eq!(out.status, 0);
}

#[test]
fn default_test_v_assoc_literal_at_key_is_true() {
    let out = run("declare -A m=([@]=1); test -v \"m[@]\" && echo true || echo false");
    assert_eq!(out.stdout, "true\n");
    assert_eq!(out.status, 0);
}

#[test]
fn cond_v_assoc_at_follows_compat_level() {
    let out = run("declare -A m=([a]=1); [[ -v \"m[@]\" ]] && echo true || echo false");
    assert_eq!(out.stdout, "false\n");
    let out = run(
        "declare -A m=([a]=1); shopt -s compat44; [[ -v \"m[@]\" ]] && echo true || echo false",
    );
    assert_eq!(out.stdout, "true\n");
    assert_eq!(out.status, 0);
}

// --- level derivation shared by the =~ reader (rubash#346 family) ---

#[test]
fn bash_compat_31_selects_compat_regex_mode() {
    let out = run("BASH_COMPAT=31; re='a+b'; v='xa+b'; [[ $v =~ \"$re\" ]] && echo m || echo nm");
    assert_eq!(out.stdout, "nm\n");
    assert_eq!(out.status, 0);
}
