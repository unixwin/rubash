//! Issue unixwin/rubash#2256: opening a regular file through its
//! `/dev/fd/N` alias spelling must be a FRESH open (new open file
//! description, offset 0), not a dup of the descriptor.
//!
//! GNU's alias open is the plain open() of the magic link —
//! redir.c's RF_DEVFD dup special case (`_redir_special_filenames`,
//! redir.c:603; the F_DUPFD serve at redir.c:629) compiles only
//! without HAVE_DEV_FD, and the oracle build defines it. So the
//! endpoint's KIND decides: a regular file re-opens its target
//! (for reading AND for writing — the source descriptor's mode and
//! offset are irrelevant), while pipes/sockets/devices/byte
//! channels cannot be re-opened and keep the dup/stream model.
//! The `<&N` / `>&N` instructions are dup2 (make_cmd.c r_dup) and
//! stay shared-offset on every endpoint kind.
//!
//! Expectations below are the GNU Bash 5.3.20 outputs of the probe
//! scripts in the cluster workdir (probes/probe-2256*.sh, run
//! against the vendored oracle build). Pins run rubash as a
//! subprocess and observe stdout/stderr/files only.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SEED: &str = "alpha\nbeta\ngamma\n";

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2256-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn collect(child: std::process::Child) -> (String, String, Option<i32>) {
    let mut child = child;
    let mut out = child.stdout.take().expect("stdout pipe");
    let mut err = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut err, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let status = child.wait().expect("wait rubash");
    (
        out_thread.join().unwrap_or_default(),
        err_thread.join().unwrap_or_default(),
        status.code(),
    )
}

/// Run `script` as rubash's stdin inside `dir` (seed pre-written).
fn run_script(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    std::fs::write(dir.join("seed"), SEED).expect("write seed");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write script");
    drop(child.stdin.take());
    collect(child)
}

/// Run `rubash -c script` in `dir` with the process stdin bound to
/// the seed FILE (the fd-0 alias cells).
fn run_with_file_stdin(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    std::fs::write(dir.join("seed"), SEED).expect("write seed");
    let stdin_file = std::fs::File::open(dir.join("seed")).expect("open seed as stdin");
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(dir)
        .arg("-c")
        .arg(script)
        .stdin(Stdio::from(stdin_file))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -c");
    collect(child)
}

fn wait_file_content(path: &Path, want: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if std::fs::read_to_string(path).is_ok_and(|got| got == want) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::fs::read_to_string(path).is_ok_and(|got| got == want)
}

#[test]
fn issue2256_operand_double_read_is_fresh() {
    // GNU R1: `cat /dev/fd/6` twice prints the file twice.
    let dir = test_dir("operand-double");
    let (stdout, stderr, code) = run_script(&dir, "exec 6<seed\ncat /dev/fd/6\ncat /dev/fd/6\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("{SEED}{SEED}"), "stderr: {stderr}");
}

#[test]
fn issue2256_redirect_double_read_is_fresh() {
    // GNU R2: `cat < /dev/fd/6` twice prints the file twice.
    let dir = test_dir("redirect-double");
    let (stdout, stderr, code) = run_script(&dir, "exec 6<seed\ncat </dev/fd/6\ncat </dev/fd/6\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("{SEED}{SEED}"), "stderr: {stderr}");
}

#[test]
fn issue2256_dup_instruction_stays_shared() {
    // GNU R3 (control): `<&6` is dup2 — the second reader sees EOF.
    let dir = test_dir("dup-control");
    let (stdout, stderr, code) = run_script(&dir, "exec 6<seed\ncat <&6\ncat <&6\necho done\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("{SEED}done\n"), "stderr: {stderr}");
}

#[test]
fn issue2256_alias_open_leaves_source_offset() {
    // GNU R4: a partial alias read does not move fd 6 — the dup
    // afterwards still serves the whole file from offset 0.
    let dir = test_dir("offset-untouched");
    let (stdout, stderr, code) =
        run_script(&dir, "exec 6<seed\nhead -c 5 /dev/fd/6\necho\ncat <&6\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("alpha\n{SEED}"), "stderr: {stderr}");
}

#[test]
fn issue2256_readwrite_fd_alias_is_fresh() {
    // GNU R5: a `<>`-bound fd's alias opens are fresh too, and the
    // descriptor's own offset stays at 0 for the later dup.
    let dir = test_dir("readwrite-fresh");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<>seed\ncat /dev/fd/6\ncat /dev/fd/6\ncat <&6\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("{SEED}{SEED}{SEED}"), "stderr: {stderr}");
}

#[test]
fn issue2256_stdin_file_operand_is_fresh() {
    // GNU R6: with stdin a regular file, `cat /dev/fd/0` twice
    // prints the file twice.
    let dir = test_dir("stdin-operand");
    let (stdout, stderr, code) = run_with_file_stdin(&dir, "cat /dev/fd/0; cat /dev/fd/0");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("{SEED}{SEED}"), "stderr: {stderr}");
}

#[test]
fn issue2256_devstdin_redirect_is_fresh() {
    // GNU R6b: `cat < /dev/stdin` twice on a file stdin likewise.
    let dir = test_dir("stdin-redirect");
    let (stdout, stderr, code) = run_with_file_stdin(&dir, "cat </dev/stdin; cat </dev/stdin");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("{SEED}{SEED}"), "stderr: {stderr}");
}

#[test]
fn issue2256_pipe_endpoint_stays_stream() {
    // GNU R7: a pipe endpoint cannot be re-opened — the alias open
    // attaches to the stream, so the second read sees only EOF.
    let dir = test_dir("pipe-stream");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6< <(printf 'l1\\nl2\\nl3\\n')\ncat /dev/fd/6\necho --\ncat /dev/fd/6\necho end\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "l1\nl2\nl3\n--\nend\n", "stderr: {stderr}");
}

