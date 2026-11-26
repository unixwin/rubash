//! rubash#2892 — after a coproc's output reached EOF, a second
//! `read <&${C[0]}` failed with "Bad file descriptor" because the
//! read machinery closed the input fd on EOF. GNU's read builtin
//! (builtins/read.def) never closes the fd it reads from: repeated
//! reads at EOF keep returning the plain EOF result (empty, rc 1).
//!
//! Expectations cross-checked against GNU bash 5.3.20 (script-file
//! probes in the r75 hunt).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2892_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

#[test]
fn repeated_reads_past_coproc_eof_report_plain_eof() {
    // The issue's regression seed, verbatim: the child closes its
    // own stdout but stays alive, so the coproc never retires and
    // every read meets a true EOF on a live, registered coproc.
    // (A dying-child shape would let coproc retirement mask the
    // closed-fd defect.)
    let run = run_script(
        "coproc { exec >&-; sleep 5; }\n\
         read -r r <&${COPROC[0]}; echo \"eof1:$?\"\n\
         read -r r2 <&${COPROC[0]}; echo \"eof2:$?\"\n\
         read -r r3 <&${COPROC[0]}; echo \"eof3:$?\"\n\
         kill $COPROC_PID 2>/dev/null; wait $COPROC_PID 2>/dev/null; echo done\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "eof1:1\neof2:1\neof3:1\ndone\n",
        "stdout: {}",
        run.stdout
    );
    assert_eq!(run.stderr, "", "stderr: {}", run.stderr);
    let _ = run.dir;
}
