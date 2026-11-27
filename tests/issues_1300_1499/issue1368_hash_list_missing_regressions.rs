//! rubash#1368: `hash -l <missing>` must diagnose and fail exactly
//! like the plain form — GNU prints `hash: name: not found` and
//! returns 1; rubash silently returned 0.
//!
//! GNU anchor: `builtins/hash.def`: the `-l` listing flag changes
//! only the print format; a name that cannot be found is the same
//! error on every path. Agreeing boundaries pinned alongside: bare
//! `hash name` fails the same way, and `hash -l` with a findable
//! name adds it silently for a later listing.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20
//! (cluster z1 probe p1368 plus a PATH-controlled variant).
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

fn run_script(script: &str, dir: &std::path::Path) -> (Option<i32>, String, String) {
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue1368-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1368 scratch dir");
    dir
}

/// The reported case: `-l` with a missing name errors (rc 1, GNU
/// diagnostic), exactly like the bare form; an empty-table `hash
/// -l` afterwards stays silent with rc 0.
#[test]
fn hash_list_missing_name_diagnoses() {
    let dir = scratch_dir("core");
    let (code, stdout, stderr) = run_script(
        "hash -l nonexist; echo \"rc1=$?\"\n\
         hash nonexist; echo \"rc2=$?\"\n\
         hash -l; echo \"rc3=$?\"\n",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc1=1\nrc2=1\nrc3=0\n");
    assert_eq!(
        stderr.matches("hash: nonexist: not found").count(),
        2,
        "stderr: {stderr}"
    );
}

/// The silent-add boundary: `hash -l <findable>` adds the entry
/// without printing (rc 0); a later bare `hash -l` lists it in
/// `builtin hash -p ...` form and `-t` resolves it. PATH is pinned
/// to a scratch bin dir so the stored path is deterministic.
#[test]
fn hash_list_findable_name_adds_silently() {
    let dir = scratch_dir("add");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("create bin dir");
    let mycmd = bin.join("mycmd");
    std::fs::write(&mycmd, "#!/bin/sh\necho fake\n").expect("write mycmd");
    std::fs::set_permissions(&mycmd, std::fs::Permissions::from_mode(0o755)).expect("chmod mycmd");
    let script = format!(
        "PATH=\"{}:/usr/bin:/bin\"\n\
         hash -l mycmd; echo \"rc1=$?\"\n\
         hash -l\n\
         hash -t mycmd; echo \"rc2=$?\"\n",
        bin.display()
    );
    let (code, stdout, stderr) = run_script(&script, &dir);
    let expected = format!(
        "rc1=0\nbuiltin hash -p {}/mycmd mycmd\n{}/mycmd\nrc2=0\n",
        bin.display(),
        bin.display()
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, expected);
    assert_eq!(stderr, "");
}
