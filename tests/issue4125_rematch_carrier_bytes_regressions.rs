//! rubash#4125 regressions: `[[ =~ ]]` captures (`BASH_REMATCH`) dropped or
//! rewrote the carrier bytes 0x11, 0x17, 0x18, 0x1a, 0x1c, 0x1f.
//!
//! `CompiledConditionalRegex::captures` returned the matched text raw in
//! both engine modes, bypassing the storage encoding plain assignment
//! applies (`bytes_to_assignment_shell_text`): the raw carrier bytes were
//! stored untagged and later read out as live markers — 0x11 and 0x1c
//! vanished, 0x17/0x18/0x1a/0x1f decoded to `'` / `"` / `` ` `` / `$`.
//! The fix routes both capture branches through that same existing
//! encoding, so a capture stores exactly what an assignment of the same
//! bytes stores.
//!
//! Every expectation below is byte-verified against the GNU Bash 5.3.20
//! oracle (script-file battery, both the C.UTF-8 and C locales; the C0
//! carrier set behaves identically in both). The LC_ALL=C high-byte
//! subject re-encoding face is a separate mechanism, tracked as
//! rubash#4133, and is not probed here.

use std::process::Command;

fn rubash_file(script: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-rematch4125-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .env("LC_ALL", lc_all)
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const PROBE: &str = r#"for h in 01 02 07 09 0a 0d 11 13 17 18 1a 1c 1e 1f 7f; do
  printf -v z "\x$h"
  s="a${z}b"
  [[ $s =~ ^(.*)$ ]]
  printf '%s grp1=' "$h"
  printf '%s' "${BASH_REMATCH[1]}" | od -An -tx1 | tr -d ' \n'
  printf ' len=%s grp2=' "${#BASH_REMATCH[1]}"
  [[ $s =~ ^(a)(.*)$ ]]
  printf '%s' "${BASH_REMATCH[2]}" | od -An -tx1 | tr -d ' \n'
  printf '\n'
done
for h in 11 1c 1f; do
  printf -v z "\x$h"
  arr=("a${z}b")
  printf 'assign %s=' "$h"
  printf '%s' "${arr[0]}" | od -An -tx1 | tr -d ' \n'
  printf '\n'
done
printf -v z "\x11"
s="a${z}b"
[[ $s =~ ^(.*)$ ]]
declare -p BASH_REMATCH
"#;

const EXPECTED: &str = "\
01 grp1=610162 len=3 grp2=0162
02 grp1=610262 len=3 grp2=0262
07 grp1=610762 len=3 grp2=0762
09 grp1=610962 len=3 grp2=0962
0a grp1=610a62 len=3 grp2=0a62
0d grp1=610d62 len=3 grp2=0d62
11 grp1=611162 len=3 grp2=1162
13 grp1=611362 len=3 grp2=1362
17 grp1=611762 len=3 grp2=1762
18 grp1=611862 len=3 grp2=1862
1a grp1=611a62 len=3 grp2=1a62
1c grp1=611c62 len=3 grp2=1c62
1e grp1=611e62 len=3 grp2=1e62
1f grp1=611f62 len=3 grp2=1f62
7f grp1=617f62 len=3 grp2=7f62
assign 11=611162
assign 1c=611c62
assign 1f=611f62
declare -a BASH_REMATCH=([0]=$'a\\021b' [1]=$'a\\021b')
";

#[test]
fn rematch_captures_keep_carrier_bytes_utf8_locale() {
    let (stdout, stderr, code) = rubash_file(PROBE, "C.UTF-8");
    assert_eq!(stdout, EXPECTED, "stdout mismatch");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0), "exit code mismatch");
}

#[test]
fn rematch_captures_keep_carrier_bytes_c_locale() {
    let (stdout, stderr, code) = rubash_file(PROBE, "C");
    assert_eq!(stdout, EXPECTED, "stdout mismatch");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0), "exit code mismatch");
}
