//! rubash#1382: after `hash -p <path>`, option parsing is OVER —
//! every further word is a NAME, even one spelled like a flag.
//!
//! GNU anchor: `builtins/hash.def`: `-p` consumes exactly one path
//! argument; the remaining words are names (the NOTOPT convention),
//! so `hash -p /bin/ls n -l` registers TWO entries, `n` and `-l`.
//! rubash kept parsing flags and swallowed the trailing `-l`/`-r`/
//! `-t`, registering one entry and applying the flag's effect.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20
//! (cluster z1 probes p1382a/p1382b plus an option-arity probe).

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn run_script(script: &str) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1382-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1382 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// p1382a: trailing `-l`, `-r` and `-t` words after the first name
/// all become entries; nothing is listed/flushed/translated early.
#[test]
fn words_after_first_name_are_names() {
    let (code, stdout, stderr) = run_script(
        "hash -p /bin/ls n -l; echo \"rc=$?\"; hash -l\n\
         hash -r\n\
         hash -p /bin/ls n -r; hash -l\n\
         hash -r\n\
         hash -p /bin/ls n -t; hash -l\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "rc=0\n\
         builtin hash -p /bin/ls n\n\
         builtin hash -p /bin/ls -l\n\
         builtin hash -p /bin/ls n\n\
         builtin hash -p /bin/ls -r\n\
         builtin hash -p /bin/ls n\n\
         builtin hash -p /bin/ls -t\n"
    );
    assert_eq!(stderr, "");
}

/// p1382b: `-t` given BEFORE `-p` wins the whole invocation — the
/// name is looked up (and misses), nothing is registered.
#[test]
fn translate_flag_before_p_path_wins() {
    let (code, stdout, stderr) = run_script(
        "hash -t -p /bin/ls n; echo \"rc1=$?\"\n\
         hash -l; echo \"---\"\n\
         hash -r\n\
         hash -p /bin/ls -t n; echo \"rc2=$?\"\n\
         hash -l\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc1=1\n---\nrc2=1\n");
    assert_eq!(
        stderr.matches("hash: n: not found").count(),
        2,
        "stderr: {stderr}"
    );
}

/// Option-arity boundary: `-d`/`-t` with no name anywhere is a
/// usage error (rc 1, `option requires an argument`), regardless
/// of a preceding `-r`.
#[test]
fn argument_taking_flags_require_a_name() {
    let (code, stdout, stderr) = run_script(
        "hash -r -d; echo \"rc1=$?\"\n\
         hash -d -r; echo \"rc2=$?\"\n\
         hash -r -t; echo \"rc3=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc1=1\nrc2=1\nrc3=1\n");
    assert_eq!(
        stderr
            .matches("hash: -d: option requires an argument")
            .count(),
        2,
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("hash: -t: option requires an argument"),
        "stderr: {stderr}"
    );
}
