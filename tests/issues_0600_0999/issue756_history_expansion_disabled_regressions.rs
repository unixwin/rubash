//! rubash#756 — `set +H` does NOT disable history expansion
//! (catalog case issue-756).
//!
//! GNU parse.y / subst.c: history expansion is an INTERACTIVE-shell
//! feature; in a non-interactive script `!` is an ordinary character
//! regardless of the histexpand setting, and after `set +H` an
//! interactive shell stops expanding it too. The filed catalog case
//! runs the script form: the literal line must pass through untouched
//! and execution must continue.
//!
//! Expectations byte-verified against GNU bash 5.3.20 with a SCRIPT
//! FILE (the regression-catalog case asserts the same line).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash756-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn bang_after_set_plus_h_is_literal_in_scripts() {
    let dir = tempdir("plush");
    std::fs::write(
        dir.join("probe.sh"),
        "set +H\necho \"history expansion disabled: !!\"\necho reached the next line\n",
    )
    .expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("probe.sh")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        stdout,
        "history expansion disabled: !!\nreached the next line\n"
    );
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
