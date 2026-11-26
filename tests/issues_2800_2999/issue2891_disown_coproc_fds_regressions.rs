//! rubash#2891 — `disown` on a coproc's job closed the coproc's
//! pipe endpoints, so the (still running) coproc became unusable:
//! further writes/reads on its fds failed. GNU's delete_job
//! (jobs.c:1486) only removes the job-table slot; coproc fds are
//! disposed solely by coproc_dispose (execute_cmd.c:2166, via
//! cpl_reap when the child is reaped) or coproc_closeall at shell
//! exit (execute_cmd.c:1714). A disowned coproc keeps working.
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
    let dir = std::env::temp_dir().join(format!("issue2891_{}_{}", std::process::id(), seq));
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
fn disowned_coproc_keeps_its_pipe_fds() {
    let run = run_script(
        "coproc C { while IFS= read -r l; do echo \"echo:$l\"; done; }\n\
         disown %1 2>/dev/null || disown\n\
         echo \"in1\" >&${C[1]}\n\
         read -t 2 r <&${C[0]}; echo \"r1:$r rc:$?\"\n\
         echo \"in2\" >&${C[1]}\n\
         read -t 2 r <&${C[0]}; echo \"r2:$r rc:$?\"\n\
         exec {C[1]}>&-\n\
         sleep 0.3\n\
         echo end\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "r1:echo:in1 rc:0\nr2:echo:in2 rc:0\nend\n",
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}
