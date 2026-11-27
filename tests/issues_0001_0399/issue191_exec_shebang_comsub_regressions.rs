//! rubash#191 — characterization pin (closed unreproducible).
//!
//! The filed report claimed output loss when a shebang script that
//! `exec`s another program is captured through command substitution.
//! The maintainer's close-out ran 15+ probe forms byte-identical to
//! GNU; the rbenv-shaped relative of the report is covered by
//! rubash#193's fix. This pin holds the canonical form so the
//! question stays answered: the exec'd program's stdout must arrive
//! intact inside `$()`.
//!
//! Expectations byte-verified against GNU bash 5.3.20 with script
//! files.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash191-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn exec_in_shebang_script_captured_by_comsub() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir("execer");
    let execer = dir.join("execer.sh");
    std::fs::write(&execer, "#!/bin/sh\nexec printf 'INNER\\n'\n").expect("write execer");
    std::fs::set_permissions(&execer, std::fs::Permissions::from_mode(0o755))
        .expect("chmod execer");
    std::fs::write(
        dir.join("probe.sh"),
        "out=$(./execer.sh); echo \"[$out]\"\n",
    )
    .expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("probe.sh")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout, "[INNER]\n");
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
