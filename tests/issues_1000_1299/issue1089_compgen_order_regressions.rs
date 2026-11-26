//! Regression pins for rubash#1089 (compgen candidate generation:
//! duplicate retention and enumeration order).
//!
//! Two shape bugs in `compgen -A` generation (builtins complete.def
//! compgen: candidates are emitted in generator order, duplicates kept):
//!   1. `compgen -A command echo` deduplicated the builtin hit against
//!      the PATH file hit. GNU prints `echo` once per source — a builtin
//!      AND a same-named executable on PATH yield two lines.
//!   2. `compgen -A file` SORTED its candidates. GNU emits readdir
//!      (directory) order, unsorted.
//!
//! Issue repro:
//!   compgen -A command echo   # GNU: echo twice  / old niu: once
//!   (in a dir with z2 z1 created in that order)
//!   compgen -A file -- z      # GNU: z2 z1 (dir order) / old niu: z1 z2
//!
//! The command leg pins PATH to a single scratch bin dir so the hit
//! count is exact on any host (system /bin + /usr/bin would add their own
//! `echo` files and change the count legitimately — GNU also lists one
//! line per PATH dir hit). The file leg compares against the OS's own
//! readdir order read back through std::fs, which is the only portable
//! definition of "directory order".

use std::path::PathBuf;
use std::process::Command;

fn rubash_in(dir: &PathBuf, script: &str, path_env: Option<&str>) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c").arg(script).current_dir(dir);
    if let Some(p) = path_env {
        cmd.env("PATH", p);
    }
    let out = cmd.output().expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-bfd-1089-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn issue1089_compgen_command_keeps_builtin_and_file_hits() {
    let dir = scratch("cmd");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("bin dir");
    let echo_path = bin.join("echo");
    std::fs::write(&echo_path, "#!/bin/sh\necho file-echo\n").expect("fake echo");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&echo_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake echo");
    }
    let path_env = bin.to_string_lossy().into_owned();
    let (stdout, _stderr, _code) = rubash_in(&dir, "compgen -A command echo", Some(&path_env));
    let _ = std::fs::remove_dir_all(&dir);
    let hits: Vec<&str> = stdout.lines().filter(|l| *l == "echo").collect();
    assert_eq!(
        hits.len(),
        2,
        "builtin echo + PATH file echo must both be listed (no dedup), got: {stdout:?}"
    );
}

#[test]
fn issue1089_compgen_file_uses_directory_order_not_sorted() {
    let dir = scratch("file");
    // Creation order z2, z1, z3 — sorted order would be z1 z2 z3.
    for name in ["z2", "z1", "z3"] {
        std::fs::write(dir.join(name), b"x").expect("fixture file");
    }
    // The host's own readdir sequence for the same directory is the
    // definition of directory order the shell must follow.
    let mut expected: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("read_dir") {
        let name = entry
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .into_owned();
        if name.starts_with('z') {
            expected.push(name);
        }
    }
    let (stdout, _stderr, _code) = rubash_in(&dir, "compgen -A file -- z", None);
    let _ = std::fs::remove_dir_all(&dir);
    let got: Vec<String> = stdout.lines().map(|l| l.to_string()).collect();
    assert_eq!(
        got, expected,
        "compgen -A file must emit readdir order (not sorted, not deduped)"
    );
}
