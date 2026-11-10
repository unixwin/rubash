//! niubash#198 regression: the in-engine `external_cat` streaming paths
//! (`stream_inherited_cat`, `stream_cat_stdin_operand`) called
//! `write_cat_output` once per 8 KiB block, and that helper re-created the `>`
//! target on every call — so a stdin larger than one block kept only its last
//! block (`cat > out` with an inherited pipe: 745 of 82665 bytes, rc 0, stderr
//! empty; `cat >> out` and `cat < file > out` were fine because they either
//! append or call the writer once). GNU cat.c opens the output once.
//!
//! Owner: src/executor/external_finish.rs::open_cat_output_sink plus the two
//! streaming callers in src/executor/external_file_builtins.rs.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    // The `bash` bin is a Windows-only shim (required-features
    // windows-shim); on Unix cargo still sets CARGO_BIN_EXE_bash at
    // compile time, but the file is never built. Take it only when it
    // actually exists, else fall through to rubash (the shell under
    // test on this platform). Found while verifying rubash #2885: a
    // fresh `cargo test --test cat_redirect_stream_regressions` fails
    // both tests with spawn NotFound on the never-built shim path.
    if let Some(path) = option_env!("CARGO_BIN_EXE_bash") {
        let p = PathBuf::from(path);
        if p.exists() {
            return p;
        }
    }
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) { "bash.exe" } else { "bash" })
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("rubash-cat-redirect-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run `-c <script>` with `stdin_bytes` on a real pipe — the inherited-stdin
/// shape the regression needs — inside `cwd`.
fn run_with_stdin(script: &str, stdin_bytes: &[u8], cwd: &Path) -> i32 {
    let mut child = Command::new(shell_bin())
        .args(["-c", script])
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn shell");
    let mut stdin = child.stdin.take().expect("stdin pipe");
    let bytes = stdin_bytes.to_vec();
    std::thread::spawn(move || {
        let _ = stdin.write_all(&bytes);
    });
    let output = child.wait_with_output().expect("wait for shell");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.status.code().unwrap_or(-1)
}

#[test]
fn redirect_out_keeps_every_byte_of_a_chunked_stdin() {
    let dir = scratch_dir("out");
    let payload = vec![b'x'; 82_665]; // far past one 8 KiB block
    assert_eq!(run_with_stdin("cat > out.txt", &payload, &dir), 0);
    let written = std::fs::read(dir.join("out.txt")).expect("out.txt");
    assert_eq!(
        written.len(),
        payload.len(),
        "`cat > out` kept only the last block of the stream"
    );
    assert_eq!(written, payload);
}

#[test]
fn append_and_file_redirect_paths_stay_whole() {
    let dir = scratch_dir("append");
    let payload = vec![b'y'; 82_665];
    std::fs::write(dir.join("in.txt"), &payload).expect("seed in.txt");

    assert_eq!(run_with_stdin("cat >> app.txt", &payload, &dir), 0);
    assert_eq!(
        std::fs::read(dir.join("app.txt")).expect("app.txt"),
        payload
    );

    assert_eq!(run_with_stdin("cat < in.txt > from_file.txt", &[], &dir), 0);
    assert_eq!(
        std::fs::read(dir.join("from_file.txt")).expect("from_file.txt"),
        payload
    );
}
