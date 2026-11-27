//! Regression pins for rubash#1690 — a compound array assignment whose
//! elements are process substitutions collapsed into ONE element whose
//! text was the whole parenthesized string:
//!   a=(<(echo one) <(echo two)); echo ${#a[@]}
//! printed 1 (element text `(/dev/fd/63 /dev/fd/62)`) where GNU bash
//! 5.3.20 prints 2 with the elements /dev/fd/63 and /dev/fd/62.
//!
//! GNU ownership: subst.c expand_word_internal runs process substitution
//! in assignment position exactly like argument position
//! (extract_process_subst / process_substitute, subst.c:6362), and
//! arrayfunc.c:557 expand_compound_array_assignment splits the raw body
//! into words FIRST (parse_string_to_word_list, arrayfunc.c:581) and
//! expands each element word separately — so every `<(...)`/`>(...)`
//! element materializes to its own /dev/fd word and lands as its own
//! array element. The collapse hit every shape with an unquoted procsub
//! element: pairs, singles, mixes with plain words in either position,
//! and output substitutions.
//!
//! Every expectation below is the literal GNU bash 5.3.20 output for the
//! same script file, with fd numbers normalized to /dev/fd/N (the number
//! itself is process state, not semantics).

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-1690-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run_script(dir: &PathBuf, body: &str) -> (String, String, Option<i32>) {
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// /dev/fd/63 -> /dev/fd/N (fd numbers are process state).
fn normalize_fds(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if text[i..].starts_with("/dev/fd/") {
            out.push_str("/dev/fd/N");
            i += "/dev/fd/".len();
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        } else {
            let ch_len = text[i..].chars().next().unwrap().len_utf8();
            out.push_str(&text[i..i + ch_len]);
            i += ch_len;
        }
    }
    out
}

const SCRIPT: &str = r#"
echo "== pair"
a=(<(echo one) <(echo two)); echo "n:${#a[@]}"; echo "0:${a[0]}"; echo "1:${a[1]}"
echo "== single"
b=(<(echo solo)); echo "n:${#b[@]}"; echo "0:${b[0]}"
echo "== mixed suffix"
c=(x <(echo one)); echo "n:${#c[@]}"; echo "0:${c[0]}"; echo "1:${c[1]}"
echo "== mixed prefix"
d=(<(echo one) y); echo "n:${#d[@]}"; echo "0:${d[0]}"; echo "1:${d[1]}"
echo "== triple"
e=(<(echo a) <(echo b) <(echo c)); echo "n:${#e[@]}"
echo "== output pair"
f=(>(cat > o1.txt) >(cat > o2.txt)); echo "n:${#f[@]}"; echo "0:${f[0]}"; echo "1:${f[1]}"
echo "== plain control"
g=(x y); echo "n:${#g[@]}"; echo "0:${g[0]}"; echo "1:${g[1]}"
echo "== comsub control"
h=($(echo word)); echo "n:${#h[@]}"; echo "0:${h[0]}"
echo "== declare form"
declare -a k=(<(echo one) <(echo two)); echo "n:${#k[@]}"; echo "0:${k[0]}"; echo "1:${k[1]}"
"#;

const EXPECTED: &str = "\
== pair
n:2
0:/dev/fd/N
1:/dev/fd/N
== single
n:1
0:/dev/fd/N
== mixed suffix
n:2
0:x
1:/dev/fd/N
== mixed prefix
n:2
0:/dev/fd/N
1:y
== triple
n:3
== output pair
n:2
0:/dev/fd/N
1:/dev/fd/N
== plain control
n:2
0:x
1:y
== comsub control
n:1
0:word
== declare form
n:2
0:/dev/fd/N
1:/dev/fd/N
";

#[test]
fn issue1690_compound_procsub_assignment_keeps_one_element_per_word() {
    let dir = scratch("matrix");
    let (stdout, _stderr, code) = run_script(&dir, SCRIPT);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0));
    assert_eq!(
        normalize_fds(&stdout),
        EXPECTED,
        "each procsub element of a compound assignment is its own element (GNU bash 5.3.20)"
    );
}
