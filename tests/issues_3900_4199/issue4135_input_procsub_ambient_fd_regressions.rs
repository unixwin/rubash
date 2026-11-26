//! rubash#4135 regressions: a `<(...)` producer ignored the fd 0 /
//! fd 2 bindings in effect where the substitution was expanded —
//! its stderr escaped a `{ ...; } 2> file` group redirection, a
//! `$( ... 2>&1 )` capture, an `f 2> file` function-call redirection
//! and an `exec 2>`, straight to the shell process's own stderr, and
//! its stdin was the process's own stdin instead of an enclosing
//! `< file` binding.
//!
//! Root cause: `spawn_input_process_substitution_child` set the
//! child's stdin/stderr to `Stdio::inherit()` unconditionally. GNU's
//! process_substitute fork (subst.c:6412 make_child) inherits the
//! expansion-time descriptor table and replaces only fd 1 with the
//! pipe (subst.c:6557). The ambient adjudication for that table
//! already exists (`ambient_procsub_spawn_fd`, rubash#2212) and the
//! consumer side routes its plan-untouched fds 1/2 through it since
//! rubash#4129; the producer's fd 0 / fd 2 now resolve through the
//! same adjudication and the same stdio conversion
//! (`procsub_plan_stdio`), and a CaptureWrite verdict on fd 2
//! registers the producer as a writer of the capture scope
//! (rubash#2484). No new mechanism: an `Inherit` verdict keeps
//! `Stdio::inherit()`, so here-strings, heredocs and pipes on the
//! enclosing fd 0 / fd 2 behave exactly as before.
//!
//! One ordering guard rides along (the reporter's pitfall): when a
//! `< <(...)` redirect target is bound by a compound command's own
//! binder pass, the producer must see the PRE-LIST state of any fd
//! that list itself rebinds — GNU forks the producer at the entry's
//! own turn (redir.c do_redirections), before later entries apply.
//! The binder call sites pass the list's undo saves
//! (compound_binder_saves) to the spawn, and a saved fd resolves
//! through its save slot instead of the post-binder table entry.
//! The redirect-first shapes (the redirection precedes the `<`
//! entry and GNU applies it first) are out of scope for #4135 and
//! keep their pre-fix behavior.
//!
//! Every expectation below is byte-verified against the GNU Bash
//! 5.3.20 oracle (script-file batteries; the C.UTF-8 and C locales
//! agree on all of these shapes).

use std::process::{Command, Stdio};

fn rubash_file(script: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-procsub4135-{}-{}",
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
        .stdin(Stdio::null())
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

const BATTERY: &str = r#"{ cat <(echo err1 >&2); } 2> e.txt; printf 'group 2>: [%s]\n' "$(cat e.txt)"
o=$( { cat <(echo err2 >&2); } 2>&1 ); printf 'comsub 2>&1: [%s]\n' "$o"
{ while read -r l; do :; done < <(echo err3 >&2); } 2> e.txt; printf 'done < <(): [%s]\n' "$(cat e.txt)"
f() { cat <(echo err4 >&2); }; f 2> e.txt; printf 'f 2>: [%s]\n' "$(cat e.txt)"
( exec 2> e.txt; cat <(echo err5 >&2) ); printf 'exec 2>: [%s]\n' "$(cat e.txt)"
echo in > i.txt; { cat <(cat); } < i.txt | sed 's/^/group <: /'
exec 5> f5.txt; { cat <(echo err6 >&2); } 2>&5; exec 5>&-; printf 'dup5: [%s]\n' "$(cat f5.txt)"
printf 'pre\n' > e2.txt; { cat <(echo err7 >&2); } 2>> e2.txt; printf 'append: [%s]\n' "$(cat e2.txt)"
{ { cat <(echo err8 >&2); } 2> inner.txt; } 2> outer.txt; printf 'nested: [%s|%s]\n' "$(cat inner.txt)" "$(cat outer.txt)"
o=$( cat <(printf 'q1\n') <(printf 'q2\n') ); printf 'two: [%s]\n' "$o"
read -r line < <(printf 'hello\n'); printf 'read: [%s]\n' "$line"
{ read -r line < <(echo rerr >&2; printf 'rl\n'); } 2> e.txt; printf 'readg: [%s|%s]\n' "$line" "$(cat e.txt)"
printf 'exec0: [%s]\n' "$( ( exec 0< i.txt; cat <(cat) ) )"
while read -r l; do :; done < <(echo pe >&2) 2>/dev/null; printf 'pitfall done\n'
cat <(echo top >&2); printf 'top done\n'
"#;

const BATTERY_EXPECTED_STDOUT: &str = "\
group 2>: [err1]
comsub 2>&1: [err2]
done < <(): [err3]
f 2>: [err4]
exec 2>: [err5]
group <: in
dup5: [err6]
append: [pre
err7]
nested: [err8|]
two: [q1
q2]
read: [hello]
readg: [rl|rerr]
exec0: [in]
pitfall done
top done
";

// The pitfall guard's producer must still reach the process stderr
// (GNU forks it before the list's own 2>/dev/null applies), and so
// must the top-level control's.
const BATTERY_EXPECTED_STDERR: &str = "\
pe
top
";

#[test]
fn input_procsub_ambient_fd_battery_utf8_locale() {
    let (stdout, stderr, code) = rubash_file(BATTERY, "C.UTF-8");
    assert_eq!(stdout, BATTERY_EXPECTED_STDOUT, "stdout mismatch");
    assert_eq!(stderr, BATTERY_EXPECTED_STDERR, "stderr mismatch");
    assert_eq!(code, Some(0), "exit code mismatch");
}

#[test]
fn input_procsub_ambient_fd_battery_c_locale() {
    let (stdout, stderr, code) = rubash_file(BATTERY, "C");
    assert_eq!(stdout, BATTERY_EXPECTED_STDOUT, "stdout mismatch");
    assert_eq!(stderr, BATTERY_EXPECTED_STDERR, "stderr mismatch");
    assert_eq!(code, Some(0), "exit code mismatch");
}
