//! rubash#3944 — the `-d` delimiter's operative byte is the option
//! WORD's first raw byte (builtins/read.def:385 `delim = (unsigned
//! char)*list_optarg`; read.def:834 compares it against stream bytes),
//! not the first byte of the widened delimiter CHARACTER's UTF-8
//! encoding. A byte word (`-d $'\357'`) widens to a Latin-1 char whose
//! encoding starts with 0xC3, so under a UTF-8 locale every file-backed
//! serve keyed on 0xC3: the record value was cut at the right byte but
//! the builtin reported rc 1 (delimiter "not found") and the descriptor
//! offset ran past the wrong place — only byte 0xC3 worked end to end.
//!
//! GNU bash 5.3.20 oracle expectations, measured with script files
//! under LC_ALL=C.UTF-8 (the C-locale control locks the byte-locale
//! face the historical single-form already matched).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script_locale(script: &str, locale: &str) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir: PathBuf = std::env::temp_dir().join(format!("rubash-3944-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        // The scripts below write/read the bare relative path `f`, so
        // the child must run inside this test's own scratch dir (the
        // issue3311 `rubash_script` isolation). Without it every
        // parallel sibling shares the test process's cwd and they
        // overwrite each other's `f` — the census failures showed
        // sibling test data verbatim (e.g. the C-locale case reading
        // the 0x82 case's bytes), with rotating victims.
        .current_dir(&dir)
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

fn hex_pipeline(script: &str, locale: &str) -> String {
    const HEX: &str = "hex() { printf '%s' \"$1\" | od -An -tx1 | tr -d ' \\n'; }\n";
    let (stdout, _, code) = run_script_locale(&format!("{HEX}{script}"), locale);
    assert_eq!(code, Some(0));
    stdout
}

#[test]
fn ff_delimiter_first_record_succeeds() {
    let stdout = hex_pipeline(
        "printf 'a\\377b\\377c' > f; exec 3< f; read -d $'\\377' -u 3 x; \
         rc=$?; echo \"rc=$rc x=[$(hex \"$x\")]\"\n",
        "C.UTF-8",
    );
    assert_eq!(stdout, "rc=0 x=[61]\n");
}

#[test]
fn ff_delimiter_second_record_keeps_offset() {
    let stdout = hex_pipeline(
        "printf 'a\\377b\\377c' > f; exec 3< f; read -d $'\\377' -u 3 x; \
         read -d $'\\377' -u 3 y; rc=$?; echo \"rc=$rc y=[$(hex \"$y\")]\"\n",
        "C.UTF-8",
    );
    assert_eq!(stdout, "rc=0 y=[62]\n");
}

#[test]
fn byte_word_82_via_u_fd() {
    let stdout = hex_pipeline(
        "printf 'a\\202b\\202c' > f; exec 3< f; read -d $'\\202' -u 3 x; \
         rc=$?; echo \"rc=$rc x=[$(hex \"$x\")]\"\n",
        "C.UTF-8",
    );
    assert_eq!(stdout, "rc=0 x=[61]\n");
}

#[test]
fn byte_word_fe_via_u_fd() {
    let stdout = hex_pipeline(
        "printf 'a\\376b\\376c' > f; exec 3< f; read -d $'\\376' -u 3 x; \
         rc=$?; echo \"rc=$rc x=[$(hex \"$x\")]\"\n",
        "C.UTF-8",
    );
    assert_eq!(stdout, "rc=0 x=[61]\n");
}

#[test]
fn delimiter_at_end_of_input_succeeds() {
    let stdout = hex_pipeline(
        "printf 'ab\\202' > f; exec 3< f; read -d $'\\202' -u 3 x; \
         rc=$?; echo \"rc=$rc x=[$(hex \"$x\")]\"\n",
        "C.UTF-8",
    );
    assert_eq!(stdout, "rc=0 x=[6162]\n");
}

#[test]
fn c3_control_still_works_end_to_end() {
    // 0xC3 is the one byte whose widened char's encoding starts with the
    // byte itself — the historical passing control.
    let stdout = hex_pipeline(
        "printf 'a\\303b\\303c' > f; exec 3< f; read -d $'\\303' -u 3 x; \
         rc=$?; echo \"rc=$rc x=[$(hex \"$x\")]\"\n",
        "C.UTF-8",
    );
    assert_eq!(stdout, "rc=0 x=[61]\n");
}

#[test]
fn while_loop_over_file_reads_both_records() {
    let stdout = hex_pipeline(
        "printf 'a\\377b\\377c' > f; out=''; \
         while IFS= read -rd $'\\377' s; do out=\"$out<$(hex \"$s\")>\"; done < f; \
         echo \"$out\"\n",
        "C.UTF-8",
    );
    assert_eq!(stdout, "<61><62>\n");
}

#[test]
fn c_locale_face_unchanged() {
    let stdout = hex_pipeline(
        "printf 'a\\377b\\377c' > f; exec 3< f; read -d $'\\377' -u 3 x; \
         rc=$?; echo \"rc=$rc x=[$(hex \"$x\")]\"\n",
        "C",
    );
    assert_eq!(stdout, "rc=0 x=[61]\n");
}
