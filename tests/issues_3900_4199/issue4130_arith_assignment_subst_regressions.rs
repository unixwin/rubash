//! rubash#4130 regressions: an assignment RHS that is a `$(( ... ))` /
//! `$[ ... ]` arithmetic unit containing a nested `$(...)` was stored as
//! unevaluated text whenever any quote appeared anywhere in the RHS
//! (including inside the nested substitution) or any text sat next to
//! the unit.
//!
//! Root cause, two halves of one invariant — quotes and substitutions
//! inside an arithmetic unit belong to the unit (GNU subst.c:10862
//! chk_arithsub -> expand_arith_string -> evalexp evaluates the whole
//! expression once):
//!
//! * `assignment_value_is_quoted` counted quotes inside `$((...))` /
//!   `$[...]` units, so `x=$(( $(echo "1") + 1 ))` was classified as a
//!   quoted RHS and never reached the lone-arithmetic fast path. The
//!   classifier now skips a whole top-level arithmetic unit with the
//!   lexer's corrected span scanners, like it already skipped `${...}`
//!   bodies.
//! * `expand_mixed_command_substitution_assignment` split out every
//!   `$(...)` span `scan_substitution_spans` reported, including spans
//!   nested inside a top-level arithmetic unit, leaving `$(( ` and
//!   ` + 1 ))` as plain text around the substitution output. Spans
//!   inside a top-level arithmetic region are now left to the embedded
//!   walker, exactly like spans inside a top-level `${...}` region
//!   (rubash#150); the region extents are measured by the embedded
//!   walker's own extraction collectors.
//!
//! Every expectation below is byte-verified against the GNU Bash
//! 5.3.20 oracle (script-file batteries; the C.UTF-8 and C locales
//! agree on all of these shapes).

use std::process::Command;

fn rubash_file(script: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-arith4130-{}-{}",
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
        .env("LC_ALL", lc_all)
        .env("HOME", "/home/rubash-pin")
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

const BATTERY: &str = r#"x=$(( $(echo "1") + 1 )); echo "[$x]"
x="$(( $(echo 1) + 1 ))"; echo "[$x]"
x=$(( $(echo 1) + 1 ))b; echo "[$x]"
x=0; x+=$(( $(echo "1") + 1 )); echo "[$x]"
x=$[ $(echo "1") + 1 ]; echo "[$x]"
x=$(( $(echo '1') + 1 )); echo "[$x]"
x=$[ $(echo 1) + 1 ]; echo "[$x]"
y=abcd; x=$(( ${#y} + $(echo 1) )); echo "[$x]"
x=$(( $(echo 1) ? 10 : 20 )); echo "[$x]"
x=$(( 0x$(echo "a") + 1 )); echo "[$x]"
x=$(( $(echo 1) + $(echo 2) )); echo "[$x]"
x=$(( $(echo 2) + 1 ))$(echo ")"); echo "[$x]"
unset y; x=${y:-$(( $(echo "3") + 1 ))}; echo "[$x]"
x=~/$(( $(echo "1") + 1 )); echo "[$x]"
x='$(( $(echo 1) + 1 ))'; echo "[$x]"
t=0; for f in a b c; do t=$((t+$(echo "1"))); done; echo "[$t]"
f() { echo "fn:[$x]"; }; x=$(( $(echo "1") + 1 )) f
g() { local lx=$(( $(echo "1") + 1 )); declare dx=$(( $(echo "1") + 2 )); echo "loc:[$lx][$dx]"; }; g
a[0]=$(( $(echo "1") + 1 )); echo "arr:[${a[0]}]"
export ex=$(( $(echo "1") + 1 )); echo "exp:[$ex]"
echo "arg:$(( $(echo "1") + 1 ))"
x=$(echo "a b"); echo "ctl1:[$x]"
x="pre $(echo "q") post"; echo "ctl2:[$x]"
x=$(( $(echo 1) + 1 )); echo "ctl3:[$x]"
"#;

const BATTERY_EXPECTED: &str = "\
[2]
[2]
[2b]
[02]
[2]
[2]
[2]
[5]
[10]
[11]
[3]
[3)]
[4]
[/home/rubash-pin/2]
[$(( $(echo 1) + 1 ))]
[3]
fn:[2]
loc:[2][3]
arr:[2]
exp:[2]
arg:2
ctl1:[a b]
ctl2:[pre q post]
ctl3:[2]
";

#[test]
fn arith_assignment_battery_utf8_locale() {
    let (stdout, stderr, code) = rubash_file(BATTERY, "C.UTF-8");
    assert_eq!(stdout, BATTERY_EXPECTED, "stdout mismatch");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0), "exit code mismatch");
}

#[test]
fn arith_assignment_battery_c_locale() {
    let (stdout, stderr, code) = rubash_file(BATTERY, "C");
    assert_eq!(stdout, BATTERY_EXPECTED, "stdout mismatch");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0), "exit code mismatch");
}
