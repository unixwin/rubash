//! rubash#1167 — RANDOM seeding sequence does not change under
//! compat32/44.
//!
//! GNU variables.c: the RANDOM generator's seeding follows the
//! compatibility level — compat32 and compat44 select the legacy
//! seeding (identical to each other), while the default level uses
//! the current seeding. The sequences are deterministic per seed.
//!
//! All expectations byte-verified against GNU bash 5.3.20 (`-c`
//! string), seed 42, two draws per level.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn random_seeding_follows_compat_level() {
    let (stdout, _, code) = rubash(
        "shopt -s compat32; RANDOM=42; echo $RANDOM $RANDOM\n\
         shopt -s compat44; RANDOM=42; echo $RANDOM $RANDOM\n\
         shopt -u compat32 compat44; RANDOM=42; echo $RANDOM $RANDOM",
    );
    assert_eq!(stdout, "17766 11151\n17766 11151\n17772 26794\n");
    assert_eq!(code, Some(0));
}
