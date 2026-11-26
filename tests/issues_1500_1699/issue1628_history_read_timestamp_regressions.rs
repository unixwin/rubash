//! rubash#1628 — history entries loaded from a file without `#<epoch>`
//! timestamp lines displayed as 1970-01-01 under HISTTIMEFORMAT. GNU:
//! the histfile read path adds entries through add_history, whose
//! hist_inittime stamp is the moment of the add, so an untimestamped
//! line displays the time it was read; a `#<epoch>` line still supplies
//! the following entry's exact stamp (add_history_time).
//!
//! Expectations verified against GNU bash 5.3.20 (script file, with
//! HISTTIMEFORMAT armed before the read).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1628-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

const SCRIPT: &str = "\
export HOME=$PWD/fakehome; mkdir -p \"$HOME\"
printf 'echo one\\necho two\\n' > plain.txt
printf '#1700000000\\necho stamped\\n' > stamped.txt
printf 'echo three\\n' > plain2.txt
HISTTIMEFORMAT='[%s] '
HISTFILE=$PWD/plain.txt
history -r
history
HISTFILE=$PWD/stamped.txt
history -c
history -r
history
HISTFILE=$PWD/plain2.txt
history -n
history
";

fn bracket_epochs(stdout: &str) -> Vec<i64> {
    let mut epochs = Vec::new();
    let mut rest = stdout;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else { break };
        if let Ok(value) = after[..close].parse::<i64>() {
            epochs.push(value);
        }
        rest = &after[close + 1..];
    }
    epochs
}

#[test]
fn untimestamped_entries_display_read_time() {
    let dir = tempdir("ts");
    let script = dir.join("probe.sh");
    std::fs::write(&script, SCRIPT).expect("write script");
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("[1700000000] echo stamped"),
        "explicit stamp must survive verbatim; stdout: {stdout}"
    );
    let epochs = bracket_epochs(&stdout);
    // Two plain reads + one stamped read + two in the final listing.
    assert_eq!(epochs.len(), 5, "stdout: {stdout}");
    for epoch in &epochs {
        assert_ne!(*epoch, 0, "stamp 0 is the 1970 bug; stdout: {stdout}");
        if *epoch != 1_700_000_000 {
            assert!(
                (before - 30..=after + 30).contains(epoch),
                "untimestamped entry must carry read time (~{before}..{after}), got {epoch}; stdout: {stdout}"
            );
        }
    }
}
