//! rubash#1965 — a numbered-fd here-string (`0<<<`) stores its word in
//! decoded-with-guard form (`\u{1d}a\u{E401}\tb`: the ANSI-C IFS guard
//! U+E401 before every lexer-decoded `$'...'` blank), and the two
//! `stdin_string` ladders in shell_options.rs expanded that storage
//! word without the guard strip the canonical funnel applies
//! (expand_stored_here_string_word, command_input_scope.rs — GNU
//! redir.c:373 r_reading_string serves the word through
//! expand_assignment_string_to_string and here_document_to_fd writes
//! the resulting BYTES; a transport guard is not data). On the pipeline
//! staging route the guard byte reached the delivered stream
//! (`61 ee 90 81 09 62 0a` instead of GNU's `61 09 62 0a`). Ordinary
//! `<<<` and the direct-bind route were already clean.
//!
//! All expectations measured against GNU bash 5.3.20 with script files
//! (LC_ALL=C.UTF-8), 2026-10-29.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rubash-i1965-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> (Vec<u8>, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out_path = self.dir.join("out.txt");
        let err_path = self.dir.join("err.txt");
        let out_file = fs::File::create(&out_path).unwrap();
        let err_file = fs::File::create(&err_path).unwrap();
        let status = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .env("LC_ALL", "C.UTF-8")
            .current_dir(&self.dir)
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::from(err_file))
            .status()
            .unwrap();
        let mut out = Vec::new();
        fs::File::open(&out_path)
            .unwrap()
            .read_to_end(&mut out)
            .unwrap();
        let mut err = String::new();
        fs::File::open(&err_path)
            .unwrap()
            .read_to_string(&mut err)
            .unwrap();
        (out, err, status.code().unwrap_or(-1))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue1965_numbered_herestring_pipeline_delivers_no_guard() {
    let s = Scratch::new("pipe");
    // The issue's measured shape plus its blank siblings: every decoded
    // blank class ($'..' tab / space / newline) leaked the same guard.
    let script = "cat 0<<< $'a\\tb' | od -An -tx1\n\
                  cat 0<<< $'a b' | od -An -tx1\n\
                  cat 0<<< $'a\\nb' | od -An -tx1\n";
    let (out, err, rc) = s.run(script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(
        out,
        b" 61 09 62 0a\n 61 20 62 0a\n 61 0a 62 0a\n".to_vec(),
        "delivered stream must be byte-identical to GNU, no U+E401"
    );
    assert_eq!(err, "");
}

#[test]
fn issue1965_unnumbered_and_direct_routes_stay_clean() {
    let s = Scratch::new("routes");
    // Controls (already clean on master, must not regress): the
    // direct-bind route in both spellings — `0<<<` writing to a file,
    // and unnumbered `<<<` writing to a file.
    //
    // The unnumbered PIPELINE form (`cat <<< ... | od`) is deliberately
    // NOT pinned here: on master it intermittently leaks the staged
    // body into the pipeline capture (raw body bytes + NUL segment
    // padding ahead of the last stage's output) under load — a
    // pre-existing staging race in the pipeline executor, reproduced
    // with a pristine master binary and strace (the body is written a
    // second time into the stage-collection fd), unrelated to the
    // U+E401 guard this file pins. The numbered pipeline form above
    // stages through heredoc_redirects and does not show it.
    let script = "cat 0<<< $'a\\tb' > out.txt\n\
                  od -An -tx1 out.txt\n\
                  cat <<< $'a\\tb' > out2.txt\n\
                  od -An -tx1 out2.txt\n";
    let (out, err, rc) = s.run(script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, b" 61 09 62 0a\n 61 09 62 0a\n".to_vec());
    assert_eq!(err, "");
}

#[test]
fn issue1965_user_typed_e401_stays_data() {
    let s = Scratch::new("userdata");
    // The guard strip is E400-aware (rubash#735): a user-typed literal
    // U+E401 in the here-string word is data and must survive delivery.
    let script = "cat 0<<< $'a\\uE401b' | od -An -tx1\n";
    let (out, err, rc) = s.run(script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, b" 61 ee 90 81 62 0a\n".to_vec());
    assert_eq!(err, "");
}
