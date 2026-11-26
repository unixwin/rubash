//! rubash#1658 — `=~` accepted the ERE-invalid empty-minimum
//! interval `a{,2}`, silently matching as if it were `{0,2}`. POSIX
//! ERE requires the leading number; the GNU oracle this was filed
//! against rejects the pattern at runtime (status 2, "invalid
//! repetition count(s)"). glibc regcomp accepts `{,n}` as an
//! extension — the strict reading is the filed expectation and is
//! what these pins assert.
//!
//! Expectations verified against the issue's GNU bash 5.3.20 repro
//! (script files); the valid-interval and escaped-literal controls
//! agree on both regcomp readings.

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
fn empty_min_interval_is_a_runtime_regex_error() {
    let (stdout, stderr, code) = rubash(
        "[[ a =~ a{,2} ]]; echo \"rc=$? rem=[${BASH_REMATCH[0]-U}]\"\n\
         [[ aaa =~ a{,2} ]]; echo \"rc2=$?\"",
    );
    assert_eq!(stdout, "rc=2 rem=[U]\nrc2=2\n");
    assert!(
        stderr.contains("invalid regular expression `a{,2}': invalid repetition count(s)"),
        "stderr was: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn valid_intervals_and_escaped_literals_still_work() {
    let (stdout, stderr, code) = rubash(
        "[[ aaa =~ a{0,2} ]]; echo \"ok1=$? m=[${BASH_REMATCH[0]}]\"\n\
         [[ aaa =~ a{2,} ]]; echo \"ok2=$? m=[${BASH_REMATCH[0]}]\"\n\
         [[ 'a{,2}' =~ a\\{,2\\} ]]; echo \"ok3=$? m=[${BASH_REMATCH[0]}]\"\n\
         [[ a =~ a\\{,2\\} ]]; echo \"lit=$?\"\n\
         [[ x =~ {,2} ]]; echo \"e2=$?\"",
    );
    assert_eq!(
        stdout,
        "ok1=0 m=[aa]\nok2=0 m=[aaa]\nok3=0 m=[a{,2}]\nlit=1\ne2=2\n"
    );
    assert!(
        stderr.contains("invalid regular expression `{,2}': Invalid preceding regular expression"),
        "stderr was: {stderr}"
    );
    assert_eq!(code, Some(0));
}
