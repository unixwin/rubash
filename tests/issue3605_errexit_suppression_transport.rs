//! rubash#3605 pins: CMD_IGNORE_RETURN transport across rubash's
//! in-process respawn boundaries (`coproc` / trailing `&` via the
//! `__RUBASH_SUPPRESS_ERREXIT` env stamp — GNU runs those bodies in a
//! real fork where the flag carries by inheritance,
//! execute_cmd.c:1576/:2546), and the `comsub_ignore_return` counter
//! (execute_cmd.c:4616 raises it around an exempt command's
//! expand_words; builtins/evalstring.c:496 stamps CMD_IGNORE_RETURN on
//! every command a comsub's parse_and_execute yields while it is held).
//!
//! Expectations verified against GNU bash 5.3.20 script-file probes.

use std::process::{Command, Stdio};

fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-3605-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let path = dir.join("s.sh");
    std::fs::write(&path, script).expect("seed script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn coproc_invert_and_if_test_carry_the_exemption() {
    // execute_cmd.c:2546 execute_coproc -> execute_in_subshell: the
    // flagged coproc node's body runs with the CMD_IGNORE_RETURN flag
    // its parse stamped (evalstring.c:496 sees the parent's
    // ignore_return during expansion? no — the flag is on the NODE,
    // travelling into the child command). GNU probes:
    //   `! coproc CP { set -e; false; echo reached-invert; }`
    //   `if coproc CI { set -e; false; echo reached-iftest; }; then ...`
    // both print their marker; a plain coproc does not.
    // The coproc body's stdout is the CP pipe, invisible to the
    // parent — the markers are written to a file instead.
    let script = r#"set -e
! coproc CP { set -e; false; echo invert-body >> marks; }
sleep 0.5
if coproc CI { set -e; false; echo if-body >> marks; }; then echo if-ok; fi
sleep 0.5
cat marks 2>/dev/null
echo tail
"#;
    let (stdout, _, _) = rubash_script(script);
    assert!(
        stdout.contains("invert-body"),
        "invert coproc body suppressed: {stdout:?}"
    );
    assert!(
        stdout.contains("if-body"),
        "if-test coproc body suppressed: {stdout:?}"
    );
    assert!(
        stdout.contains("tail"),
        "top-level errexit wrongly suppressed: {stdout:?}"
    );
}

#[test]
fn plain_coproc_stays_under_errexit() {
    let script = r#"coproc PL { set -e; false; echo dead >> marks; }
sleep 0.5
cat marks 2>/dev/null
echo tail
"#;
    let (stdout, _, _) = rubash_script(script);
    assert!(stdout.contains("tail"), "script died: {stdout:?}");
    assert!(
        !stdout.contains("dead"),
        "plain coproc wrongly exempted: {stdout:?}"
    );
}

#[test]
fn exempt_comsub_inherits_cmd_ignore_return() {
    // execute_cmd.c:4616 + builtins/evalstring.c:496: a comsub spawned
    // during an exempt expansion stamps CMD_IGNORE_RETURN on every
    // body command, so `! x=$(...)`, `if z=$(...)`, `! eval 'w=$(...)'`,
    // `! { v=$(...) }`, and the function-body assignment under `! f`
    // all run through the comsub's own `set -e`.
    let script = r#"! x=$(set -e; false; echo cs); echo "cs:$x"
if z=$(set -e; false; echo ifz); then echo "z:$z"; fi
! eval 'w=$(set -e; false; echo ev); echo "w:$w"'
! { v=$(set -e; false; echo grp); echo "v:$v"; }
f() { x=$(set -e; false; echo fb); }; ! f; echo "f:$x"
"#;
    let (stdout, _, _) = rubash_script(script);
    for want in ["cs:cs", "z:ifz", "w:ev", "v:grp", "f:fb"] {
        assert!(stdout.contains(want), "missing {want}: {stdout:?}");
    }
}

#[test]
fn non_exempt_comsub_and_procsub_stay_dead() {
    // The counter is raised only inside the exempt expansion bracket:
    // a plain `x=$(...)` keeps errexit, and a `< <( )` procsub is
    // expanded outside expand_words (the CMD_IGNORE_RETURN bracket at
    // execute_cmd.c:4616 covers word expansion only), so procsub
    // bodies never see the flag — GNU probe: `! cat < <(set -e;false;
    // echo px)` prints no `px`.
    let script = r#"x=$(set -e; false; echo top); echo "x:$x"
! cat < <(set -e; false; echo px); echo "e:$?"
"#;
    let (stdout, _, _) = rubash_script(script);
    assert!(stdout.contains("x:"), "x line missing: {stdout:?}");
    assert!(
        !stdout.contains("x:top"),
        "plain comsub wrongly exempted: {stdout:?}"
    );
    assert!(
        !stdout.contains("px"),
        "procsub wrongly exempted: {stdout:?}"
    );
    assert!(stdout.contains("e:"), "e line missing: {stdout:?}");
}
