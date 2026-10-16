//! rubash#651: `unset "a[@]"` flushed the mirror but left the typed cell,
//! so `${#a[@]}` disagreed with `declare -p` — the #638 dual-store class
//! on the unset path. The flush arm now syncs the cell empty like the
//! element-unset path does. GNU owner: arrayfunc.c:1180-1200
//! unbind_array_element (default compat flushes every element).

use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    status: i32,
}

fn run(script: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code().unwrap_or(-1),
    }
}

#[test]
fn flush_star_count_and_render_agree() {
    let out = run("a=(p q); unset \"a[@]\"; echo \"${#a[@]}\"; declare -p a");
    assert_eq!(out.stdout, "0\ndeclare -a a=()\n");
    assert_eq!(out.status, 0);
}

#[test]
fn flush_at_count_and_render_agree() {
    let out = run("a=(p q); unset \"a[*]\"; echo \"${#a[@]}\"; echo \"${!a[@]}\"");
    assert_eq!(out.stdout, "0\n\n");
    assert_eq!(out.status, 0);
}

#[test]
fn flush_sparse_count_agree() {
    let out = run("a=([2]=x [9]=y); unset \"a[@]\"; echo \"${#a[@]}\"");
    assert_eq!(out.stdout, "0\n");
    assert_eq!(out.status, 0);
}

#[test]
fn flush_then_reappend_lands_clean() {
    let out = run("a=(p q); unset \"a[@]\"; a[0]=n; echo \"${#a[@]}\"; declare -p a");
    assert_eq!(out.stdout, "1\ndeclare -a a=([0]=\"n\")\n");
    assert_eq!(out.status, 0);
}

#[test]
fn assoc_flush_untouched() {
    // Assoc `[@]` flush is a no-op for element removal on both shells
    // (battery guard) — pin the agreed bytes, not a guess.
    let out = run("declare -A m=([a]=1 [b]=2); unset \"m[@]\"; echo \"${#m[@]}\"");
    assert_eq!(out.stdout, "2\n");
    assert_eq!(out.status, 0);
}
