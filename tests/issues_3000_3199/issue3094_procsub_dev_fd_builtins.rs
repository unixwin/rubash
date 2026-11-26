//! Issue rubash#3094 regression: process substitution yields `/dev/fd/N` —
//! shell builtins (`source` / `.`, `test` / `[`, `[[`, `mapfile` / `readarray`,
//! `read`) must resolve in-shell `/dev/fd/N` paths produced by `<(...)` on
//! Windows without hitting the Windows filesystem or emitting spurious pipe
//! write errors.
//!
//! GNU spec:
//! - `subst.c:6362 process_substitute()` parks the pipe fd at high fd and
//!   returns `/dev/fd/N`.
//! - `builtins/source.def` -> `builtins/evalfile.c:104` opens `/dev/fd/N`.
//! - `test.c:351 filecomp()` -> `general.c:673 same_file()` and
//!   `lib/sh/eaccess.c:80 sh_stat()` fstat `/dev/fd/N`, so `[ /dev/fd/3 -ef
//!   /dev/fd/3 ]` and dup'd `[ /dev/fd/3 -ef /dev/fd/4 ]` succeed even on a
//!   process-substitution pipe.
//! - `builtins/mapfile.def` / `builtins/read.def` read from the redirected or
//!   `-u N` descriptor without spurious broken-pipe write errors.

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_script(case: &str, script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue3094-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue3094 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash probe");
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

#[test]
fn source_and_dot_resolve_procsub_dev_fd() {
    let out = run_rubash_script(
        "source",
        "\
source <(echo \"echo SOURCE_OK\")
echo \"rc1=$?\"
. <(echo \"echo DOT_OK\")
echo \"rc2=$?\"
exec 3< <(echo \"echo EXEC3_OK\")
source /dev/fd/3
echo \"rc3=$?\"
exec 3<&-
",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stderr, "");
    assert_eq!(
        out.stdout.replace("\r\n", "\n"),
        "SOURCE_OK\nrc1=0\nDOT_OK\nrc2=0\nEXEC3_OK\nrc3=0\n"
    );
}

#[test]
fn test_bracket_and_conditional_resolve_procsub_dev_fd() {
    let out = run_rubash_script(
        "test-unary",
        "\
[ -e <(echo hi) ]; echo \"b_e=$?\"
[ -r <(echo hi) ]; echo \"b_r=$?\"
[[ -e <(echo hi) ]]; echo \"d_e=$?\"
[[ -r <(echo hi) ]]; echo \"d_r=$?\"
exec 3< <(echo hi)
[ -e /dev/fd/3 ]; echo \"fd_e=$?\"
[ -r /dev/fd/3 ]; echo \"fd_r=$?\"
[ -p /dev/fd/3 ]; echo \"fd_p=$?\"
[[ -e /dev/fd/3 ]]; echo \"fd_de=$?\"
exec 3<&-
",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stderr, "");
    assert_eq!(
        out.stdout.replace("\r\n", "\n"),
        "b_e=0\nb_r=0\nd_e=0\nd_r=0\nfd_e=0\nfd_r=0\nfd_p=0\nfd_de=0\n"
    );
}

#[test]
fn test_and_conditional_ef_resolve_procsub_dev_fd_identity() {
    let out = run_rubash_script(
        "test-ef",
        "\
exec 3< <(echo a)
exec 4<&3
exec 5< <(echo b)
[ /dev/fd/3 -ef /dev/fd/3 ]; echo \"b33=$?\"
[ /dev/fd/3 -ef /dev/fd/4 ]; echo \"b34=$?\"
[ /dev/fd/3 -ef /dev/fd/5 ]; echo \"b35=$?\"
[[ /dev/fd/3 -ef /dev/fd/3 ]]; echo \"d33=$?\"
[[ /dev/fd/3 -ef /dev/fd/4 ]]; echo \"d34=$?\"
[[ /dev/fd/3 -ef /dev/fd/5 ]]; echo \"d35=$?\"
exec 3<&- 4<&- 5<&-
",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stderr, "");
    assert_eq!(
        out.stdout.replace("\r\n", "\n"),
        "b33=0\nb34=0\nb35=1\nd33=0\nd34=0\nd35=1\n"
    );
}

#[test]
fn mapfile_and_while_read_resolve_procsub_without_spurious_write_error() {
    let out = run_rubash_script(
        "mapfile-read",
        "\
mapfile -t arr < <(printf 'a\\nb\\n')
echo \"mf=${#arr[@]}:${arr[*]}\"
readarray -t arr2 < <(printf 'c\\nd\\n')
echo \"ra=${#arr2[@]}:${arr2[*]}\"
exec 3< <(printf 'e\\nf\\n')
mapfile -t -u 3 arr3
echo \"mfu=${#arr3[@]}:${arr3[*]}\"
exec 3<&-
while read l; do echo \"L=$l\"; done < <(printf 'a\\nb\\n')
echo \"wr=$?\"
",
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stderr, "");
    assert_eq!(
        out.stdout.replace("\r\n", "\n"),
        "mf=2:a b\nra=2:c d\nmfu=2:e f\nL=a\nL=b\nwr=0\n"
    );
}
