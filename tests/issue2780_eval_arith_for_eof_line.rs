//! rubash#2780 — an unclosed arith-for header reached through the
//! `eval` route reports the matched-pair EOF form at the line GNU
//! names: the opener line in the CALLER line numbering
//! (evalstring.c parse_and_execute parses the string as a fresh
//! reader unit continuing the caller counter; parse.y:3908-3915
//! parse_matched_pair names its start_lineno). rubash used to name
//! line 3 for a line-1 eval — its eval line accounting inflated
//! the count. Expectations measured against GNU bash 5.3.20
//! (2026-11-08).

// HARNESS-HEAD
use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    run_file_env(tag, script.as_bytes(), &[])
}

fn run_file_env(tag: &str, script: &[u8], extra_env: &[(&str, &str)]) -> Run {
    let dir: PathBuf = std::env::temp_dir().join(format!("rubashP1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), script).expect("write fixture");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir);
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let output = command.output().expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn run_c(script: &str) -> Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin");
    // rubash#2714 (decision D1): the diagnostic prolog follows
    // argv[0], so the expectations below — measured against GNU
    // invoked as `bash` — pin the invocation name to `bash` where
    // the platform allows it (std exposes no argv[0] override on
    // Windows; there the harness's own exe-path token is normalized
    // to `bash` in the captured stderr instead, the same
    // normalization the GNU suites apply to the baseline's path).
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.arg0("bash");
    }
    let output = command.output().expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    #[cfg(windows)]
    let stderr = {
        let exe = env!("CARGO_BIN_EXE_rubash");
        let slashed = exe.replace('\\', "/");
        let mut normalized = stderr.replace(exe, "bash").replace(&slashed, "bash");
        if let Some(drive) = slashed.chars().next().filter(|c| c.is_ascii_alphabetic()) {
            if slashed.as_bytes().get(1) == Some(&b':') {
                let posix = format!("/{}/{}", drive.to_ascii_lowercase(), &slashed[3..]);
                normalized = normalized.replace(&posix, "bash");
            }
        }
        normalized
    };
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr,
        code: output.status.code(),
    }
}

fn assert_run(tag: &str, run: &Run, stdout: &str, stderr: &str, code: i32) {
    assert_eq!(run.stdout, stdout, "{tag}: stdout");
    assert_eq!(run.stderr, stderr, "{tag}: stderr");
    assert_eq!(run.code, Some(code), "{tag}: rc");
}

#[test]
fn issue2780_eval_unclosed_arith_for_names_caller_line_file() {
    let run = run_file("eval-l1", "eval 'for ((i=0;i<3'\n");
    assert_run(
        "eval-l1",
        &run,
        "",
        "case.sh: eval: line 1: unexpected EOF while looking for matching `)'\n",
        2,
    );
}

#[test]
fn issue2780_eval_unclosed_arith_for_names_caller_line_2_file() {
    let run = run_file("eval-l2", "x=1\neval 'for ((i=0;i<3'\n");
    assert_run(
        "eval-l2",
        &run,
        "",
        "case.sh: eval: line 2: unexpected EOF while looking for matching `)'\n",
        2,
    );
}

#[test]
fn issue2780_eval_unclosed_arith_for_names_caller_line_dash_c() {
    let run = run_c("eval 'for ((i=0;i<3'");
    assert_run(
        "eval-c",
        &run,
        "",
        "bash: eval: line 1: unexpected EOF while looking for matching `)'\n",
        2,
    );
}

#[test]
fn issue2780_eval_multiline_header_names_opener_line() {
    // The header opens on string line 1 (the eval sits on file line 1),
    // so GNU names line 1 even though the string runs on to line 2.
    let run = run_file("eval-ml", "eval 'for ((i=0;\ni<3'\n");
    assert_run(
        "eval-ml",
        &run,
        "",
        "case.sh: eval: line 1: unexpected EOF while looking for matching `)'\n",
        2,
    );
}

#[test]
fn issue2780_direct_unclosed_arith_for_control() {
    // Without the eval route the same EOF form already names line 1
    // (rubash#2749); this control keeps the two routes from drifting.
    let run = run_file("direct", "for ((i=0;i<3\n");
    assert_run(
        "direct",
        &run,
        "",
        "case.sh: line 1: unexpected EOF while looking for matching `)'\n",
        2,
    );
}
