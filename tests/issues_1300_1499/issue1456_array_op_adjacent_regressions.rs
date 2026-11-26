//! rubash#1456 — `"${arr[@] op}"` adjacent to literal text collapsed
//! all elements into one space-joined word; GNU applies the operator
//! per element and the adjacent text glues to the first/last element,
//! exactly like the unadjacent form.
//!
//! Probe: clusters/x1/issues/issue-1456.md repro.
//! Expectations verified against GNU bash 5.3.20 (script files).

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
fn array_op_with_adjacent_text_expands_per_element() {
    let (stdout, stderr, code) = rubash(
        "arr=(a.txt b.md)\n\
         printf '<%s>\\n' \"${arr[@]%.txt}.bak\"\n\
         printf '<%s>\\n' \"pre${arr[@]%.txt}\"\n\
         printf '<%s>\\n' \"${arr[@]%.txt}\"",
    );
    assert_eq!(stdout, "<a>\n<b.md.bak>\n<prea>\n<b.md>\n<a>\n<b.md>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
