//! rubash#2958 (Stage-2 ④ Phase 3) — pins for the shared
//! numeric core the five numeric builtins migrate onto.
//! GNU `valid_number` (general.c:248) is strtoimax-based: the
//! C isspace set skipped around the digits is space, \t, \n,
//! \v, \f, \r — vertical tab (0x0B) included, so
//! `exit $'\v42'` exits 42 and `exit $'\f7'` exits 7.
//!
//! Two defects meet here, one per padding byte:
//! - \v rides expansion as a raw byte. The core
//!   `builtin_diag::valid_number` trimmed with
//!   `str::trim_ascii` (no 0x0B), so routing the five onto
//!   `Diag::numeric_arg` without fixing the core's trim set
//!   would regress the \v cells — on master they pass through
//!   `builtins::exit::gnu_valid_number`, whose isspace set
//!   includes 0x0B. Guard half of this file.
//! - \f is a carrier byte ({0x0C, 0x11, 0x13},
//!   `is_carrier_byte` in substitution_metadata.rs): it rides
//!   expansion as a raw-byte marker pair (U+E000 U+E00D), and
//!   every numeric consumer — master's per-builtin copies and
//!   the shared core alike — judged the encoded pair instead
//!   of the byte it carries. `exit $'\f7'` failed on master
//!   too (verified on 4f747412: this file ran 4 passed /
//!   1 failed there, the \f cell the failure). The core now
//!   decodes raw-byte marker pairs before judging, which is
//!   the form GNU — having no carrier — always sees.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2958_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

#[test]
fn exit_accepts_vertical_tab_padded_status() {
    let run = run_script("exit $'\\v42'\n");
    assert_eq!(run.status, 42, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn exit_accepts_form_feed_padded_status() {
    let run = run_script("exit $'\\f7'\n");
    assert_eq!(run.status, 7, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn shift_accepts_vertical_tab_padded_count() {
    let run = run_script("set -- a b c d\nshift $'\\v2'\necho \"rc=$? n=$# rest=$*\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=0 n=2 rest=c d\n", "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn return_accepts_vertical_tab_padded_status() {
    let run = run_script("f() { return $'\\v42'; }\nf\necho \"rc=$?\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=42\n", "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn break_accepts_vertical_tab_padded_count() {
    let run = run_script(
        "for i in 1 2 3; do echo iter$i; break $'\\v1'; echo after; done\necho \"rc=$? done\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "iter1\nrc=0 done\n", "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    let _ = run.dir;
}
