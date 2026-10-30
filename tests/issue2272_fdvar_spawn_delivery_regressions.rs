//! rubash#2272 — `{fd}` variable-form redirects were missing from
//! the spawn delivery face for every kind: the dynamic-fd applier
//! bound the var's descriptor into the shell's fd table, but
//! `apply_external_fd_delivery` skipped fd_var entries outright, so
//! an external child either never received the descriptor or
//! received it at a number that disagreed with the `{fd}` variable.
//!
//! GNU binds a command's whole redirect list in the forked child
//! (redir.c:237 do_redirections; the {var} assignment is
//! redir_varassign, redir.c:1010 and :1113; the descriptor itself is
//! allocated at the lowest free slot >= SHELL_FD_BASE 10,
//! redir.c:86). The child therefore sees the binding at exactly the
//! number the variable names, for every kind — file, heredoc,
//! here-string, dup, close, move — while the parent variable and
//! the parent's fd table are untouched by the command (the
//! assignment lives and dies in the fork). Closes a `{var}` entry
//! asks for (close forms, move-form sources) apply in the child
//! only; the parent's bindings survive.
//!
//! Expectations byte-verified against GNU bash 5.3.20 with the
//! probe cells in the cluster workdir (probes/cells, probes/fcells,
//! probes/dcells): every kind was run with a foreign observer child
//! (GNU bash / dd) reporting the descriptor it actually received.
//!
//! OBSERVER NOTE: the child in these pins is a real spawned
//! process — the rubash binary itself re-invoked, or dd — never a
//! `./reader.sh` script: rubash runs ENOEXEC scripts in-process,
//! where the child shares the parent's fd table and the pins would
//! pass vacuously.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2272-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    fs::write(dir.join("seed.txt"), "seed-line-1\nseed-line-2\n").expect("write seed");
    fs::write(dir.join("amb.txt"), "amb-line-1\n").expect("write amb");
    dir
}

