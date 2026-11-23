//! rubash#4151 regressions: redirecting, `touch`ing or `cp`ing to
//! `/dev/tty`, `/dev/stdin`, `/dev/stdout` or `/dev/stderr` created
//! real `CON`, `CONIN$` and `CONOUT$` files in the current directory.
//!
//! Root cause: the Windows arm of `shell_path_to_windows_inner`
//! mapped the four spellings to BARE relative device names
//! (`CONIN$`, `CONOUT$`, `CON`). Consumers anchor relative products
//! at the session cwd (`session_path`) or open them process-cwd
//! relative, and a directory-qualified `C:\dir\CON` is an ordinary
//! file on Windows, not the console device — so the opens created
//! files. The mapping now mints device-namespace forms
//! (`\\.\CONIN$`, `\\.\CONOUT$`, `\\.\CON`): absolute, so no
//! consumer can cwd-join them, and naming the console object itself
//! from any directory. `/dev/null` is the control: its product stays
//! the bare `NUL`, the one device name that still names the device
//! in every directory (and the reported shapes never affected it).
//!
//! The redirect face of the /dev/std* spellings is NOT part of this
//! defect and must not become one: every redirect walk classifies
//! them through `special_paths::fd_alias_target` before any
//! filesystem open (GNU redir.c:601 `_redir_special_filenames` /
//! redir_open over the OS fd-alias layer), so `> /dev/stdout` binds
//! the shell's then-current fd 1 — a pipe when piped. The last test
//! below pins that binding so the funnel fix can never console-pin
//! the std* redirects by accident.
//!
//! The device-file shapes are Windows-gated (the funnel arms are
//! `cfg!(windows)`); on the reporter's Windows 11 host each of them
//! left the named files behind (entries 1 / 3 / 3), while Git Bash
//! 5.3.15 left none. Real-host re-verification on X299 is owed.

use std::process::{Command, Stdio};

/// Run `script` with the built rubash in a fresh scratch directory;
/// return (stdout, stderr, exit code, sorted scratch entry names
/// after the run).
fn rubash_file_entries(script: &str) -> (String, String, Option<i32>, Vec<String>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-dev4151-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let mut entries: Vec<String> = std::fs::read_dir(&dir)
        .expect("read scratch dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "case.sh")
        .collect();
    entries.sort();
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        entries,
    )
}

#[cfg(windows)]
#[test]
fn redirect_to_dev_tty_creates_no_con_file() {
    // Reporter shape 1: `echo x > /dev/tty` wrote `x` into ./CON.
    let (stdout, _stderr, _code, entries) =
        rubash_file_entries("{ echo x > /dev/tty; } 2>/dev/null\nprintf 'done\\n'\n");
    assert_eq!(stdout, "done\n");
    assert_eq!(entries, Vec::<String>::new(), "entries: {entries:?}");
}

#[cfg(windows)]
#[test]
fn readwrite_redirect_to_dev_tty_creates_no_con_file() {
    // The `<>` face: the CON read+write pair bind (niubash#519) must
    // recognize the device-namespace form; pre-fix the joined path
    // was created as a regular file by the readwrite open.
    let (stdout, _stderr, _code, entries) =
        rubash_file_entries("( exec 3<> /dev/tty ) 2>/dev/null\nprintf 'done\\n'\n");
    assert_eq!(stdout, "done\n");
    assert_eq!(entries, Vec::<String>::new(), "entries: {entries:?}");
}

#[cfg(windows)]
#[test]
fn touch_dev_devices_creates_no_files() {
    // Reporter shape 2: the emulated `touch` created CON, CONIN$ and
    // CONOUT$ (the four spellings deduplicate to three products).
    let (stdout, _stderr, _code, entries) = rubash_file_entries(
        "touch /dev/tty /dev/stdin /dev/stdout /dev/stderr 2>/dev/null\nprintf 'done\\n'\n",
    );
    assert_eq!(stdout, "done\n");
    assert_eq!(entries, Vec::<String>::new(), "entries: {entries:?}");
}

#[cfg(windows)]
#[test]
fn cp_to_dev_devices_creates_no_files() {
    // Reporter shape 3: `cp f /dev/stdin` copied into ./CONIN$, and
    // the /dev/stdout and /dev/tty copies left their products too.
    let (stdout, _stderr, _code, entries) = rubash_file_entries(
        "printf 'y\\n' > f\n\
         cp f /dev/stdin 2>/dev/null\n\
         cp f /dev/stdout >/dev/null 2>&1\n\
         cp f /dev/tty 2>/dev/null\n\
         rm -f f\n\
         printf 'done\\n'\n",
    );
    assert_eq!(stdout, "done\n");
    assert_eq!(entries, Vec::<String>::new(), "entries: {entries:?}");
}

#[cfg(windows)]
#[test]
fn dev_null_shapes_still_create_nothing() {
    // Control: /dev/null was never affected (bare NUL names the
    // device in every directory) and must stay that way.
    let (stdout, _stderr, code, entries) = rubash_file_entries(
        "echo x > /dev/null\n\
         touch /dev/null 2>/dev/null\n\
         printf 'y\\n' > f\n\
         cp f /dev/null\n\
         rm -f f\n\
         printf 'done\\n'\n",
    );
    assert_eq!(stdout, "done\n");
    assert_eq!(code, Some(0), "exit code mismatch");
    assert_eq!(entries, Vec::<String>::new(), "entries: {entries:?}");
}

#[test]
fn redirect_dev_stdout_binds_the_current_fd1() {
    // Design-conclusion control (not Windows-gated): /dev/stdout and
    // /dev/fd/N redirect targets bind the shell's current fd, never
    // the console device — inside a capture the bytes land in the
    // capture, in a pipeline they land in the pipe.
    let (stdout, _stderr, code, _entries) = rubash_file_entries(
        "o=$(echo hello > /dev/stdout); printf 'cap: [%s]\\n' \"$o\"\n\
         printf 'line\\n' | while read -r l; do echo \"got: $l\" > /dev/stdout; done\n\
         exec 3> bound.txt; echo via-fd3 > /dev/fd/3; exec 3>&-\n\
         printf 'fd3: [%s]\\n' \"$(cat bound.txt)\"\n",
    );
    assert_eq!(
        stdout, "cap: [hello]\ngot: line\nfd3: [via-fd3]\n",
        "stdout mismatch"
    );
    assert_eq!(code, Some(0), "exit code mismatch");
}
