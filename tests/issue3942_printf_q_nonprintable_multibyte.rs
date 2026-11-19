//! rubash#3942 — `printf %q` of a non-printable multibyte character in
//! a UTF-8 locale emits each of its UTF-8 bytes as a 3-digit octal
//! escape, exactly like GNU strtrans.c ansic_quote (printf.def:698):
//! a valid multibyte character is copied verbatim only on the
//! iswprint branch, so C1 controls render `$'\302\200'` and U+2028
//! renders `$'\342\200\250'`. Rubash pushed both raw — the C1 char
//! inside the `$'...'` wrapper, U+2028 with no quoting at all (its
//! ansic_shouldquote gate tested only Rust's `is_control()`).
//!
//! GNU bash 5.3.20 oracle expectations, measured with script files
//! under LC_ALL=C.UTF-8. The controls pin the faces both shells agree
//! on: printable multibyte characters and U+200B/U+00AD stay raw, a
//! lone invalid byte is one octal escape, and ${x@Q} (a different
//! quoter) already escaped C1 correctly.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script_locale(script: &str, locale: &str) -> String {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir: PathBuf = std::env::temp_dir().join(format!("rubash-3942-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .env("LC_ALL", locale)
        .env("TMPDIR", &dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(output.status.success(), "script failed");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn c1_controls_octal_escape_their_utf8_bytes() {
    let script = concat!(
        "printf %q $'\\302\\200'; echo\n",
        "printf %q $'\\302\\202'; echo\n",
        "printf %q $'\\302\\205'; echo\n",
        "printf %q $'\\302\\237'; echo\n",
    );
    let stdout = run_script_locale(script, "C.UTF-8");
    assert_eq!(
        stdout,
        concat!(
            "$'\\302\\200'\n",
            "$'\\302\\202'\n",
            "$'\\302\\205'\n",
            "$'\\302\\237'\n",
        )
    );
}

#[test]
fn u2028_escapes_octal_in_dollar_quote() {
    let script = "printf %q $'\\342\\200\\250'; echo\n";
    let stdout = run_script_locale(script, "C.UTF-8");
    assert_eq!(stdout, "$'\\342\\200\\250'\n");
}

#[test]
fn mixed_literal_and_escaped_parts() {
    let script = concat!(
        "printf %q $'a\\302\\202b'; echo\n",
        "printf %q $'a\\342\\200\\250b'; echo\n",
    );
    let stdout = run_script_locale(script, "C.UTF-8");
    assert_eq!(stdout, "$'a\\302\\202b'\n$'a\\342\\200\\250b'\n");
}

#[test]
fn printable_multibyte_and_format_chars_stay_raw() {
    // Controls: e-acute, zero-width space, soft hyphen and the emoji are
    // iswprint in glibc's C.UTF-8 — both shells copy them verbatim.
    let script = concat!(
        "printf %q $'\\303\\251'; echo\n",
        "printf %q $'\\342\\200\\213'; echo\n",
        "printf %q $'\\302\\255'; echo\n",
        "printf %q $'\\360\\237\\230\\200'; echo\n",
    );
    let stdout = run_script_locale(script, "C.UTF-8");
    assert_eq!(stdout, "é\n​\n­\n😀\n");
}

#[test]
fn lone_invalid_byte_stays_one_octal_escape() {
    let script = "printf %q $'\\200'; echo\n";
    let stdout = run_script_locale(script, "C.UTF-8");
    assert_eq!(stdout, "$'\\200'\n");
}

#[test]
fn atQ_control_unchanged() {
    // ${x@Q} is a different quoter (already byte-correct on both shells).
    let script = "x=$'\\302\\202'; printf '%s\\n' \"${x@Q}\"\n";
    let stdout = run_script_locale(script, "C.UTF-8");
    assert_eq!(stdout, "$'\\302\\202'\n");
}
