//! rubash#1311: `printf -v 'ro2[k2]'` against a readonly assoc
//! array must report the BARE array name — GNU prints
//! `ro2: readonly variable`; rubash leaked a raw 0x1E byte and
//! named `ro2[k2]`.
//!
//! GNU anchor: `builtins/printf.def`: the `-v` assignment path
//! reports a readonly target by its variable name (the subscript
//! is part of the value designator, not the name). The plain
//! assignment and scalar forms already named the bare variable in
//! both shells.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20
//! (stderr captured raw and checked at the byte level).

use std::process::Command;

fn rubash_bytes(script: &str) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash");
    (output.stdout, output.stderr, output.status.code())
}

/// The reported case: rc 1, diagnostic names `ro2` only, no 0x1E
/// byte anywhere, and the array is left unchanged (no `k2`).
#[test]
fn printf_v_readonly_assoc_names_bare_variable() {
    let (stdout, stderr, code) = rubash_bytes(
        "readonly -A ro2=([k]=v); printf -v 'ro2[k2]' '%s' x; echo \"rc=$?\"; declare -p ro2",
    );
    assert_eq!(code, Some(0));
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let stderr_text = String::from_utf8_lossy(&stderr).into_owned();
    assert!(stdout.contains("rc=1\n"), "stdout: {stdout}");
    assert!(stdout.contains("[k]=\"v\""), "stdout: {stdout}");
    assert!(!stdout.contains("k2"), "stdout: {stdout}");
    assert!(
        stderr_text.contains("ro2: readonly variable"),
        "stderr: {stderr_text}"
    );
    assert!(!stderr_text.contains("ro2["), "stderr: {stderr_text}");
    assert!(
        !stderr.contains(&0x1E),
        "stderr leaked a 0x1E byte: {stderr:?}"
    );
}

/// Control that already agreed: `printf -v` into a readonly
/// SCALAR produces the same bare-name diagnostic.
#[test]
fn printf_v_readonly_scalar_control() {
    let (stdout, stderr, _) = rubash_bytes("readonly ro=x; printf -v ro '%s' y; echo \"rc=$?\"");
    assert_eq!(String::from_utf8_lossy(&stdout), "rc=1\n");
    let stderr_text = String::from_utf8_lossy(&stderr).into_owned();
    assert!(
        stderr_text.contains("ro: readonly variable"),
        "stderr: {stderr_text}"
    );
    assert!(!stderr.contains(&0x1E), "stderr: {stderr:?}");
}
