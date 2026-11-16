//! rubash#3650 — a value filled by `read` compares unequal under `[`/`test`
//! for control bytes the command-substitution payload protocol carries.
//!
//! Root cause (two halves of one protocol leak in the stdin-body path,
//! LC_ALL=C only — the UTF-8 locale never encodes the payload):
//!
//! 1. `protect_command_substitution_output` encodes the bytes outside the
//!    assignment-carrier set (0x01/0x02/0x03/0x05/0x10/0x12) as the
//!    U+E319 + hex + `;` payload when a comsub result is spliced into a
//!    word. The heredoc final boundary decodes that payload
//!    (`expand_heredoc_body_mut`), but the here-string boundary
//!    (`expand_here_string_mut`) never did, so the payload spelling leaked
//!    into the stdin body and `read`/`mapfile` stored it verbatim —
//!    `[ "$v" = "$(...)" ]` then compared payload text against the raw
//!    byte the RHS decoded to. `[[ ]]`/`case` failed the same way: the
//!    stored value itself was wrong (od showed `<E319>01;`).
//! 2. Byte 0x03 never even reached the payload: `substitution_result_visible_text`
//!    runs `decode_to_visible_text` on the unprotected dispatcher text,
//!    and that decoder consumes a bare 0x03 as the live
//!    DEFERRED_COMPOUND_BODY marker — the byte vanished from heredoc AND
//!    here-string bodies (while `echo "$(...)"` and assignments kept it
//!    via the fragment path, which never runs that decode).
//!
//! GNU: builtins/read.def binds the record byte-identical to the input in
//! every locale, and subst.c:7143 command_substitute yields the child's
//! raw stdout bytes. Expectations below are the GNU 5.2.21 oracle's
//! (box, script file, LC_ALL=C unless noted).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_file(name: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-issue3650-{name}-{}", std::process::id()));
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

