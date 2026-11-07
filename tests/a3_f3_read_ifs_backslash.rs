//! Deep-audit lane A3 finding F3: `read` treated a backslash in IFS as a
//! delimiter, where GNU treats it as an escape introducer (builtins/
//! read.def:823-833 consumes a backslash as an escape before testing the
//! delimiter, so `IFS='\' read a b` on `x\y` binds a="xy", b unset).
//!
//! GNU bash 5.3.20 oracle behavior (script-file probes, twice confirmed):
//! - `IFS='\' read a b` on `x\y` -> a="xy", b unset
//! - `IFS='\' read -r a b` on `x\y` -> a="x", b="y" (raw: backslash IS a
//!   delimiter once escape processing is off)

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
fn read_ifs_backslash_is_escape_not_delimiter() {
    let (stdout, _, code) =
        rubash("printf '%s\\n' 'x\\y' | { IFS='\\' read a b; echo \"a=[$a] b=[$b]\"; }");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a=[xy] b=[]\n");
}

#[test]
fn read_r_ifs_backslash_is_delimiter() {
    let (stdout, _, code) =
        rubash("printf '%s\\n' 'x\\y' | { IFS='\\' read -r a b; echo \"a=[$a] b=[$b]\"; }");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a=[x] b=[y]\n");
}
