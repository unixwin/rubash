//! `hash` family — regression pins for rubash#1361, rubash#1368 and
//! rubash#1389, three z1-family issues closed on workspace-probe
//! evidence (rubash-fix3 z1-evidence probes p1361*/p1368/p1389*) that
//! never landed in the repo.
//!
//! * #1361: `hash -t` prints a `./`-prefixed path for a relative
//!   `hash -p` entry whose file exists; nonexistent relative entries
//!   print as stored. Plain `hash` lists the stored form (GNU
//!   builtins/hash.def: the `-t` print path normalizes, the table
//!   listing does not).
//! * #1368: `hash -l <missing>` must report `hash: <name>: not found`
//!   with status 1 instead of succeeding silently.
//! * #1389: `hash -t/-l` and `enable -p` must consume a trailing `--`
//!   as end-of-options instead of rejecting it as an invalid option.
//!
//! All expectations are GNU bash 5.3.20 behaviour for the verbatim
//! probe scripts, run from a file in a fresh directory.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rubash-bfa-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }

    fn rubash_file(&self, script: &str) -> (String, String, Option<i32>) {
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

/// Where `ls` resolves under the pinned PATH (merged-usr layouts put it
/// in /usr/bin; older layouts in /bin).
fn ls_path() -> &'static str {
    if PathBuf::from("/usr/bin/ls").exists() {
        "/usr/bin/ls"
    } else {
        "/bin/ls"
    }
}

#[test]
fn issue1361_hash_t_prefixes_existing_relative_entry() {
    let scratch = Scratch::new("1361");
    let script = "mkdir -p bin\nprintf '#!/bin/sh\\necho T\\n' > bin/tool\nchmod +x bin/tool\nhash -p bin/tool tool\nhash -t tool; echo \"t-rc=$?\"\ntool; echo \"exec-rc=$?\"\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(stdout, "./bin/tool\nt-rc=0\nT\nexec-rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1361_hash_t_relative_forms() {
    let scratch = Scratch::new("1361b");
    // Verbatim probe p1361b: only the entry whose file exists (stored
    // as ./bin/tool) and the headline form print with the prefix GNU
    // gives them; nonexistent relative entries print as stored.
    let script = "mkdir -p bin\nprintf '#!/bin/sh\\necho T\\n' > bin/tool\nchmod +x bin/tool\nhash -p ./bin/tool tool; hash -t tool; echo \"rc1=$?\"\nhash -p bin/nope tool2; hash -t tool2; echo \"rc2=$?\"\nhash -p ./bin/nope tool3; hash -t tool3; echo \"rc3=$?\"\nhash -p ../nope tool4; hash -t tool4; echo \"rc4=$?\"\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(
        stdout,
        "./bin/tool\nrc1=0\nbin/nope\nrc2=0\n./bin/nope\nrc3=0\n../nope\nrc4=0\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1361_hash_t_single_component_dot_slash_not_found() {
    let scratch = Scratch::new("1361c");
    // Verbatim probe p1361c: a stored `./nope` (single component) is
    // reported not found by GNU, while `./../nope` prints as stored.
    let script = "hash -p ./nope t5; hash -t t5; echo \"rc5=$?\"\nhash -p ./../nope t6; hash -t t6; echo \"rc6=$?\"\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(stdout, "rc5=1\n./../nope\nrc6=0\n");
    assert!(stderr.contains("hash: t5: not found"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1368_hash_l_missing_name_reports_not_found() {
    let scratch = Scratch::new("1368");
    // Verbatim probe p1368.
    let script = "hash -l nonexist; echo \"rc1=$?\"\nhash -l ls; echo \"rc2=$?\"\nhash -t ls; echo \"rc3=$?\"\nhash -lt nonexist; echo \"rc4=$?\"\nhash -l; echo \"rc5=$?\"\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    let ls = ls_path();
    assert_eq!(
        stdout,
        format!("rc1=1\nrc2=0\n{ls}\nrc3=0\nrc4=1\nbuiltin hash -p {ls} ls\nrc5=0\n")
    );
    assert_eq!(
        stderr.matches("hash: nonexist: not found").count(),
        2,
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue1389_hash_consumes_trailing_dashdash() {
    let scratch = Scratch::new("1389a");
    // Verbatim probe p1389a.
    let script = "hash -t -- ls; echo \"rc1=$?\"\nhash -l -- nonexist; echo \"rc2=$?\"\nhash -t --; echo \"rc3=$?\"\nhash -r --; echo \"rc4=$?\"\nhash -- ls; echo \"rc5=$?\"; hash -t ls; echo \"rc6=$?\"\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    let ls = ls_path();
    assert_eq!(
        stdout,
        format!("rc1=1\nrc2=1\nrc3=1\nrc4=0\nrc5=0\n{ls}\nrc6=0\n")
    );
    for part in [
        "hash: ls: not found",
        "hash: nonexist: not found",
        "hash: -t: option requires an argument",
    ] {
        assert!(stderr.contains(part), "stderr {stderr:?} missing {part:?}");
    }
    assert_eq!(code, Some(0));
}

#[test]
fn issue1389_enable_p_consumes_trailing_dashdash() {
    let scratch = Scratch::new("1389b");
    // Verbatim probe p1389b (listings truncated with head -2 exactly as
    // probed; the first two builtins in GNU's table are `.` and `:`).
    let script = "enable -p -- 2>&1 | head -2; enable -p -- >/dev/null 2>&1; echo \"rc1=$?\"\nenable -p -- echo | head -2\nenable -- -p; echo \"rc2=$?\"\n";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(
        stdout,
        "enable .\nenable :\nrc1=0\nenable .\nenable :\nrc2=1\n"
    );
    assert!(
        stderr.contains("enable: -p: not a shell builtin"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
