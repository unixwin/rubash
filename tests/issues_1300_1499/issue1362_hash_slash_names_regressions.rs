//! rubash#1362: names containing a slash are NEVER hashed — `hash`
//! accepts them silently (rc 0) and stores nothing, for the plain,
//! `-t` and `-d` forms alike.
//!
//! GNU anchor: `builtins/hash.def` / `hashlib.c`: a pathname is run
//! directly, never entered into the hash table, so there is nothing
//! to find, print or delete. rubash diagnosed `hash: foo/bar: not
//! found` (rc 1) and even stored relative pathnames given with `-p`
//! semantics applied inconsistently.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20
//! (cluster z1 probes p1362a/p1362b, 20/20 differential set).

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn run_script_in_scratch(script: &str) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1362-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1362 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
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

/// p1362a: slash names are silent no-ops and the table stays empty.
#[test]
fn slash_names_are_never_hashed() {
    let (code, stdout, stderr) = run_script_in_scratch(
        "hash foo/bar; echo \"rc1=$?\"\n\
         hash ./foo; echo \"rc2=$?\"\n\
         hash /abs/path/x; echo \"rc3=$?\"\n\
         hash; echo \"rc4=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "rc1=0\nrc2=0\nrc3=0\nhash: hash table empty\nrc4=0\n"
    );
    assert_eq!(stderr, "");
}

/// p1362b: an existing executable addressed by a slash path is still
/// not stored (`-t` misses, rc 1), `-d` on slash names is a silent
/// no-op, and a `-d ./name` does not evict the plain name's entry.
#[test]
#[cfg(unix)]
fn slash_path_of_real_file_is_not_stored() {
    let (code, stdout, stderr) = run_script_in_scratch(
        "mkdir -p sb; printf '#!/bin/sh\\necho X\\n' > sb/prog; chmod +x sb/prog\n\
         hash sb/prog; echo \"rc=$?\"\n\
         hash -t sb/prog; echo \"t-rc=$?\"\n\
         hash -d foo/bar; echo \"d-rc=$?\"\n\
         hash -p /bin/ls n; hash -d ./n; echo \"d2-rc=$?\"; hash -t n; echo \"t2-rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\nt-rc=1\nd-rc=0\nd2-rc=0\n/bin/ls\nt2-rc=0\n");
    assert!(
        stderr.contains("hash: sb/prog: not found"),
        "stderr: {stderr}"
    );
}
