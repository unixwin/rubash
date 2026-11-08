//! rubash#3017: the `set` listing (and the bare `declare`/`typeset`
//! listings that dispatch to the same renderer) printed scalars holding
//! bare C0 bytes without the `$'...'` quoting GNU applies.
//!
//! GNU mechanism: variables.c print_var_value (1126-1148) renders a
//! scalar value through strtrans.c ansic_shouldquote/ansic_quote over
//! the RAW BYTE stream whenever any byte is non-printing; only then
//! does it fall to sh_single_quote / bare. In rubash storage, part of
//! the C0 range (and raw high bytes) travels as U+E000 raw-byte marker
//! pairs rather than bare chars, so a char-level `is_control` scan of
//! the stored text cannot see them. The declare/array faces judge the
//! decoded byte stream (executor ansic ports); the `set` face kept a
//! private char-level copy and so fell through to the single-quote /
//! bare branches with the raw bytes inside.
//!
//! Expectations measured against GNU bash 5.3.20 (oracle binary),
//! script probes 2026-11-08 under LC_ALL=C.UTF-8. POSIX mode keeps the
//! GNU wart (raw bytes, no ansic branch) and is pinned as a control.

use std::process::{Command, Stdio};
use std::time::Duration;

mod common;
use common::wait_bounded;

fn run_script(body: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3017-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .current_dir(&dir)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("TERM", "xterm")
        .env("HOME", &dir)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = wait_bounded(&mut child, Duration::from_secs(30));
    assert!(!out.timed_out, "probe timed out");
    // Lossy on purpose: the unfixed renderer emits raw high bytes, and
    // the pin must fail as a clean assertion, not a UTF-8 panic. Every
    // expected line below is pure ASCII, so lossy conversion is exact
    // for the passing shapes.
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    stdout
}

fn listing_line(body: &str, listing: &str, name: &str) -> String {
    let script = format!("{body}\n{listing} | grep '^{name}='\n");
    let stdout = run_script(&script);
    stdout
        .lines()
        .find(|line| line.starts_with(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no {name}= line in {listing} output: {stdout:?}"))
        .to_string()
}

#[test]
fn set_listing_ansic_quotes_marker_carried_c0_scalars() {
    // The filed shape: \x11 and \x1f ride in storage as raw-byte
    // marker pairs; GNU renders the decoded bytes as octal escapes.
    assert_eq!(
        listing_line("v=$'a\\x11b\\x1fc'", "set", "v"),
        "v=$'a\\021b\\037c'"
    );
    // ESC has a named escape in ansic_quote (strtrans.c: ESC -> \E).
    assert_eq!(listing_line("e=$'r\\x1bs'", "set", "e"), "e=$'r\\Es'");
    // The rest of the marker-carried C0 block: FS/GS/RS/US and FF.
    assert_eq!(listing_line("x=$'a\\x1cb'", "set", "x"), "x=$'a\\034b'");
    assert_eq!(listing_line("x=$'a\\x1db'", "set", "x"), "x=$'a\\035b'");
    assert_eq!(listing_line("x=$'a\\x1eb'", "set", "x"), "x=$'a\\036b'");
    assert_eq!(listing_line("x=$'a\\x0cb'", "set", "x"), "x=$'a\\fb'");
}

#[test]
fn bare_declare_and_typeset_listings_share_the_set_renderer() {
    // declare.def: no-operand declare/typeset dispatch to the same
    // listing as `set`, so the ansic form must appear there too.
    assert_eq!(
        listing_line("v=$'a\\x11b\\x1fc'", "declare", "v"),
        "v=$'a\\021b\\037c'"
    );
    assert_eq!(
        listing_line("v=$'a\\x11b\\x1fc'", "typeset", "v"),
        "v=$'a\\021b\\037c'"
    );
}

#[test]
fn set_listing_ansic_quotes_raw_high_bytes() {
    // A lone raw 0x80 byte also rides as a marker pair; GNU octal-escapes it.
    assert_eq!(listing_line("h=$'a\\x80b'", "set", "h"), "h=$'a\\200b'");
}

#[test]
fn set_listing_ansic_branch_wins_over_single_quote_branch() {
    // C0 + data quote: the ansic branch is checked FIRST in
    // print_var_value, and ansic_quote escapes the quote as \'.
    assert_eq!(
        listing_line("s=$'a\\x11\\'b'", "set", "s"),
        "s=$'a\\021\\'b'"
    );
    // C0 + space (a shell meta): still the ansic form, not '...'.
    assert_eq!(listing_line("t=$'a b\\x11c'", "set", "t"), "t=$'a b\\021c'");
}

#[test]
fn set_listing_controls_stay_put() {
    // Raw-carried control bytes already rendered the ansic form.
    assert_eq!(listing_line("w=$'x\\ty\\nz'", "set", "w"), "w=$'x\\ty\\nz'");
    assert_eq!(listing_line("u=$'p\\x7fq'", "set", "u"), "u=$'p\\177q'");
    // Metas without non-printing bytes stay on the single-quote branch.
    assert_eq!(listing_line("m='a b$c'", "set", "m"), "m='a b$c'");
    // Plain values stay bare.
    assert_eq!(listing_line("p=plain", "set", "p"), "p=plain");
    // declare -p was already correct (guard against collateral change).
    let stdout = run_script("v=$'a\\x11b\\x1fc'\ndeclare -p v\n");
    assert!(
        stdout.contains("declare -- v=$'a\\021b\\037c'"),
        "declare -p line: {stdout:?}"
    );
}

#[test]
fn set_listing_posix_mode_keeps_raw_bytes() {
    // print_var_value gates the ansic branch on posixly_correct == 0;
    // GNU POSIX mode prints the raw bytes (no quoting branch fires for
    // a meta-free C0 value). Pin the wart so the fix does not move it.
    let stdout = run_script("v=$'a\\x11b\\x1fc'\nset -o posix\nset | grep '^v='\n");
    assert!(
        stdout.contains("v=a\u{11}b\u{1f}c"),
        "posix set line: {stdout:?}"
    );
}
