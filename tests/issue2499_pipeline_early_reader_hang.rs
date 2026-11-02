//! rubash#2499 — `2>&1 | head -N` under an early-exiting reader hung a
//! long-output function (the `nvm ls-remote --lts` shape).
//!
//! Signature (issue #2499 + the niubash#797a matrix): `nvm ls-remote --lts
//! 2>&1 | head -3` deadlocked the pre-fix engine, while `2>/dev/null | head
//! -3` and `... | cat` completed. The deadlock reproduced standalone — no
//! nvm, no network — once three axes line up:
//!
//! 1. a command substitution whose pipeline hands `sed` a MULTI-LINE program
//!    (a newline inside argv, exactly how nvm.sh spells its index.tab filter),
//! 2. running inside a function pipeline stage whose stderr is merged into
//!    the stage pipe (`2>&1`), and
//! 3. a payload larger than the OS pipe buffer (the nvm index.tab is
//!    ~213 KB; 500 lines complete even pre-fix, 5000 hang).
//!
//! Root cause: the pre-#2238 spawn accounting — spawning a foreground
//! external set the FUNCTION_STDIN cursor to EOF and consumed the whole
//! fd-0 text endpoint up front (`stdin_string_for_command_mut` /
//! `apply_default_external_stdin_file`), so the comsub's downstream reader
//! wedged against the stage's merged-stderr capture. Fixed by 7e283719
//! ("executor: foreground externals share the stdin stream cursor instead of
//! draining it", Refs #2238); these pins keep the early-reader class from
//! regressing. Verified against the boundary pair: engine 5291b165 (parent
//! of the fix) fails `nvm_ls_remote_shape_mocked_download_terminates` with
//! the 5 s deadline; 7e283719 and later (1c93ea87 / fa34dbff / master —
//! every engine rev the issue could still hang, fa64d58d, predates the fix)
//! complete it in ~1 s.
//!
//! GNU contract: `execute_cmd.c:2620 execute_pipeline` forks each left
//! element on pipe(2) and the parent closes each write end immediately after
//! the fork (`close (fildes[1])`, execute_cmd.c:2711), while the fd_bitmap
//! makes every child close the read end it does not use so SIGPIPE can be
//! delivered (execute_cmd.c:2656-2665); the members are reaped by
//! `jobs.c:3064 wait_for` with no wall-clock cap, and SIGPIPE keeps its
//! default disposition (sig.c:172), so a writer that outlives its reader
//! dies instead of wedging the pipeline.
//!
//! Expectations byte-verified against WSL GNU bash 5.3.20 with LC_ALL=C.
//!
//! Every test is wrapped in a short deadline because the failure mode is a
//! hang, not a wrong byte.

#![cfg(windows)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Deadline for one probe run. The fixed binary finishes every shape here in
/// ~1 s; the pre-fix binary never returns.
const RUN_LIMIT: Duration = Duration::from_secs(5);

