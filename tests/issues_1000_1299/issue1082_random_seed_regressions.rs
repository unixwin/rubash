//! rubash#1082 — RANDOM invalid-seed fallback differs; unset-then-assign
//! stays empty.
//!
//! GNU variables.c (make_variable / bind_variable for RANDOM): assigning
//! a non-numeric seed still seeds the generator (the value goes through
//! the same numeric conversion as a valid seed), so draws are numeric
//! and successive draws differ. After `unset RANDOM` the name loses its
//! specialness: a later `RANDOM=42` creates a PLAIN variable whose value
//! reads back verbatim (`[42] [42]`), it does not reseed and it is not
//! empty.
//!
//! All expectations byte-verified against GNU bash 5.3.20 (`-c` strings);
//! seeded draws are deterministic, so the literals below are stable.

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
fn random_seed_forms_match_gnu() {
    let (stdout, stderr, code) = rubash(
        "RANDOM=abc; echo $RANDOM; echo $RANDOM\n\
         RANDOM=42; echo $RANDOM\n\
         unset RANDOM; RANDOM=42; echo \"[$RANDOM] [$RANDOM]\"",
    );
    // Invalid seed: generator still seeded -> two distinct numeric draws
    // (deterministic for this seed). Valid seed 42: first draw 17772.
    // unset-then-assign: plain variable, reads back literally.
    assert_eq!(stdout, "20814\n24386\n17772\n[42] [42]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
