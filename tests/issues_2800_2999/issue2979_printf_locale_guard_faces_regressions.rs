//! rubash#2979: printf's remaining locale faces must observe the
//! shell's variable-table locale (the rubash#1488 ShellLocaleGuard,
//! installed at printf's dispatch funnel by rubash#2965), not only the
//! process environment.
//!
//! The faces: the wide-conversion byte/char decision behind `%S`
//! (value.rs `byte_mode`), the `\u`/`\U` escape encoding gate
//! (escape.rs, format string and `%b`), and `decimal_point()`
//! (number.rs/float.rs). GNU resolves all of them in the shell's
//! CURRENT locale, which a variable assignment changes at once
//! (locale.c:185 set_locale_var, the sv_locale watcher body;
//! locale.c:345 get_locale_var for the per-category chain):
//!   * wide conversions take the character path iff
//!     locale_mb_cur_max > 1 (builtins/printf.def:502,532);
//!   * `\u` goes through u32cconv (printf.def:1121-1143): encoded in
//!     the active locale when multibyte, re-emitted as the canonical
//!     literal `\u%04X` in a single-byte locale;
//!   * the float decimal point is localeconv()->decimal_point
//!     (lib/sh/snprintf.c:442-443) under the LC_NUMERIC chain
//!     (LC_ALL > LC_NUMERIC > LANG).
//!
//! Why the pins drive the flip through `printf -v`: plain assignments
//! to the LC_* family are mirrored into the process env by the
//! executor's assignment funnel (the rubash#1531 mirror), so the
//! env-reading faces happen to agree on that path (pinned here as
//! controls). A `printf -v` store writes the variable through the
//! builtin's own store, bypassing that funnel — the table and the
//! process env diverge, and pre-fix the faces split: after
//! `printf -v LC_ALL C` under a UTF-8 environment the guard-reading
//! char-constant face (%d "'é" -> 195) already follows the table
//! while the env-reading \u face still encodes UTF-8. GNU has one
//! locale; both faces must follow the table.
//!
//! Oracle: GNU bash 5.3.20, byte-compared for the %S and \u faces
//! (including the printf -v shapes above). The decimal-point cells
//! assert rubash's established name-based model (de_DE -> ','):
//! GNU produces ',' only when the named locale is installed in libc,
//! which this host lacks (only C.utf8), so the cells pin that a
//! store-side assignment has exactly the effect a process-env or
//! plain assignment already has today.

#![cfg(unix)]

use std::process::{Command, Stdio};

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

