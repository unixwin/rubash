//! rubash#1632 — a coproc command carrying its own stdout
//! redirection (`coproc RC { cat; } >file`) broke the coproc pipe:
//! the parent's read end reported immediate EOF (rc 1) instead of
//! blocking, because apply_coproc_redirects replaced the child's
//! stdout Stdio and DROPPED the pipe writer before the spawn — the
//! only write end the pipe ever had.
//!
//! GNU applies the redirect in the forked child (execute_cmd.c),
//! where the displaced pipe end survives in a redirect save slot
//! for the child's whole life: the read end sees no EOF while the
//! coproc lives (a `read -t 1` times out with rc 142) and the data
//! lands in the file. The fix hands the displaced writer back from
//! apply_coproc_redirects: on Unix it rides into the child via
//! pre_exec (dup to the first free fd in the save-slot range
//! 10..64, which also clears CLOEXEC); elsewhere the parent holds
//! it in coproc_displaced_stdout_writers until the coproc retires.
//!
//! Expectations verified against GNU bash 5.3.20 script-file probes.

use std::process::{Command, Stdio};

fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1632-{}",
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
fn coproc_redirected_stdout_read_blocks_and_file_receives_data() {
    // The issue's regression seed verbatim.
    let (stdout, stderr, code) = rubash_script(
        "coproc RC { cat; } >redir_out.txt\n\
         echo abc >&${RC[1]}\n\
         read -t 1 -u ${RC[0]} line; echo \"read rc=$? line=[$line]\"\n\
         echo \"file: $(cat redir_out.txt)\"\n\
         exec {RC[0]}>&- {RC[1]}>&-\n",
    );
    assert_eq!(
        stdout, "read rc=142 line=[]\nfile: abc\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_append_redirected_stdout_also_keeps_pipe_open() {
    let (stdout, stderr, code) = rubash_script(
        "coproc RC { cat; } >>redir_out.txt\n\
         echo abc >&${RC[1]}\n\
         read -t 1 -u ${RC[0]} line; echo \"read rc=$? line=[$line]\"\n\
         echo \"file: $(cat redir_out.txt)\"\n\
         exec {RC[0]}>&- {RC[1]}>&-\n",
    );
    assert_eq!(
        stdout, "read rc=142 line=[]\nfile: abc\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
