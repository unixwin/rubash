//! rubash#3727: a function call with its own stdin redirect must leave
//! the caller's fd 0 reading the pre-call target from its saved offset
//! — after `f < inner`, `f <<< text`, `f <&3`, `f </dev/tcp/...`,
//! `f <> file` and the `/dev/null` faces, fd 0 is not restored to the
//! outer file.
//!
//! GNU ground truth: a function's redirections are undoable
//! (execute_cmd.c:854 `do_redirections (command->redirects,
//! RX_ACTIVE|RX_UNDOABLE)`, applied by execute_function), and the
//! undo save IS a kernel duplicate of the displaced descriptor
//! (`fcntl (fd, F_DUPFD, SHELL_FD_BASE)`, redir.c:965/1082/1144,
//! recorded by `add_undo_redirect`), so the pre-call descriptor stays
//! alive for the body's duration and `undo_redirections` dups it back.
//! Rubash's `function_call_stdin` (src/executor/function_calls.rs)
//! saved only a bare `Rc` clone of the fd-0 entry at seven sites
//! (:1599 text/heredoc, :1663 fd-alias dup, :1703 /dev/tcp,
//! :1764 `<>` device, :1778 `<` device, :1811 `<>` file, :1825 `<`
//! file), so the bind-time retire (retire_read_arm/retire_write_arm,
//! rubash#3213) closed the very descriptor the restore
//! (`restore_function_call_fd0`, via `FunctionCallStdinBinding`) was
//! about to replay — the restore re-inserted a retired entry, and the
//! next read off fd 0 failed EBADF. Same hazard as #3706 (group
//! redirects), on the function-call stdin face; same fix (kernel-dup
//! snapshot via `FdTable::saved_entry`).
//!
//! Every cell except the no-redirect control is fail-first on the
//! unfixed base (post-call read rc=1, empty line).

use std::io::Write;
use std::process::{Command, Stdio};

