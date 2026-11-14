//! Regression pins for rubash#3102 — the typed indexed cell and its
//! mirror serialization disagree when a multi-element array assignment
//! carries raw-byte pairs (U+E000 family, the storage encoding of C0
//! bytes): the compound field-split transport falls back to synthetic
//! `U+E309 + ' '` separators when it cannot locate a field verbatim
//! (the field holds the raw byte, the value holds the pair), and the
//! indexed re-splitter only honors `U+E309` before an IFS character —
//! under a non-whitespace IFS (`IFS=$'\n'`) the synthetic separator
//! is invisible, the fields glue into one element, and the mirror
//! renders a single garbled `$'...'` element while the typed cell
//! holds two garbled ones. Debug builds panic in
//! `verify_typed_cells` (`variable_state.rs`); release builds keep
//! the corrupted elements silently.
//!
//! GNU ownership: `expand_compound_array_assignment`
//! (arrayfunc.c:557) tokenizes the compound body and
//! `expand_words_no_vars` (arrayfunc.c:610) field-splits each word
//! exactly once on the live IFS; assoc targets skip the split
//! (arrayfunc.c:627-635). GNU has ONE array object, so the elements
//! are stored verbatim after that single split — the rubash mirror
//! is an internal dual write and its re-serialization must be
//! lossless for arbitrary element bytes. The `$'...'` quoting of an
//! element holding a C0 byte is `strtrans.c ansic_quote` (octal
//! escapes), which `declare -p` prints verbatim.
//!
//! All expectations are the literal GNU bash 5.3.20 outputs for the
//! same script file.
#![cfg(unix)]

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

const SCRIPT: &str = r#"dump() { /usr/bin/printf '%s' "$1" | od -An -tx1 | tr -d '\n'; echo " <- $2"; }
raw1f=$(printf 'x\037y\nz\037w')
raw1c1f=$(printf 'x\034y\nz\037w')
raw1c=$(printf 'x\034y\nz\034w')
raw1d=$(printf 'x\035y\nz\035w')
raw1e=$(printf 'x\036y\nz\036w')
echo "== A local -a fn, IFS newline, x1f only"
h() { local IFS=$'\n'; local -a a=($1); printf 'n=%s\n' "${#a[@]}"; dump "${a[0]}" "a0"; dump "${a[1]}" "a1"; declare -p a; }
h "$raw1f"
echo "== A2 local -a fn, IFS newline, x1c+x1f"
h "$raw1c1f"
echo "== A3 local -a fn, IFS newline, x1c only"
h "$raw1c"
echo "== A4 local -a fn, IFS newline, x1d only"
h "$raw1d"
echo "== A5 local -a fn, IFS newline, x1e only"
h "$raw1e"
echo "== B top-level, IFS newline, x1f only"
IFS=$'\n'; b=($raw1f); unset IFS
printf 'n=%s\n' "${#b[@]}"; dump "${b[0]}" "b0"; dump "${b[1]}" "b1"; declare -p b
echo "== C COMPREPLY, x1f only"
k() { local IFS=$'\n'; COMPREPLY=($1); printf 'n=%s\n' "${#COMPREPLY[@]}"; dump "${COMPREPLY[0]}" "c0"; dump "${COMPREPLY[1]}" "c1"; }
k "$raw1f"
echo "== D single element, x1f"
s() { local IFS=$'\n'; local -a a=($1); printf 'n=%s\n' "${#a[@]}"; dump "${a[0]}" "s0"; }
s "$(printf 'x\037y')"
echo "== E IFS colon, x1f, colon-separated"
e() { local IFS=':'; local -a a=($1); printf 'n=%s\n' "${#a[@]}"; dump "${a[0]}" "e0"; dump "${a[1]}" "e1"; }
e "$(printf 'x\037y:z\037w')"
echo PIN-DONE
"#;

const EXPECTED: &str = r#"== A local -a fn, IFS newline, x1f only
n=2
 78 1f 79 <- a0
 7a 1f 77 <- a1
declare -a a=([0]=$'x\037y' [1]=$'z\037w')
== A2 local -a fn, IFS newline, x1c+x1f
n=2
 78 1c 79 <- a0
 7a 1f 77 <- a1
declare -a a=([0]=$'x\034y' [1]=$'z\037w')
== A3 local -a fn, IFS newline, x1c only
n=2
 78 1c 79 <- a0
 7a 1c 77 <- a1
declare -a a=([0]=$'x\034y' [1]=$'z\034w')
== A4 local -a fn, IFS newline, x1d only
n=2
 78 1d 79 <- a0
 7a 1d 77 <- a1
declare -a a=([0]=$'x\035y' [1]=$'z\035w')
== A5 local -a fn, IFS newline, x1e only
n=2
 78 1e 79 <- a0
 7a 1e 77 <- a1
declare -a a=([0]=$'x\036y' [1]=$'z\036w')
== B top-level, IFS newline, x1f only
n=2
 78 1f 79 <- b0
 7a 1f 77 <- b1
declare -a b=([0]=$'x\037y' [1]=$'z\037w')
== C COMPREPLY, x1f only
n=2
 78 1f 79 <- c0
 7a 1f 77 <- c1
== D single element, x1f
n=1
 78 1f 79 <- s0
== E IFS colon, x1f, colon-separated
n=2
 78 1f 79 <- e0
 7a 1f 77 <- e1
PIN-DONE
"#;

fn run_script(body: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3102-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write probe");
    let output = Command::new(BIN)
        .arg(&script)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, output.status.code())
}

#[test]
fn issue3102_array_mirror_raw_byte_pairs() {
    let (stdout, stderr, code) = run_script(SCRIPT);
    assert_eq!(code, Some(0), "exit code; stderr:\n{stderr}");
    assert!(stderr.is_empty(), "expected empty stderr, got:\n{stderr}");
    assert_eq!(stdout, EXPECTED, "stdout differs from GNU 5.3.20");
}
