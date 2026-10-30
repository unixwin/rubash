//! rubash#2279 — the ENOEXEC script-child face (execute_direct_shell_
//! script: a text file the kernel refuses to exec, run in-process as a
//! fresh shell, GNU execute_cmd.c:6237-6260 shell_execve ENOEXEC tail)
//! scoped the command's file descriptors wrong in three ways:
//!
//! (i) a `{var}` fd_var binding leaked into the parent: the funnel's
//! apply_dynamic_fd_var_redirects records its undo in single-slot
//! lists, and the first simple command the child ran re-entered the
//! applier and cleared them, so execute_external's paired
//! undo_child_fd_var_redirects replayed nothing. GNU's redir_varassign
//! (redir.c:1133-1166) runs in the forked child under do_redirections
//! (execute_cmd.c:5884) and dies with it. An empty script leaked
//! nothing — no inner command ever cleared the lists.
//! (ii)+(iii) the command's NUMBERED redirects never bound at all:
//! only numbered outputs (open_compound_output_redirects) and fd 0
//! were installed, so `./reader.sh 3<seed` ran with fd 3 unbound
//! (EBADF on read) — and a parent fd 3 stayed unshadowed, so the
//! child read and consumed the PARENT's binding. GNU applies the
//! whole redirect list in the child (redir.c:767-955
//! do_redirection_internal). The face now binds the numbered
//! heredocs and inputs through the compound binders
//! (bind_numbered_heredoc_redirects /
//! open_compound_numbered_input_redirects) for the run's duration and
//! restores them at the boundary, and parks the caller's fd_var
//! delivery records across the run so the paired undo still lands.
//!
//! Companion fix in the same cluster: a DISK command whose redirects
//! are rejected in execute_materialized_command never reaches
//! execute_external, so its speculative `{var}` apply was never
//! undone either (`/bin/true {v}<seed < /missing` leaked v). The
//! rejection path now undoes for the disk class only — GNU keeps the
//! binding for builtins/functions, whose varassign lives in the
//! shell's own context (verified: `: {v}<seed < /missing` leaves v
//! set in GNU 5.3.20).
//!
//! Expectations byte-verified against GNU bash 5.3.20 with the probe
//! scripts in the cluster workdir (probes/p1..p9, e1, t2). Observers
//! are the script's own builtin reads (the face under test reads its
//! own fd table), the nested rubash binary, or the shell's own later
//! reads — never another ENOEXEC script child observing a spawn
//! delivery (that harness defect is rubash#2270/#2286's subject).
//!
//! Known cross-face gap NOT pinned here (predates this issue, hits
//! compound/function faces identically): an ambient heredoc Text
//! endpoint bound by an enclosing face is not materialized for a
//! spawned grandchild (`{ ./py.sh; } 3<<EOF` — the grandchild sees
//! EBADF in GNU-verified divergence on every in-process face; File
//! endpoints deliver fine, see the ambient control pin below).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2279-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    fs::write(&path, body).expect("write fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if name.ends_with(".sh") {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod fixture");
        }
    }
}

