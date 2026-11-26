//! Issue rubash#540: `$'\xHH'` and the printf builtin's `\xHH` (format AND
//! `%b` operand paths) are BYTE escapes — GNU printf.def:1108-1124 tescape
//! does `*cp = evalue & 0xFF` and strtrans.c ansic_quote escapes the raw
//! byte stream — so no UTF-8 codepoint re-encoding may appear: `printf
//! 'a\xe9b'` emits 61 e9 62 and `declare -p x` for `x=$'\x81'` renders
//! `$'\201'` (one octal escape for byte 0x81), not `$'\302\201'`.

#![cfg(windows)]

use std::process::{Command, Stdio};

fn run_script(script: &str) -> String {
    let dir = std::env::temp_dir().join(format!("rubash-issue540-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue540 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn od_line(script: &str) -> String {
    let stdout = run_script(script);
    stdout
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn printf_hex_escapes_emit_raw_bytes() {
    // WSL GNU bash 5.3.0: `printf 'a\xe9b'` -> 61 e9 62.
    assert_eq!(od_line("printf 'a\\xe9b' > f\nod -An -tx1 f\n"), "61 e9 62");
    // %b operand path shares tescape.
    assert_eq!(
        od_line("printf '%b' 'a\\xe9b' > f\nod -An -tx1 f\n"),
        "61 e9 62"
    );
    // The octal byte path stays byte-transparent.
    assert_eq!(od_line("printf '\\351' > f\nod -An -tx1 f\n"), "e9");
    // $'\xHH' values were already byte-transparent.
    assert_eq!(
        od_line("y=$'\\xe9'\nprintf '%s' \"$y\" | od -An -tx1\n"),
        "e9"
    );
}

#[test]
fn declare_p_quotes_raw_bytes_per_byte() {
    // WSL GNU bash 5.3.0: `x=$'\x81'; declare -p x` -> `declare -- x=$'\201'`
    // — one three-digit octal escape for the single byte 0x81.
    let stdout = run_script("x=$'\\x81'\ndeclare -p x\n");
    assert_eq!(stdout, "declare -- x=$'\\201'\n");
}
