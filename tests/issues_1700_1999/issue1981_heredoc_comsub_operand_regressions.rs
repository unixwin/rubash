//! rubash#1981 — the command-substitution heredoc shortcut swallowed
//! `cat`'s FILE OPERANDS: `x=$(cat file <<EOF)` printed the heredoc
//! body and never read `file`. GNU subst.c:7143 command_substitute()
//! runs parse_and_execute — there is no operand-blind fast path. The
//! fast plan may only be entered when every command it models has the
//! operand shape it can execute: a `cat` with no operands or one bare
//! `-` (the heredoc dash). Any other operand composition — a file, an
//! option, a `-` plus a file and its pipelines — declines to the real
//! executor. The three-stage pipeline shape was also silently
//! truncated to two stages and is declined by the same admission.
//!
//! All expectations measured against GNU bash 5.3.20.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rubash-i1981-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out_path = self.dir.join("out.txt");
        let err_path = self.dir.join("err.txt");
        let out_file = fs::File::create(&out_path).unwrap();
        let err_file = fs::File::create(&err_path).unwrap();
        let start = Instant::now();
        let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::from(err_file))
            .spawn()
            .unwrap();
        let rc = loop {
            match child.try_wait().unwrap() {
                Some(status) => break status.code().unwrap_or(-1),
                None => {
                    if start.elapsed() > Duration::from_secs(20) {
                        let _ = child.kill();
                        let _ = child.wait();
                        break -1;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        };
        let read = |p: &PathBuf| {
            let mut s = String::new();
            fs::File::open(p).unwrap().read_to_string(&mut s).unwrap();
            s
        };
        (
            read(&out_path),
            read(&err_path).replace(&case.to_string_lossy().to_string(), "SH"),
            rc,
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue1981_cat_with_file_operand_reads_the_file_not_the_heredoc() {
    let s = Scratch::new("fileop");
    fs::write(s.dir.join("f.txt"), "FILEDATA\n").unwrap();
    // GNU: the heredoc goes unread (no `-` operand); the file is the
    // whole output. The shortcut used to print the body instead.
    let script = format!(
        "x=$(cat {} <<EOF\nBODY\nEOF\n)\necho \"V=[$x]\"\n",
        s.dir.join("f.txt").display()
    );
    let (out, err, rc) = s.run(&script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "V=[FILEDATA]\n");
    assert_eq!(err, "");
    // Two file operands: both files, in order.
    fs::write(s.dir.join("g.txt"), "GDATA\n").unwrap();
    let script = format!(
        "x=$(cat {} {} <<EOF\nBODY\nEOF\n)\necho \"V=[$x]\"\n",
        s.dir.join("f.txt").display(),
        s.dir.join("g.txt").display()
    );
    let (out, err, rc) = s.run(&script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "V=[FILEDATA\nGDATA]\n");
    assert_eq!(err, "");
}

#[test]
fn issue1981_dash_plus_file_reads_heredoc_then_file() {
    let s = Scratch::new("dashfile");
    fs::write(s.dir.join("f.txt"), "FILEDATA\n").unwrap();
    // GNU: the `-` operand reads the heredoc at its position in the
    // operand list, then the file.
    let script = format!(
        "x=$(cat - {} <<EOF\nBODY\nEOF\n)\necho \"V=[$x]\"\n",
        s.dir.join("f.txt").display()
    );
    let (out, err, rc) = s.run(&script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "V=[BODY\nFILEDATA]\n");
    assert_eq!(err, "");
}

#[test]
fn issue1981_three_stage_pipeline_is_not_truncated() {
    let s = Scratch::new("pipe3");
    // GNU: sort -u leaves two unique lines, wc -l reports 2. The plan
    // used to stop at the sort stage and substitute the sorted text.
    let (out, err, rc) =
        s.run("x=$(cat <<EOF | sort -u | wc -l\nbanana\napple\nbanana\nEOF\n)\necho \"V=[$x]\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "V=[2]\n");
    assert_eq!(err, "");
}

#[test]
fn issue1981_bare_and_dash_only_shapes_still_take_the_fast_plan() {
    let s = Scratch::new("fastok");
    // GNU agreement guards: the shapes the plan DOES model keep their
    // exact output through it.
    let (out, err, rc) = s.run(
        "a=$(cat <<EOF\nPLAIN-BODY\nEOF\n)\nb=$(cat - <<EOF\nDASH-BODY\nEOF\n)\nc=$(cat <<EOF | sort -u\nbanana\napple\nbanana\nEOF\n)\necho \"plain=[$a] dash=[$b] sorted=[$c]\"\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(
        out,
        "plain=[PLAIN-BODY] dash=[DASH-BODY] sorted=[apple\nbanana]\n"
    );
    assert_eq!(err, "");
}
