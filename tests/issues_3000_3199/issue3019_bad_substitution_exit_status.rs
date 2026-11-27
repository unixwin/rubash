//! rubash#3019 — after a `bad substitution` error GNU exits 1;
//! rubash exits 0 (stderr already byte-identical). 17 mvdan/sh
//! snippet shapes (eco-2906 REPORT §6 M1), plus the t0548 second
//! face: with two bad substitutions in one command GNU reports only
//! the first.
//!
//! GNU: the bad_substitution label in parameter_brace_expand
//! (subst.c:10273-10288) does set_exit_status (EXECUTION_FAILURE)
//! and returns the &expand_wdesc_error sentinel;
//! call_expand_word_internal (subst.c:4288-4296) maps the error
//! sentinel to exp_jump_to_top_level (DISCARD), which unwinds the
//! whole non-interactive shell with last_command_exit_value =
//! EXECUTION_FAILURE — rc 1, and no later word of the command is
//! ever expanded (hence one report, not two).
//!
//! rubash at base: the diagnostic is printed but the command word
//! expands to nothing and the resulting empty command exits 0 —
//! indistinguishable from the legitimate empty command of the
//! `x=""; $x` control (rc 0 in GNU too). The fix must carry the
//! expansion-error fact to the exit status, not special-case
//! emptiness.
//!
//! Expectations measured byte-for-byte against GNU bash 5.3.20 in
//! script-file mode (2026-11-09).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash3019-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), format!("{script}\n")).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn check(tag: &str, run: &Run, stdout: &str, stderr: &str, code: i32) {
    assert_eq!(run.stdout, stdout, "{tag}: stdout");
    assert_eq!(run.stderr, stderr, "{tag}: stderr");
    assert_eq!(run.code, Some(code), "{tag}: rc");
}

macro_rules! bad_subst {
    ($fn_name:ident, $tag:literal, $script:literal, $stderr:literal) => {
        #[test]
        fn $fn_name() {
            let run = run_file($tag, $script);
            check($tag, &run, "", $stderr, 1);
        }
    };
}

// t0224
bad_subst!(
    issue3019_t0224_empty_braces,
    "t0224",
    "${}",
    "case.sh: line 1: ${}: bad substitution\n"
);
// t0257
bad_subst!(
    issue3019_t0257_at_negative_subscript,
    "t0257",
    "${@[-1]}",
    "case.sh: line 1: ${@[-1]}: bad substitution\n"
);
// t0258
bad_subst!(
    issue3019_t0258_star_negative_subscript,
    "t0258",
    "${*[-1]}",
    "case.sh: line 1: ${*[-1]}: bad substitution\n"
);
// t0297
bad_subst!(
    issue3019_t0297_percent_head,
    "t0297",
    "${%foo}",
    "case.sh: line 1: ${%foo}: bad substitution\n"
);
// t0532
bad_subst!(
    issue3019_t0532_plus_head,
    "t0532",
    "${+foo}",
    "case.sh: line 1: ${+foo}: bad substitution\n"
);
// t0534
bad_subst!(
    issue3019_t0534_nested_braced_head,
    "t0534",
    "${${foo#head}%tail}",
    "case.sh: line 1: ${${foo#head}%tail}: bad substitution\n"
);
// t0540
bad_subst!(
    issue3019_t0540_equals_head,
    "t0540",
    "${=foo}",
    "case.sh: line 1: ${=foo}: bad substitution\n"
);
// t0541
bad_subst!(
    issue3019_t0541_tilde_head,
    "t0541",
    "${~foo}",
    "case.sh: line 1: ${~foo}: bad substitution\n"
);
// t0542
bad_subst!(
    issue3019_t0542_caret_head,
    "t0542",
    "${^foo}",
    "case.sh: line 1: ${^foo}: bad substitution\n"
);
// t0543
bad_subst!(
    issue3019_t0543_double_equals,
    "t0543",
    "${==foo}",
    "case.sh: line 1: ${==foo}: bad substitution\n"
);
// t0544
bad_subst!(
    issue3019_t0544_double_tilde,
    "t0544",
    "${~~foo}",
    "case.sh: line 1: ${~~foo}: bad substitution\n"
);
// t0545
bad_subst!(
    issue3019_t0545_double_caret,
    "t0545",
    "${^^foo}",
    "case.sh: line 1: ${^^foo}: bad substitution\n"
);
// t0547
bad_subst!(
    issue3019_t0547_zsh_flags_equals,
    "t0547",
    "${(o)=items}",
    "case.sh: line 1: ${(o)=items}: bad substitution\n"
);
// t0549
bad_subst!(
    issue3019_t0549_nameless_default,
    "t0549",
    "${:-word}",
    "case.sh: line 1: ${:-word}: bad substitution\n"
);
// t0550
bad_subst!(
    issue3019_t0550_nameless_alternate,
    "t0550",
    "${:+word}",
    "case.sh: line 1: ${:+word}: bad substitution\n"
);
// t0551
bad_subst!(
    issue3019_t0551_zsh_flags_default,
    "t0551",
    "${(%):-%N}",
    "case.sh: line 1: ${(%):-%N}: bad substitution\n"
);
// t0548 — two bad substitutions in one command: GNU reports only
// the first (DISCARD unwinds before the second word expands).
bad_subst!(
    issue3019_t0548_only_first_reported,
    "t0548",
    "${(aO)foo} ${(s/x/)foo}",
    "case.sh: line 1: ${(aO)foo}: bad substitution\n"
);

#[test]
fn issue3019_control_empty_quoted_command_not_found() {
    // Control from the issue: the empty command itself is not the
    // divergence — both shells give 127 here.
    let run = run_file("ctl1", "\"\"");
    check(
        "ctl1",
        &run,
        "",
        "case.sh: line 1: : command not found\n",
        127,
    );
}

#[test]
fn issue3019_control_legit_empty_expansion_stays_zero() {
    // Control: a word that legitimately expands to nothing is an
    // empty command with rc 0 in GNU too — the fix must not turn
    // emptiness itself into a failure.
    let run = run_file("ctl2", "x=\"\"; $x");
    check("ctl2", &run, "", "", 0);
}
