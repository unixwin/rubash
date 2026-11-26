//! rubash#1298: `read -a` into an associative array fails with
//! status 1 like GNU, not 2.
//!
//! GNU anchor: `builtins/read.def`: `-a` requires an indexed
//! array; against an assoc array it reports
//! `read: h: not an indexed array` and the builtin exits 1.
//! rubash printed the same diagnostic but returned its internal
//! usage-error status 2, which caller-side `||` handling and
//! `set -e` decisions can observe.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::process::{Command, Stdio};

fn run_script(script: &str) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!("rubash-issue1298-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1298 scratch dir");
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

/// The reported case (rc 1), the already-agreeing indexed-array
/// success path, and the already-agreeing invalid-identifier
/// status (also 1) as boundaries.
#[test]
fn read_into_assoc_array_returns_one() {
    let (code, stdout, stderr) = run_script(
        "declare -A h\n\
         echo \"a b\" | { read -a h; echo \"rc=$?\"; }\n\
         echo \"x y\" | { declare -a arr; read -a arr; echo \"ok:${arr[0]}/${arr[1]}\"; }\n\
         read -a \"bad-name\" </dev/null; echo \"badname-rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\nok:x/y\nbadname-rc=1\n");
    assert!(
        stderr.contains("read: h: not an indexed array"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("not a valid identifier"),
        "stderr: {stderr}"
    );
}
