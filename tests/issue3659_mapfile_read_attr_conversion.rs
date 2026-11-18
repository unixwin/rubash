//! rubash#3659 regressions: `mapfile` (and `read -a`) storing into a
//! `declare -i` / `-l` / `-u` array skipped attribute conversion, storing
//! the raw strings verbatim. GNU applies the array's attributes on every
//! element store (make_variable_value, variables.c:2905, reached from
//! bind_array_var_internal via arrayfunc.c:166 and from
//! assign_array_var_from_word_list in read.def:976): `-i` evaluates the
//! value arithmetically (`010` -> `8`, `2+3` -> `5`, `0x10` -> `16`,
//! empty/non-numeric -> `0`), `-l`/`-u` fold case.
//!
//! The seed is `seed_r123a.sh` from the issue (10 checks); checks 9-10 are
//! the already-green controls (direct element assignment, scalar read).
//! Checks 11-12 extend the same conversion to the sibling faces
//! (`read -a -u`, `mapfile -c`).
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3659-{}-{}",
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
fn mapfile_integer_octal() {
    let (stdout, _, code) =
        rubash_file("declare -i a; mapfile -t a < <(printf \"010\\n\"); declare -p a\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -ai a=([0]=\"8\")\n");
}

#[test]
fn mapfile_integer_expression() {
    let (stdout, _, code) =
        rubash_file("declare -i a; mapfile -t a < <(printf \"2+3\\n\"); declare -p a\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -ai a=([0]=\"5\")\n");
}

#[test]
fn mapfile_integer_hex() {
    let (stdout, _, code) =
        rubash_file("declare -i a; mapfile -t a < <(printf \"0x10\\n\"); declare -p a\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -ai a=([0]=\"16\")\n");
}

#[test]
fn mapfile_integer_empty_line() {
    let (stdout, _, code) =
        rubash_file("declare -i a; mapfile -t a < <(printf \"\\n\"); declare -p a\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -ai a=([0]=\"0\")\n");
}

#[test]
fn mapfile_integer_nonnumeric() {
    let (stdout, _, code) =
        rubash_file("declare -i a; mapfile -t a < <(printf \"abc\\n\"); declare -p a\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -ai a=([0]=\"0\")\n");
}

#[test]
fn mapfile_lowercase() {
    let (stdout, _, code) =
        rubash_file("declare -l a; mapfile -t a < <(printf \"AbC\\nXyZ\\n\"); declare -p a\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -al a=([0]=\"abc\" [1]=\"xyz\")\n");
}

#[test]
fn mapfile_uppercase() {
    let (stdout, _, code) =
        rubash_file("declare -u a; mapfile -t a < <(printf \"AbC\\nXyZ\\n\"); declare -p a\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -au a=([0]=\"ABC\" [1]=\"XYZ\")\n");
}

#[test]
fn read_array_integer() {
    let (stdout, _, code) = rubash_file("declare -i a; read -a a <<< \"010 2+3\"; declare -p a\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -ai a=([0]=\"8\" [1]=\"5\")\n");
}

#[test]
fn direct_assignment_integer_control() {
    let (stdout, _, code) = rubash_file("declare -i a; a[0]=010; a[1]=\"2+3\"; declare -p a\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -ai a=([0]=\"8\" [1]=\"5\")\n");
}

#[test]
fn scalar_read_integer_control() {
    let (stdout, _, code) = rubash_file("declare -i x; read x <<< \"010\"; declare -p x\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -i x=\"8\"\n");
}

#[test]
fn read_array_uppercase() {
    let (stdout, _, code) = rubash_file("declare -u a; read -a a <<< \"aBc dEf\"; declare -p a\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -au a=([0]=\"ABC\" [1]=\"DEF\")\n");
}

#[test]
fn mapfile_capcase() {
    let (stdout, _, code) =
        rubash_file("declare -c a; mapfile -t a < <(printf \"hello\\nWORLD\\n\"); declare -p a\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -ac a=([0]=\"Hello\" [1]=\"World\")\n");
}
