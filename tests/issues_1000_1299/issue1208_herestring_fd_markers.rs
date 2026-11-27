//! rubash#1208 regressions: `exec N<<<word` staged the here-string word's
//! transport text on the descriptor — ANSI_C_IFS_GUARD (U+E401) pairs
//! around decoded `$'...'` blanks and DATA_* carriers leaked into every
//! `mapfile -u N` / `read -u N` element. The fd byte channel has no later
//! marker boundary, so the word-position carriers now resolve at the
//! staging entry like the fd-0 here-string path
//! (expand_here_string_mut's decode tail).
//!
//! Expectations verified byte-for-byte against WSL GNU Bash 5.3.20.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1208-{}-{}",
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

#[test]
fn mapfile_herestring_fd_has_no_marker_residue() {
    let (stdout, _, code) = rubash_file(
        "exec 3<<< $'p\\nq\\n'\nmapfile -t -u 3 arr; echo \"n=${#arr[@]}\"; echo \"[${arr[0]}]\" | od -An -c | head -1\nexec 3<&-\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=3\n   [   p   ]  \\n\n");
}

#[test]
fn read_herestring_fd_has_no_marker_residue() {
    let (stdout, _, code) = rubash_file(
        "exec 3<<< $'a\\tb'\nread -u 3 line; echo \"[$line]\" | od -An -c | head -1\nexec 3<&-\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "   [   a  \\t   b   ]  \\n\n");
}

#[test]
fn herestring_fd_plain_word_unchanged() {
    let (stdout, _, code) = rubash_file(
        "exec 3<<< 'plain'\nmapfile -t -u 3 arr; echo \"n=${#arr[@]} [${arr[0]}]\"\nexec 3<&-\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=1 [plain]\n");
}
