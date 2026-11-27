//! rubash#359: append-redirect to a `:`-containing filename under a /d/
//! drive-form path failed "Invalid argument" where GNU Bash and Git Bash
//! both succeed.
//!
//! GNU spec: `:` is an ordinary filename byte — redir.c:298
//! redirection_expand() expands the target as a single word and
//! redir.c:706 redir_open() passes it to open(2) verbatim
//! (O_CREAT|O_APPEND per instruction). Nothing in GNU interprets a colon.
//!
//! Windows decision (ours, matching the MSYS2/cygwin reference layer):
//! the Win32/NTFS layer reads `:` in the LAST path component as the
//! `name:stream` alternate-data-stream separator — a two-colon name like
//! `2026-09-30 14:48:26 UTC.log` is invalid ADS syntax (open fails EINVAL
//! "Invalid argument") and a single-colon name "succeeds" but silently
//! writes a stream onto a different host file. MSYS2 stores such names by
//! encoding `:` as U+F03A from the Unicode private-use area (verified on
//! this volume: Git Bash's `a:b.log` occupies the on-disk name
//! `61 F03A 62 2E 6C 6F 67`). executor/path.rs
//! map_ntfs_colon_final_component does the same in the single
//! shell-name->NT-name funnel, and shell_path_display_from_windows
//! reverses it for shell-visible names, so writes, reads, stat, test and
//! glob all agree on one backing file across the /d/, D:/ and relative
//! spellings. `/dev/null` (and the other mapped devices) carry no colon
//! and are untouched.

use std::fs;
use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
    )
}

#[test]
fn colon_filename_append_read_stat_roundtrip() {
    let dir = std::env::temp_dir().join("rubash359-roundtrip");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let run = |script: String| {
        Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(script)
            .current_dir(&dir)
            .output()
            .expect("run rubash")
    };

    // The issue reproducer shape: append to a two-colon timestamped name.
    let out = run(format!(
        "printf 'x\\n' >>\"{}\"; echo rc=$?",
        "2026-09-30 14:48:26 UTC.log"
    ));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "rc=0\n");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
    assert_eq!(out.status.code(), Some(0));

    // Colon first / middle / last all behave like ordinary bytes: each
    // name appends independently and reads back what was written.
    for (name, content) in [
        (":lead.log", "A"),
        ("mid:dle.log", "B"),
        ("trail:.log", "C"),
        ("a:b.log", "D"),
    ] {
        let out = run(format!("printf '{content}\\n' >>\"{name}\""));
        assert_eq!(out.status.code(), Some(0), "{name}");
        let out = run(format!("cat \"{name}\""));
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("{content}\n"),
            "{name}"
        );
        assert_eq!(out.status.code(), Some(0), "{name}");
    }

    // Two appends accumulate (a real file, not a reopened ADS).
    let out = run(
        "printf '1\\n' >>\"twice:a.log\"; printf '2\\n' >>\"twice:a.log\"; cat twice:a.log"
            .to_string(),
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "1\n2\n");

    // `>` truncates; test -f / [ -s ] see the same file the redirects
    // wrote; glob patterns match colon names in the shell dialect.
    let out = run("printf 'X\\n' >\"t:q.log\"; printf 'Y\\n' >\"t:q.log\"; echo \"$(wc -c <t:q.log) $(test -f t:q.log && echo f) $([ -s t:q.log ] && echo s) $(echo t:q*)\"".to_string());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "2 f s t:q.log\n");

    // noclobber wording on a colon name matches GNU.
    let out = run("set -C; printf 'Z\\n' >\"t:q.log\"".to_string());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.ends_with("t:q.log: cannot overwrite existing file\n"),
        "{stderr}"
    );
    assert_eq!(out.status.code(), Some(1));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn dev_null_family_untouched() {
    let (stdout, stderr, code) =
        rubash("printf 'N\\n' >>/dev/null; echo a=$?; printf 'M\\n' >/dev/null; echo b=$?");
    assert_eq!(stdout, "a=0\nb=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn drive_letter_colon_is_not_final_component() {
    // The mapping only touches the LAST component: a plain drive-absolute
    // target still resolves to the same file as its /d/ spelling.
    let dir = std::env::temp_dir().join("rubash359-drive");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let dir_str = dir.to_string_lossy().replace('\\', "/");
    let (stdout, stderr, code) = rubash(&format!(
        "printf 'B\\n' >>\"{dir_str}/mid:dle.log\"; printf 'E\\n' >>\"{dir_str}/mid:dle.log\"; cat \"{dir_str}/mid:dle.log\""
    ));
    assert_eq!(stdout, "B\nE\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let _ = fs::remove_dir_all(&dir);
}