fn run_driver(tag: &str, driver: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3727-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let mut file = std::fs::File::create(dir.join("driver.sh")).expect("write driver");
    file.write_all(driver.as_bytes()).expect("write driver");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("driver.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// :1825 (the primitive): `f < inner.txt` must leave fd 0 reading
/// the outer file from its saved offset — o1, then o2, both rc=0.
#[test]
fn regular_file_call_restores_outer_stdin() {
    let (stdout, stderr, rc) = run_driver(
        "regular",
        "printf 'o1\\no2\\no3\\n' > outer.txt\n\
         printf 'i1\\ni2\\n' > inner.txt\n\
         exec 0<outer.txt\n\
         f() { IFS= read -r x; echo \"in-f:[$x]\"; }\n\
         f < inner.txt\n\
         IFS= read -r l1 <&0; r1=$?\n\
         IFS= read -r l2 <&0; r2=$?\n\
         echo \"r1=$r1 l1=[$l1] r2=$r2 l2=[$l2]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "in-f:[i1]\nr1=0 l1=[o1] r2=0 l2=[o2]\n");
}

/// :1811: `f <> inner.txt` (read-write pair, one open file
/// description) restores fd 0 the same way.
#[test]
fn regular_file_readwrite_call_restores_outer_stdin() {
    let (stdout, stderr, rc) = run_driver(
        "readwrite",
        "printf 'o1\\no2\\n' > outer.txt\n\
         printf 'i1\\ni2\\n' > inner.txt\n\
         exec 0<outer.txt\n\
         f() { IFS= read -r x; echo \"in-f:[$x]\"; }\n\
         f <> inner.txt\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo \"r1=$r1 l1=[$l1]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "in-f:[i1]\nr1=0 l1=[o1]\n");
}

/// :1778: `f < /dev/null` (non-regular target) restores fd 0.
#[test]
fn dev_null_call_restores_outer_stdin() {
    let (stdout, stderr, rc) = run_driver(
        "devnull",
        "printf 'o1\\no2\\n' > outer.txt\n\
         exec 0<outer.txt\n\
         f() { echo \"in-f\"; }\n\
         f < /dev/null\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo \"r1=$r1 l1=[$l1]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "in-f\nr1=0 l1=[o1]\n");
}

/// :1764: `f <> /dev/null` (non-regular read-write pair) restores
/// fd 0.
#[test]
fn dev_null_readwrite_call_restores_outer_stdin() {
    let (stdout, stderr, rc) = run_driver(
        "devnullrw",
        "printf 'o1\\no2\\n' > outer.txt\n\
         exec 0<outer.txt\n\
         f() { echo \"in-f\"; }\n\
         f <> /dev/null\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo \"r1=$r1 l1=[$l1]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "in-f\nr1=0 l1=[o1]\n");
}

/// :1599 (heredoc face): `f <<EOF` parks the body as a text endpoint
/// on fd 0 and must restore the outer file afterwards.
#[test]
fn heredoc_call_restores_outer_stdin() {
    let (stdout, stderr, rc) = run_driver(
        "heredoc",
        "printf 'o1\\no2\\n' > outer.txt\n\
         exec 0<outer.txt\n\
         f() { IFS= read -r x; echo \"in-f:[$x]\"; }\n\
         f <<EOF\n\
         hello\n\
         EOF\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo \"r1=$r1 l1=[$l1]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "in-f:[hello]\nr1=0 l1=[o1]\n");
}

/// :1599 (here-string face): `f <<< text` is the same text bind.
#[test]
fn here_string_call_restores_outer_stdin() {
    let (stdout, stderr, rc) = run_driver(
        "herestring",
        "printf 'o1\\no2\\n' > outer.txt\n\
         exec 0<outer.txt\n\
         f() { IFS= read -r x; echo \"in-f:[$x]\"; }\n\
         f <<< \"hello\"\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo \"r1=$r1 l1=[$l1]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "in-f:[hello]\nr1=0 l1=[o1]\n");
}

/// :1663 (dup form): `f <&3` dups fd 3 onto fd 0 for the call and
/// must restore the outer file afterwards.
#[test]
fn fd_alias_call_restores_outer_stdin() {
    let (stdout, stderr, rc) = run_driver(
        "alfdup",
        "printf 'o1\\no2\\n' > outer.txt\n\
         printf 'a\\nb\\n' > f3.txt\n\
         exec 3<f3.txt\n\
         exec 0<outer.txt\n\
         f() { IFS= read -r x; echo \"in-f:[$x]\"; }\n\
         f <&3\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo \"r1=$r1 l1=[$l1]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "in-f:[a]\nr1=0 l1=[o1]\n");
}

/// :1663 (spelling form): `f < /dev/fd/3` binds a fresh open of fd
/// 3's target for the call and must restore the outer file.
#[test]
fn dev_fd_spelling_call_restores_outer_stdin() {
    let (stdout, stderr, rc) = run_driver(
        "devfd",
        "printf 'o1\\no2\\n' > outer.txt\n\
         printf 'a\\nb\\n' > f3.txt\n\
         exec 3<f3.txt\n\
         exec 0<outer.txt\n\
         f() { IFS= read -r x; echo \"in-f:[$x]\"; }\n\
         f < /dev/fd/3\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo \"r1=$r1 l1=[$l1]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "in-f:[a]\nr1=0 l1=[o1]\n");
}

/// :1703: `f </dev/tcp/127.0.0.1/PORT` binds the connected socket
/// for the call and must restore the outer file afterwards.
#[test]
fn tcp_call_restores_outer_stdin() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
    let port = listener.local_addr().expect("listener addr").port();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.write_all(b"netline\n");
        }
    });
    let driver = format!(
        "printf 'o1\\no2\\n' > outer.txt\n\
         exec 0<outer.txt\n\
         f() {{ IFS= read -r x; echo \"in-f:[$x]\"; }}\n\
         f </dev/tcp/127.0.0.1/{port}\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo \"r1=$r1 l1=[$l1]\"\n"
    );
    let (stdout, stderr, rc) = run_driver("tcp", &driver);
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "in-f:[netline]\nr1=0 l1=[o1]\n");
}

/// End-to-end: a `while read` loop whose body calls `f < inner.txt`
/// every iteration must keep reading the outer file — o1, o2, o3.
#[test]
fn loop_call_restores_outer_stdin_each_iteration() {
    let (stdout, stderr, rc) = run_driver(
        "loop",
        "printf 'o1\\no2\\no3\\n' > outer.txt\n\
         printf 'i1\\ni2\\n' > inner.txt\n\
         f() { IFS= read -r x; echo \"in-f:[$x]\"; }\n\
         while IFS= read -r l; do f < inner.txt; echo \"after:[$l]\"; done < outer.txt\n\
         echo \"loop-done\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        "in-f:[i1]\nafter:[o1]\nin-f:[i1]\nafter:[o2]\nin-f:[i1]\nafter:[o3]\nloop-done\n"
    );
}

/// Nesting: `g` calls `f < inner.txt`; the outer file survives both
/// frames' save/restore.
#[test]
fn nested_call_restores_outer_stdin() {
    let (stdout, stderr, rc) = run_driver(
        "nested",
        "printf 'o1\\no2\\n' > outer.txt\n\
         printf 'i1\\ni2\\n' > inner.txt\n\
         exec 0<outer.txt\n\
         f() { IFS= read -r x; echo \"in-f:[$x]\"; }\n\
         g() { f < inner.txt; echo \"in-g\"; }\n\
         g\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo \"r1=$r1 l1=[$l1]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "in-f:[i1]\nin-g\nr1=0 l1=[o1]\n");
}

/// Control: a call with NO redirect inherits fd 0 unchanged (GNU
/// execute_function) — the body consumes o1, the post-call read
/// serves o2. Guards the fix against changing the inherit path.
#[test]
fn plain_call_inherits_outer_stdin_unchanged() {
    let (stdout, stderr, rc) = run_driver(
        "control",
        "printf 'o1\\no2\\n' > outer.txt\n\
         exec 0<outer.txt\n\
         f() { IFS= read -r x; echo \"in-f:[$x]\"; }\n\
         f\n\
         IFS= read -r l1 <&0; r1=$?\n\
         echo \"r1=$r1 l1=[$l1]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "in-f:[o1]\nr1=0 l1=[o2]\n");
}