fn rubash_in(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(RUBASH)
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_in_stdin_file(
    dir: &Path,
    script: &str,
    stdin_file: &Path,
) -> (String, String, Option<i32>) {
    let file = fs::File::open(stdin_file).expect("open stdin file");
    let output = Command::new(RUBASH)
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .stdin(Stdio::from(file))
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The spawned child: a nested rubash whose own external `cat`
/// reads the descriptor — two real spawns between the binding and
/// the read, so nothing passes through the in-process face.
fn child(inner: &str) -> String {
    format!("\"{RUBASH}\" -c '{inner}'")
}

#[test]
fn issue2272_fdvar_input_file_reaches_external_child() {
    let dir = scratch("infile");
    let script = format!(
        "unset v\n{} {{v}}<seed.txt\necho \"rc=$? v=[${{v-unset}}]\"\n",
        child("cat <&10")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "seed-line-1\nseed-line-2\nrc=0 v=[unset]\n");
}

#[test]
fn issue2272_fdvar_output_file_written_by_child() {
    let dir = scratch("outfile");
    let script = format!(
        "unset v\n{} {{v}}>out.txt\necho \"rc=$? v=[${{v-unset}}]\"\n",
        child("echo payload >&10")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0 v=[unset]\n");
    assert_eq!(
        fs::read_to_string(dir.join("out.txt")).expect("read out.txt"),
        "payload\n"
    );
}

#[test]
fn issue2272_fdvar_append_keeps_existing_content() {
    let dir = scratch("append");
    fs::write(dir.join("out.txt"), "old\n").expect("seed out.txt");
    let script = format!("{} {{v}}>>out.txt\n", child("echo new >&10"));
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert_eq!(
        fs::read_to_string(dir.join("out.txt")).expect("read out.txt"),
        "old\nnew\n"
    );
}

#[test]
fn issue2272_fdvar_readwrite_shares_one_offset() {
    let dir = scratch("readwrite");
    // GNU: `<>` is ONE description — the child reads line 1, then
    // its write lands at the shared offset, overwriting line 2.
    let script = format!(
        "{} {{v}}<>rw.txt\n",
        child("IFS= read -r l <&10; printf %s seed-LINE-2 >&10")
    );
    fs::write(dir.join("rw.txt"), "seed-line-1\nseed-line-2\n").expect("seed rw.txt");
    let (_stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        fs::read_to_string(dir.join("rw.txt")).expect("read rw.txt"),
        "seed-line-1\nseed-LINE-2\n"
    );
}

#[test]
fn issue2272_fdvar_heredoc_body_reaches_external_child() {
    let dir = scratch("heredoc");
    let script = format!(
        "who=world\nunset v\n{} {{v}}<<EOF\nhello $who\nsecond line\nEOF\necho \"rc=$? v=[${{v-unset}}]\"\n",
        child("cat <&10")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "hello world\nsecond line\nrc=0 v=[unset]\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2272_fdvar_herestring_reaches_external_child() {
    let dir = scratch("herestring");
    let script = format!(
        "unset v\n{} {{v}}<<<\"hello str\"\necho \"rc=$? v=[${{v-unset}}]\"\n",
        child("cat <&10")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hello str\nrc=0 v=[unset]\n");
}

#[test]
fn issue2272_fdvar_two_vars_allocate_in_list_order() {
    let dir = scratch("twovars");
    fs::write(dir.join("seed-b.txt"), "b-line\n").expect("write seed-b");
    let script = format!(
        "unset a b\n{} {{a}}<seed.txt {{b}}<seed-b.txt\necho \"rc=$? a=[${{a-unset}}] b=[${{b-unset}}]\"\n",
        child("cat <&10; cat <&11")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "seed-line-1\nseed-line-2\nb-line\nrc=0 a=[unset] b=[unset]\n"
    );
}

#[test]
fn issue2272_fdvar_allocation_skips_ambient_slots() {
    let dir = scratch("collision");
    // GNU: fd 10 is already open in the parent, so the var binding
    // takes 11 in the child; the ambient fd 10 is inherited as-is.
    // The trailing parent `cat <&10` prints NOTHING: the child's own
    // `cat <&10` read the inherited descriptor to EOF, and a forked
    // child's duplicate descriptor shares the parent's open file
    // description — offset included — so the parent resumes at EOF.
    // (Oracle-verified against GNU bash 5.3.20: exact-shape,
    // child-skips-ambient, and plain-fork probes in the
    // fix-2272-ambient-pin workdir. The trailing read still guards
    // that the parent's binding survives: a destroyed fd 10 would
    // fail the redirect and the script's exit code.)
    let script = format!(
        "exec 10<amb.txt\nunset v\n{} {{v}}<seed.txt\necho \"rc=$? v=[${{v-unset}}]\"\ncat <&10\n",
        child("cat <&11; cat <&10")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "seed-line-1\nseed-line-2\namb-line-1\nrc=0 v=[unset]\n"
    );
}

#[test]
fn issue2272_fdvar_dup_of_ambient_file_fd() {
    let dir = scratch("dupamb");
    let script = format!(
        "exec 3<seed.txt\nunset v\n{} {{v}}<&3\necho \"rc=$? v=[${{v-unset}}]\"\n",
        child("cat <&10")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "seed-line-1\nseed-line-2\nrc=0 v=[unset]\n");
}

#[test]
fn issue2272_fdvar_dup_of_own_list_input_binding() {
    let dir = scratch("dupown");
    // GNU's single ordered walk: `3<seed` earlier in the SAME list
    // is the dup's source, even though the ambient table has no 3.
    let script = format!(
        "unset v\n{} 3<seed.txt {{v}}<&3\necho \"rc=$? v=[${{v-unset}}]\"\n",
        child("cat <&10")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "seed-line-1\nseed-line-2\nrc=0 v=[unset]\n");
}

#[test]
fn issue2272_fdvar_dup_of_own_list_output_binding() {
    let dir = scratch("dupownout");
    let script = format!(
        "unset v\n{} 3>cap3.txt {{v}}>&3\necho \"rc=$? v=[${{v-unset}}]\"\n",
        child("echo X6 >&10")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0 v=[unset]\n");
    assert_eq!(
        fs::read_to_string(dir.join("cap3.txt")).expect("read cap3"),
        "X6\n"
    );
}

#[test]
fn issue2272_fdvar_dup_of_ambient_heredoc() {
    let dir = scratch("dupambhd");
    let script = format!(
        "exec 3<<HD\nambient hd body\nHD\nunset v\n{} {{v}}<&3\necho \"rc=$? v=[${{v-unset}}]\"\n",
        child("cat <&10")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "ambient hd body\nrc=0 v=[unset]\n");
}

#[cfg(unix)]
#[test]
fn issue2272_fdvar_dup_input_from_own_stdin_redirect() {
    let dir = scratch("dupstdin");
    // The command's own `<seed.txt` binds fd 0 first; the dup must
    // copy THAT binding (GNU's ordered walk), not the ambient
    // stdin. dd is the foreign observer: it reads the raw fd.
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "unset v\ndd if=/dev/fd/10 status=none <seed.txt {v}<&0\necho \"rc=$? v=[${v-unset}]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "seed-line-1\nseed-line-2\nrc=0 v=[unset]\n");
}

#[cfg(unix)]
#[test]
fn issue2272_fdvar_dup_input_from_ambient_stdin() {
    let dir = scratch("dupambstdin");
    let seed = dir.join("seed.txt");
    let (stdout, stderr, code) = rubash_in_stdin_file(
        &dir,
        "unset v\ndd if=/dev/fd/10 status=none {v}<&0\necho \"rc=$? v=[${v-unset}]\"\n",
        &seed,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "seed-line-1\nseed-line-2\nrc=0 v=[unset]\n");
}

#[cfg(unix)]
#[test]
fn issue2272_fdvar_dup_output_to_child_stdout_and_stderr() {
    let dir = scratch("dupstdio");
    fs::write(dir.join("payload.txt"), "payload-line\n").expect("write payload");
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "unset v\ndd if=payload.txt of=/dev/fd/10 status=none {v}>&1\ndd if=payload.txt of=/dev/fd/10 status=none {v}>&2\necho \"v=[${v-unset}]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "payload-line\nv=[unset]\n");
    assert_eq!(stderr, "payload-line\n");
}

#[test]
fn issue2272_fdvar_close_form_closes_in_child_only() {
    let dir = scratch("closeform");
    // GNU: the child sees fd 9 closed; the parent's binding and the
    // variable survive the command untouched.
    let script = format!(
        "exec 9<seed.txt\nv=9\n{} {{v}}<&-\necho \"child-rc=$?\"\ncat <&9\necho \"parent-rc=$? v=[$v]\"\n",
        child("if : <&9 2>/dev/null; then echo fd9-open; else echo fd9-closed; fi")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "fd9-closed\nchild-rc=0\nseed-line-1\nseed-line-2\nparent-rc=0 v=[9]\n"
    );
}

#[test]
fn issue2272_fdvar_close_unset_var_is_ambiguous_redirect() {
    let dir = scratch("closeunset");
    let script = format!(
        "unset v\n{} {{v}}<&-\necho \"rc=$?\"\n",
        child("echo should-not-run")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("v: ambiguous redirect"),
        "stderr should name the ambiguous redirect, got: {stderr}"
    );
}

#[test]
fn issue2272_fdvar_move_form_child_copy_source_closed_parent_keeps() {
    let dir = scratch("moveform");
    // GNU: in the child the move is a copy at the var's number with
    // the source closed; the parent's source binding survives.
    let script = format!(
        "exec 3<seed.txt\nunset v\n{} {{v}}<&3-\necho \"child-rc=$? v=[${{v-unset}}]\"\nif : <&3; then echo parent-3-open; else echo parent-3-closed; fi\n",
        child("cat <&10; if : <&3 2>/dev/null; then echo src-open; else echo src-closed; fi")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "seed-line-1\nseed-line-2\nsrc-closed\nchild-rc=0 v=[unset]\nparent-3-open\n"
    );
}

#[test]
fn issue2272_fdvar_missing_input_file_aborts_like_gnu() {
    let dir = scratch("missing");
    let script = format!(
        "unset v\n{} {{v}}</missing-nope\necho \"rc=$? v=[${{v-unset}}]\"\n",
        child("echo should-not-run")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1 v=[unset]\n");
    assert!(
        stderr.contains("/missing-nope"),
        "stderr should name the missing path, got: {stderr}"
    );
}

#[test]
fn issue2272_fdvar_pipeline_stage_delivers() {
    let dir = scratch("stage");
    let script = format!(
        "unset v\n{} {{v}}<seed.txt | cat\necho \"PIPE: ${{PIPESTATUS[*]}}\"\n",
        child("cat <&10")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "seed-line-1\nseed-line-2\nPIPE: 0 0\n");
}

#[test]
fn issue2272_fdvar_pipeline_stage_heredoc_delivers() {
    let dir = scratch("stagehd");
    let script = format!(
        "unset v\n{} {{v}}<<EOF | cat\nstage body\nEOF\necho \"PIPE: ${{PIPESTATUS[*]}}\"\n",
        child("cat <&10")
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "stage body\nPIPE: 0 0\n");
}

#[test]
fn issue2272_fdvar_pipeline_stage_missing_file_fails_element_only() {
    let dir = scratch("stagemissing");
    let script = format!(
        "unset v\n{} {{v}}</missing-nope | cat\necho \"PIPE: ${{PIPESTATUS[*]}}\"\n",
        child("cat <&10")
    );
    let (stdout, _stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "PIPE: 1 0\n");
}

#[test]
fn issue2272_numbered_fd_control_still_delivers() {
    let dir = scratch("control");
    let script = format!("{} 3<seed.txt\n", child("cat <&3"));
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "seed-line-1\nseed-line-2\n");
}

#[test]
fn issue2272_fdvar_builtin_binding_still_persists() {
    let dir = scratch("builtin");
    // In-process faces keep GNU's persistent {var} semantics: the
    // binding and the variable survive a builtin command, and a
    // close form on a builtin does not destroy the parent's
    // ambient binding either (the close is scoped to the command).
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "unset v\necho hi {v}<seed.txt\necho \"v=[${v-unset}]\"\ncat <&10\nexec 9<amb.txt\nv=9\necho hi {v}<&-\nif : <&9; then echo fd9-alive; else echo fd9-dead; fi\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "hi\nv=[10]\nseed-line-1\nseed-line-2\nhi\nfd9-alive\n"
    );
}
