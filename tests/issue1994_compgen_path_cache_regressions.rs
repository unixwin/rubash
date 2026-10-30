//! rubash#1994 — engine-side compspec evaluation re-walked the whole
//! PATH universe on every call (no cache). The walk is now memoized
//! under the command-lookup fingerprint (PATH/PWD/PATHEXT + path
//! translation keys), the same discipline as the executor's command
//! lookup cache and the niubash host-side completion cache.
//!
//! These pins assert GNU-observable behavior only: repeated `compgen
//! -c` calls in one process return identical lists, duplicates across
//! PATH directories are preserved (rubash#1089), non-executables are
//! excluded and symlinks followed (rubash#1265), and a PATH change
//! refreshes the candidate set (the fingerprint moved). Expectations
//! verified byte-for-byte against GNU bash 5.3.20.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rubash-1994-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }

    fn executable(&self, rel: &str) {
        let path = self.0.join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
        fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write executable");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod +x");
    }

    fn rubash_script(&self, script: &str) -> (String, String, Option<i32>) {
        let file = self.0.join("case.sh");
        fs::write(&file, script).expect("write script");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&file)
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.0)
            .output()
            .expect("run rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn compgen_c_stable_across_calls_and_refreshes_on_path_change() {
    let scratch = Scratch::new("walk");
    scratch.executable("d1/dupcmd");
    scratch.executable("d1/onlyone");
    scratch.executable("d2/dupcmd");
    let notexec = scratch.0.join("d1/notexec");
    fs::write(&notexec, "text\n").expect("write non-executable");
    fs::set_permissions(&notexec, fs::Permissions::from_mode(0o644)).expect("chmod -x");
    std::os::unix::fs::symlink(scratch.0.join("d1/onlyone"), scratch.0.join("d2/linkcmd"))
        .expect("symlink");

    let d1 = scratch.0.join("d1").display().to_string();
    let d2 = scratch.0.join("d2").display().to_string();
    let script = format!(
        "count() {{ local n=0 x; while IFS= read -r x; do if [ \"$x\" = \"$1\" ]; then n=$((n+1)); fi; done; echo $n; }}\n\
         ORIG=$PATH\n\
         PATH={d1}:{d2}:$ORIG\n\
         a=$(compgen -c)\n\
         b=$(compgen -c)\n\
         if [ \"$a\" = \"$b\" ]; then echo stable=yes; else echo stable=no; fi\n\
         echo \"dup=$(compgen -c | count dupcmd) only=$(compgen -c | count onlyone) notexec=$(compgen -c | count notexec) link=$(compgen -c | count linkcmd)\"\n\
         PATH={d2}:$ORIG\n\
         echo \"after-path-change: dup=$(compgen -c | count dupcmd) only=$(compgen -c | count onlyone)\"\n"
    );
    let (stdout, stderr, code) = scratch.rubash_script(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "stable=yes\n\
         dup=2 only=1 notexec=0 link=1\n\
         after-path-change: dup=1 only=0\n"
    );
    assert_eq!(stderr, "");
}
