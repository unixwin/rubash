//! rubash#2333: a rubash child inheriting TWO OR MORE delivered
//! descriptors got them crossed in its startup fd-table restore.
//!
//! The parent's delivery is correct (its env stamps name a
//! distinct carrier per fd, and a foreign child reads each body
//! at its own fd). The child's restore (init.rs) built each
//! FileFd from the STAMPED handle value — the PARENT's descriptor
//! number. On Unix the spawn's pin pass installs each delivered
//! descriptor AT its slot fd in the child (pre_exec dup2), so the
//! stamped number designates whatever the pins left there: with
//! two heredoc carriers the child served a +1 rotation
//! (`read -u 3` -> fd 4's body, `read -u 4` -> EOF; three carriers
//! rotated by one as well). With one carrier the numbers
//! coincide, which is why the issue2292 pins (18/18) never saw
//! it. GNU semantics: plain fd inheritance — each inherited
//! descriptor keeps its own open file description
//! (execute_cmd.c execute_in_subshell: the fork copies the whole
//! fd table). Oracle: GNU bash 5.3.20, script files.
//!
//! FD-ENVIRONMENT SENSITIVITY: these pins spawn the shell as a
//! child process and bind fds 3-7 in the scripts. An inherited
//! ambient descriptor on one of those numbers in the test harness
//! can shift the parent's descriptor arithmetic and mask or fake
//! a result — the collector re-runs this file under a scrubbed
//! fd environment (only fds 0/1/2 open).

use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn run(test: &str, outer_script: &str) -> (String, String, i32) {
    let scratch =
        std::env::temp_dir().join(format!("rubash-issue2333-{}-{}", std::process::id(), test));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("create scratch");
    std::fs::write(scratch.join("data.txt"), "file-body\n").expect("write data file");
    std::fs::write(scratch.join("f1.txt"), "body-one\n").expect("write f1");
    std::fs::write(scratch.join("f2.txt"), "body-two\n").expect("write f2");
    std::fs::write(scratch.join("outer.sh"), outer_script).expect("write outer script");
    let output = Command::new(RUBASH)
        .arg("outer.sh")
        .current_dir(&scratch)
        .env("HOME", &scratch)
        .env("RUBASH_BIN", RUBASH)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn issue2333_two_heredoc_carriers_read_in_child() {
    // The issue's regression seed, run through script files: the
    // nested child's in-process reads must serve each carrier at
    // its own fd.
    let (stdout, stderr, code) = run(
        "two_read",
        "exec 3<<HD3\nbody-three\nHD3\nexec 4<<HD4\nbody-four\nHD4\n\
         \"$RUBASH_BIN\" -c 'read -u 3 a; echo \"n3=[$a]\"; read -u 4 b; echo \"n4=[$b]\"'\n",
    );
    assert_eq!(
        stdout, "n3=[body-three]\nn4=[body-four]\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn issue2333_two_heredoc_carriers_delivered_to_grandchild() {
    // The seed's cat form: the restored table feeds the child's
    // own spawn delivery, so a crossed restore materializes the
    // wrong body at every slot downstream.
    let (stdout, stderr, code) = run(
        "two_cat",
        "exec 3<<HD3\nbody-three\nHD3\nexec 4<<HD4\nbody-four\nHD4\n\
         \"$RUBASH_BIN\" -c 'cat <&3'\n\"$RUBASH_BIN\" -c 'cat <&4'\n",
    );
    assert_eq!(stdout, "body-three\nbody-four\n", "stderr: {stderr}");
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn issue2333_three_heredoc_carriers_no_rotation() {
    // Three carriers rotated by one pre-fix (n3=four, n4=five,
    // n5=EOF) — the rotation witness that pins the mechanism.
    let (stdout, stderr, code) = run(
        "three",
        "exec 3<<HD3\nbody-three\nHD3\nexec 4<<HD4\nbody-four\nHD4\nexec 5<<HD5\nbody-five\nHD5\n\
         \"$RUBASH_BIN\" -c 'read -u 3 a; echo \"n3=[$a]\"; read -u 4 b; echo \"n4=[$b]\"; read -u 5 c; echo \"n5=[$c]\"'\n",
    );
    assert_eq!(
        stdout, "n3=[body-three]\nn4=[body-four]\nn5=[body-five]\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn issue2333_mixed_heredoc_and_file_carriers() {
    // A heredoc carrier and a plain-file binding delivered by the
    // same spawn crossed pre-fix (n3 served the file's body).
    let (stdout, stderr, code) = run(
        "mixed",
        "exec 3<<HD3\nbody-three\nHD3\nexec 6<data.txt\n\
         \"$RUBASH_BIN\" -c 'read -u 3 a; echo \"n3=[$a]\"; read -u 6 b; echo \"n6=[$b]\"'\n",
    );
    assert_eq!(
        stdout, "n3=[body-three]\nn6=[file-body]\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn issue2333_two_plain_file_fds_read_in_child() {
    // The restore mechanism is endpoint-kind-blind: two delivered
    // plain-file descriptors crossed the same way (n7 served EOF
    // pre-fix).
    let (stdout, stderr, code) = run(
        "two_files",
        "exec 6<f1.txt\nexec 7<f2.txt\n\
         \"$RUBASH_BIN\" -c 'read -u 6 a; echo \"n6=[$a]\"; read -u 7 b; echo \"n7=[$b]\"'\n",
    );
    assert_eq!(stdout, "n6=[body-one]\nn7=[body-two]\n", "stderr: {stderr}");
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn issue2333_parent_reads_unaffected_after_child() {
    // Control: spawning the crossed child must not disturb the
    // parent's own bindings.
    let (stdout, stderr, code) = run(
        "parent_after",
        "exec 3<<HD3\nbody-three\nHD3\nexec 4<<HD4\nbody-four\nHD4\n\
         \"$RUBASH_BIN\" -c 'read -u 3 a; read -u 4 b'\n\
         read -u 3 x; echo \"p3=[$x]\"\nread -u 4 y; echo \"p4=[$y]\"\n",
    );
    assert_eq!(
        stdout, "p3=[body-three]\np4=[body-four]\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[test]
fn issue2333_single_heredoc_control() {
    // Control: the single-carrier shape the issue2292 family pins
    // — it passed pre-fix by numerical coincidence and must stay
    // green.
    let (stdout, stderr, code) = run(
        "single",
        "exec 3<<HD3\nbody-three\nHD3\n\
         \"$RUBASH_BIN\" -c 'read -u 3 a; echo \"n3=[$a]\"'\n",
    );
    assert_eq!(stdout, "n3=[body-three]\n", "stderr: {stderr}");
    assert_eq!(code, 0, "stderr: {stderr}");
}