/// Run `script` as a `-c` string in `dir` with stdin closed (null).
fn rubash_in(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(RUBASH)
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Run `script` as a `-c` string in `dir` with `stdin_text` as the
/// shell's own stdin (the ambient descriptor the dup pins resolve
/// against), written to a real file so offsets are shared kernel
/// state exactly as under GNU.
fn rubash_in_with_stdin(
    dir: &Path,
    script: &str,
    stdin_text: &str,
) -> (String, String, Option<i32>) {
    let stdin_path = dir.join("runner-stdin.txt");
    let mut file = fs::File::create(&stdin_path).expect("create stdin file");
    file.write_all(stdin_text.as_bytes()).expect("write stdin");
    drop(file);
    let stdin = fs::File::open(&stdin_path).expect("open stdin file");
    let output = Command::new(RUBASH)
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .stdin(Stdio::from(stdin))
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const READER3: &str =
    "line=\"INIT\"\nIFS= read -r -u 3 line\necho \"script: rc=$? line=[$line]\"\n";

#[test]
fn issue2279_numbered_heredoc_visible_inside_script() {
    let dir = scratch("heredoc3");
    write(&dir, "reader3.sh", READER3);
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "./reader3.sh 3<<EOF\nheredoc-body-line\nEOF\necho \"parent: rc=$?\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "script: rc=0 line=[heredoc-body-line]\nparent: rc=0\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2279_numbered_file_visible_inside_script() {
    let dir = scratch("file3");
    write(&dir, "reader3.sh", READER3);
    write(&dir, "seed.txt", "seed-line\n");
    let (stdout, stderr, code) =
        rubash_in(&dir, "./reader3.sh 3<seed.txt\necho \"parent: rc=$?\"\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "script: rc=0 line=[seed-line]\nparent: rc=0\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2279_dup_input_binds_pre_command_stdin() {
    // GNU list order: `3<&0` dups the stdin as it was BEFORE the
    // command's own `<seed.txt` replaces fd 0.
    let dir = scratch("dupnum");
    write(&dir, "reader3.sh", READER3);
    write(&dir, "seed.txt", "seed-line\n");
    let (stdout, stderr, code) = rubash_in_with_stdin(
        &dir,
        "./reader3.sh 3<&0 <seed.txt\necho \"parent: rc=$?\"\n",
        "runner-stdin-line\nrunner-stdin-line-2\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "script: rc=0 line=[runner-stdin-line]\nparent: rc=0\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2279_dup_input_shares_ambient_offset() {
    // A dup shares the open file description (GNU redir.c dup2): the
    // script's read advances the parent's stdin cursor.
    let dir = scratch("dupamb");
    write(&dir, "reader3.sh", READER3);
    let (stdout, stderr, code) = rubash_in_with_stdin(
        &dir,
        "./reader3.sh 3<&0\necho \"parent: rc=$?\"\nline=\"INIT\"\nIFS= read -r line\necho \"parent-stdin-after: rc=$? line=[$line]\"\n",
        "runner-stdin-line\nrunner-stdin-line-2\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "script: rc=0 line=[runner-stdin-line]\nparent: rc=0\nparent-stdin-after: rc=0 line=[runner-stdin-line-2]\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2279_fdvar_does_not_leak_variable_or_descriptor() {
    let dir = scratch("fdvar");
    write(
        &dir,
        "readerv.sh",
        "echo \"inside: v=[${v-unset}]\"\nline=\"INIT\"\nIFS= read -r -u 10 line\necho \"script-fd10: rc=$? line=[$line]\"\n",
    );
    write(&dir, "seed.txt", "seed-A\nseed-B\n");
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "unset v\n./readerv.sh {v}<seed.txt\necho \"parent: rc=$? v=[${v-unset}]\"\nline=\"INIT\"\nIFS= read -r -u 10 line\necho \"parent-fd10-after: rc=$? line=[$line]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "inside: v=[unset]\nscript-fd10: rc=0 line=[seed-A]\nparent: rc=0 v=[unset]\nparent-fd10-after: rc=1 line=[INIT]\n",
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("10: invalid file descriptor"),
        "parent fd 10 must be closed after the child (GNU EBADF): {stderr}"
    );
}

#[test]
fn issue2279_fdvar_heredoc_does_not_leak() {
    let dir = scratch("fdvarhd");
    write(
        &dir,
        "readerv.sh",
        "line=\"INIT\"\nIFS= read -r -u 10 line\necho \"script-fd10: rc=$? line=[$line]\"\n",
    );
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "unset v\n./readerv.sh {v}<<EOF\nfdvar-heredoc-line\nEOF\necho \"parent: rc=$? v=[${v-unset}]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "script-fd10: rc=0 line=[fdvar-heredoc-line]\nparent: rc=0 v=[unset]\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2279_fdvar_preset_value_restored() {
    let dir = scratch("fdvarpreset");
    write(&dir, "readerv.sh", "echo \"inside: v=[${v-unset}]\"\n");
    write(&dir, "seed.txt", "seed-A\n");
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "v=PRESET\n./readerv.sh {v}<seed.txt\necho \"parent: rc=$? v=[${v-unset}]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "inside: v=[unset]\nparent: rc=0 v=[PRESET]\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2279_parent_fd_shadowed_and_restored() {
    // The command's `3<seed.txt` shadows the parent's fd 3 for the
    // run; the parent's own binding (and its unread offset) survives.
    let dir = scratch("shadow");
    write(&dir, "reader3.sh", READER3);
    write(&dir, "seed.txt", "seed-line\n");
    write(&dir, "amb.txt", "amb-line\n");
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "exec 3<amb.txt\n./reader3.sh 3<seed.txt\necho \"parent: rc=$?\"\nline=\"INIT\"\nIFS= read -r -u 3 line\necho \"parent-fd3-after: rc=$? line=[$line]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "script: rc=0 line=[seed-line]\nparent: rc=0\nparent-fd3-after: rc=0 line=[amb-line]\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2279_rejected_disk_command_fdvar_undone() {
    let dir = scratch("reject");
    write(&dir, "reader.sh", "exit 0\n");
    write(&dir, "seed.txt", "seed-line\n");
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "unset v\n/bin/true {v}<seed.txt < /nonexistent-xyz\necho \"spawn-fail: rc=$? v=[${v-unset}]\"\nunset v\n./reader.sh {v}<seed.txt < /nonexistent-xyz\necho \"script-fail: rc=$? v=[${v-unset}]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "spawn-fail: rc=1 v=[unset]\nscript-fail: rc=1 v=[unset]\n",
        "stderr: {stderr}"
    );
    assert_eq!(
        stderr
            .matches("/nonexistent-xyz: No such file or directory")
            .count(),
        2,
        "stderr: {stderr}"
    );
}

#[test]
fn issue2279_control_builtin_fdvar_persists() {
    // GNU in-process class: the varassign persists in the shell's own
    // context — on success AND when a later redirect fails. These are
    // controls: the disk-class undo above must not touch them.
    let dir = scratch("builtinctl");
    write(&dir, "seed.txt", "seed-line\n");
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "unset v\n: {v}<seed.txt\necho \"builtin-ok: rc=$? v=[${v-unset}]\"\nunset v\n: {v}<seed.txt < /nonexistent-xyz\necho \"builtin-fail: rc=$? v=[${v-unset}]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    // The first bind's descriptor stays open (persistent class), so
    // the second bind allocates the next free slot — GNU 5.3.20 reads
    // v=[11] for the same script.
    assert_eq!(
        stdout, "builtin-ok: rc=0 v=[10]\nbuiltin-fail: rc=1 v=[11]\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2279_control_spawn_face_fdvar() {
    // The spawn face's pairing (apply -> execute_external undo) was
    // already GNU-correct; the nested rubash binary is the observer.
    let dir = scratch("spawnctl");
    write(&dir, "seed.txt", "seed-A\nseed-B\n");
    let script = format!(
        "unset v\n\"{RUBASH}\" -c 'echo \"ctrl-inside: v=[${{v-unset}}]\"' {{v}}<seed.txt\necho \"parent: rc=$? v=[${{v-unset}}]\"\n"
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "ctrl-inside: v=[unset]\nparent: rc=0 v=[unset]\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2279_control_stdin_faces_still_work() {
    let dir = scratch("stdinctl");
    write(
        &dir,
        "reader0.sh",
        "line=\"INIT\"\nIFS= read -r line\necho \"script: rc=$? line=[$line]\"\n",
    );
    write(&dir, "seed.txt", "seed-line\n");
    let (stdout, stderr, code) = rubash_in(
        &dir,
        "./reader0.sh <<EOF\nstdin-body-line\nEOF\necho \"parent: rc=$?\"\n./reader0.sh <seed.txt\necho \"parent: rc=$?\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "script: rc=0 line=[stdin-body-line]\nparent: rc=0\nscript: rc=0 line=[seed-line]\nparent: rc=0\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2279_ambient_file_fd_reaches_script_children() {
    // A File endpoint bound by the command's `3<seed.txt` is a real
    // descriptor for the child's whole process tree: a process the
    // script spawns (the nested rubash binary) inherits it, exactly
    // like GNU's fork copies the descriptor.
    let dir = scratch("ambient");
    write(&dir, "seed.txt", "seed-line\n");
    let script = "./spawner.sh 3<seed.txt\n";
    write(
        &dir,
        "spawner.sh",
        &format!(
            "\"{RUBASH}\" -c 'line=\"INIT\"; IFS= read -r -u 3 line; echo \"child: rc=$? line=[$line]\"'\n"
        ),
    );
    let (stdout, stderr, code) = rubash_in(&dir, &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "child: rc=0 line=[seed-line]\n", "stderr: {stderr}");
}
