//! rubash#2522 (wave-3 residual) — a `{var}` redirection inside a
//! `<(...)` process-substitution body lost its REDIR_WORD identity in the
//! producer: the body is re-printed by joining tokens with spaces
//! (`process_substitution_source`), which turned `hostname 9>o6.txt
//! {w}>&9` into `hostname 9> o6.txt {w} >& 9`. The producer respawn
//! (`rubash -c`, the port of subst.c:6362 process_substitute's forked
//! child) re-parsed that re-print, so `{w}` became a literal argument and
//! `>&9` a plain fd-1 dup: the external child's fd 1 was wired to the dup
//! source (its stdout vanished into the `9>` file) and the `{w}` varfd
//! descriptor was never allocated (`>&10` → EBADF). The wave-3 headline
//! `cat < <(CHILD reader.sh 9>o1.txt {w}>&9 {v}>&-)` printed
//! `rc=0 o1=[]` where GNU prints `w10=OK` / `got4=[]` / `rc=0 o1=[W10]`.
//!
//! GNU owners: the body string handed to the forked producer is VERBATIM
//! (subst.c:6362 process_substitute → parse_and_execute; the span is
//! pulled from the word by subst.c:1311 extract_process_subst), and the
//! one spacing-semantic boundary is the REDIR_WORD adjacency —
//! parse.y:5826-5837 read_token_word returns REDIR_WORD only when the
//! '<'/‘>' immediately follows the `{name}` word; GNU's own printer
//! re-prints every REDIR_VARASSIGN spelling glued (print_cmd.c:1237-1297:
//! `{%s}>&%d` at :1244, `{%s}>&%s` at :1260, `{%s}>&-` at :1297). The fix
//! reuses the parser's own column-adjacency predicate
//! (redirect_fd_var_prefix) so the re-print keeps that boundary; every
//! other boundary keeps its space (operator→target spacing is
//! re-parse-neutral: `>& -` is still r_close_this in both shells).
//!
//! Expectations byte-verified against WSL GNU Bash 5.3.20 (script-file
//! probes; evidence under D:/repo/nbbuild/evidence/procsub-residual/).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2522-procsub-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &Path, name: &str, body: &str) {
    fs::write(dir.join(name), body).expect("write fixture");
}

