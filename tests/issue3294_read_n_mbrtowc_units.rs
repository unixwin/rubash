//! rubash#3294 — `read -n` counted each raw byte of an invalid UTF-8
//! sequence as its own character, but GNU's unit is one read_mbchar
//! consumption (builtins/read.def: read_mbchar accumulates bytes
//! while mbrtowc reports an incomplete sequence, and on an invalid
//! completion the whole accumulated run — lead byte, deferred
//! continuation bytes, and the byte that invalidated the sequence —
//! is ONE unit; glibc mbrtowc defers continuation-range validation
//! until the sequence completes). For bytes [E0 80 80] under
//! C.UTF-8, GNU `-n 1` consumes all three bytes; rubash consumed one
//! (text-domain serves) or stopped after the lead byte (fd_table
//! serves), and `-n 2` diverged by up to sequence_length - 1 bytes.
//!
//! GNU bash 5.3.20 oracle expectations, measured with script files
//! under LC_ALL=C.UTF-8 across the pipe / function-stdin / file /
//! herestring serves. Controls lock the valid-character face, the
//! truncated-at-EOF face, and the LC_ALL=C per-byte face, which
//! already agree with GNU.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script_locale(script: &str, locale: &str) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir: PathBuf = std::env::temp_dir().join(format!("rubash-3294-{}-{n}", std::process::id()));
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

// The three-read body as inline statements (for brace-group and
// redirection serves) and as a function definition (for the
// function-stdin mirror serve). Both forms print the same lines.
const READS: &str = "read -n 1 a; r1=$?\n\
read -n 1 b; r2=$?\n\
read -n 9 c; r3=$?\n\
echo \"n1=[$(hex \"$a\")] rc=$r1\"\n\
echo \"n2=[$(hex \"$b\")] rc=$r2\"\n\
echo \"rest=[$(hex \"$c\")] rc=$r3\"\n";

const EXPECTED_STREAM: &str = "n1=[e08080] rc=0\nn2=[41] rc=0\nrest=[42] rc=1\n";

#[test]
fn read_n_invalid_unit_pipe_brace_serve() {
    let script = format!("{HEX}printf '\\340\\200\\200\\101\\102' | {{ {READS} }}\n");
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, EXPECTED_STREAM);
}

#[test]
fn read_n_invalid_unit_function_stdin_serve() {
    let script = format!("{HEX}f() {{\n{READS}}}\nprintf '\\340\\200\\200\\101\\102' | f\n");
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, EXPECTED_STREAM);
}

#[test]
fn read_n_invalid_unit_file_serve() {
    let script = format!(
        "{HEX}printf '\\340\\200\\200\\101\\102' > \"$TMPDIR/d.bin\"\n{{ {READS} }} < \"$TMPDIR/d.bin\"\n"
    );
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, EXPECTED_STREAM);
}

#[test]
fn read_n_invalid_unit_herestring_serve() {
    // The herestring appends a newline, so the rest read terminates
    // on the delimiter (rc=0) with the same values.
    let script =
        format!("{HEX}s=$(printf '\\340\\200\\200\\101\\102')\n{{ {READS} }} <<< \"$s\"\n");
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n1=[e08080] rc=0\nn2=[41] rc=0\nrest=[42] rc=0\n");
}

#[test]
fn read_n2_invalid_unit_counts_once() {
    // -n 2 = the invalid 3-byte unit plus 'A': all four bytes.
    let script = format!(
        "{HEX}f() {{ read -n 2 a; echo \"n2=[$(hex \"$a\")] rc=$?\"; }}\n\
         printf '\\340\\200\\200\\101' | f\n"
    );
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n2=[e0808041] rc=0\n");
}

#[test]
fn read_n_invalidating_byte_joins_unit() {
    // [C3 28]: mbrtowc fails only once the second byte is seen, and
    // read_mbchar consumes it into the same unit. GNU -n 1 = [c3 28].
    let script = format!(
        "{HEX}f() {{ read -n 1 a; echo \"n1=[$(hex \"$a\")] rc=$?\"; \
         read -n 1 b; echo \"next=[$(hex \"$b\")] rc=$?\"; }}\n\
         printf '\\303\\050\\101' | f\n"
    );
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n1=[c328] rc=0\nnext=[41] rc=0\n");
}

#[test]
fn read_big_n_invalid_unit_counts_once() {
    // -N (exact) shares the unit accounting with -n.
    let script = format!(
        "{HEX}f() {{ read -N 2 a; echo \"N2=[$(hex \"$a\")] rc=$?\"; \
         read -N 1 b; echo \"next=[$(hex \"$b\")] rc=$?\"; }}\n\
         printf '\\340\\200\\200\\101\\102' | f\n"
    );
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "N2=[e0808041] rc=0\nnext=[42] rc=0\n");
}

#[test]
fn read_n_valid_char_file_serve_not_truncated() {
    // The fd-table file serve used to truncate even VALID multibyte
    // characters at the lead byte (-n 1 over é served [c3]); GNU
    // serves the whole character, and -n 2 over éè serves both.
    let script = format!(
        "{HEX}printf '\\303\\251\\132' > \"$TMPDIR/v.bin\"\n\
         exec 3< \"$TMPDIR/v.bin\"\n\
         read -n 1 a <&3; echo \"n1=[$(hex \"$a\")] rc=$?\"\n\
         read -n 1 b <&3; echo \"rest=[$(hex \"$b\")] rc=$?\"\n\
         exec 3<&-\n\
         printf '\\303\\251\\303\\250' > \"$TMPDIR/w.bin\"\n\
         exec 3< \"$TMPDIR/w.bin\"\n\
         read -n 2 c <&3; echo \"n2=[$(hex \"$c\")] rc=$?\"\n"
    );
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "n1=[c3a9] rc=0\nrest=[5a] rc=0\nn2=[c3a9c3a8] rc=0\n"
    );
}

#[test]
fn read_n_valid_char_control() {
    // Control: a valid multibyte character already counts once.
    let script = format!(
        "{HEX}f() {{ read -n 1 a; echo \"n1=[$(hex \"$a\")] rc=$?\"; \
         read -n 1 b; echo \"next=[$(hex \"$b\")] rc=$?\"; }}\n\
         printf '\\303\\251\\132' | f\n"
    );
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n1=[c3a9] rc=0\nnext=[5a] rc=0\n");
}

#[test]
fn read_n_truncated_sequence_control() {
    // Control: a truncated sequence at EOF is served whole by both
    // shells (GNU rc=0 since the -n budget ends the record).
    let script = format!(
        "{HEX}f() {{ read -n 3 a; echo \"n3=[$(hex \"$a\")] rc=$?\"; }}\n\
         printf '\\340\\200' | f\n"
    );
    let (stdout, _, code) = run_script_locale(&script, "C.UTF-8");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n3=[e080] rc=0\n");
}

#[test]
fn read_n_byte_locale_control() {
    // Control: under LC_ALL=C every byte is one unit already.
    let script = format!("{HEX}printf '\\340\\200\\200\\101' | {{ {READS} }}\n");
    let (stdout, _, code) = run_script_locale(&script, "C");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n1=[e0] rc=0\nn2=[80] rc=0\nrest=[8041] rc=1\n");
}