/// The issue's seed one-liner: an array `read -d ""` of an ANSI escape
/// sequence must equal the identical literal under `[`.
#[test]
fn seed_array_read_esc_sequence_compares_equal_in_c_locale() {
    let script = r#"export LC_ALL=C
out=$(printf "\033[0m\033[31;1m"); IFS=$'\n' read -d "" -r -a lines <<<"$out"; [ "${lines[0]}" = $'\033[0m\033[31;1m' ] && echo PASS || echo FAIL
"#;
    let (stdout, stderr, status) = rubash_file("seed", script);
    assert_eq!(stdout, "PASS\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// Full trigger scan: every control byte 0x01-0x1f (except 0x0a, see the
/// newline test below) plus 0x7f, fed by a comsub directly inside the
/// here-string word, scalar `read`, compared with `[ = ]` against the
/// same comsub literal. GNU passes every one of these.
#[test]
fn direct_comsub_herestring_scan_all_control_bytes_in_c_locale() {
    let script = r#"export LC_ALL=C
for h in 01 02 03 04 05 06 07 08 09 0b 0c 0d 0e 0f 10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f 7f; do
  IFS= read -r v <<<"$(printf "A\x$h""B")"
  if [ "$v" = "$(printf "A\x$h""B")" ]; then echo "$h PASS"; else echo "$h FAIL"; fi
done
"#;
    let (stdout, stderr, status) = rubash_file("scan", script);
    let expected: String = [
        "01", "02", "03", "04", "05", "06", "07", "08", "09", "0b", "0c", "0d", "0e", "0f", "10",
        "11", "12", "13", "14", "15", "16", "17", "18", "19", "1a", "1b", "1c", "1d", "1e", "1f",
        "7f",
    ]
    .iter()
    .map(|h| format!("{h} PASS\n"))
    .collect();
    assert_eq!(stdout, expected, "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// 0x0a is not part of the bug: `read` splits lines on it, so the stored
/// value is `A` on both shells and `[` fails on GNU too. Pinned so a
/// future change to the scan loop cannot mistake it for this bug.
#[test]
fn newline_byte_matches_gnu_both_fail() {
    let script = r#"export LC_ALL=C
IFS= read -r v <<<"$(printf "A\x0a""B")"
[ "$v" = "$(printf "A\x0a""B")" ] && echo PASS || echo FAIL
"#;
    let (stdout, stderr, status) = rubash_file("newline", script);
    assert_eq!(stdout, "FAIL\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// The same payload bytes through the other stdin-body consumers: a
/// heredoc body with a direct comsub, and `mapfile` from a here-string.
/// Both failed before the fix (heredoc lost 0x03, mapfile stored the
/// E319 spelling for 0x01).
#[test]
fn heredoc_fed_and_mapfile_reads_keep_payload_bytes_in_c_locale() {
    let script = r#"export LC_ALL=C
IFS= read -r v <<EOF
$(printf "A\x01B")
EOF
[ "$v" = "$(printf "A\x01B")" ] && echo heredoc-01-PASS || echo heredoc-01-FAIL
IFS= read -r w <<EOF
$(printf "A\x03B")
EOF
[ "$w" = "$(printf "A\x03B")" ] && echo heredoc-03-PASS || echo heredoc-03-FAIL
mapfile -t lines <<<"$(printf "A\x01B")"; [ "${lines[0]}" = "$(printf "A\x01B")" ] && echo mapfile-01-PASS || echo mapfile-01-FAIL
mapfile -t rows <<<"$(printf "A\x03B")"; [ "${rows[0]}" = "$(printf "A\x03B")" ] && echo mapfile-03-PASS || echo mapfile-03-FAIL
"#;
    let (stdout, stderr, status) = rubash_file("heredoc-mapfile", script);
    assert_eq!(
        stdout, "heredoc-01-PASS\nheredoc-03-PASS\nmapfile-01-PASS\nmapfile-03-PASS\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// `[[ ]]` and `case` compare against the same stored value, so healing
/// the stored value heals them too (both failed for 0x01 before the fix).
#[test]
fn double_bracket_and_case_agree_after_fix_in_c_locale() {
    let script = r#"export LC_ALL=C
IFS= read -r v <<<"$(printf "A\x01B")"; [[ "$v" == "$(printf "A\x01B")" ]] && echo dbl-01-PASS || echo dbl-01-FAIL
IFS= read -r w <<<"$(printf "A\x03B")"; [[ "$w" == "$(printf "A\x03B")" ]] && echo dbl-03-PASS || echo dbl-03-FAIL
IFS= read -r x <<<"$(printf "A\x01B")"; case "$x" in "$(printf "A\x01B")") echo case-01-PASS;; *) echo case-01-FAIL;; esac
IFS= read -r y <<<"$(printf "A\x03B")"; case "$y" in "$(printf "A\x03B")") echo case-03-PASS;; *) echo case-03-FAIL;; esac
"#;
    let (stdout, stderr, status) = rubash_file("dbl-case", script);
    assert_eq!(
        stdout, "dbl-01-PASS\ndbl-03-PASS\ncase-01-PASS\ncase-03-PASS\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// Copying the read value first, and feeding `read` from a file or a pipe
/// (paths that never carried the payload), all agree with GNU.
#[test]
fn copy_and_file_pipe_feeds_agree_in_c_locale() {
    let script = r#"export LC_ALL=C
IFS= read -r v <<<"$(printf "A\x01B")"; c="$v"; [ "$c" = "$(printf "A\x01B")" ] && echo copy-01-PASS || echo copy-01-FAIL
IFS= read -r w <<<"$(printf "A\x03B")"; d="$w"; [ "$d" = "$(printf "A\x03B")" ] && echo copy-03-PASS || echo copy-03-FAIL
printf "A\x01B" > ./in1.bin; IFS= read -r f < ./in1.bin; [ "$f" = "$(printf "A\x01B")" ] && echo file-01-PASS || echo file-01-FAIL
printf "A\x03B" | { IFS= read -r p; [ "$p" = "$(printf "A\x03B")" ] && echo pipe-03-PASS || echo pipe-03-FAIL; }
"#;
    let (stdout, stderr, status) = rubash_file("copy-file-pipe", script);
    assert_eq!(
        stdout, "copy-01-PASS\ncopy-03-PASS\nfile-01-PASS\npipe-03-PASS\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}

/// Guard for the fix itself: typed U+E319 data must NOT be decoded as a
/// payload. Under a UTF-8 locale `$'\uE31941;x'` is the PUA char plus
/// `41;x` (GNU agrees); the here-string boundary's new payload decode
/// is E400-aware and must leave it alone, direct and via a variable.
#[test]
fn typed_pua_data_is_not_decoded_as_payload_in_utf8_locale() {
    let script = r#"export LC_ALL=C.UTF-8
IFS= read -r l1 <<< $'\uE31941;x'; printf '%s\n' "$l1"
v=$'\uE319'; IFS= read -r l2 <<<"$v""41;x"; printf '%s\n' "$l2"
IFS= read -r l3 <<< $'\uE319'; printf '%s\n' "$l3"
"#;
    let (stdout, stderr, status) = rubash_file("pua", script);
    assert_eq!(
        stdout, "\u{E319}41;x\n\u{E319}41;x\n\u{E319}\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "");
    assert_eq!(status, Some(0));
}
