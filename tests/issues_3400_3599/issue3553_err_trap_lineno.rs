//! rubash#3553: the ERR trap's `$LINENO` is GNU's
//! `line_number_for_err_trap` (execute_cmd.c:291), a tracker that only
//! the leaf stamps feed — simple :936, subshell :696, arith :3904,
//! cond :4138 — plus the ambient pins at pipeline :2895 and the
//! misc-compound frame :1141. A compound command's OWN redirect
//! failure (execute_cmd.c:854-883) therefore reports the previously
//! stamped line, not the compound's own; inside a function the
//! inherited trap is silent but the call itself reports the CALL line,
//! because execute_function unwind-protects the tracker
//! (execute_cmd.c:5242).

use std::path::PathBuf;
use std::process::Command;

fn run_script(name: &str, script: &str) -> (String, String, Option<i32>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash-issue3553-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create probe directory");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        dir,
    )
}

#[test]
fn issue_script_brace_group_redirect_reports_previous_leaf() {
    let (stdout, _stderr, _code, _dir) = run_script(
        "brace",
        "exec 2>/dev/null\n\
         trap \"echo ERR@\\$LINENO\" ERR\n\
         echo x\n\
         { :; } </bad\n\
         echo y\n\
         echo done\n",
    );
    assert_eq!(stdout, "x\nERR@3\ny\ndone\n", "stdout={stdout:?}");
}

#[test]
fn issue_script_compound_redirect_matrix() {
    // Failing-redirect compounds each fire once, reporting the LAST
    // stamped line: the bare compounds keep the pipeline's line 3
    // stamp, arith/cond pin to the ambient stamp 8, and the subshell
    // reports its own line (execute_cmd.c:696 stamps before the
    // child-side do_redirections).
    let (stdout, _stderr, _code, _dir) = run_script(
        "matrix",
        "exec 2>/dev/null\n\
         trap \"echo ERR@\\$LINENO\" ERR\n\
         false | true\n\
         { :; } </bad\n\
         while :; do break; done </bad\n\
         for i in x; do :; done </bad\n\
         case x in x) :;; esac </bad\n\
         (( 0 )) || echo arith-continue\n\
         (( 1 )) </bad\n\
         [[ x = x ]] </bad\n\
         ( : ) </bad\n\
         echo last\n",
    );
    assert_eq!(
        stdout, "ERR@3\nERR@3\nERR@3\nERR@3\narith-continue\nERR@8\nERR@8\nERR@11\nlast\n",
        "stdout={stdout:?}"
    );
}

#[test]
fn issue_script_conditional_redirect_fires_once() {
    // rubash#2659's marked-conversion fire must record
    // err_trap_fired_at_dispatch so the list loop's own look does not
    // fire the same command's trap a second time.
    let (stdout, _stderr, _code, _dir) = run_script(
        "cond-once",
        "exec 2>/dev/null\n\
         trap \"echo ERR@\\$LINENO\" ERR\n\
         : line3\n\
         [[ x = x ]] </bad\n\
         echo done\n",
    );
    assert_eq!(stdout, "ERR@3\ndone\n", "stdout={stdout:?}");
}

#[test]
fn issue_script_function_frame_restores_tracker() {
    // `{ }` inside f earns no fire (the inherited trap is cleared
    // inside a function without errtrace). The `if` reports the g
    // CALL's line: g's body stamp is unwound at function return
    // (execute_cmd.c:5242). `( : )` inside h fails silently there, but
    // h's failing call reports the call line.
    let (stdout, _stderr, _code, _dir) = run_script(
        "function-frame",
        "exec 2>/dev/null\n\
         trap \"echo ERR@\\$LINENO\" ERR\n\
         f() {\n\
           echo body1\n\
           { :; } </bad\n\
           echo after\n\
         }\n\
         f\n\
         echo sep1\n\
         g() { echo g1; }\n\
         g\n\
         if :; then :; fi </bad\n\
         echo sep2\n\
         until :; do break; done </bad\n\
         echo sep3\n\
         h() { ( : ) </bad; }\n\
         h\n\
         echo sep4\n\
         while :; do break; done </bad; echo tail\n",
    );
    assert_eq!(
        stdout, "body1\nafter\nsep1\ng1\nERR@11\nsep2\nERR@13\nsep3\nERR@17\nsep4\nERR@18\ntail\n",
        "stdout={stdout:?}"
    );
}
