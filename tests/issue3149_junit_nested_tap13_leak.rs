//! rubash#3149: bats junit-formatter's nested fixture-suite tap13
//! lines leak into the outer TAP stream in full-file runs
//! (`Executed 15 instead of expected 13 tests`, rc=1; GNU: 13 ok,
//! rc=0). Minimal shape, reduced from bats' `bats_tee`
//! (`libexec/bats-core/bats`: `exec 3<&1; tee >(cat >&3) | "$@"`)
//! as used by the `--report-formatter` test:
//!
//!   producer | mid | cat          (outer pipeline)
//!   mid() { exec 3<&1; tee >(cat >&3) | cat > report.txt; }
//!
//! GNU (execute_cmd.c:2620 execute_pipeline forks every element;
//! do_piping, execute_cmd.c:6373, dup2's the inter-stage pipe onto
//! each element's fd 0; subst.c:6362 runs the `>(...)` consumer as a
//! forked async child): the producer's bytes reach `tee` through
//! the live pipe, `tee` copies them to the `>(cat >&3)` consumer
//! (which writes them to mid's stdout, hence to the final `cat`)
//! and to `cat > report.txt`.
//!
//! Rubash on the base drops the whole stream: the outer pipeline is
//! claimed by the streaming-sandwich path (exactly one shell-side
//! stage amid externals), which binds mid's fd 0 to a File endpoint
//! over the live producer-pipe read handle; the inner pipeline
//! declines the concurrent path (its `tee` carries a process
//! substitution) and falls to the sequential stage loop, which
//! classifies stage 0 as live (`pipeline_stage0_stdin_live`) but
//! the sequential external-stage spawner models stdin only as
//! inherit-or-payload-string, so `tee` is spawned on a fresh empty
//! pipe and reads immediate EOF — the producer dies EPIPE/SIGPIPE
//! and nothing lands anywhere (total loss, not a leak past a
//! filter).
//!
//! Expectations are GNU bash 5.3.20 behavior, one test per face:
//!   01 — the failing cell: procsub-carrying inner pipeline inside
//!        a sandwich middle function stage (total loss on base)
//!   02 — control: same shape without the procsub (concurrent path
//!        claims the inner pipeline; green on base and head)
//!   03 — control: function stage body is a plain `cat` (no inner
//!        pipeline at all; green on base and head)

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn rubash_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_SHELL_ROOT");
    command
}

fn shell_test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3149-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write_producer(dir: &Path) {
    fs::write(
        dir.join("producer.sh"),
        "#!/usr/bin/env bash\nprintf 'line1\\n'\nprintf 'line2\\n'\n",
    )
    .expect("write producer");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(dir.join("producer.sh"))
            .expect("producer metadata")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(dir.join("producer.sh"), perms).expect("chmod producer");
    }
}

fn run_script(dir: &Path, name: &str, body: &str) -> std::process::Output {
    let script = dir.join(name);
    fs::write(&script, body).expect("write script");
    rubash_command()
        .current_dir(dir)
        .arg(shell_test_path(&script))
        .output()
        .expect("run script")
}

// 01 — the failing cell (see the file header): the producer's
// bytes must reach `tee` through the live inter-stage pipe, out
// through the `>(cat >&3)` consumer onto mid's stdout (hence the
// final `cat`), and through `cat > report.txt` into the file.
// On the base the whole stream is lost: stdout is just the `done`
// line with both captures empty, and report.txt is empty.
#[test]
fn issue3149_01_sandwich_function_stage_procsub_pipeline_keeps_stream() {
    let dir = test_dir("01");
    write_producer(&dir);
    let output = run_script(
        &dir,
        "main.sh",
        "exec 3>marker.txt\nmid() { exec 3<&1; tee >(cat >&3) | cat > report.txt; }\n./producer.sh | mid | cat\necho \"done marker:[$(cat marker.txt | tr '\\n' '|')] report:[$(cat report.txt | tr '\\n' '|')]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "line1\nline2\ndone marker:[] report:[line1|line2|]\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report = fs::read_to_string(dir.join("report.txt")).unwrap_or_default();
    assert_eq!(report, "line1\nline2\n", "report.txt: {report:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 02 — control: the same outer shape with the procsub removed
// from the inner pipeline (`tee | cat > report.txt`). The inner
// pipeline is claimed by the concurrent path, whose stage-0 stdin
// comes from the slurped File-endpoint text, so the stream flows
// on the base as well as the head.
#[test]
fn issue3149_02_same_shape_without_procsub_flows_on_base() {
    let dir = test_dir("02");
    write_producer(&dir);
    let output = run_script(
        &dir,
        "main.sh",
        "mid() { tee | cat > report.txt; }\n./producer.sh | mid | cat\necho \"done report:[$(cat report.txt | tr '\\n' '|')]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "done report:[line1|line2|]\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report = fs::read_to_string(dir.join("report.txt")).unwrap_or_default();
    assert_eq!(report, "line1\nline2\n", "report.txt: {report:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 03 — control: the function stage body is a plain `cat` (no
// inner pipeline). The body is served by the top-level external
// spawn, which resolves fd 0 from the fd table, so the stream
// flows on the base as well as the head.
#[test]
fn issue3149_03_plain_cat_function_stage_flows_on_base() {
    let dir = test_dir("03");
    write_producer(&dir);
    let output = run_script(
        &dir,
        "main.sh",
        "mid() { cat > report.txt; }\n./producer.sh | mid | cat\necho \"done report:[$(cat report.txt | tr '\\n' '|')]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "done report:[line1|line2|]\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report = fs::read_to_string(dir.join("report.txt")).unwrap_or_default();
    assert_eq!(report, "line1\nline2\n", "report.txt: {report:?}");
    let _ = fs::remove_dir_all(&dir);
}
