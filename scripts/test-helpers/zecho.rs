// zecho - bare-bones echo. Faithful Windows port of
// third_party/bash/support/zecho.c (FSF): arguments space-separated with a
// single trailing newline, no option or escape processing. Built by
// scripts/true-baseline.sh ensure_test_helpers into bash-tests-rw/zecho.exe.
use std::io::Write;

fn main() {
    let mut out = Vec::new();
    let mut first = true;
    for arg in std::env::args_os().skip(1) {
        if !first {
            out.push(b' ');
        }
        first = false;
        out.extend_from_slice(arg.to_string_lossy().as_bytes());
    }
    out.push(b'\n');
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(&out);
}
