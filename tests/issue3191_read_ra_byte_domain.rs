//! rubash#3191 — `read -r -a` in a single-byte locale dropped empty
//! fields when the IFS contained a multibyte character: the producer
//! sliced the original line by the byte ranges of
//! `split_read_field_ranges`, and in the byte domain those ranges fall
//! inside multibyte characters (IFS 'é' is the two byte-delimiters
//! 0xC3 and 0xA9 under LC_ALL=C), so the empty fields between the two
//! delimiter bytes could not be expressed.
//!
//! GNU bash 5.3.20 oracle (builtins/read.def: the -a assignment runs
//! list_string over the record and assign_array_var_from_word_list on
//! the resulting VALUES; subst.c list_string tests separator
//! membership per byte when MB_CUR_MAX == 1). Expectations measured
//! against the oracle with script files under LC_ALL=C; the two
//! control cells lock the sibling faces (read -a without -r, scalar
//! read) and the multibyte-locale face, which already agree with GNU
//! and must not move.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script_locale(script: &str, locale: &str) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir: PathBuf = std::env::temp_dir().join(format!("rubash-3191-{}-{n}", std::process::id()));
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
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const DUMP: &str =
    "dump() { echo \"n=${#B[@]}\"; for v in \"${B[@]}\"; do printf '<%s>' \"$v\"; done; echo; }\n";

#[test]
fn read_ra_byte_domain_herestring() {
    // The issue's own shape: IFS 'é' under LC_ALL=C is the two byte
    // delimiters 0xC3, 0xA9; each delimits its own field, so the
    // fields between them are empty. GNU: n=5 <x><><y><><z>.
    let script = format!("{DUMP}IFS='é' read -r -a B <<< 'xéyéz'; dump\n");
    let (stdout, _, code) = run_script_locale(&script, "C");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=5\n<x><><y><><z>\n");
}

#[test]
fn read_ra_byte_domain_pipe() {
    let script = format!("{DUMP}printf 'xéyéz\\n' | {{ IFS='é' read -r -a B; dump; }}\n");
    let (stdout, _, code) = run_script_locale(&script, "C");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=5\n<x><><y><><z>\n");
}

#[test]
fn read_ra_byte_domain_trailing_delimiters() {
    // Trailing delimiter pair: the field after 'z' is started by 0xC3
    // and terminated empty by 0xA9. GNU: n=6.
    let script = format!("{DUMP}IFS='é' read -r -a B <<< 'xéyézé'; dump\n");
    let (stdout, _, code) = run_script_locale(&script, "C");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=6\n<x><><y><><z><>\n");
}

#[test]
fn read_ra_byte_domain_leading_delimiters() {
    let script = format!("{DUMP}IFS='é' read -r -a B <<< 'éxéy'; dump\n");
    let (stdout, _, code) = run_script_locale(&script, "C");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=5\n<><><x><><y>\n");
}

#[test]
fn read_ra_byte_domain_short() {
    let script = format!("{DUMP}IFS='é' read -r -a B <<< 'xéy'; dump\n");
    let (stdout, _, code) = run_script_locale(&script, "C");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=3\n<x><><y>\n");
}

#[test]
fn read_a_without_r_control() {
    // Sibling lock: read -a without -r already splits at value level
    // through the byte domain and agrees with GNU; it must not move.
    let script = format!("{DUMP}IFS='é' read -a B <<< 'xéyéz'; dump\n");
    let (stdout, _, code) = run_script_locale(&script, "C");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=5\n<x><><y><><z>\n");
}

#[test]
fn read_scalar_control() {
    // Sibling lock: scalar read agrees with GNU (the last variable
    // keeps the unsplit remainder, delimiters included).
    let (stdout, _, code) = run_script_locale(
        "IFS='é' read -r s1 s2 s3 <<< 'xéyéz'; echo \"s1=<$s1> s2=<$s2> s3=<$s3>\"\n",
        "C",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "s1=<x> s2=<> s3=<yéz>\n");
}

#[test]
fn read_ra_mb_locale_control() {
    // Multibyte-locale lock: IFS 'é' is ONE character delimiter, so
    // GNU gives n=3 and rubash already agrees; the byte-domain route
    // must stay out of this face.
    let script = format!("{DUMP}IFS='é' read -r -a B <<< 'xéyéz'; dump\n");
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=3\n<x><y><z>\n");
}
