//! rubash#3305 — the coproc serve of `read` (read_coproc_stdout) still
//! scanned byte by byte: the delimiter compare ran on every byte (so a
//! multibyte delimiter's lead byte ended the record and leaked into it),
//! the `-n` budget counted bytes (cutting an invalid sequence at its
//! first byte), and a continuation-byte delimiter fired inside a
//! character. It now drives the same FdRecordScan replay as the fd-table
//! serves.
//!
//! GNU model (third_party/bash, Bash-5.3 patch 20):
//! - builtins/read.def:834 compares the stream byte against the FIRST
//!   byte of the `-d` word, only at the start of a read_mbchar unit.
//! - read.def:884-886 counts the -n/-N budget per completed unit
//!   (`nr++` after read_mbchar), post-escape.
//!
//! Expectations measured with WSL GNU bash 5.3.20 (script file,
//! LC_ALL=C.UTF-8 unless noted); the coproc children stay alive while
//! read runs (`exec >&-` gives EOF without the coproc being reaped).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run(script: &str) -> (String, String) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir: PathBuf = std::env::temp_dir().join(format!("rubash-3305-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, format!("{HEX}{script}")).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

const HEX: &str = "hex() { printf %s \"$1\" | od -An -tx1 | tr -d ' \\n'; }\n";

#[test]
fn c1_multibyte_delimiter_lead_byte_does_not_leak() {
    let (stdout, stderr) = run("export LC_ALL=C.UTF-8\n\
         coproc P { printf 'a\\303\\251b\\303\\251'; sleep 2; }\n\
         read -r -d é x <&\"${P[0]}\"; rc=$?; echo \"C1a [$(hex \"$x\")] rc=$rc\"\n\
         read -r -d é x <&\"${P[0]}\"; rc=$?; echo \"C1b [$(hex \"$x\")] rc=$rc\"\n\
         kill $P_PID 2>/dev/null; wait 2>/dev/null\n");
    assert_eq!(
        stdout, "C1a [61] rc=0\nC1b [a962] rc=0\n",
        "stderr: {stderr}"
    );
}

#[test]
fn c2_n_unit_is_the_whole_invalid_sequence() {
    let (stdout, stderr) = run("export LC_ALL=C.UTF-8\n\
         coproc Q { printf '\\340\\200\\200A'; sleep 2; }\n\
         read -r -n 1 x <&\"${Q[0]}\"; rc=$?; echo \"C2a [$(hex \"$x\")] rc=$rc\"\n\
         read -r -n 1 x <&\"${Q[0]}\"; rc=$?; echo \"C2b [$(hex \"$x\")] rc=$rc\"\n\
         kill $Q_PID 2>/dev/null; wait 2>/dev/null\n");
    assert_eq!(
        stdout, "C2a [e08080] rc=0\nC2b [41] rc=0\n",
        "stderr: {stderr}"
    );
}

#[test]
fn c3_continuation_byte_delimiter_does_not_fire_mid_character() {
    let (stdout, stderr) = run("export LC_ALL=C.UTF-8\n\
         coproc R { printf '\\303\\251'; exec >&-; sleep 2; }\n\
         read -r -d $'\\251' x <&\"${R[0]}\"; rc=$?; echo \"C3a [$(hex \"$x\")] rc=$rc\"\n\
         kill $R_PID 2>/dev/null; wait 2>/dev/null\n");
    assert_eq!(stdout, "C3a [c3a9] rc=1\n", "stderr: {stderr}");
}

#[test]
fn c4_escaped_multibyte_character_is_one_n_unit() {
    let (stdout, stderr) = run("export LC_ALL=C.UTF-8\n\
         coproc S { printf '\\\\\\303\\251'; sleep 2; }\n\
         read -n 1 x <&\"${S[0]}\"; rc=$?; echo \"C4a [$(hex \"$x\")] rc=$rc\"\n\
         kill $S_PID 2>/dev/null; wait 2>/dev/null\n");
    assert_eq!(stdout, "C4a [c3a9] rc=0\n", "stderr: {stderr}");
}

#[test]
fn byte_locale_multibyte_delimiter_keeps_sequence_form() {
    let (stdout, stderr) = run("export LC_ALL=C\n\
         coproc P { printf 'a\\303\\251b\\303\\251c:d\\n'; sleep 2; }\n\
         read -r -d é x <&\"${P[0]}\"; rc=$?; echo \"B1 [$(hex \"$x\")] rc=$rc\"\n\
         read -r -d é x <&\"${P[0]}\"; rc=$?; echo \"B2 [$(hex \"$x\")] rc=$rc\"\n\
         read -r -n 1 x <&\"${P[0]}\"; rc=$?; echo \"B3 [$(hex \"$x\")] rc=$rc\"\n\
         read -r -N 3 x <&\"${P[0]}\"; rc=$?; echo \"B4 [$(hex \"$x\")] rc=$rc\"\n\
         kill $P_PID 2>/dev/null; wait 2>/dev/null\n");
    assert_eq!(
        stdout, "B1 [61] rc=0\nB2 [a962] rc=0\nB3 [a9] rc=0\nB4 [633a64] rc=0\n",
        "stderr: {stderr}"
    );
}

#[test]
fn utf8_budgets_lines_and_empty_record() {
    let (stdout, stderr) = run("export LC_ALL=C.UTF-8\n\
         coproc Q { printf 'x\\303\\251y\\nz\\303\\251\\303\\251w\\n\\n'; sleep 2; }\n\
         read -r -N 2 x <&\"${Q[0]}\"; rc=$?; echo \"U1 [$(hex \"$x\")] rc=$rc\"\n\
         read -r -n 5 x <&\"${Q[0]}\"; rc=$?; echo \"U2 [$(hex \"$x\")] rc=$rc\"\n\
         read -r -n 2 x <&\"${Q[0]}\"; rc=$?; echo \"U3 [$(hex \"$x\")] rc=$rc\"\n\
         read -r x <&\"${Q[0]}\"; rc=$?; echo \"U4 [$(hex \"$x\")] rc=$rc\"\n\
         read -r x <&\"${Q[0]}\"; rc=$?; echo \"U5 [$(hex \"$x\")] rc=$rc\"\n\
         kill $Q_PID 2>/dev/null; wait 2>/dev/null\n");
    assert_eq!(
        stdout, "U1 [78c3a9] rc=0\nU2 [79] rc=0\nU3 [7ac3a9] rc=0\nU4 [c3a977] rc=0\nU5 [] rc=0\n",
        "stderr: {stderr}"
    );
}

#[test]
fn escapes_and_continuations_with_custom_delimiter() {
    let (stdout, stderr) = run(
        "coproc U { printf 'a\\\\:b:c\\\\\\nd:e'; exec >&-; sleep 2; }\n\
         read -d : x <&\"${U[0]}\"; rc=$?; echo \"E1 [$x] rc=$rc\"\n\
         read -d : x <&\"${U[0]}\"; rc=$?; echo \"E2 [$x] rc=$rc\"\n\
         read -d : x <&\"${U[0]}\"; rc=$?; echo \"E3 [$x] rc=$rc\"\n\
         read -d : x <&\"${U[0]}\"; rc=$?; echo \"E4 [$x] rc=$rc\"\n\
         kill $U_PID 2>/dev/null; wait 2>/dev/null\n",
    );
    assert_eq!(
        stdout, "E1 [a:b] rc=0\nE2 [cd] rc=0\nE3 [e] rc=1\nE4 [] rc=1\n",
        "stderr: {stderr}"
    );
}
