//! rubash#427 — `complete`/`compgen` reject dashed function names.
//!
//! GNU accepts any bash function NAME as the `-F` argument: function
//! definitions are words, not identifiers (parse.y), so
//! `complete -F _dirs-complete dirs` registers cleanly and `compgen -F`
//! invokes the dashed helper. 16 oh-my-bash completion assets (dirs,
//! G-R, ...) emit exactly this class and rubash answered
//! "`_dirs-complete': not a valid identifier" (rc 2) because the `-F`
//! argument ran through the variable-identifier rule
//! ([A-Za-z_][A-Za-z0-9_]*). The `-V` varname argument stays strict.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The verbatim harvest construct: dashed helper name accepted, spec
/// registered (rc 0, silent), and `complete -p` echoes it back verbatim.
#[test]
fn complete_f_accepts_dashed_name_and_roundtrips() {
    let (out, err, code) = rubash("complete -F _dirs-complete dirs && complete -p dirs");
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(err, "");
    assert_eq!(out, "complete -F _dirs-complete dirs\n");
}

/// `compgen -F` invokes the dashed function (COMPREPLY propagates).
#[test]
fn compgen_f_invokes_dashed_function() {
    let (out, err, code) =
        rubash("function _dirs-complete { COMPREPLY=(aa bb); }; compgen -F _dirs-complete ''");
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(out, "aa\nbb\n");
}

/// Dotted names are function-name-legal too (bash words allow '.').
#[test]
fn complete_f_accepts_dotted_name() {
    let (out, err, code) = rubash("complete -F _npm.run npm && echo ok");
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(out, "ok\n");
}

/// The relaxed rule is still a NAME rule: the empty string is rejected with
/// the GNU wording and EX_USAGE (rc 2).
#[test]
fn complete_f_still_rejects_empty_name() {
    let (out, err, code) = rubash("complete -F '' dirs");
    assert_eq!(code, Some(2));
    assert_eq!(out, "");
    assert!(
        err.contains("complete: `': not a valid identifier"),
        "stderr: {err}"
    );
}

/// `-V` takes a VARIABLE name and keeps the strict identifier rule.
#[test]
fn compgen_v_still_rejects_dashed_varname() {
    let (out, err, code) = rubash("compgen -V bad-name ''");
    assert_eq!(code, Some(2));
    assert_eq!(out, "");
    assert!(
        err.contains("compgen: `bad-name': not a valid identifier"),
        "stderr: {err}"
    );
}

/// Plain identifiers keep working end to end (no regression on the
/// oh-my-bash-free path).
#[test]
fn complete_f_plain_identifier_still_works() {
    let (out, err, code) = rubash("complete -F _dirs_complete dirs && complete -p dirs");
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(out, "complete -F _dirs_complete dirs\n");
}