#[test]
fn issue2256_directory_endpoint_read_fails_eisdir() {
    // GNU R8: the alias open of a directory succeeds; the READ
    // fails EISDIR on both the operand and the redirect face.
    let dir = test_dir("directory");
    std::fs::create_dir_all(dir.join("sub")).expect("mkdir sub");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<sub\ncat /dev/fd/6\necho op=$?\ncat </dev/fd/6\necho redir=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "op=1\nredir=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("Is a directory"),
        "stderr should report EISDIR, got: {stderr}"
    );
}

#[test]
fn issue2256_closed_fd_alias_errors() {
    // GNU R9: a dangling alias reports ENOENT on both faces.
    let dir = test_dir("closed");
    let (stdout, stderr, code) = run_script(
        &dir,
        "cat /dev/fd/9\necho op=$?\ncat </dev/fd/9\necho redir=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "op=1\nredir=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr should report ENOENT, got: {stderr}"
    );
}

#[test]
fn issue2256_write_alias_truncates_read_fds_file() {
    // GNU W1: `echo X > /dev/fd/6` on a READ-bound fd still opens
    // the target file O_TRUNC — the descriptor's mode is irrelevant
    // to the alias open.
    let dir = test_dir("write-trunc");
    let (stdout, stderr, code) = run_script(&dir, "exec 6<seed\necho X > /dev/fd/6\ncat seed\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "X\n", "stderr: {stderr}");
}

#[test]
fn issue2256_write_alias_keeps_source_offset() {
    // GNU W2: the alias write re-opens (truncates) independently;
    // fd 6's own offset (4 after `printf zzzz`) survives, so the
    // later `printf Q >&6` lands at offset 4 of the truncated file.
    let dir = test_dir("write-offset");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6>out\nprintf zzzz >&6\necho a > /dev/fd/6\nprintf Q >&6\nset -- $(wc -c < out); echo $1\ncat out\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "5\na\n\0\0Q", "stderr: {stderr}");
}

#[test]
fn issue2256_append_alias_appends() {
    // GNU W3: `>> /dev/fd/6` appends, independently of fd 6.
    let dir = test_dir("append");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6>out\necho a >> /dev/fd/6\necho b >> /dev/fd/6\ncat out\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "a\nb\n", "stderr: {stderr}");
}

#[test]
fn issue2256_noclobber_refuses_alias_trunc() {
    // GNU W4: `set -C` refuses `> /dev/fd/6` naming the spelling,
    // and the file is preserved.
    let dir = test_dir("noclobber");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<seed\nset -C\necho x > /dev/fd/6\necho rc=$?\ncat seed\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("rc=1\n{SEED}"), "stderr: {stderr}");
    assert!(
        stderr.contains("cannot overwrite existing file"),
        "stderr should carry the noclobber refusal, got: {stderr}"
    );
}

#[test]
fn issue2256_noclobber_force_and_append_allowed() {
    // GNU W4b: `>|` forces and `>>` appends under `set -C`.
    let dir = test_dir("noclobber-force");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<seed\nset -C\necho x >| /dev/fd/6\necho y >> /dev/fd/6\ncat seed\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "x\ny\n", "stderr: {stderr}");
}

