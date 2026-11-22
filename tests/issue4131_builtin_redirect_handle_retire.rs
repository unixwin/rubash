//! Issue unixwin/rubash#4131: a redirect handle pre-opened for an
//! in-process builtin stayed open after the builtin returned — the
//! shell still held the target write-open when the NEXT command ran:
//!
//! ```sh
//! echo x > hold.txt
//! ls -l /proc/$$/fd | grep hold.txt   # GNU: no match; rubash: held
//! ```
//!
//! On Windows the same window makes a following pipeline's
//! `[IO.File]::OpenRead` with FileShare.Read fail (the reporter's
//! shape 1); any intervening `:` retired the handle, which is how the
//! leak window was proven to be the previous command's gate.
//!
//! GNU contract: a command's redirections are undone when the command
//! returns (execute_cmd.c:5606/5693 undo_partial_redirects around the
//! builtin dispatch). The pending-preopened channel's own contract
//! (Executor::preopened_generation) already says a handle lives only
//! within the command whose walks parked it; retirement just never
//! happened at that command's own end — it waited for the next
//! command's reject gate.
//!
//! Linux /proc evidence, byte-verified against GNU bash 5.3.20
//! (script files, cloud oracle). Unix-only: the witness is /proc.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash4131-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    let file = dir.join("case.sh");
    std::fs::write(&file, script).expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&file)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn issue4131_builtin_redirect_handle_retires_at_command_end() {
    // Q1: the pipeline right after the builtin must not see the
    // handle. Q2 (control): an intervening `:` also leaves it free.
    // Q3: a command substitution reading /proc/$$/fd counts zero.
    let script = "echo x > hold.txt\n\
                  if ls -l /proc/$$/fd | grep -q hold.txt; then echo 'Q1 HELD'; else echo 'Q1 free'; fi\n\
                  echo x > hold.txt\n\
                  :\n\
                  if ls -l /proc/$$/fd | grep -q hold.txt; then echo 'Q2 HELD'; else echo 'Q2 free'; fi\n\
                  echo x > hold.txt\n\
                  r=$(ls -l /proc/$$/fd | grep -c hold.txt)\n\
                  echo \"Q3 count=$r\"\n";
    let (stdout, stderr, code) = run_script("retire", script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "Q1 free\nQ2 free\nQ3 count=0\n");
}

#[test]
fn issue4131_guards_umask_adoption_group_and_exec() {
    // Guards: the #3875 umask adoption shape (the parked handle is
    // the ONLY open — a re-open by path would fail EACCES once the
    // created mode drops owner-write), a compound group's redirect,
    // and a persistent `exec` redirect all keep working.
    let script = "umask 0277\n\
                  echo data > prot.txt\n\
                  printf 'Q4 '\n\
                  cat prot.txt\n\
                  umask 022\n\
                  { echo grouped; } > grp.txt\n\
                  printf 'Q5 '\n\
                  cat grp.txt\n\
                  exec 3> exec.txt\n\
                  echo via3 >&3\n\
                  exec 3>&-\n\
                  printf 'Q6 '\n\
                  cat exec.txt\n";
    let (stdout, stderr, code) = run_script("guards", script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "Q4 data\nQ5 grouped\nQ6 via3\n");
}

#[test]
fn issue4131_pipeline_stage_redirect_still_lands() {
    // Guard: a pipeline stage's own redirect still receives the
    // stage's bytes (the stage-transfer entries are adopted, not
    // retired early), and the pipe itself stays empty for the reader.
    let script = "echo piped > stage.txt | wc -c\n\
                  printf 'stage: '\n\
                  cat stage.txt\n";
    let (stdout, stderr, code) = run_script("stage", script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "0\nstage: piped\n");
}
