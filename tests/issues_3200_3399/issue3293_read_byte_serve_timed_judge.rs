//! rubash#3293 — the FUNCTION_STDIN mirror's byte-domain serve (the
//! joined-delimiter scan of rubash#3212) returned before the timed
//! judge, so a `read -t` on that path never timed out: with a slow
//! producer the second read returned the late record with rc=0 where
//! GNU's alarm fires at the deadline (rc=142, only the in-time prefix
//! assigned). The mirror rewrite also changed the mirror's total
//! length, so the arrival schedule's `matches_len` failed for every
//! later read on the stream and the judge stayed silently off.
//!
//! GNU bash 5.3.20 oracle, measured with script files under
//! LC_ALL=C.UTF-8: read1 serves `x` at once; read2 (-t 1) times out
//! with rc=142 and the single in-time byte 0xA9 assigned; read3
//! (-t 5) then serves the late record `y`. In rubash's
//! capture-then-serve mirror architecture the production wait itself
//! is not observable per-read (the capture drains before the function
//! body runs), so the discriminating observables are the exit codes
//! and assigned values; a coarse whole-script elapsed bound guards
//! against a fix that blocks in real time instead of judging on the
//! virtual clock. The ASCII-delimiter control exercises the legacy
//! judge (already correct) and the fast-producer control locks the
//! in-time byte-serve outcome.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script_locale(script: &str, locale: &str) -> (String, String, Option<i32>, Duration) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir: PathBuf = std::env::temp_dir().join(format!("rubash-3293-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let started = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .env("LC_ALL", locale)
        .env("TMPDIR", &dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let elapsed = started.elapsed();
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        elapsed,
    )
}

const HEX: &str = "hex() { printf '%s' \"$1\" | od -An -tx1 | tr -d ' \\n'; }\n";

#[test]
fn read_byte_serve_second_read_times_out() {
    // Slow producer: "x<é>" at t=0, sleep 3, "y<é>". GNU read2 (-t 1)
    // times out: rc=142, value = the in-time prefix byte 0xA9 alone.
    let script = format!(
        "{HEX}f() {{\n\
         IFS= read -r -t 1 -d $'\\303\\251' a; echo \"read1 rc=$? val=<$a>\"\n\
         IFS= read -r -t 1 -d $'\\303\\251' b; echo \"read2 rc=$? hex=[$(hex \"$b\")]\"\n\
         }}\n\
         {{ printf 'x\\303\\251'; sleep 3; printf 'y\\303\\251'; sleep 0.2; }} | f\n"
    );
    let (stdout, _, code, elapsed) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "read1 rc=0 val=<x>\nread2 rc=142 hex=[a9]\n");
    assert!(elapsed < Duration::from_secs(20), "elapsed {elapsed:?}");
}

#[test]
fn read_byte_serve_judge_survives_for_later_reads() {
    // After the byte-serve rewrite (and after a timeout), the arrival
    // schedule must stay aligned with the mirror: a third read with a
    // generous deadline serves the late record normally.
    let script = format!(
        "{HEX}f() {{\n\
         IFS= read -r -t 1 -d $'\\303\\251' a; echo \"read1 rc=$? val=<$a>\"\n\
         IFS= read -r -t 1 -d $'\\303\\251' b; echo \"read2 rc=$? hex=[$(hex \"$b\")]\"\n\
         IFS= read -r -t 5 -d $'\\303\\251' c; echo \"read3 rc=$? val=<$c>\"\n\
         }}\n\
         {{ printf 'x\\303\\251'; sleep 3; printf 'y\\303\\251'; sleep 0.2; }} | f\n"
    );
    let (stdout, _, code, elapsed) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "read1 rc=0 val=<x>\nread2 rc=142 hex=[a9]\nread3 rc=0 val=<y>\n"
    );
    assert!(elapsed < Duration::from_secs(20), "elapsed {elapsed:?}");
}

#[test]
fn read_ascii_delimiter_timed_control() {
    // Control: an ASCII delimiter never takes the byte serve; the
    // legacy timed judge already produces GNU's verdict (rc=142,
    // empty partial). It must not move.
    let script = format!(
        "{HEX}f() {{\n\
         IFS= read -r -t 1 -d ';' a; echo \"read1 rc=$? val=<$a>\"\n\
         IFS= read -r -t 1 -d ';' b; echo \"read2 rc=$? hex=[$(hex \"$b\")]\"\n\
         }}\n\
         {{ printf 'x;'; sleep 3; printf 'y;'; sleep 0.2; }} | f\n"
    );
    let (stdout, _, code, elapsed) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "read1 rc=0 val=<x>\nread2 rc=142 hex=[]\n");
    assert!(elapsed < Duration::from_secs(20), "elapsed {elapsed:?}");
}

#[test]
fn read_byte_serve_in_time_control() {
    // Control: everything arrives at once and both deadlines are
    // generous — the byte serve must produce both records with rc=0
    // (the reconnected judge must not false-timeout).
    let script = format!(
        "{HEX}f() {{\n\
         IFS= read -r -t 5 -d $'\\303\\251' a; echo \"read1 rc=$? val=<$a>\"\n\
         IFS= read -r -t 5 -d $'\\303\\251' b; echo \"read2 rc=$? hex=[$(hex \"$b\")]\"\n\
         }}\n\
         printf 'x\\303\\251y\\303\\251' | f\n"
    );
    let (stdout, _, code, elapsed) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "read1 rc=0 val=<x>\nread2 rc=0 hex=[a979]\n");
    assert!(elapsed < Duration::from_secs(20), "elapsed {elapsed:?}");
}
