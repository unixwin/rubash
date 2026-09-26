// recho -- really echo args, bracketed with <> and with invisible chars
// made visible. Faithful Windows port of third_party/bash/support/recho.c
// (Chet Ramey, FSF): `argv[i] = <str>` per argument, control chars as ^X,
// DEL as ^?. Built by scripts/true-baseline.sh ensure_test_helpers into
// bash-tests-rw/recho.exe so the rubash side spawns a REAL helper process
// instead of carrying a test-only emulation in the executor.
use std::io::Write;

fn strprint(out: &mut Vec<u8>, s: &[u8]) {
    for &b in s {
        if b < b' ' {
            out.push(b'^');
            out.push(b + 64);
        } else if b == 127 {
            out.push(b'^');
            out.push(b'?');
        } else {
            out.push(b);
        }
    }
}

fn main() {
    let mut out = Vec::new();
    for (index, arg) in std::env::args_os().skip(1).enumerate() {
        let bytes = arg.to_string_lossy().as_bytes().to_vec();
        out.extend_from_slice(format!("argv[{}] = <", index + 1).as_bytes());
        strprint(&mut out, &bytes);
        out.extend_from_slice(b">\n");
    }
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(&out);
}
