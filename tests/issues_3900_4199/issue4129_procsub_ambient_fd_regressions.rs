//! rubash#4129 regressions: a `>(...)` consumer ignored the fd 1 /
//! fd 2 bindings in effect where the substitution was expanded — its
//! output escaped an enclosing `$( )` capture, a `{ ...; } > file` /
//! `2> file` group redirection and an `exec >` straight to the shell
//! process's own stdout/stderr.
//!
//! Root cause: the consumer spawn took fds 1-2 from the spawn plan
//! (rubash#2195), which only records redirections that precede the
//! substitution in the consuming command's own list. For a
//! plan-untouched fd the `None` arm of `procsub_plan_stdio` fell back
//! to `Stdio::inherit()` — the process standard handles. GNU's
//! process_substitute fork (subst.c:6362) inherits the shell's whole
//! ambient fd table, and rubash's ambient adjudication for that table
//! already existed (`ambient_procsub_spawn_fd`, rubash#2212) but its
//! pass stopped at fd 3. Plan-untouched fds 1/2 now resolve through
//! the same adjudication, and a CaptureWrite verdict registers the
//! consumer as a writer of the capture scope (the rubash#2484
//! registration the fd >= 3 transport already performs). No new
//! mechanism: plan entries still win where present (the prefix-plan
//! shape below), and a top-level consumer still prints to the
//! terminal (the top-level shape below).
//!
//! Every expectation below is byte-verified against the GNU Bash
//! 5.3.20 oracle (script-file batteries; the C.UTF-8 and C locales
//! agree on all of these shapes).

use std::process::Command;

fn rubash_file(script: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-procsub4129-{}-{}",
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

const BATTERY: &str = r#"o=$( : > >(echo out); wait )
printf 'comsub: [%s]\n' "$o"
{ : > >(echo grp); wait; } > g.txt
printf 'group >: [%s]\n' "$(cat g.txt)"
{ : > >(echo err >&2); wait; } 2> e.txt
printf 'group 2>: [%s]\n' "$(cat e.txt)"
o=$( echo hi > >(cat); wait )
printf 'consumer: [%s]\n' "$o"
( exec > x.txt; : > >(echo ex); wait )
printf 'exec >: [%s]\n' "$(cat x.txt)"
: 2>p.txt > >(echo pre >&2); wait
printf 'prefix-plan: [%s]\n' "$(cat p.txt)"
: > >(echo top); wait
o=$( { : > >(echo sub); wait; } ); printf 'subshell-comsub: [%s]\n' "$o"
o=$( echo abc | tee >(cat) >/dev/null; wait ); printf 'tee: [%s]\n' "$o"
o=$( : 2>&1 > >(echo e21 >&2); wait ); printf 'pre21: [%s]\n' "$o"
o=$( cat <(echo in) ); printf 'input: [%s]\n' "$o"
exec 3> f3.txt; : > >(echo v3 >&3); wait; exec 3>&-; printf 'fd3: [%s]\n' "$(cat f3.txt)"
"#;

const BATTERY_EXPECTED: &str = "\
comsub: [out]
group >: [grp]
group 2>: [err]
consumer: [hi]
exec >: [ex]
prefix-plan: [pre]
top
subshell-comsub: [sub]
tee: [abc]
pre21: [e21]
input: [in]
fd3: [v3]
";

#[test]
fn procsub_ambient_fd_battery_utf8_locale() {
    let (stdout, stderr, code) = rubash_file(BATTERY, "C.UTF-8");
    assert_eq!(stdout, BATTERY_EXPECTED, "stdout mismatch");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0), "exit code mismatch");
}

#[test]
fn procsub_ambient_fd_battery_c_locale() {
    let (stdout, stderr, code) = rubash_file(BATTERY, "C");
    assert_eq!(stdout, BATTERY_EXPECTED, "stdout mismatch");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0), "exit code mismatch");
}
