//! rubash#4114 — subscript arithmetic error (`${a[1x]}`) in redirections
//! must be evaluated in `do_redirections` order, routed through earlier
//! redirections (`2>/dev/null`), and contained per GNU Bash 5.3.20
//! (`arrayfunc.c:1388 array_expand_index` -> `jump_to_top_level (DISCARD)`,
//! `execute_cmd.c:5884` forked child vs `execute_cmd.c:5606` main shell).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("rubash_issue4114_{}_{}", std::process::id(), id));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let dir = TempDir::new();
    let script_path = dir.0.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir.0)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn subscript_arith_error_in_redirects_matches_gnu_order_and_containment() {
    let script = "\
cat > ${a[1x]}
echo \"c1=$?\"
echo ${a[1x]} > created_check.txt
echo \"c2=$? exists=$(test -f created_check.txt && echo yes || echo no)\"
cat 2>/dev/null < ${a[1x]}
echo \"c3=$?\"
cat ${a[1x]}
echo \"c4=$?\"
echo 2>/dev/null > ${a[1x]}
echo \"c5=$?\"
> ${a[1x]}
echo \"c6=$?\"
cat > \"${a[1x]}\"
echo \"c7=$?\"
";
    let (stdout, stderr, code) = run_script(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "c1=1\nc2=1 exists=no\nc3=1\nc4=1\nc5=1\nc6=1\nc7=1\n",
        "stdout: {stdout}"
    );
    let err_lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(
        err_lines.len(),
        3,
        "expected GNU's 3 diagnostic lines (script lines 1, 3, 7), got:\n{stderr}"
    );
    for (line, script_line) in err_lines.iter().zip([1, 3, 7]) {
        assert!(
            line.contains(&format!("line {script_line}:")),
            "diagnostic does not name script line {script_line}: {line}"
        );
        assert!(
            line.contains("1x: value too great for base (error token is \"1x\")"),
            "unexpected diagnostic line: {line}"
        );
        assert!(
            !line.contains("ambiguous redirect")
                && !line.contains("No such file or directory")
                && !line.contains("Is a directory"),
            "unexpected secondary redirect error: {line}"
        );
    }
}

#[test]
fn subscript_arith_error_redirect_same_line_and_errexit_containment() {
    let script = "\
cat > ${a[1x]}; echo \"ext_same=$?\"
echo \"ext_next=$?\"
echo > ${a[1x]}; echo \"blt_same=$?\"
echo \"blt_next=$?\"
( set -e; cat > ${a[1x]}; echo \"after_errexit\" )
echo \"errexit_subshell=$?\"
( set -o posix; cat > ${a[1x]}; echo \"after_posix=$?\" )
echo \"posix_subshell=$?\"
";
    let (stdout, stderr, code) = run_script(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "ext_same=1\next_next=0\nblt_next=1\nerrexit_subshell=1\nafter_posix=1\nposix_subshell=0\n",
        "stdout: {stdout}\nstderr: {stderr}"
    );
}
