//! rubash#2170 — a compound command's text-mirror fd-0 binding
//! (`< /dev/null`, heredoc, here-string) only governed when the
//! fd-0 fd-table entry it superseded was the ambient inherited
//! entry. After `exec 0<file`, the entry is a File binding, and it
//! kept governing every consumer inside the compound: the builtin
//! read's fd-table arm read the old file, a spawned child inherited
//! the file, and the fd-alias fact sync stated the file's facts —
//! while the staged FUNCTION_STDIN mirror claimed /dev/null.
//!
//! GNU installs a command's own redirection on fd 0 before the
//! command runs (execute_cmd.c + redir.c:767-955
//! do_redirection_internal), superseding whatever the enclosing
//! scope bound there; the superseded binding is restored when the
//! command's scope ends (RX_UNDOABLE). The mirror therefore has to
//! displace the fd-table entry for its scope — every descriptor
//! branch of the bind already replaces the entry and lets the
//! guard restore it; the text branch now does the same (the #2131
//! fact pass already treated an absent slot as mirror-governed).
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script
//! files (probes p2170_a..k in the cluster workdir).

use std::path::PathBuf;
use std::process::Command;

fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2170-pins-{tag}"));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::write(dir.join("input.txt"), b"hello-from-input\nsecond-line\n")
        .expect("write fixture");
    std::fs::write(dir.join("other.txt"), b"other-file-bytes\n").expect("write fixture");
    dir
}

/// Runs the script with `$F` bound to the fixture input file and
/// `$O` to a second regular file, cwd = the fixture directory.
fn run_with_fixture(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = fixture_dir(tag);
    let shell_f = dir.join("input.txt").to_string_lossy().replace('\\', "/");
    let shell_o = dir.join("other.txt").to_string_lossy().replace('\\', "/");
    let script = script.replace("$F", &shell_f).replace("$O", &shell_o);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", &script])
        .stdin(std::process::Stdio::null())
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue2170_devnull_mirror_governs_over_exec_file_binding() {
    let (stdout, _stderr, code) = run_with_fixture(
        "devnull",
        "exec 0<$F\n\
         { if read line; then echo \"read-ok [$line]\"; else echo read-fail; fi; \
           [ -f /dev/stdin ] && echo f=T || echo f=F; \
           [ -c /dev/stdin ] && echo c=T || echo c=F; } < /dev/null\n",
    );
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("read-fail"),
        "the mirror's /dev/null is fd 0, not the exec file: {stdout}"
    );
    assert!(stdout.contains("f=F"), "mirror source fact: {stdout}");
    assert!(stdout.contains("c=T"), "mirror source fact: {stdout}");
    assert!(
        !stdout.contains("hello-from-input"),
        "the superseded file must not be read: {stdout}"
    );
}

#[test]
fn issue2170_heredoc_mirror_governs_over_exec_file_binding() {
    let (stdout, _stderr, code) = run_with_fixture(
        "heredoc",
        "exec 0<$F\n\
         { if read line; then echo \"read-ok [$line]\"; else echo read-fail; fi; \
           [ -p /dev/stdin ] && echo p=T || echo p=F; \
           [ -f /dev/stdin ] && echo f=T || echo f=F; } <<EOF\n\
         heredoc-line\n\
         EOF\n",
    );
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("read-ok [heredoc-line]"),
        "the body reads the heredoc, not the exec file: {stdout}"
    );
    assert!(stdout.contains("p=T"), "heredoc is a pipe in GNU: {stdout}");
    assert!(stdout.contains("f=F"), "mirror source fact: {stdout}");
}

#[test]
fn issue2170_external_child_reads_the_mirror_not_the_file() {
    let (stdout, _stderr, code) = run_with_fixture(
        "extchild",
        "exec 0<$F\n{ cat; echo cat_rc=$?; } < /dev/null\n",
    );
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("cat_rc=0"),
        "cat runs on /dev/null: {stdout}"
    );
    assert!(
        !stdout.contains("hello-from-input"),
        "the child must not inherit the superseded file: {stdout}"
    );
}

#[test]
fn issue2170_exec_binding_restored_after_mirrored_compound() {
    let (stdout, _stderr, code) = run_with_fixture(
        "restore",
        "exec 0<$F\n\
         { read a; echo \"inner=[$a]\"; } < /dev/null\n\
         if read b; then echo \"after=[$b]\"; else echo after-fail; fi\n",
    );
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("inner=[]"),
        "the mirrored read hits /dev/null EOF: {stdout}"
    );
    assert!(
        stdout.contains("after=[hello-from-input]"),
        "the exec binding is back, cursor untouched: {stdout}"
    );
}

#[test]
fn issue2170_subshell_region_mirror_governs_over_exec_file_binding() {
    let (stdout, _stderr, code) = run_with_fixture(
        "subshell",
        "exec 0<$F\n\
         ( if read line; then echo \"sub-read-ok [$line]\"; else echo sub-read-fail; fi ) < /dev/null\n",
    );
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("sub-read-fail"),
        "the region's staged mirror governs its readers: {stdout}"
    );
    assert!(
        !stdout.contains("hello-from-input"),
        "the superseded file must not be read: {stdout}"
    );
}

#[test]
fn issue2170_loop_mirror_governs_over_exec_file_binding() {
    let (stdout, _stderr, code) = run_with_fixture(
        "loop",
        "exec 0<$F\n\
         n=0\n\
         while read x; do n=$((n+1)); done < /dev/null\n\
         echo iters=$n\n\
         if read y; then echo \"after=[$y]\"; else echo after-fail; fi\n",
    );
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("iters=0"),
        "the loop reads /dev/null, not the file: {stdout}"
    );
    assert!(
        stdout.contains("after=[hello-from-input]"),
        "the exec file was never consumed: {stdout}"
    );
}

#[test]
fn issue2170_stream_file_target_still_binds_a_descriptor() {
    // Guard (green both ways): a regular-file compound target takes
    // the descriptor branch, not the mirror — unchanged by the fix.
    let (stdout, _stderr, code) = run_with_fixture(
        "streamfile",
        "exec 0<$F\n\
         { if read line; then echo \"read-ok [$line]\"; else echo read-fail; fi; \
           [ -f /dev/stdin ] && echo f=T || echo f=F; } < $O\n",
    );
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("read-ok [other-file-bytes]"),
        "the file target binds: {stdout}"
    );
    assert!(stdout.contains("f=T"), "file fact: {stdout}");
}

#[test]
fn issue2170_mirror_over_inherited_stdin_unchanged() {
    // Guard (green both ways): the mirror over the ambient inherited
    // entry already governed; the fix must not move it.
    let (stdout, _stderr, code) = run_with_fixture(
        "inherited",
        "{ if read line; then echo \"read-ok [$line]\"; else echo read-fail; fi; \
           [ -f /dev/stdin ] && echo f=T || echo f=F; \
           [ -c /dev/stdin ] && echo c=T || echo c=F; } < /dev/null\n",
    );
    assert_eq!(code, Some(0));
    assert!(stdout.contains("read-fail"), "control shape: {stdout}");
    assert!(stdout.contains("f=F"), "control fact: {stdout}");
    assert!(stdout.contains("c=T"), "control fact: {stdout}");
}
