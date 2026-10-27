//! rubash#1842: a command substitution used DIRECTLY as a here-string
//! word (`<<<"$(<file)"`) staged the substitution output's backslash
//! bytes as raw 0x15 (PROTECTED_BACKSLASH) into the command's stdin.
//! Root: the substitution splice inside the expansion walker protects
//! output backslashes as 0x15 so later word passes cannot reinterpret
//! them; every final-value boundary must restore them (marker
//! registry: PROTECTED_BACKSLASH/DATA_BACKSLASH are Boundary::Output).
//! The heredoc boundary restores them via
//! restore_command_substitution_output (expand_heredoc_body_mut), but
//! expand_here_string_mut's final decode
//! (decode_word_position_carriers) restored only the quote/dollar
//! carriers. GNU: redir.c:373-374 heredoc_expand runs the word through
//! expand_assignment_string_to_string and heredoc_write emits the
//! result verbatim; subst.c:7143 command_substitute yields the
//! child's raw stdout bytes, never re-interpreted.
//!
//! Oracle: GNU bash 5.3.20, byte-for-byte. Assertions read the raw
//! stdout bytes of the shell process — never a $(cat) readback, whose
//! carrier channel self-decodes and would mask the leak (the bats
//! chain's setup_file false green in the pipe-fd cluster REPORT).
//! Downstream: this was the remaining blocker of the rubash#1720
//! bats chain (bats-preprocess output had every backslash turned
//! into 0x15, splitting teardown continuation lines).

#![cfg(unix)]

use std::process::{Command, Stdio};

struct ScratchDir(std::path::PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch_tempdir() -> ScratchDir {
    let dir = std::env::temp_dir().join(format!(
        "rubash-scratch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    ScratchDir(dir)
}

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

/// File bytes the bats-preprocess shape carries: literal backslash
/// sequences plus a trailing continuation backslash.
const INPUT_BYTES: &[u8] = b"setup_file:chain\\n\nsecond\\tline\nlast\\\n";

/// What a `while IFS= read -r` loop re-emits for INPUT_BYTES: the
/// slurp strips the file's final newline, the here-string adds one
/// back, and read prints each line verbatim (backslashes are data).
const LOOP_EMITTED: &[u8] = b"setup_file:chain\\n\nsecond\\tline\nlast\\\n";

fn run_script(script: &str) -> Vec<u8> {
    let dir = scratch_tempdir();
    std::fs::write(dir.0.join("in.txt"), INPUT_BYTES).expect("write input");
    let script_path = dir.0.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&script_path)
        .current_dir(&dir.0)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn issue1842_comsub_slurp_direct_quoted_herestring_keeps_backslash_bytes() {
    let out = run_script(
        "while IFS= read -r line; do printf '%s\\n' \"$line\"; done <<<\"$(<in.txt)\"\n",
    );
    assert_eq!(out, LOOP_EMITTED);
}

#[test]
fn issue1842_comsub_slurp_direct_unquoted_herestring_keeps_backslash_bytes() {
    let out =
        run_script("while IFS= read -r line; do printf '%s\\n' \"$line\"; done <<<$(<in.txt)\n");
    assert_eq!(out, LOOP_EMITTED);
}

#[test]
fn issue1842_comsub_command_direct_herestring_keeps_backslash_bytes() {
    // Not just the $(<file) slurp fast path: a real command's output
    // spliced straight into the here-string word.
    let out = run_script(
        "while IFS= read -r line; do printf '%s\\n' \"$line\"; done <<<\"$(cat in.txt)\"\n",
    );
    assert_eq!(out, LOOP_EMITTED);
}

#[test]
fn issue1842_comsub_direct_herestring_to_external_consumer_keeps_backslashes() {
    // The staged bytes themselves, read by an external consumer
    // (cat), not the in-process read mirror.
    let out = run_script("cat <<<\"$(<in.txt)\"\n");
    assert_eq!(out, LOOP_EMITTED);
}

#[test]
fn issue1842_variable_mediated_herestring_control() {
    // Control shape (was already clean): the substitution result is
    // materialized into a variable first, then expanded into the
    // here-string word.
    let out = run_script(
        "x=$(<in.txt)\nwhile IFS= read -r line; do printf '%s\\n' \"$line\"; done <<<\"$x\"\n",
    );
    assert_eq!(out, LOOP_EMITTED);
}

#[test]
fn issue1842_literal_herestring_control() {
    // Control shape (was already clean): a literal word whose
    // backslash comes from source quoting, not from a substitution.
    let out = run_script("while IFS= read -r line; do printf '%s\\n' \"$line\"; done <<<'a\\b'\n");
    assert_eq!(out, b"a\\b\n");
}

#[test]
fn issue1842_comsub_raw_0x15_output_byte_is_data_not_restored() {
    // A raw 0x15 byte in the substitution's OUTPUT is user data: the
    // restore must only claim the transport encoding, never invent a
    // backslash out of a data byte (the unicode1.sub failure mode).
    let out = run_script(
        "while IFS= read -r line; do printf '%s\\n' \"$line\"; done <<<\"$(printf 'w\\025z')\"\n",
    );
    assert_eq!(out, b"w\x15z\n");
}

#[test]
fn issue1842_user_typed_0x15_ansi_c_literal_stays_0x15() {
    // A user-typed $'\x15' in the word itself is likewise data.
    let out =
        run_script("while IFS= read -r line; do printf '%s\\n' \"$line\"; done <<<$'u\\x15v'\n");
    assert_eq!(out, b"u\x15v\n");
}