struct RunOutcome {
    timed_out: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Run one probe script with a sandboxed CWD (a fresh temp dir under the
/// system temp), a closed stdin, and both streams drained live — reading
/// them only after exit would re-create a write-before-drain deadlock
/// whenever the probe's output exceeds the pipe capacity.
/// `tag` unique per calling test: the five `run_rubash_script` callers
/// otherwise share one pid-keyed scratch dir, and under parallel test
/// threads (`--test-threads=4`) two concurrent tests would race the same
/// dir's remove/create/probe.sh writes (the -j4 false-red family the
/// Windows acceptance lane flagged in wave 1).
fn run_rubash_script(tag: &str, script: &str) -> RunOutcome {
    // `-run-` keeps this CWD sandbox distinct from the calling test's
    // same-tag FIXTURE dir (`rubash-issue2499-{tag}-{pid}`): the helper
    // remove_dir_all's its dir first and must never wipe the fixtures.
    let dir =
        std::env::temp_dir().join(format!("rubash-issue2499-{tag}-run-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue2499 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("create probe script");
    // LF line endings: the repo's .gitattributes pins *.sh eol=lf.
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);

    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("LC_ALL", "C")
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .spawn()
        .expect("spawn rubash");

    let stdout_pipe = child.stdout.take().expect("piped stdout");
    let stderr_pipe = child.stderr.take().expect("piped stderr");
    let stdout_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = stdout_pipe;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        buffer
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = stderr_pipe;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        buffer
    });

    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break Some(status),
            None => {
                if started.elapsed() >= RUN_LIMIT {
                    timed_out = true;
                    let _ = child.kill();
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let stdout = stdout_reader.join().expect("join stdout reader");
    let stderr = stderr_reader.join().expect("join stderr reader");
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        timed_out,
        code: status.and_then(|status| status.code()),
        stdout,
        stderr,
    }
}

/// Resolve an external tool to an absolute, quoteable script word so the
/// probe does not depend on the child's PATH. Windows path separators are
/// normalized to `/` (CreateProcess accepts them and the shell will not
/// treat `\` as an escape).
fn resolve_tool(name: &str) -> String {
    let with_exe = format!("{name}.exe");
    for dir in std::env::var("PATH")
        .unwrap_or_default()
        .split(';')
        .filter(|entry| !entry.is_empty())
    {
        for candidate in [format!("{dir}\\{with_exe}"), format!("{dir}\\{name}")] {
            let path = PathBuf::from(&candidate);
            if path.is_file() {
                return path.to_string_lossy().replace('\\', "/");
            }
        }
    }
    panic!(
        "issue2499 tests need a real external `{name}` on PATH (the hang \
         lives in the external pipeline spawn path)"
    );
}

/// Scratch payload comfortably beyond the OS pipe buffer (12000 lines,
/// ~480 KB — the recorded pre-fix threshold sits between 500 and 5000
/// 48-byte rows, so half a megabyte discriminates with margin) so
/// the comsub's producer cannot hand everything over in one slurp. Returns
/// the payload path (forward slashes, quoteable in a script word) and the
/// character count `${#V}` sees: every byte after the `1d`-dropped header
/// line, minus the one trailing newline command substitution strips
/// (subst.c command_substitute strips all trailing newlines).
fn write_bulk_payload(dir: &std::path::Path) -> (String, usize) {
    let mut payload = String::new();
    let header = "header\n";
    payload.push_str(header);
    for i in 0..12000 {
        payload.push_str(&format!("line-{i} padding padding padding padding\n"));
    }
    let remaining = payload.len() - header.len() - 1;
    let path = dir.join("bulk.tab");
    std::fs::write(&path, payload.as_bytes()).expect("write bulk payload");
    (path.to_string_lossy().replace('\\', "/"), remaining)
}

/// The minimal reproducer: a function pipeline stage (merged stderr, early
/// reader) whose body runs a command substitution whose pipeline hands sed
/// a MULTI-LINE program, over a payload larger than the pipe buffer. Under
/// an interactive-style stdio setup this exact script deadlocked the
/// pre-#2238 engine; under the null-stdin test harness the fuller nvm-shape
/// pin below is the discriminating one, and this pin keeps the minimal
/// shape's bytes and termination pinned (the fixed engine prints the head
/// lines and the head-shaped status in ~0.3 s).
#[test]
fn early_reader_merged_stderr_multiline_argv_comsub_terminates() {
    let dir = std::env::temp_dir().join(format!("rubash-issue2499-min-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue2499 min scratch dir");
    let (payload, remaining) = write_bulk_payload(&dir);
    let cat = resolve_tool("cat");
    let sed = resolve_tool("sed");
    let head = resolve_tool("head");
    let script = format!(
        "f() {{\n\
         \x20 V=\"$('{cat}' '{payload}' | '{sed}' \"\n\
         \x20   1d;\n\
         \x20   s/^//;\n\
         \x20 \")\"\n\
         \x20 printf 'v1 %s\\nv2\\nv3\\nv4\\nv5\\n' \"${{#V}}\"\n\
         }}\n\
         f 2>&1 | '{head}' -3\n\
         echo \"rc=$?\"\n"
    );
    let out = run_rubash_script("min", &script);
    assert!(
        !out.timed_out,
        "minimal #2499 shape did not terminate within {RUN_LIMIT:?} (pre-#2238 \
         stdin-drain deadlock: comsub pipeline with a multi-line sed program \
         inside a 2>&1 function stage under an early-exiting head)"
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("v1 {remaining}\nv2\nv3\nrc=0\n"),
        "stdout must match the GNU 5.3.20 byte baseline (LC_ALL=C)"
    );
    assert_eq!(out.code, Some(0), "probe exit status");
    assert!(
        out.stderr.is_empty(),
        "stderr must stay empty, got: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// PIPESTATUS parity for the same shape: the remaining v4/v5 bytes fit the
/// pipe buffer after head exits, so GNU reports the pipeline as (0 0) — no
/// fabricated SIGPIPE death for the function stage.
#[test]
fn early_reader_shape_pipestatus_matches_gnu() {
    let dir = std::env::temp_dir().join(format!("rubash-issue2499-ps-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue2499 ps scratch dir");
    let (payload, _remaining) = write_bulk_payload(&dir);
    let cat = resolve_tool("cat");
    let sed = resolve_tool("sed");
    let head = resolve_tool("head");
    let script = format!(
        "f() {{\n\
         \x20 V=\"$('{cat}' '{payload}' | '{sed}' \"\n\
         \x20   1d;\n\
         \x20   s/^//;\n\
         \x20 \")\"\n\
         \x20 printf 'v1 %s\\nv2\\nv3\\nv4\\nv5\\n' \"${{#V}}\"\n\
         }}\n\
         f 2>&1 | '{head}' -3\n\
         echo \"ps=${{PIPESTATUS[*]}} rc=$?\"\n"
    );
    let out = run_rubash_script("ps", &script);
    assert!(
        !out.timed_out,
        "pipestatus probe did not terminate within {RUN_LIMIT:?}"
    );
    let mut stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    // Drop the three head lines; keep the ps/rc line for the byte pin.
    let ps_line = stdout.split_off(stdout.find("ps=").expect("ps= line present"));
    assert_eq!(
        ps_line, "ps=0 0 rc=0\n",
        "PIPESTATUS must match GNU 5.3.20 (both members settle 0)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The nvm `ls-remote --lts` shape, offline: the verbatim multi-line sed
/// filter nvm.sh spells for index.tab, the awk | while-read alias loop with
/// externals in the body, and the final long list — `nvm_download` mocked as
/// a local file. Byte-pinned against WSL GNU bash 5.3.20 (LC_ALL=C); the
/// pre-#2238 engine hangs this exact script (rc=124 under a 8 s kill).
#[test]
fn nvm_ls_remote_shape_mocked_download_terminates() {
    let dir = std::env::temp_dir().join(format!("rubash-issue2499-nvm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue2499 nvm scratch dir");

    // Payload: the index.tab column layout (11 fields, $10 = LTS codename),
    // codenames on even rows exactly like the recorded GNU run.
    let codenames = [
        "Argon", "Boron", "Carbon", "Dubnium", "Erbium", "Fermium", "Gallium", "Hydrogen", "Iron",
        "Jod", "Krypton",
    ];
    let mut payload =
        String::from("version\tdate\tfiles\tnpm\tv8\tuv\tzlib\topenssl\tmodules\tlts\tsecurity\n");
    for i in 0..5000usize {
        let lts = if i % 2 == 0 { codenames[i % 11] } else { "-" };
        payload.push_str(&format!(
            "v{}.{}.{}\t2026-01-02\taix_ppc64\tnpm\tv8\tuv\tzlib\topenssl\tmodules\t{lts}\t0\n",
            i % 23,
            i % 17,
            i % 13,
        ));
    }
    let payload_path = dir.join("index.tab");
    std::fs::write(&payload_path, payload.as_bytes()).expect("write nvm payload");

    let cat = resolve_tool("cat");
    let sed = resolve_tool("sed");
    let head = resolve_tool("head");
    let awk = resolve_tool("awk");
    let sort = resolve_tool("sort");
    let mkdir = resolve_tool("mkdir");
    let script = format!(
        "SANDBOX=\"$(cd \"$(dirname \"$0\")\" && pwd)\"\n\
         export NVM_DIR=\"$SANDBOX/nvmdir\"\n\
         '{mkdir}' -p \"$NVM_DIR\"\n\
         nvm_echo() {{ printf '%b\\n' \"$*\"; }}\n\
         nvm_download() {{ '{cat}' '{payload}'; }}\n\
         nvm_make_alias() {{ '{mkdir}' -p \"$NVM_DIR/alias/lts\"; printf '%s' \"$2\" > \"$NVM_DIR/alias/lts/$1\"; }}\n\
         nvm_ls_remote_index_tab() {{\n\
         \x20 local VERSION_LIST\n\
         \x20 VERSION_LIST=\"$(nvm_download -L -s \"https://nodejs.org/dist/index.tab\" -o - \\\n\
         \x20   | '{sed}' \"\n\
         \x20       1d;\n\
         \x20       s/^//;\n\
         \x20     \" \\\n\
         \x20 )\"\n\
         \x20 '{mkdir}' -p \"$NVM_DIR/alias\"\n\
         \x20 {{ '{awk}' '{{\n\
         \x20       if ($10 ~ /^\\-?$/) {{ next }}\n\
         \x20       if ($10 && !a[tolower($10)]++) {{ print \"lts/\" tolower($10) }}\n\
         \x20     }}' \\\n\
         \x20   | while read -r LTS_ALIAS_LINE; do\n\
         \x20     nvm_make_alias \"${{LTS_ALIAS_LINE#lts/}}\" \"v1.0.0\" >/dev/null 2>&1\n\
         \x20   done; }} << EOF\n\
         $VERSION_LIST\n\
         EOF\n\
         \x20 local VERSIONS\n\
         \x20 VERSIONS=\"$( {{ '{awk}' '{{ if ($10 !~ /^\\-?$/) print $1, $10, \"*\" }}' | '{sort}'; }} << EOF\n\
         $VERSION_LIST\n\
         EOF\n\
         )\"\n\
         \x20 nvm_echo \"${{VERSIONS}}\"\n\
         }}\n\
         nvm_ls_remote_index_tab node std '.*' 2>&1 | '{head}' -3\n\
         echo \"rc=$?\"\n",
        payload = payload_path.to_string_lossy().replace('\\', "/"),
    );
    let out = run_rubash_script("nvm", &script);
    assert!(
        !out.timed_out,
        "nvm ls-remote shape did not terminate within {RUN_LIMIT:?} (rubash#2499: \
         the `2>&1 | head -3` early-reader deadlock of the nvm ls-remote chain)"
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "v0.0.0 Argon *\nv0.0.10 Fermium *\nv0.0.12 Gallium *\nrc=0\n",
        "stdout must match the GNU 5.3.20 byte baseline of the mocked nvm \
         ls-remote --lts | head -3 run (LC_ALL=C)"
    );
    assert_eq!(out.code, Some(0), "probe exit status");
    assert!(
        out.stderr.is_empty(),
        "stderr must stay empty, got: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Control from the issue's own matrix: without the stderr merge the same
/// chain completes — pin that the merged-capture path is what must not
/// wedge, not the early reader alone.
#[test]
fn control_without_stderr_merge_completes() {
    let dir = std::env::temp_dir().join(format!("rubash-issue2499-ctl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue2499 ctl scratch dir");
    let (payload, remaining) = write_bulk_payload(&dir);
    let cat = resolve_tool("cat");
    let sed = resolve_tool("sed");
    let head = resolve_tool("head");
    let script = format!(
        "f() {{\n\
         \x20 V=\"$('{cat}' '{payload}' | '{sed}' \"\n\
         \x20   1d;\n\
         \x20   s/^//;\n\
         \x20 \")\"\n\
         \x20 printf 'v1 %s\\nv2\\nv3\\nv4\\nv5\\n' \"${{#V}}\"\n\
         }}\n\
         f 2>/dev/null | '{head}' -3\n\
         echo \"rc=$?\"\n"
    );
    let out = run_rubash_script("ctl", &script);
    assert!(
        !out.timed_out,
        "control (2>/dev/null) did not terminate within {RUN_LIMIT:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("v1 {remaining}\nv2\nv3\nrc=0\n"),
        "control stdout must match the GNU byte baseline"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Control from the issue's own matrix: a full reader (`| cat`) keeps the
/// pipe open to the end, so the chain completes even with the stderr merge.
#[test]
fn control_full_reader_completes() {
    let dir = std::env::temp_dir().join(format!("rubash-issue2499-cat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue2499 cat scratch dir");
    let (payload, remaining) = write_bulk_payload(&dir);
    let cat = resolve_tool("cat");
    let sed = resolve_tool("sed");
    let script = format!(
        "f() {{\n\
         \x20 V=\"$('{cat}' '{payload}' | '{sed}' \"\n\
         \x20   1d;\n\
         \x20   s/^//;\n\
         \x20 \")\"\n\
         \x20 printf 'v1 %s\\nv2\\nv3\\nv4\\nv5\\n' \"${{#V}}\"\n\
         }}\n\
         f 2>&1 | '{cat}'\n\
         echo \"rc=$?\"\n"
    );
    let out = run_rubash_script("cat", &script);
    assert!(
        !out.timed_out,
        "control (| cat) did not terminate within {RUN_LIMIT:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("v1 {remaining}\nv2\nv3\nv4\nv5\nrc=0\n"),
        "control stdout must match the GNU byte baseline"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
