//! Regression pins for rubash#1691 — the backing temp files of process
//! substitutions (`rubash-process-subst-<pid>-<nanos>.tmp` in $TMPDIR)
//! were left behind after the shell exited. GNU bash uses anonymous
//! pipes/FIFOs for `<(...)` and never creates a file at all, so after
//! ANY process substitution the temp directory must be exactly as it
//! was before the run.
//!
//! Measured leak surface on the pre-fix tree (2/2 runs, isolated
//! TMPDIR diffed before/after shell exit): external commands consuming
//! a redirect process substitution (wc/head/grep/sort), and assignment
//! contexts (scalar and compound), leaked one file per substitution.
//! Argument form, builtin redirect consumers, the writer form,
//! persistent-fd form and the assignment-OUTPUT finish were already
//! clean. These pins cover both sets so the fix cannot trade one for
//! the other; each case runs in its own fresh TMPDIR and the assertion
//! is on the directory listing after the shell process has exited —
//! not on exit codes.
#![cfg(unix)]

use std::process::Command;

fn run_with_tmpdir(tag: &str, body: &str) -> (Vec<String>, String, Option<i32>) {
    let base = std::env::temp_dir().join(format!("rubash-1691-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let tmpdir = base.join("tmp");
    let workdir = base.join("work");
    std::fs::create_dir_all(&tmpdir).expect("tmpdir");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let script = workdir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .current_dir(&workdir)
        .env("TMPDIR", &tmpdir)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let mut leftovers: Vec<String> = std::fs::read_dir(&tmpdir)
        .expect("read tmpdir after exit")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    leftovers.sort();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let code = output.status.code();
    let _ = std::fs::remove_dir_all(&base);
    (leftovers, stdout, code)
}

fn assert_no_leak(tag: &str, body: &str, expected_stdout: &str) {
    let (leftovers, stdout, code) = run_with_tmpdir(tag, body);
    assert_eq!(code, Some(0), "{tag}: shell must succeed");
    assert_eq!(stdout, expected_stdout, "{tag}: output must stay correct");
    assert!(
        leftovers.is_empty(),
        "{tag}: process-substitution temp files leaked into $TMPDIR after exit: {leftovers:?}"
    );
}

#[test]
fn issue1691_external_redirect_consumers_do_not_leak() {
    assert_no_leak("wc", "wc -l < <(seq 1 5)\n", "5\n");
    assert_no_leak("head", "head -1 < <(seq 1 5)\n", "1\n");
    assert_no_leak("grep", "grep -c . < <(seq 1 3)\n", "3\n");
    assert_no_leak("sort", "sort < <(printf 'b\\na\\n')\n", "a\nb\n");
}

#[test]
fn issue1691_assignment_contexts_do_not_leak() {
    assert_no_leak("scalar", "f=<(echo hi); echo done\n", "done\n");
    assert_no_leak("scalar-unused", "f=<(echo hi)\n", "");
    assert_no_leak(
        "compound",
        "a=(<(echo one) <(echo two)); echo \"count:${#a[@]}\"\n",
        "count:2\n",
    );
}

#[test]
fn issue1691_previously_clean_contexts_stay_clean() {
    assert_no_leak("argument", "cat <(echo hi)\n", "hi\n");
    assert_no_leak("builtin-redirect", "cat < <(echo hi)\n", "hi\n");
    assert_no_leak(
        "read-loop",
        "while read -r x; do echo \"got:$x\"; done < <(printf 'a\\nb\\n')\n",
        "got:a\ngot:b\n",
    );
    assert_no_leak("nested", "cat <(cat <(echo deep))\n", "deep\n");
    assert_no_leak(
        "persistent-fd",
        "exec 3< <(echo hi); cat <&3; exec 3<&-\n",
        "hi\n",
    );
    assert_no_leak(
        "writer-form",
        "cat > >(cat > out.txt) <<< hello; sleep 0.2; cat out.txt\n",
        "hello\n",
    );
}
