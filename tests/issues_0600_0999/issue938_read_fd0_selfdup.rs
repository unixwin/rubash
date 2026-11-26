//! rubash#938: a fd-0 self-dup is a GNU no-op — `read x <&0`,
//! `read x </dev/stdin`, the `/dev/fd/0` / `/proc/self/fd/0` aliases and
//! the `<&0-` move form all leave fd 0 bound exactly as it was, so the
//! builtin must serve the ordinary fd-0 transports (a pipeline stage's
//! FUNCTION_STDIN mirror included), not an fd-table-only descriptor
//! lookup that reports EOF where the pipe holds data.
//!
//! GNU redir.c:1128 do_redirection_internal gates the whole dup action —
//! the dup2 AND the r_move_input close — on `redir_fd != redirector`;
//! `-u 0` likewise names fd 0 itself (read.def:294). Reference output
//! verified against WSL GNU bash 5.3.0 on the issue ticket.
//!
//! Every case runs as a script FILE: the -c shape binds a different fd-0
//! transport (inherited process stdin), which masked the bug — same
//! driver discipline as issue276_read_fd0_file_loop_regressions.rs.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn run_script(script: &str, stdin: &[u8]) -> (String, String, Option<i32>) {
    let stamp = || {
        format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        )
    };
    let path = std::env::temp_dir().join(format!("rubash-938-{}.sh", stamp()));
    std::fs::write(&path, script).expect("write script");
    // The scripts create probe files with relative names; run the child in
    // its own sandbox so none land in the repository root.
    let sandbox = std::env::temp_dir().join(format!("rubash-938-dir-{}", stamp()));
    std::fs::create_dir_all(&sandbox).expect("create sandbox");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.current_dir(&sandbox)
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&sandbox);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The ticket shape: `/dev/stdin` and `<&0` inside a pipeline group stage
/// must read the pipe, byte-for-byte the GNU output.
#[test]
fn pipeline_group_dev_stdin_and_dup0_read_the_pipe() {
    let script = "\
printf 'SIN\\n' | { read -r z </dev/stdin; echo \"devstdin=[$z]\"; }
printf 'SIN\\n' | { read -r w <&0; echo \"dup0=[$w]\"; }
printf 'SIN\\n' | { read -r v; echo \"plain=[$v]\"; }
printf 'FILE\\n' > f938.txt; { read -r y </dev/stdin; echo \"gotf=[$y]\"; } < f938.txt
";
    let (stdout, stderr, rc) = run_script(script, b"");
    assert_eq!(
        stdout, "devstdin=[SIN]\ndup0=[SIN]\nplain=[SIN]\ngotf=[FILE]\n",
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// The remaining fd-0 self-reference forms: the `<&0-` move no-op, the
/// numbered `/dev/fd/0` and `/proc/self/fd/0` aliases, and `read -u 0`
/// all designate fd 0 itself — GNU reads the stage's pipe record.
#[test]
fn pipeline_group_fd0_selfdup_family_reads_the_pipe() {
    let script = "\
printf 'MV\\n' | { read -r m <&0-; echo \"move=[$m]\"; }
printf 'FD0\\n' | { read -r d </dev/fd/0; echo \"devfd0=[$d]\"; }
printf 'PF\\n' | { read -r p </proc/self/fd/0; echo \"procfd=[$p]\"; }
printf 'U0\\n' | { read -u0 -r u; echo \"u0=[$u]\"; }
";
    let (stdout, stderr, rc) = run_script(script, b"");
    assert_eq!(
        stdout, "move=[MV]\ndevfd0=[FD0]\nprocfd=[PF]\nu0=[U0]\n",
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// EOF discipline is unchanged: an empty pipe under a fd-0 self-dup still
/// fails the builtin (read.def:762-768 eof break → :949 EXECUTION_FAILURE)
/// while assigning the empty record.
#[test]
fn fd0_selfdup_still_reports_eof_on_empty_pipe() {
    let script = "\
printf '' | { read -r w <&0; echo \"dup0=[$w] rc=$?\"; }
printf '' | { read -r z </dev/stdin; echo \"devstdin=[$z] rc=$?\"; }
";
    let (stdout, stderr, rc) = run_script(script, b"");
    assert_eq!(
        stdout, "dup0=[] rc=1\ndevstdin=[] rc=1\n",
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}
