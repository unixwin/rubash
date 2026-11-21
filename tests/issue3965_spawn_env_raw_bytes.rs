//! rubash#3965 — External command spawn must preserve raw bytes (such as 0xFF)
//! in environment variable values without widening to UTF-8 `U+00FF` (`0xC3 0xBF`)
//! or leaking `U+E000` raw-byte marker pairs.
//!
//! GNU C source:
//! - `variables.c:4850 make_env_array_from_var_list` -> `mk_env_string`: copies
//!   the variable's value cell bytes verbatim into `envp` with no re-encoding.
//!
//! Platform boundary:
//! - On Unix (`#[cfg(unix)]`), `OsString::from_vec` / `CString::new` passes raw
//!   bytes (`0x80..=0xFF`) verbatim through `execve`.
//! - On Windows (`#[cfg(not(unix))]`), `CreateProcessW` requires UTF-16
//!   `OsString`, so raw bytes map to `U+0080..=U+00FF` (`decode_raw_byte_markers_to_byte_chars`).

#[cfg(unix)]
use std::process::Command;

#[cfg(unix)]
fn rubash_bytes(script: &str) -> (Vec<u8>, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C")
        .output()
        .expect("run rubash");
    (
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[cfg(unix)]
#[test]
fn issue3965_spawn_preserves_0xff_in_exported_and_prefix_env() {
    let (stdout, stderr, code) = rubash_bytes(
        "export x=$'\\xff'\n\
         /usr/bin/printenv x\n\
         y=$'\\xff' /usr/bin/printenv y\n\
         export z=$(printf '\\377')\n\
         /usr/bin/printenv z\n\
         w=$(printf '\\377') /usr/bin/printenv w",
    );
    assert_eq!(stdout, vec![0xff, b'\n', 0xff, b'\n', 0xff, b'\n', 0xff, b'\n']);
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn issue3965_spawn_preserves_0xff_in_env_builtin_and_pipeline() {
    let (stdout, stderr, code) = rubash_bytes(
        "export x=$'\\xff'\n\
         /usr/bin/env | /bin/grep '^x='\n\
         y=$'\\xff' /usr/bin/printenv y | /bin/cat\n\
         export a=$'\\xff'\n\
         /bin/bash -c 'printf \"%s\" \"$a\"'",
    );
    assert_eq!(
        stdout,
        vec![b'x', b'=', 0xff, b'\n', 0xff, b'\n', 0xff]
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
