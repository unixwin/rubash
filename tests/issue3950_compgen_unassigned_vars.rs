//! rubash#3950 — `compgen -A arrayvar` / `-A variable` / `-A export` must
//! exclude declared-but-never-assigned variables (`att_invisible` /
//! `DECLARED_UNSET_VARS`), matching GNU Bash 5.3.20:
//! - `pcomplete.c:400 it_init_arrayvars` -> `variables.c:4416 all_array_variables`
//!   (`visible_array_vars`: `invisible_p(var) == 0 && (array_p(var) || assoc_p(var))`)
//! - `pcomplete.c:619 it_init_variables` -> `variables.c:4319 all_visible_variables`
//!   (`visible_var`: `invisible_p(var) == 0`)
//! - `variables.c:4350 all_exported_variables`
//!   (`visible_and_exported`: `invisible_p(var) == 0 && exported_p(var)`)

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue3950_declared_unassigned_arrays_excluded_from_compgen() {
    let (stdout, stderr, code) = rubash(
        "declare -a a1 a2; compgen -A arrayvar a; echo \"rc1=$?\"\n\
         declare -a b1 b2; compgen -A variable b; echo \"rc2=$?\"\n\
         declare -A h1; compgen -A arrayvar h; echo \"rc3=$?\"\n\
         declare -A h2; compgen -A variable h; echo \"rc4=$?\"\n\
         f() { local -a la; compgen -A arrayvar l; echo \"rc5=$?\"; }; f\n\
         declare -x x1; compgen -A export x; echo \"rc6=$?\"\n\
         declare s1; compgen -A variable s1; echo \"rc7=$?\"",
    );
    assert_eq!(
        stdout,
        "rc1=1\nrc2=1\nrc3=1\nrc4=1\nrc5=1\nrc6=1\nrc7=1\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3950_assigned_and_empty_arrays_still_listed() {
    let (stdout, stderr, code) = rubash(
        "declare -a a1=(); compgen -A arrayvar a1; compgen -A variable a1; echo \"rc1=$?\"\n\
         declare -A h1=(); compgen -A arrayvar h1; compgen -A variable h1; echo \"rc2=$?\"\n\
         declare -a a2; a2=(); compgen -A arrayvar a2; echo \"rc3=$?\"\n\
         declare -x x1=; compgen -A export x1; echo \"rc4=$?\"\n\
         s1=x; compgen -A arrayvar s1; echo \"rc5=$?\"",
    );
    assert_eq!(
        stdout,
        "a1\na1\nrc1=0\nh1\nh1\nrc2=0\na2\nrc3=0\nx1\nrc4=0\nrc5=1\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
