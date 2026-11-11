//! rubash#3291 — the fd_table byte readers (Text / process
//! substitution endpoints and both file-reader arms) matched a
//! multibyte delimiter as a whole UTF-8 SEQUENCE, but GNU compares
//! only the delimiter's FIRST byte at the head of each unit
//! (builtins/read.def:385 takes the first byte of the `-d` word;
//! read.def:834 compares it before read_mbchar consumes the rest of
//! the character). For stream `a C3 A9 b C3 A9` with `-d é` under
//! C.UTF-8 the sequence match consumed the continuation byte 0xA9
//! into the first record's delimiter, so the second record lost its
//! leading byte; for stream `a C3 A8 b C3 A8` (è) no sequence ever
//! matched and the loop read to EOF without printing anything.
//!
//! GNU bash 5.3.20 oracle expectations, measured with script files
//! under LC_ALL=C.UTF-8. The pipe-loop control goes through the
//! FUNCTION_STDIN mirror (already byte-exact) and the LC_ALL=C file
//! loop control locks the byte-locale face of the same readers.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script_locale(script: &str, locale: &str) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir: PathBuf = std::env::temp_dir().join(format!("rubash-3291-{}-{n}", std::process::id()));
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

const HEX: &str = "hex() { printf '%s' \"$1\" | od -An -tx1 | tr -d ' \\n'; }\n";

#[test]
fn read_d_fd_table_file_loop_mb() {
    // The issue's own shape: while-loop over a redirected regular
    // file (buffered file-reader arm). GNU records: [61], [a9 62].
    let script = format!(
        "{HEX}printf 'a\\303\\251b\\303\\251' > \"$TMPDIR/d.bin\"\n\
         while IFS= read -r -d $'\\303\\251' x; do echo \"rec=[$(hex \"$x\")]\"; done < \"$TMPDIR/d.bin\"\n"
    );
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rec=[61]\nrec=[a962]\n");
}

#[test]
fn read_d_fd_table_sequential_fd_reads_mb() {
    // Sequential reads on one open fd: the third read sees the final
    // continuation byte as a partial record at EOF (rc=1, value kept).
    let script = format!(
        "{HEX}printf 'a\\303\\251b\\303\\251' > \"$TMPDIR/d.bin\"\n\
         exec 7< \"$TMPDIR/d.bin\"\n\
         IFS= read -r -d $'\\303\\251' a <&7; echo \"seq1 rc=$? [$(hex \"$a\")]\"\n\
         IFS= read -r -d $'\\303\\251' b <&7; echo \"seq2 rc=$? [$(hex \"$b\")]\"\n\
         IFS= read -r -d $'\\303\\251' c <&7; echo \"seq3 rc=$? [$(hex \"$c\")]\"\n\
         exec 7<&-\n"
    );
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "seq1 rc=0 [61]\nseq2 rc=0 [a962]\nseq3 rc=1 [a9]\n");
}

#[test]
fn read_d_fd_table_procsub_loop_mb() {
    // Process substitution endpoint (fd_table Text arm, in-memory
    // scan): same first-byte rule.
    let script = format!(
        "{HEX}while IFS= read -r -d $'\\303\\251' x; do echo \"rec=[$(hex \"$x\")]\"; done < <(printf 'a\\303\\251b\\303\\251')\n"
    );
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rec=[61]\nrec=[a962]\n");
}

#[test]
fn read_d_fd_table_other_char_lead_byte_file_loop_mb() {
    // The stream holds è (C3 A8); the delimiter word names é. GNU
    // splits at the shared lead byte 0xC3 inside è as well.
    let script = format!(
        "{HEX}printf 'a\\303\\250b\\303\\250' > \"$TMPDIR/d.bin\"\n\
         while IFS= read -r -d $'\\303\\251' x; do echo \"rec=[$(hex \"$x\")]\"; done < \"$TMPDIR/d.bin\"\n"
    );
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rec=[61]\nrec=[a862]\n");
}

#[test]
fn read_d_fd_table_file_loop_timed_arm_mb() {
    // A -t deadline routes the file endpoint through the byte-wise
    // reader arm; the delimiter rule is the same.
    let script = format!(
        "{HEX}printf 'a\\303\\251b\\303\\251' > \"$TMPDIR/d.bin\"\n\
         while IFS= read -r -t 2 -d $'\\303\\251' x; do echo \"rec=[$(hex \"$x\")]\"; done < \"$TMPDIR/d.bin\"\n"
    );
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rec=[61]\nrec=[a962]\n");
}

#[test]
fn read_d_pipe_loop_mirror_control() {
    // Control: the pipe loop is served by the FUNCTION_STDIN mirror
    // byte scan and already agrees with GNU; it must not move.
    let script = format!(
        "{HEX}printf 'a\\303\\251b\\303\\251' | while IFS= read -r -d $'\\303\\251' x; do echo \"rec=[$(hex \"$x\")]\"; done\n"
    );
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rec=[61]\nrec=[a962]\n");
}

#[test]
fn read_d_file_loop_byte_locale_control() {
    // Control: the same file loop under LC_ALL=C already agrees with
    // GNU (the byte-locale sequence forms are pinned by issue3212 as
    // well); the multibyte change must not move it.
    let script = format!(
        "{HEX}printf 'a\\303\\251b\\303\\251' > \"$TMPDIR/d.bin\"\n\
         while IFS= read -r -d $'\\303\\251' x; do echo \"rec=[$(hex \"$x\")]\"; done < \"$TMPDIR/d.bin\"\n"
    );
    let (stdout, _, code) = run_script_locale(&script, "C");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rec=[61]\nrec=[a962]\n");
}
