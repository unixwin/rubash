//! rubash#1284 — `printf '%(fmt)T'`: a second batch of strftime specs
//! (%c %C %D %g %j %n %P %R %t %U %w %W %+) passed through literally
//! instead of being formatted.
//!
//! GNU: every strftime(3) conversion is honored by the %(...)T format.
//! Probe: clusters/k5/repro/r1284.sh (epoch 0, TZ=UTC).
//! Expectations byte-verified against GNU bash 5.3.20 (script files).

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
fn printf_time_second_batch_specs_are_formatted() {
    let (stdout, stderr, code) = rubash(
        "TZ=UTC printf '%(%D %R %j %U %w)T\\n' 0\n\
         TZ=UTC printf '%(%c|%C|%g|%n|%P|%t|%W)T\\n' 0",
    );
    assert_eq!(
        stdout,
        "01/01/70 00:00 001 00 4\nThu Jan  1 00:00:00 1970|19|70|\n|am|\t|00\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
