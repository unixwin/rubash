//! rubash#2454 — the declare-door DIRSTACK bad-subscript diagnostic
//! must spell the subscript as written, not as evaluated.
//!
//! GNU: err_badarraysub (error.c:440-443) prints the string its caller
//! hands it, and the assignment path always hands it the full operand
//! text as written — assign_array_element (arrayfunc.c:334) carries
//! `name` through assign_array_element_internal, whose indexed arm
//! calls err_badarraysub (name) at arrayfunc.c:431 after the negative
//! wrap still leaves the index negative. The evaluated integer never
//! appears in the diagnostic. The bind itself lands in
//! variables.c:1612 assign_dirstack, as with the plain door.
//!
//! rubash's declare router (the #2424/#2432 stack-setter door) runs on
//! operands whose subscripts the rewrite pass has already evaluated,
//! so `declare DIRSTACK[abc]=x` with abc=-5 reported `DIRSTACK[-5]`.
//! The router now recovers the spelling from the pre-rewrite operand
//! at the same position. The #2432 pins used literal subscripts
//! (written == evaluated), which is why this slipped through.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle with
//! script-file probes; the scratch path is normalized to WORK and the
//! script path to SCRIPT, mirroring tests/issue2432_*.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2454-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for sub in ["a", "b"] {
        fs::create_dir_all(dir.join(sub)).expect("create scratch subdir");
    }
    dir
}

/// Run `script` as a script FILE whose $1 is the scratch dir; return
/// (stdout, stderr) with the scratch path normalized to WORK and the
/// script path normalized to SCRIPT.
fn run_script(dir: &Path, script: &str) -> (String, String) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .arg(dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    let normalize = |text: &str| {
        text.replace(&script_path.to_string_lossy().into_owned(), "SCRIPT")
            .replace(&dir.to_string_lossy().into_owned(), "WORK")
    };
    (
        normalize(&String::from_utf8_lossy(&output.stdout)),
        normalize(&String::from_utf8_lossy(&output.stderr)),
    )
}

#[test]
fn issue2454_declare_door_prints_written_subscript() {
    // The issue probe (abc=-5) across the declare spellings of the
    // door: plain declare, typeset, an arithmetic spelling, and the
    // append form. Every diagnostic names the written subscript.
    let dir = scratch("written");
    let (out, err) = run_script(
        &dir,
        r#"abc=-5
declare DIRSTACK[abc]=x; echo "declare rc=$?"
typeset DIRSTACK[abc]=x; echo "typeset rc=$?"
declare DIRSTACK[1+abc]=x; echo "expr rc=$?"
declare DIRSTACK[abc]+=x; echo "append rc=$?"
echo done
"#,
    );
    assert_eq!(
        out,
        "declare rc=1\ntypeset rc=1\nexpr rc=1\nappend rc=1\ndone\n"
    );
    assert_eq!(
        err,
        "SCRIPT: line 2: DIRSTACK[abc]: bad array subscript\n\
         SCRIPT: line 3: DIRSTACK[abc]: bad array subscript\n\
         SCRIPT: line 4: DIRSTACK[1+abc]: bad array subscript\n\
         SCRIPT: line 5: DIRSTACK[abc]: bad array subscript\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2454_multi_operand_and_global_declare_in_function() {
    // An earlier operand still binds (its slot-0 write is dropped by
    // the setter, so the top stays the cwd), the bad operand reports
    // its written spelling, and `declare -g` inside a function takes
    // the same door with the same spelling.
    let dir = scratch("multi");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1/a" || exit 1
abc=-5
declare DIRSTACK[0]=ok DIRSTACK[abc]=x; echo "multi rc=$? top=${DIRSTACK[0]}"
f() { declare -g DIRSTACK[abc]=x; echo "declg rc=$?"; }
f
echo done
"#,
    );
    assert_eq!(out, "multi rc=1 top=WORK/a\ndeclg rc=1\ndone\n");
    assert_eq!(
        err,
        "SCRIPT: line 3: DIRSTACK[abc]: bad array subscript\n\
         SCRIPT: line 4: DIRSTACK[abc]: bad array subscript\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2454_quoted_expression_subscripts_keep_written_text() {
    // A quoted operand reaches the builtin as one word; its subscript
    // text (spaces, operators) is what GNU prints, byte for byte.
    let dir = scratch("expr");
    let (out, err) = run_script(
        &dir,
        r#"abc=-5
declare 'DIRSTACK[abc + 1]=x'; echo "spaces rc=$?"
declare 'DIRSTACK[abc*2]=x'; echo "mul rc=$?"
echo done
"#,
    );
    assert_eq!(out, "spaces rc=1\nmul rc=1\ndone\n");
    assert_eq!(
        err,
        "SCRIPT: line 2: DIRSTACK[abc + 1]: bad array subscript\n\
         SCRIPT: line 3: DIRSTACK[abc*2]: bad array subscript\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2454_literal_and_special_subscripts_unchanged() {
    // Shapes whose written and evaluated texts coincide (a literal
    // negative, the all-elements spellings, the empty subscript) keep
    // their #2432 behaviour through the same door.
    let dir = scratch("special");
    let (out, err) = run_script(
        &dir,
        r#"abc=-5
declare DIRSTACK[-9]=x; echo "lit rc=$?"
declare 'DIRSTACK[@]=x'; echo "at rc=$?"
declare 'DIRSTACK[]=x'; echo "empty rc=$?"
echo done
"#,
    );
    assert_eq!(out, "lit rc=1\nat rc=1\nempty rc=1\ndone\n");
    assert_eq!(
        err,
        "SCRIPT: line 2: DIRSTACK[-9]: bad array subscript\n\
         SCRIPT: line 3: DIRSTACK[@]: bad array subscript\n\
         SCRIPT: line 4: DIRSTACK[]: bad array subscript\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2454_in_range_negative_and_plain_door_unchanged() {
    // Control: an in-range evaluated subscript still binds through the
    // door (abc=-1 resolves to the last stack element), and the plain
    // door keeps its written spelling and its discard-the-line class.
    let dir = scratch("control");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1/a" || exit 1
pushd "$1/b" >/dev/null
abc=-1
declare DIRSTACK[abc]=VIA; echo "neg rc=$? all=${DIRSTACK[@]}"
DIRSTACK[-9]=x
echo "after plain"
"#,
    );
    assert_eq!(out, "neg rc=0 all=WORK/b VIA\nafter plain\n");
    assert_eq!(err, "SCRIPT: line 5: DIRSTACK[-9]: bad array subscript\n");
    let _ = fs::remove_dir_all(&dir);
}
