//! rubash#3489 — the bats junit formatter (`bats-format-junit`
//! `xml_escape`) left `[0m` / `[K` residue in the XML and mangled the
//! `$'\x01..\x1f'` control-character run, while GNU bash 5.3.20 strips the
//! ANSI CSI sequences cleanly.
//!
//! Root cause: the `read` builtin's single-byte-locale byte domain
//! (read_split.rs `read_byte_domain`, rubash#2819). A record served from a
//! byte transport is first carrier-tagged (`tag_read_carrier_text`: every
//! ASCII carrier byte — 0x0c, 0x11, 0x13, 0x14..=0x1f, which includes ESC —
//! rides as a raw-byte marker pair). Those pairs are non-ASCII, so under
//! `LC_ALL=C` (bats runs formatters under `env -i`) the record took the byte
//! domain, and `byte_domain_value` turned the fields back into text by
//! emitting every byte < 0x80 as a bare char — undoing the tagging. The
//! lexer's carrier consumers then claimed the bytes: ESC stopped matching
//! `$'\e'` patterns, 0x14 became `\`, 0x1f `$`, 0x11/0x13 vanished.
//!
//! GNU: read.def:838-845 escapes CTLESC/CTLNUL in the record and
//! read.def:1127 (single variable) / :1061 (split fields) `dequote_string`
//! undoes it, so the bound value is byte-identical to the input bytes in
//! every locale. Expectations below are the GNU 5.3.20 oracle's
//! (`/usr/local/bin/bash`, script file, `LC_ALL=C`).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_file(name: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-issue3489-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    let script_path: PathBuf = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash script file");
    let _ = fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The formatter's own escape loop, fed a TAP line through `read` under
/// the C locale: GNU strips both CSI sequences before printing.
#[test]
fn junit_xml_escape_strips_csi_from_read_line_in_c_locale() {
    let script = r#"export LC_ALL=C
xml_escape() {
  local output=${1//&/\&amp;}
  output=${output//</\&lt;}
  output=${output//>/\&gt;}
  local CONTROL_CHAR=$'\033'
  local REGEX="$CONTROL_CHAR\[[0-9;]*[a-zA-Z]"
  while [[ "$output" =~ $REGEX ]]; do
      output=${output//${BASH_REMATCH[0]}/}
  done
  printf "%s" "$output"
}
printf 'ok 1 x<>&\033[0m\033[K (0x1b)\n' > ./in1.txt
IFS= read -r line < ./in1.txt
printf '[%s]\n' "$(xml_escape "$line")"
"#;
    let (stdout, stderr, status) = rubash_file("xml-escape", script);
    assert_eq!(stdout, "[ok 1 x&lt;&gt;&amp; (0x1b)]\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// Every carrier byte survives a single-variable `read` byte-exact in the
/// C locale and still matches its `$'...'` spelling as a pattern.
#[test]
fn scalar_read_keeps_carrier_bytes_in_c_locale() {
    let script = r#"export LC_ALL=C
printf 'a\033b\014c\021d\023e\024f\037g\001h\177i\n' > ./in2.txt
IFS= read -r v < ./in2.txt
[[ $v == $'a\033b\014c\021d\023e\024f\037g\001h\177i' ]] && echo same || echo differ
echo "len=${#v}"
for t in "${v//$'\033'/}" "${v//$'\014'/}" "${v//$'\021'/}" "${v//$'\023'/}" "${v//$'\024'/}" "${v//$'\037'/}"; do printf '%s ' "${#t}"; done; echo
"#;
    let (stdout, stderr, status) = rubash_file("scalar", script);
    assert_eq!(
        stdout, "same\nlen=17\n16 16 16 16 16 16 \n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// Split fields (two names, and `read -a`) take the same byte-domain
/// write-back and must keep their carrier bytes too.
#[test]
fn split_and_array_read_keep_carrier_bytes_in_c_locale() {
    let script = r#"export LC_ALL=C
printf 'p\033q r\024s\n' > ./in3.txt
read -r x y < ./in3.txt
printf '%s|%s\n' "${x//$'\033'/E}" "${y//$'\024'/T}"
read -r -a arr <<< $'u\033v w\037z'
printf '%s|%s\n' "${arr[0]//$'\033'/E}" "${arr[1]//$'\037'/U}"
"#;
    let (stdout, stderr, status) = rubash_file("split", script);
    assert_eq!(stdout, "pEq|rTs\nuEv|wUz\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// Guard for the rubash#2819 byte domain itself: a valid UTF-8 character
/// next to a carrier byte still re-forms (GNU counts bytes in the C locale).
#[test]
fn byte_domain_still_reforms_utf8_next_to_carrier() {
    let script = "export LC_ALL=C\nprintf 'n\\303\\251\\033m\\n' > ./in4.txt\nIFS= read -r u < ./in4.txt\necho \"len=${#u} ${u//$'\\033'/E}\"\n";
    let (stdout, stderr, status) = rubash_file("utf8", script);
    assert_eq!(stdout, "len=5 n\u{e9}Em\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}
