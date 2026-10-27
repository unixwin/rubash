//! rubash#1852 — `history -r` / `history -n` read failures are SILENT
//! in GNU bash 5.3.20: the read-error report in builtins/history.def
//! (history_builtin) is compiled out — the RFLAG branch wraps
//! `builtin_error ("%s: read error: ...")` in `#if 0` (history.def:303)
//! and the NFLAG branch does the same (history.def:322). A failed read
//! therefore returns EXECUTION_FAILURE with zero stderr bytes, for a
//! missing file (via $HISTFILE or an explicit operand) and for a
//! directory target alike.
//!
//! rubash printed `history: <path>: cannot open: <err>` on both the
//! session path (history_exec.rs) and the host-provider path
//! (job_builtins.rs). The write face is NOT in scope: `-a` still prints
//! its single `cannot create` line on the not-exist branch
//! (bashhist.c:450 maybe_append_history) and `-w` failures stay silent
//! — both controls are asserted below so the silence cannot overreach.
//!
//! Test 1 drives the standalone binary's interactive session (session
//! path); Test 2 drives an Executor with a file-backed HistoryProvider
//! (the niubash REPL shape) through run_source. GNU 5.3.20 oracle
//! matrix behind the expectations (script probes, stderr captured
//! separately): -r/-n on missing file -> rc 1, 0 stderr bytes;
//! -r/-n on a directory -> rc 1, 0 stderr bytes; -w on missing dir ->
//! rc 1, 0 stderr bytes; -a with session lines on missing dir -> rc 1,
//! one `cannot create` line.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1852-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Poll-wait with a hard bound (same form as the issue1830 harness): a
/// wedged REPL must fail the test, not hang the suite.
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
fn issue1852_session_path_read_failures_are_silent() {
    let dir = tempdir("session");
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
HISTFILE=nodir/histfile
history -r; echo \"R rc=$?\"
history -n; echo \"N rc=$?\"
history -r nodir/f; echo \"R2 rc=$?\"
history -n nodir/f; echo \"N2 rc=$?\"
mkdir somedir
history -r somedir; echo \"RD rc=$?\"
history -n somedir; echo \"ND rc=$?\"
history -w nodir/f; echo \"W rc=$?\"
history -a nodir/f; echo \"A rc=$?\"
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
    // Every failure is rc=1 (GNU history.def: result ? FAILURE :
    // SUCCESS); reads and the -w failure print nothing at all.
    assert_eq!(
        stdout, "R rc=1\nN rc=1\nR2 rc=1\nN2 rc=1\nRD rc=1\nND rc=1\nW rc=1\nA rc=1\n",
        "stderr: {stderr}"
    );
    assert_eq!(
        stderr.matches("cannot open").count(),
        0,
        "read failures must be silent (history.def #if 0), stderr: {stderr}"
    );
    // The write-face control survives: exactly one `cannot create`
    // line, from the -a on the not-exist branch.
    assert_eq!(
        stderr
            .matches("history: nodir/f: cannot create: No such file or directory")
            .count(),
        1,
        "stderr: {stderr}"
    );
    assert_eq!(
        stderr.matches("cannot create").count(),
        1,
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// File-backed HistoryProvider standing in for a host line editor's
/// history (same shape as the issue1830 harness): the file modes do
/// real file IO and surface the raw io errors, exactly like a host
/// implementation.
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
fn issue1852_provider_path_read_failures_are_silent() {
    use rubash::executor::Executor;
    use std::cell::RefCell;
    use std::rc::Rc;

    let dir = tempdir("provider");
    let d = dir.to_string_lossy().replace('\\', "/");
    std::fs::create_dir_all(dir.join("somedir")).expect("create somedir");
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
history -r {d}/nodir/f 2>e_r.txt
echo \"R rc=$?\" >> rc.txt
history -n {d}/nodir/f 2>e_n.txt
echo \"N rc=$?\" >> rc.txt
history -r {d}/somedir 2>e_rd.txt
echo \"RD rc=$?\" >> rc.txt
history -n {d}/somedir 2>e_nd.txt
echo \"ND rc=$?\" >> rc.txt
history -w {d}/nodir/f 2>e_w.txt
echo \"W rc=$?\" >> rc.txt
history -a {d}/nodir/f 2>e_a.txt
echo \"A rc=$?\" >> rc.txt
"
    );
    let code = rubash::script_driver::run_source(&mut executor, &script, true);
    assert_eq!(code, 0, "the driver run itself must complete");

    let rc = std::fs::read_to_string(dir.join("rc.txt")).expect("rc capture");
    assert_eq!(rc, "R rc=1\nN rc=1\nRD rc=1\nND rc=1\nW rc=1\nA rc=1\n");
    // Read failures: total silence on the builtin's stderr (GNU
    // history.def #if 0) — not even the builtin frame.
    for name in ["e_r.txt", "e_n.txt", "e_rd.txt", "e_nd.txt", "e_w.txt"] {
        assert_eq!(
            std::fs::read_to_string(dir.join(name)).unwrap_or_else(|_| panic!("read {name}")),
            "",
            "{name} must be empty"
        );
    }
    // The write-face control survives on this path too.
    let e_a = std::fs::read_to_string(dir.join("e_a.txt")).expect("append-missing stderr");
    assert_eq!(
        e_a,
        format!("niu: history: {d}/nodir/f: cannot create: No such file or directory\n"),
        "append-missing stderr"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
