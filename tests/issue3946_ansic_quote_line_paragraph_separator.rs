//! rubash#3946 — `declare -p` and `${x@Q}` of U+2028 LINE SEPARATOR /
//! U+2029 PARAGRAPH SEPARATOR in a UTF-8 locale emit each UTF-8 byte as
//! a 3-digit octal escape inside `$'...'`, exactly like GNU
//! strtrans.c ansic_quote: `declare -p` reaches it through
//! print_var_value (variables.c:1133-1135) and the array/assoc element
//! printers (array.c:912-913, assoc.c:362-372), `${x@Q}`/`${x@K}`/
//! `${x@k}` through sh_quote_reusable (subst.c:8760-8772 ->
//! shquote.c:365-366). The iswprint verdict comes from
//! ansic_wshouldquote's mbstowcs + iswprint walk (strtrans.c:312-336),
//! and glibc's print class excludes the Zl/Zp separators while keeping
//! Cf format characters printable. Rubash's ports tested only Rust's
//! `is_control()` (category Cc), so both separators printed raw —
//! `declare -- x="<e2 80 a8>"` and `'...'` instead of
//! `$'\342\200\250'`.
//!
//! GNU bash 5.3.20 oracle expectations, measured with script files
//! under LC_ALL=C.utf8. The controls pin the faces both shells agree
//! on: C1 controls (U+0085/U+0082) stay octal, and the Cf characters
//! U+200B/U+FEFF/U+00AD print raw (`"..."` for declare -p, `'...'`
//! for the @ transforms).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script_locale(script: &str, locale: &str) -> String {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir: PathBuf = std::env::temp_dir().join(format!("rubash-3946-{}-{n}", std::process::id()));
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
fn declare_p_escapes_line_and_paragraph_separators() {
    let script = concat!(
        "x=$'\\u2028'; declare -p x\n",
        "y=$'\\u2029'; declare -p y\n",
        "z=$'a\\u2028b'; declare -p z\n",
    );
    let stdout = run_script_locale(script, "C.UTF-8");
    assert_eq!(
        stdout,
        concat!(
            "declare -- x=$'\\342\\200\\250'\n",
            "declare -- y=$'\\342\\200\\251'\n",
            "declare -- z=$'a\\342\\200\\250b'\n",
        )
    );
}

#[test]
fn at_transforms_escape_line_and_paragraph_separators() {
    // ${x@Q}, ${x@K} and ${x@k} all render through GNU
    // sh_quote_reusable (subst.c:8760-8772).
    let script = concat!(
        "x=$'\\u2028'\n",
        "printf '%s\\n' \"${x@Q}\" \"${x@K}\" \"${x@k}\"\n",
        "y=$'\\u2029'\n",
        "printf '%s\\n' \"${y@Q}\"\n",
        "z=$'a\\u2029b'\n",
        "printf '%s\\n' \"${z@Q}\"\n",
    );
    let stdout = run_script_locale(script, "C.UTF-8");
    assert_eq!(
        stdout,
        concat!(
            "$'\\342\\200\\250'\n",
            "$'\\342\\200\\250'\n",
            "$'\\342\\200\\250'\n",
            "$'\\342\\200\\251'\n",
            "$'a\\342\\200\\251b'\n",
        )
    );
}

#[test]
fn c1_controls_stay_octal_escaped() {
    // Controls: U+0085 NEXT LINE and U+0082 BREAK PERMITTED are Cc —
    // both shells already rendered them $'...' octal.
    let script = concat!(
        "w=$'\\u0085'; declare -p w\n",
        "printf '%s\\n' \"${w@Q}\"\n",
        "v=$'\\u0082'; declare -p v\n",
        "printf '%s\\n' \"${v@Q}\"\n",
    );
    let stdout = run_script_locale(script, "C.UTF-8");
    assert_eq!(
        stdout,
        concat!(
            "declare -- w=$'\\302\\205'\n",
            "$'\\302\\205'\n",
            "declare -- v=$'\\302\\202'\n",
            "$'\\302\\202'\n",
        )
    );
}

#[test]
fn format_characters_stay_raw() {
    // Controls: U+200B ZERO WIDTH SPACE, U+FEFF and U+00AD SOFT HYPHEN
    // are Cf — glibc iswprint passes them, so GNU prints the raw
    // character: sh_double_quote "..." for declare -p, '...' for the
    // @ transforms.
    let script = concat!(
        "u=$'\\u200b'; declare -p u\n",
        "printf '%s\\n' \"${u@Q}\"\n",
        "t=$'\\ufeff'; declare -p t\n",
        "printf '%s\\n' \"${t@Q}\"\n",
        "s=$'\\u00ad'; declare -p s\n",
        "printf '%s\\n' \"${s@Q}\"\n",
    );
    let stdout = run_script_locale(script, "C.UTF-8");
    assert_eq!(
        stdout,
        concat!(
            "declare -- u=\"\u{200b}\"\n",
            "'\u{200b}'\n",
            "declare -- t=\"\u{feff}\"\n",
            "'\u{feff}'\n",
            "declare -- s=\"\u{ad}\"\n",
            "'\u{ad}'\n",
        )
    );
}

#[test]
fn array_and_assoc_elements_escape_separators() {
    // array.c:912-913 / assoc.c:362-372 run every element through the
    // same ansic_shouldquote/ansic_quote pair as scalar values.
    let script = concat!(
        "arr=($'\\u2028' plain)\n",
        "declare -p arr\n",
        "declare -A h=([k]=$'\\u2029')\n",
        "declare -p h\n",
        "printf '%s\\n' \"${arr[0]@Q}\" \"${h[k]@Q}\"\n",
    );
    let stdout = run_script_locale(script, "C.UTF-8");
    assert_eq!(
        stdout,
        concat!(
            "declare -a arr=([0]=$'\\342\\200\\250' [1]=\"plain\")\n",
            "declare -A h=([k]=$'\\342\\200\\251' )\n",
            "$'\\342\\200\\250'\n$'\\342\\200\\251'\n",
        )
    );
}
