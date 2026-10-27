//! rubash#1830 — two engine-side residuals in the `history` builtin's
//! file modes, both measured against GNU bash 5.3.20:
//!
//! 1. Filename-null rc (builtins/history.def history_builtin): with
//!    HISTFILE unset/empty (or an explicitly empty operand) the builtin
//!    prints `history: HISTFILE: parameter null or not set` (or
//!    `history: empty filename`) and returns
//!    `interactive ? EXECUTION_SUCCESS : EXECUTION_FAILURE` — the
//!    `interactive` global (shell.c:143) is 1 for an interactive shell,
//!    so an interactive `history -a` with HISTFILE unset exits 0.
//!    rubash returned 1 on both the session path and the host-provider
//!    path regardless of interactivity.
//! 2. Append diagnostic context (bashhist.c:450 maybe_append_history):
//!    `history -a <missing-dir>/f` reports exactly one
//!    `history: <file>: cannot create: No such file or directory` line
//!    and fails with rc=1; a directory target and `-w` failures stay
//!    silent. On the provider path the io error escaped through `?` as
//!    a bare executor error (`niu: No such file or directory`), losing
//!    the builtin name, the target path and the `cannot create` frame.
//!
//! Test 1 drives the standalone binary's own interactive session
//! (session path) with the repository's pipe-driven `-i` form.
//! Test 2 drives an Executor with a file-backed HistoryProvider (the
//! niubash REPL shape) through run_source, capturing the builtin's
//! stderr through in-script redirection.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1830-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Poll-wait with a hard bound (same form as the bf-c interactive
/// harness): a wedged REPL must fail the test, not hang the suite.
fn wait_bounded(child: &mut Child, timeout: Duration) -> (Vec<u8>, Vec<u8>) {
    let start = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    panic!("child did not exit within {timeout:?}");
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    let mut out = Vec::new();
    let mut err = Vec::new();
    if let Some(mut so) = child.stdout.take() {
        use std::io::Read;
        let _ = so.read_to_end(&mut out);
    }
    if let Some(mut se) = child.stderr.take() {
        use std::io::Read;
        let _ = se.read_to_end(&mut err);
    }
    (out, err)
}