/// Run `driver.sh` (with `@RUBASH@` replaced by this build's binary path)
/// as the outer shell's main script, stdin null.
fn run_driver(dir: &Path, driver: &str) -> (String, String, Option<i32>) {
    write(dir, "driver.sh", &driver.replace("@RUBASH@", RUBASH));
    let output = Command::new(RUBASH)
        .arg("driver.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash driver");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Wave-3 headline (pc1b): a procsub body whose external child carries a
/// close-bearing `{v}>&-` list. GNU: the reader's reports reach the
/// procsub pipe (`w10=OK`, `got4=[]`), the `{w}>&9` descriptor writes W10
/// into o1.txt, and the only diagnostic is the reader's own fd-4 EBADF
/// (fd 4 is not inherited into the procsub body). Pre-fix: the re-print
/// split `{w}>&9` and `{v}>&-`, so fd 1 was closed by the misread `>& -`,
/// fd 10 never existed, and stdout printed only `rc=0 o1=[]`.
#[test]
fn procsub_body_varfd_close_list_delivered() {
    let dir = scratch("headline");
    write(
        &dir,
        "reader.sh",
        "if echo W10 >&10; then echo \"w10=OK\"; else echo \"w10=FAIL\"; fi\nline=$(cat <&4 2>&1)\necho \"got4=[$line]\"\n",
    );
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9>seedout.txt\nv=9\ncat < <(\"@RUBASH@\" reader.sh 9>o1.txt {w}>&9 {v}>&-)\necho \"rc=$? o1=[$(cat o1.txt)]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "w10=OK\ngot4=[]\nrc=0 o1=[W10]\n");
    assert_eq!(
        stderr, "reader.sh: line 2: 4: Bad file descriptor\n",
        "the reader's own fd-4 diagnostic is the only stderr"
    );
}

/// Minimal dup face: `{w}>&9` alone (no close entry) inside the procsub
/// body. GNU: `B` rides the pipe to the consumer, `A` lands in the `9>`
/// file through the `{w}` descriptor. Pre-fix: fd 1 crossed onto the `9>`
/// file (`o=[B]`) and `>&10` failed EBADF inside the child.
#[test]
fn procsub_body_varfd_dup_external_child() {
    let dir = scratch("dup-face");
    let (stdout, stderr, code) = run_driver(
        &dir,
        "cat < <(\"@RUBASH@\" -c 'echo A >&10; echo B' 9>o.txt {w}>&9)\necho \"rc=$? o=[$(cat o.txt)]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "B\nrc=0 o=[A]\n");
    assert_eq!(stderr, "");
}

/// Foreign child face: the miswiring was OS-level (the spawn's stdout
/// handle), not an env-stamp artifact — a foreign child (`hostname`
/// here) must keep its fd 1 on the procsub pipe while the `{w}` varfd
/// leaves the `9>` file empty (nothing writes fd 9).
#[test]
fn procsub_body_varfd_dup_foreign_child_stdout_not_crossed() {
    let dir = scratch("foreign");
    let (stdout, stderr, code) = run_driver(
        &dir,
        "host=$(hostname)\ngot=$(cat < <(hostname 9>o.txt {w}>&9))\n[ \"$got\" = \"$host\" ] && echo \"same=true\" || echo \"same=false\"\necho \"o=[$(cat o.txt)]\"\n",
    );
    assert_eq!(code, Some(0));
    // hostname's output rode the procsub pipe to the consumer; o.txt
    // stays empty (nothing writes fd 9); no diagnostics.
    assert_eq!(stdout, "same=true\no=[]\n");
    assert_eq!(stderr, "");
}

/// The fix must PRESERVE original spacing, not glue every `{name}` word:
/// a deliberately spaced `{w} >&9` in the body stays the WORD `{w}` plus
/// a plain fd-1 `>&9` dup — the child's stdout lands in the `9>` file in
/// both shells (GNU parse.y:5826 requires the operator to directly follow
/// the `}`; a space defeats REDIR_WORD).
#[test]
fn procsub_body_spaced_var_stays_word_and_fd1_dup() {
    let dir = scratch("spaced");
    let (stdout, stderr, code) = run_driver(
        &dir,
        "cat < <(\"@RUBASH@\" -c 'echo S' 9>s.txt {w} >&9)\necho \"rc=$? s=[$(cat s.txt)]\"\n",
    );
    assert_eq!(code, Some(0));
    // S goes to the dup'd fd 9 file, nothing rides the pipe.
    assert_eq!(stdout, "rc=0 s=[S]\n");
    assert_eq!(stderr, "");
}

/// `{var}` FILE open inside the body (`{q}>q.txt`): the allocated
/// descriptor exists for the body's own shell, the command's stdout still
/// rides the procsub pipe, and the file is created empty.
#[test]
fn procsub_body_varfd_file_open() {
    let dir = scratch("file-open");
    let (stdout, stderr, code) = run_driver(
        &dir,
        "cat < <(\"@RUBASH@\" -c 'echo V' {q}>q.txt)\necho \"rc=$? q=[$(cat q.txt)]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "V\nrc=0 q=[]\n");
    assert_eq!(stderr, "");
}

/// Command-substitution control (wave-3 pc1c): the identical list through
/// `$()` was already correct — the comsub path does not run the procsub
/// token-join re-print. Pins the family boundary.
#[test]
fn comsub_body_varfd_close_list_untouched() {
    let dir = scratch("comsub");
    write(
        &dir,
        "reader.sh",
        "if echo W10 >&10; then echo \"w10=OK\"; else echo \"w10=FAIL\"; fi\nline=$(cat <&4 2>&1)\necho \"got4=[$line]\"\n",
    );
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9>seedout.txt\nv=9\nx=$(\"@RUBASH@\" reader.sh 9>o1.txt {w}>&9 {v}>&-)\necho \"rc=$? x=[$x] o1=[$(cat o1.txt)]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 x=[w10=OK\ngot4=[]] o1=[W10]\n");
    assert_eq!(stderr, "reader.sh: line 2: 4: Bad file descriptor\n");
}

/// The consumer-side fd juggling variant (wave-3 pc1): the procsub pipe
/// opened at fd 4 first, then dup'd to stdin — the reader's completion
/// must still order before the parent continues.
#[test]
fn procsub_body_varfd_with_pipe_at_fd4() {
    let dir = scratch("fd4");
    write(
        &dir,
        "reader.sh",
        "if echo W10 >&10; then echo \"w10=OK\"; else echo \"w10=FAIL\"; fi\nline=$(cat <&4 2>&1)\necho \"got4=[$line]\"\n",
    );
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9>seedout.txt\nv=9\ncat 4< <(echo PSIN; \"@RUBASH@\" reader.sh 9>o1.txt {w}>&9 {v}>&-) <&4\necho \"rc=$? o1=[$(cat o1.txt)]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "PSIN\nw10=OK\ngot4=[]\nrc=0 o1=[W10]\n");
    assert_eq!(stderr, "reader.sh: line 2: 4: Bad file descriptor\n");
}