/// Run `script` (as a file) under rubash with a cleared environment
/// plus `envs`, returning stdout as raw bytes.
fn run_with_env(script: &str, envs: &[(&str, &str)]) -> Vec<u8> {
    let dir = std::env::temp_dir().join(format!(
        "rubash-scratch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let script_path = dir.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let mut command = Command::new(rubash_bin());
    command
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", &dir)
        .current_dir(&dir)
        .stdin(Stdio::null());
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("spawn rubash");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        output.status.code(),
        Some(0),
        "run must succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

const C: &str = "C";
const UTF8: &str = "C.UTF-8";

// ---- the split itself: guard face vs env face after one -v store ----

#[test]
fn issue2979_faces_agree_after_printf_v_locale_store() {
    // Under a UTF-8 process env, `printf -v LC_ALL C` puts C in the
    // table only. The char-constant face already reads the table
    // (195 = first byte of é); the \u face must agree (canonical
    // literal), not keep encoding from the process env.
    assert_eq!(
        run_with_env(
            "printf -v LC_ALL C; printf '%d ' \"'é\"; printf '[\\u00e9]\\n'\n",
            &[("LC_ALL", UTF8)]
        ),
        "195 [\\u00E9]\n".as_bytes().to_vec()
    );
}

// ---- byte_mode: the %S width/precision byte-vs-char decision ----

#[test]
fn issue2979_wide_precision_follows_printf_v_flip_up() {
    assert_eq!(
        run_with_env(
            "printf -v LC_ALL C.UTF-8; printf '[%.2S]\\n' 'ééé'\n",
            &[("LC_ALL", C)]
        ),
        "[éé]\n".as_bytes().to_vec()
    );
}

#[test]
fn issue2979_wide_width_follows_printf_v_flip_up() {
    // %4S of a two-character string pads to four CHARACTERS under the
    // flipped locale; byte counting sees four bytes and pads nothing.
    assert_eq!(
        run_with_env(
            "printf -v LC_ALL C.UTF-8; printf '[%4S]\\n' 'éé'\n",
            &[("LC_ALL", C)]
        ),
        "[  éé]\n".as_bytes().to_vec()
    );
}

#[test]
fn issue2979_wide_plain_assignment_and_env_controls() {
    // Plain assignments ride the process-env mirror and the env face
    // itself was always right: both agree before and after the fix.
    assert_eq!(
        run_with_env("LC_ALL=C.UTF-8; printf '[%.2S]\\n' 'ééé'\n", &[]),
        "[éé]\n".as_bytes().to_vec()
    );
    assert_eq!(
        run_with_env("printf '[%.2S]\\n' 'ééé'\n", &[("LC_ALL", UTF8)]),
        "[éé]\n".as_bytes().to_vec()
    );
    // ASCII precision is byte/char-identical in every locale.
    assert_eq!(
        run_with_env("printf '[%.2s]\\n' 'abcd'\n", &[("LC_ALL", C)]),
        "[ab]\n".as_bytes().to_vec()
    );
}

// ---- the \u / \U escape gate (format string and %b) ----

#[test]
fn issue2979_format_unicode_escape_printf_v_flip_up_encodes() {
    assert_eq!(
        run_with_env(
            "printf -v LC_ALL C.UTF-8; printf '[\\u00e9]\\n'\n",
            &[("LC_ALL", C)]
        ),
        "[é]\n".as_bytes().to_vec()
    );
}

#[test]
fn issue2979_percent_b_unicode_escape_printf_v_flip_down_stays_literal() {
    assert_eq!(
        run_with_env(
            "printf -v LC_ALL C; printf '[%b]\\n' '\\u00e9'\n",
            &[("LC_ALL", UTF8)]
        ),
        "[\\u00E9]\n".as_bytes().to_vec()
    );
}

#[test]
fn issue2979_unicode_escape_plain_assignment_controls() {
    assert_eq!(
        run_with_env("LC_ALL=C.UTF-8; printf '[\\u00e9]\\n'\n", &[("LC_ALL", C)]),
        "[é]\n".as_bytes().to_vec()
    );
    assert_eq!(
        run_with_env("LC_ALL=C; printf '[\\u00e9]\\n'\n", &[("LC_ALL", UTF8)]),
        "[\\u00E9]\n".as_bytes().to_vec()
    );
}

// ---- decimal_point(): the LC_NUMERIC chain from the variable table ----

#[test]
fn issue2979_decimal_point_follows_printf_v_lc_all() {
    assert_eq!(
        run_with_env(
            "printf -v LC_ALL de_DE.UTF-8; printf '[%.2f]\\n' 3\n",
            &[("LC_ALL", C)]
        ),
        "[3,00]\n".as_bytes().to_vec()
    );
}

#[test]
fn issue2979_decimal_point_follows_printf_v_lc_numeric() {
    // No LC_ALL anywhere: the LC_NUMERIC slot of the chain governs.
    assert_eq!(
        run_with_env(
            "printf -v LC_NUMERIC de_DE.UTF-8; printf '[%.2f]\\n' 3\n",
            &[]
        ),
        "[3,00]\n".as_bytes().to_vec()
    );
}

#[test]
fn issue2979_decimal_point_operand_parse_follows_printf_v_lc_all() {
    // The operand-parse face shares decimal_point(): '1,5' parses as
    // one-and-a-half once the store flips the locale.
    assert_eq!(
        run_with_env(
            "printf -v LC_ALL de_DE.UTF-8; printf '[%.1f]\\n' '1,5'\n",
            &[("LC_ALL", C)]
        ),
        "[1,5]\n".as_bytes().to_vec()
    );
}

#[test]
fn issue2979_decimal_point_assignment_controls() {
    // Plain-assignment flips (mirror path) and the raw process-env
    // face agree before and after.
    assert_eq!(
        run_with_env(
            "LC_ALL=de_DE.UTF-8; printf '[%.2f]\\n' 3\n",
            &[("LC_ALL", C)]
        ),
        "[3,00]\n".as_bytes().to_vec()
    );
    assert_eq!(
        run_with_env("LC_NUMERIC=de_DE.UTF-8; printf '[%.2f]\\n' 3\n", &[]),
        "[3,00]\n".as_bytes().to_vec()
    );
    assert_eq!(
        run_with_env("LANG=de_DE.UTF-8; printf '[%.2f]\\n' 3\n", &[]),
        "[3,00]\n".as_bytes().to_vec()
    );
    assert_eq!(
        run_with_env("printf '[%.2f]\\n' 3\n", &[("LC_ALL", "de_DE.UTF-8")]),
        "[3,00]\n".as_bytes().to_vec()
    );
    // A plain flip back down restores the dot.
    assert_eq!(
        run_with_env(
            "LC_ALL=C; printf '[%.2f]\\n' 3\n",
            &[("LC_ALL", "de_DE.UTF-8")]
        ),
        "[3.00]\n".as_bytes().to_vec()
    );
}
