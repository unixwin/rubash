//! rubash#1168 / rubash#1169 — `set -u` and evaluation order in
//! default-value words.
//!
//! #1168: GNU subst.c — nounset applies INSIDE the word of a `:-`/`-`
//! default: expanding an unset variable there is an unbound-variable
//! error (it is not a "safe" position), in all three shapes below.
//! #1169: GNU subst.c — `${e:-d}` must NOT observe the assignment a
//! LATER `${e:=x}` in the same word performs: the `:-` expansion runs
//! first, while `e` is still empty, and yields `d`.
//!
//! #1168 expectations verified against GNU bash 5.3.20 with SCRIPT
//! FILES (fatal-expansion status is 1 in file mode); #1169 with a
//! `-c` string.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1168-{tag}-{}", std::process::id()));
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
fn issue1168_setu_applies_inside_default_words() {
    for (tag, script) in [
        ("a", "set -u; echo \"${u1:-$u2}\"; echo rc=$?\n"),
        ("b", "set -u; e=\"\"; echo \"${e:-$u2}\"; echo rc=$?\n"),
        ("c", "set -u; echo \"${u1-$u2}\"; echo rc=$?\n"),
    ] {
        let (stdout, stderr, code) = rubash_file(tag, script);
        assert_eq!(stdout, "", "shape {tag}");
        assert!(
            stderr.contains("u2: unbound variable"),
            "shape {tag} stderr: {stderr}"
        );
        assert_eq!(code, Some(1), "shape {tag}");
    }
}

#[test]
fn issue1169_colon_minus_does_not_see_later_assign() {
    let (stdout, _, code) = rubash("e=\"\"; echo \"${e:-d}/${e:=x}/after=$e\"");
    assert_eq!(stdout, "d/x/after=x\n");
    assert_eq!(code, Some(0));
}
