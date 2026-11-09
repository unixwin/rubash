//! rubash#2789 (read-time half of #2787's D3): command-substitution
//! bodies are validated at EXECUTION time in rubash, but GNU validates
//! them when the enclosing command is READ.
//!
//! GNU: parse_comsub (parse.y:4451) hands the body to the grammar
//! while the enclosing word is being lexed (the parsed body is
//! reprinted at parse.y:4632), so a body grammar failure kills the
//! input unit at read time — even when the body would never execute
//! (`false && echo $(a |)`), and the abort status follows the driver
//! (a `-c` string takes the 127 remap; a file takes 2; a file sourced
//! FROM a `-c` string still takes 127). rubash parks the body's
//! parse-error marker nodes on CommandSubstitutionNode and screens
//! them only on execution paths
//! (src/executor/command_substitution.rs `report_comsub_body_error_if_any`),
//! so skipped bodies sail through and `-c 'echo $(case x in)'` exits 2
//! through the plain parse-error path instead of 127.
//!
//! Assertion scope (per the issue): the STATUS and the read-time
//! abort are this issue's face; the diagnostic WORDING is the
//! #2783-family face. These pins therefore assert exit status and
//! stdout byte-for-byte, and require stderr to carry a syntax-error
//! report without pinning its exact text.
//!
//! Expectations verified against GNU Bash 5.3.20 probes
//! (self-compiled oracle; `-c` and script-file routes).

use std::process::Command;

struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rubash-issue2789-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Scratch(dir)
    }
    fn file(&self, name: &str, script: &str) -> (String, String, Option<i32>) {
        std::fs::write(self.0.join(name), script).expect("write script");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(name)
            .current_dir(&self.0)
            .output()
            .expect("run rubash file");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
    fn dash_c(&self, script: &str) -> (String, String, Option<i32>) {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(script)
            .current_dir(&self.0)
            .output()
            .expect("run rubash -c");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
    fn write_only(&self, name: &str, script: &str) {
        std::fs::write(self.0.join(name), script).expect("write script");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn check_read_abort(run: (String, String, Option<i32>), expected_code: i32, shape: &str) {
    let (stdout, stderr, code) = run;
    assert_eq!(
        code,
        Some(expected_code),
        "shape {shape}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(
        stdout, "",
        "shape {shape} wrote to stdout (read must abort)"
    );
    assert!(
        stderr.contains("syntax error"),
        "shape {shape} stderr lacks a syntax error: {stderr}"
    );
}

/// Skipped body in a `-c` string: nothing runs, the string dies at
/// read with the -c remap status 127.
#[test]
fn skipped_body_dash_c_aborts_127() {
    let s = Scratch::new();
    check_read_abort(
        s.dash_c("false && echo $(a |); echo after"),
        127,
        "-c skipped $(a |)",
    );
}

/// Skipped body in a file: the file dies at read, status 2, `after`
/// never prints.
#[test]
fn skipped_body_file_aborts_2() {
    let s = Scratch::new();
    check_read_abort(
        s.file("case.sh", "false && echo $(a |)\necho after\n"),
        2,
        "file skipped $(a |)",
    );
}

/// The unterminated-case body via `-c`: GNU classifies it as a body
/// grammar error at read (127), not as an unclosed-EOF (2).
#[test]
fn unterminated_case_body_dash_c_aborts_127() {
    let s = Scratch::new();
    check_read_abort(s.dash_c("echo $(case x in)"), 127, "-c $(case x in)");
}

/// The unterminated-case body in a file: status 2.
#[test]
fn unterminated_case_body_file_aborts_2() {
    let s = Scratch::new();
    check_read_abort(
        s.file("case.sh", "echo $(case x in)\necho after\n"),
        2,
        "file $(case x in)",
    );
}

/// A bad body inside a file SOURCED from a `-c` string still takes
/// the string driver's 127 remap.
#[test]
fn sourced_file_body_takes_dash_c_remap() {
    let s = Scratch::new();
    s.write_only("g.sh", "echo $(case x in)\n");
    check_read_abort(
        s.dash_c(". ./g.sh; echo after"),
        127,
        "sourced g.sh from -c",
    );
}

/// A bad body nested inside a good body, both skipped: the read-time
/// screen is recursive.
#[test]
fn nested_skipped_body_dash_c_aborts_127() {
    let s = Scratch::new();
    check_read_abort(
        s.dash_c("false && echo $(echo $(a |)); echo after"),
        127,
        "-c nested skipped",
    );
}

/// Control: a bad body that DOES execute — already fatal today via
/// the execution-time screen (#2787); must stay 127.
#[test]
fn executed_body_still_aborts_127() {
    let s = Scratch::new();
    check_read_abort(s.dash_c("echo $(a |)"), 127, "-c executed $(a |)");
}

/// Control: heredoc bodies are DATA, not read units — a quoted
/// heredoc whose text looks like a broken comsub prints literally.
#[test]
fn heredoc_body_is_data_not_a_unit() {
    let s = Scratch::new();
    let (stdout, stderr, code) = s.file("case.sh", "cat <<'EOF'\n$(a |)\nEOF\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "$(a |)\n");
    assert_eq!(stderr, "");
}

/// Control: a VALID body in a skipped command must not trip the
/// read-time screen — the line after it runs.
#[test]
fn valid_skipped_body_does_not_abort() {
    let s = Scratch::new();
    let (stdout, stderr, code) = s.dash_c("false && echo $(echo ok); echo after");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after\n");
    assert_eq!(stderr, "");
}
