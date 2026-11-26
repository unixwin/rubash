//! rubash#201 — Alias expansion is a PARSE-time act; diagnostics
//! name the word that failed, at its own line.
//!
//! GNU parse.y (alias expansion happens while reading a complete
//! command): inside a `${ ...; }` nofork substitution, the whole
//! body is parsed before anything runs, so an alias defined at the
//! top of the script but only ENABLED mid-body (`shopt expand_aliases`
//! — here without -s, so it merely PRINTS the setting) is not in
//! effect for the `p` calls in the loop: both iterations report
//! `p: command not found`, citing line 5 (the loop line). And a
//! `select` whose variable word expands to `a b` reports
//! `` `$1': not a valid identifier ``, rc=1.
//!
//! Expectations byte-verified against GNU bash 5.3.20 with SCRIPT
//! FILES (the filed probes verbatim).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash201-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("probe.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("probe.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn alias_enabled_mid_body_does_not_apply_to_parsed_body() {
    let (stdout, stderr, code) = rubash_file(
        "nofork",
        "alias p=printf\n\
         alias e='echo aliasval'\n\
         echo \"${ typeset x;\n\
         \tfor f in 1 2; do p '%s\\n' $f ; shopt expand_aliases; done\n\
         \tunalias p\n\
         \talias e='echo inside redefine'\n\
         \tx=42 ; return; echo gone; }\"\n\
         echo done\n",
    );
    assert_eq!(
        stdout,
        "expand_aliases      \toff\nexpand_aliases      \toff\ndone\n"
    );
    assert_eq!(
        stderr.matches("line 5: p: command not found").count(),
        2,
        "both loop iterations must name `p` at line 5; stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn select_with_multiword_variable_is_invalid_identifier() {
    let (stdout, stderr, code) = rubash_file(
        "select",
        "bad-select()\n\
         {\n\
         \tselect $1 in a b c ; do echo $REPLY ; done\n\
         }\n\
         bad-select 'a b'\n",
    );
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("line 2: `$1': not a valid identifier"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
}