#[test]
fn interactive_histfile_null_rc_and_append_diag_match_gnu() {
    let dir = tempdir("interactive");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).expect("create home");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .current_dir(&dir)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("TERM", "xterm")
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    let input = "\
unset HISTFILE
history -a; echo \"A rc=$?\"
HISTFILE=
history -a; echo \"B rc=$?\"
history -a \"\"; echo \"C rc=$?\"
history -a /nonexistent-dir/f; echo \"D rc=$?\"
mkdir somedir
history -a somedir; echo \"G rc=$?\"
history -w /nonexistent-dir/f; echo \"E rc=$?\"
";
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("feed input");
    let (out, err) = wait_bounded(&mut child, Duration::from_secs(60));
    let stdout = String::from_utf8_lossy(&out).into_owned();
    let stderr = String::from_utf8_lossy(&err).into_owned();
    // Interactive filename-null returns success (history.def); the
    // file-error shapes fail. GNU 5.3.20 interactive reference run of
    // the same lines: A/B/C rc=0, D/G/E rc=1.
    assert_eq!(
        stdout, "A rc=0\nB rc=0\nC rc=0\nD rc=1\nG rc=1\nE rc=1\n",
        "stderr: {stderr}"
    );
    assert_eq!(
        stderr
            .matches("history: HISTFILE: parameter null or not set")
            .count(),
        2,
        "stderr: {stderr}"
    );
    assert_eq!(
        stderr.matches("history: empty filename").count(),
        1,
        "stderr: {stderr}"
    );
    assert_eq!(
        stderr
            .matches("history: /nonexistent-dir/f: cannot create: No such file or directory")
            .count(),
        1,
        "stderr: {stderr}"
    );
    // The directory target (G) and the -w failure (E) print nothing.
    assert_eq!(
        stderr.matches("cannot create").count(),
        1,
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// File-backed HistoryProvider standing in for a host line editor's
/// history (niubash wires reedline's file-backed list here): the file
/// modes do real file IO and surface the raw io errors, exactly like a
/// host implementation.
#[derive(Debug, Default)]
struct FileProvider {
    entries: Vec<String>,
}

impl rubash::history::HistoryProvider for FileProvider {
    fn entries(&mut self) -> std::io::Result<Vec<String>> {
        Ok(self.entries.clone())
    }
    fn clear(&mut self) -> std::io::Result<()> {
        self.entries.clear();
        Ok(())
    }
    fn append(&mut self, command: String) -> std::io::Result<()> {
        self.entries.push(command);
        Ok(())
    }
    fn replace(&mut self, entries: Vec<String>) -> std::io::Result<()> {
        self.entries = entries;
        Ok(())
    }
    fn write_history(&mut self, path: &str) -> std::io::Result<()> {
        let mut out = String::new();
        for entry in &self.entries {
            out.push_str(entry);
            out.push('\n');
        }
        std::fs::write(path, out)
    }
    fn read_history(&mut self, path: &str) -> std::io::Result<()> {
        let body = std::fs::read_to_string(path)?;
        self.entries = body.lines().map(str::to_string).collect();
        Ok(())
    }
    fn append_history(&mut self, path: &str) -> std::io::Result<()> {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        for entry in &self.entries {
            writeln!(file, "{entry}")?;
        }
        Ok(())
    }
    fn read_new_history(&mut self, path: &str) -> std::io::Result<()> {
        let body = std::fs::read_to_string(path)?;
        for line in body.lines() {
            if !self.entries.contains(&line.to_string()) {
                self.entries.push(line.to_string());
            }
        }
        Ok(())
    }
}

#[test]
fn provider_path_histfile_null_rc_and_append_diag_match_gnu() {
    use rubash::executor::Executor;
    use std::cell::RefCell;
    use std::rc::Rc;

    let dir = tempdir("provider");
    let d = dir.to_string_lossy().replace('\\', "/");
    let provider = Rc::new(RefCell::new(FileProvider {
        entries: vec!["echo seed".to_string()],
    }));
    let mut executor = Executor::new();
    executor.set_history_provider(provider.clone());
    // The niubash REPL executor shape: interactive reader session whose
    // shell name is the host's.
    executor.set_env("__RUBASH_INTERACTIVE", "1");
    executor.set_env("__RUBASH_SHELL_NAME", "niu");
    executor.unset_env("HISTFILE");

    let script = format!(
        "\
cd {d}
unset HISTFILE
history -a 2>e_a.txt
echo \"A rc=$?\" >> rc.txt
history -a {d}/nodir/f 2>e_d.txt
echo \"D rc=$?\" >> rc.txt
mkdir {d}/somedir
history -a {d}/somedir 2>e_g.txt
echo \"G rc=$?\" >> rc.txt
history -w {d}/nodir/f 2>e_e.txt
echo \"E rc=$?\" >> rc.txt
history -r {d}/nodir/f 2>e_f.txt
echo \"F rc=$?\" >> rc.txt
"
    );
    let code = rubash::script_driver::run_source(&mut executor, &script, true);
    assert_eq!(code, 0, "the driver run itself must complete");

    let rc = std::fs::read_to_string(dir.join("rc.txt")).expect("rc capture");
    assert_eq!(rc, "A rc=0\nD rc=1\nG rc=1\nE rc=1\nF rc=1\n");
    // The filename-null diagnostic carries the shell + builtin context
    // (GNU builtin_error form), and the interactive return is success.
    let e_a = std::fs::read_to_string(dir.join("e_a.txt")).expect("append-unset stderr");
    assert_eq!(
        e_a, "niu: history: HISTFILE: parameter null or not set\n",
        "append-unset stderr"
    );
    // The append failure is the builtin's own `cannot create` frame —
    // not a bare io error leaking out of the executor.
    let e_d = std::fs::read_to_string(dir.join("e_d.txt")).expect("append-missing stderr");
    assert_eq!(
        e_d,
        format!("niu: history: {d}/nodir/f: cannot create: No such file or directory\n"),
        "append-missing stderr"
    );
    // Directory target and -w failure stay silent (GNU).
    assert_eq!(
        std::fs::read_to_string(dir.join("e_g.txt")).expect("append-dir stderr"),
        ""
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("e_e.txt")).expect("write-missing stderr"),
        ""
    );
    // Read failure: a builtin outcome (rc=1 above) and SILENT —
    // rubash#1852 superseded the #1289-era `cannot open` orientation
    // this pin originally asserted: GNU's read-error report is
    // compiled out (builtins/history.def #if 0 at :303/:322), so the
    // read face prints nothing at all on either path.
    let e_f = std::fs::read_to_string(dir.join("e_f.txt")).expect("read-missing stderr");
    assert_eq!(e_f, "", "read-missing stderr must be empty (rubash#1852)");
    let _ = std::fs::remove_dir_all(&dir);
}