#[test]
fn issue2256_readwrite_spelling_open_reads_fresh() {
    // GNU R10: `cat <> /dev/fd/6` opens the target O_RDWR fresh.
    let dir = test_dir("rw-spelling");
    let (stdout, stderr, code) = run_script(&dir, "exec 6<seed\ncat <> /dev/fd/6\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, SEED, "stderr: {stderr}");
}

#[test]
fn issue2256_append_fd_readback_is_fresh() {
    // GNU R11 (control for the #1963 write-endpoint branch): a
    // write-bound fd's target re-opens for reading from offset 0.
    let dir = test_dir("append-readback");
    let (stdout, stderr, code) = run_script(&dir, "exec 6>>seed\ncat /dev/fd/6\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, SEED, "stderr: {stderr}");
}

#[test]
fn issue2256_procsub_write_alias_delivers() {
    // GNU R12: writing through a procsub write end's alias reaches
    // the consumer (pipe endpoints keep the attach/dup model).
    let dir = test_dir("procsub-write");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 7> >(cat > cap)\necho hi > /dev/fd/7\nexec 7>&-\nsleep 0.5\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(
        wait_file_content(&dir.join("cap"), "hi\n"),
        "cap should hold the alias-written line"
    );
}

#[test]
fn issue2256_heredoc_fd_consumed_once() {
    // GNU R13: a heredoc (byte-channel) endpoint is a stream — the
    // second alias read sees EOF, exactly the dup model.
    let dir = test_dir("heredoc");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<<EOF\nbody-line\nEOF\ncat /dev/fd/6\ncat /dev/fd/6\necho end\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "body-line\nend\n", "stderr: {stderr}");
}

#[test]
fn issue2256_source_dev_fd_runs_twice() {
    // GNU F1: `source /dev/fd/6` re-opens the file each time.
    let dir = test_dir("source");
    std::fs::write(dir.join("lib.sh"), "echo from-lib\n").expect("write lib");
    let (stdout, stderr, code) =
        run_script(&dir, "exec 6<lib.sh\nsource /dev/fd/6\nsource /dev/fd/6\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "from-lib\nfrom-lib\n", "stderr: {stderr}");
}

#[test]
fn issue2256_read_alias_repeats_first_line() {
    // GNU F2: each `< /dev/fd/6` read starts at offset 0; the
    // closing `<&6` dup then still sees the file from offset 0
    // because the alias opens never moved fd 6.
    let dir = test_dir("read-alias");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<seed\nread line < /dev/fd/6; echo \"1:$line\"\nread line < /dev/fd/6; echo \"2:$line\"\nread line <&6; echo \"3:$line\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "1:alpha\n2:alpha\n3:alpha\n", "stderr: {stderr}");
}

#[test]
fn issue2256_mapfile_alias_reads_whole_file_twice() {
    // GNU F3: mapfile over the alias spelling gets the whole file
    // on every invocation.
    let dir = test_dir("mapfile");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<seed\nmapfile -t a < /dev/fd/6; echo \"n1=${#a[@]}\"\nmapfile -t b < /dev/fd/6; echo \"n2=${#b[@]}\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "n1=3\nn2=3\n", "stderr: {stderr}");
}

#[test]
fn issue2256_function_fd0_alias_is_fresh() {
    // GNU F4: a function call's `< /dev/fd/6` binds a fresh fd 0
    // per call; the `<&6` control call shares the (unmoved) stream.
    let dir = test_dir("function");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<seed\nf() { cat; }\nf < /dev/fd/6\nf < /dev/fd/6\necho --\nf <&6\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        format!("{SEED}{SEED}--\n{SEED}"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2256_cat_herestring_alias_output_is_fresh() {
    // GNU W5: an in-shell cat writing through the alias spelling
    // truncates independently; fd 6's offset stays 0, so the
    // following `printf Z >&6` overwrites the first byte.
    let dir = test_dir("cat-alias-out");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6>out\ncat <<<hello > /dev/fd/6\nprintf Z >&6\ncat out\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "Zello\n", "stderr: {stderr}");
}

#[test]
fn issue2256_numbered_bind_then_dup_reads_fresh() {
    // GNU F5: `3< /dev/fd/6` binds fd 3 fresh for the command; the
    // same list's `<&3` then reads that fresh binding from 0.
    let dir = test_dir("numbered-bind");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<seed\ncat 3< /dev/fd/6 <&3\ncat 3< /dev/fd/6 <&3\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("{SEED}{SEED}"), "stderr: {stderr}");
}
