//! rubash#4189 regressions: a `<(...)` producer spawned from a
//! redirect target saw the fd state from BEFORE the command's whole
//! redirect list instead of the state at its own entry's turn — a
//! `2>/dev/null` written BEFORE the `<` entry never reached the
//! producer (its stderr leaked to the shell's stderr), and a `0<file`
//! prefix never became the producer's stdin. The capture face had
//! the mirror defect: producer bytes held in the background capture
//! pipe were drained only at scope end, so they landed AFTER
//! foreground bytes the producer preceded in GNU's single pipe.
//!
//! GNU's do_redirections (redir.c:237) walks the list in order,
//! expanding each entry's word at its own turn (redirection_expand,
//! redir.c:298); process_substitute (subst.c:6362) forks the
//! producer at that turn with the descriptor table entries [0..p)
//! leave behind. The producer's fd 0 / fd 2 now resolve to that
//! entry-time state on every spawn face:
//!
//! - compound lists: the undo saves are grouped by earning entry
//!   (compound_list_saves_by_index); the state of fd at position p
//!   is the first save of that fd earned at or after p, and the
//!   current table entry when no later entry rebinds the fd.
//! - simple commands: the #2195 prefix spawn plan, computed for the
//!   producer's own list position with fd 0 included (an input
//!   producer keeps its fd 0; only a `>(...)` consumer's fd 0 is
//!   overridden by the substitution pipe).
//! - capture merge: a foreground capture write first drains the
//!   producer bytes already sitting in the background capture pipe,
//!   mirroring the kernel merge order of GNU's one substitution
//!   pipe; the scope-end drain stays as the backstop.
//!
//! The reverse shapes (the redirection AFTER the `<` entry) are the
//! #4135 pitfall family and must keep their behavior exactly: the
//! producer is forked before those entries apply.
//!
//! Every expectation below is byte-verified against the GNU Bash
//! 5.3.20 oracle (script-file batteries, three rounds for the
//! capture battery; the C.UTF-8 and C locales agree on all shapes).

use std::process::{Command, Stdio};

fn rubash_file(script: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-procsub4189-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .env("LC_ALL", lc_all)
        .stdin(Stdio::null())
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

// p1 (simple command, `2>` first), p2 (while loop), p3 (brace
// group), the numbered-binder sibling (n1: `3<` entry after `2>`),
// and a dup prefix (g1: the producer's fd 2 is fd 1's state at the
// dup's turn). p5 rides along as the reverse-order lines (a2/b2/
// c2/n2): their producers are forked before the list's `2>` applies
// and must keep printing to the shell's stderr.
const DISPOSITION_BATTERY: &str = r#"printf 'a1: '; cat 2>/dev/null < <(printf 'o\n'; printf 'e1\n' >&2; printf 'e2\n' >&2; printf 'e3\n' >&2)
printf 'a2: '; cat < <(printf 'o\n'; printf 'A2E\n' >&2) 2>/dev/null
while read -r x; do printf 'b1: %s\n' "$x"; done 2>/dev/null < <(printf 'o\n'; printf 'B1E\n' >&2)
while read -r x; do printf 'b2: %s\n' "$x"; done < <(printf 'o\n'; printf 'B2E\n' >&2) 2>/dev/null
{ cat; } 2>/dev/null < <(printf 'c1o\n'; printf 'C1E\n' >&2)
echo c1done
{ cat; } < <(printf 'c2o\n'; printf 'C2E\n' >&2) 2>/dev/null
echo c2done
{ read -r x <&3; printf 'n1: %s\n' "$x"; } 2>/dev/null 3< <(printf 'o\n'; printf 'N1E\n' >&2)
{ read -r x <&3; printf 'n2: %s\n' "$x"; } 3< <(printf 'o\n'; printf 'N2E\n' >&2) 2>/dev/null
printf 'g1: '; cat 2>&1 1>/dev/null < <(printf 'o\n'; printf 'G1E\n' >&2); echo g1done
"#;

const DISPOSITION_EXPECTED_STDOUT: &str = "\
a1: o
a2: o
b1: o
b2: o
c1o
c1done
c2o
c2done
n1: o
n2: o
g1: G1E
g1done
";

// Only the reverse-order producers reach the shell's stderr.
const DISPOSITION_EXPECTED_STDERR: &str = "\
A2E
B2E
C2E
N2E
";

// The producer's fd 0 at its entry's turn: f1 (simple) and c3
// (brace group) bind `0<seed.txt` before the `<` entry, so the
// producer body `cat` reads the seed through the pipe. f2/c4 put
// the `0<` entry last: the seed reaches the output through the
// CONSUMER's own final fd 0 binding instead — same bytes, other
// path, and a guard that neither face disturbs it.
const FD0_BATTERY: &str = r#"printf 'seed\n' > seed.txt
printf 'f1: ['; cat 0<seed.txt < <(cat); printf ']\n'
printf 'f2: ['; cat < <(cat) 0<seed.txt; printf ']\n'
printf 'c3: ['; { cat; } 0<seed.txt < <(cat); printf ']\n'
printf 'c4: ['; { cat; } < <(cat) 0<seed.txt; printf ']\n'
"#;

const FD0_EXPECTED_STDOUT: &str = "\
f1: [seed
]
f2: [seed
]
c3: [seed
]
c4: [seed
]
";

// p4 (the redirect-first shape inside a command substitution: the
// producer's stderr is /dev/null'd at its turn and never reaches
// the shell's stderr), the capture merge-order pin (the producer's
// `e` enters the capture pipe at spawn, before the consumer's `o`
// transits the substitution pipe — GNU [e\no], not [o\ne]), and
// the reverse-order capture guard (LASTE still escapes).
const CAPTURE_BATTERY: &str = r#"x=$( { cat < <(printf 'o\n'; printf 'e\n' >&2); } 2>&1 ); printf 'merge: [%s]\n' "$x"
x=$(cat 2>/dev/null < <(printf 'o\n'; printf 'CAPE\n' >&2)); printf 'first: [%s]\n' "$x"
x=$(cat < <(printf 'o\n'; printf 'LASTE\n' >&2) 2>/dev/null); printf 'last: [%s]\n' "$x"
"#;

const CAPTURE_EXPECTED_STDOUT: &str = "\
merge: [e
o]
first: [o]
last: [o]
";

const CAPTURE_EXPECTED_STDERR: &str = "\
LASTE
";

#[test]
fn redirect_first_disposition_battery_utf8_locale() {
    let (stdout, stderr, code) = rubash_file(DISPOSITION_BATTERY, "C.UTF-8");
    assert_eq!(stdout, DISPOSITION_EXPECTED_STDOUT, "stdout mismatch");
    assert_eq!(stderr, DISPOSITION_EXPECTED_STDERR, "stderr mismatch");
    assert_eq!(code, Some(0), "exit code mismatch");
}

#[test]
fn redirect_first_disposition_battery_c_locale() {
    let (stdout, stderr, code) = rubash_file(DISPOSITION_BATTERY, "C");
    assert_eq!(stdout, DISPOSITION_EXPECTED_STDOUT, "stdout mismatch");
    assert_eq!(stderr, DISPOSITION_EXPECTED_STDERR, "stderr mismatch");
    assert_eq!(code, Some(0), "exit code mismatch");
}

#[test]
fn redirect_first_fd0_battery_utf8_locale() {
    let (stdout, stderr, code) = rubash_file(FD0_BATTERY, "C.UTF-8");
    assert_eq!(stdout, FD0_EXPECTED_STDOUT, "stdout mismatch");
    assert_eq!(stderr, "", "stderr mismatch");
    assert_eq!(code, Some(0), "exit code mismatch");
}

#[test]
fn redirect_first_fd0_battery_c_locale() {
    let (stdout, stderr, code) = rubash_file(FD0_BATTERY, "C");
    assert_eq!(stdout, FD0_EXPECTED_STDOUT, "stdout mismatch");
    assert_eq!(stderr, "", "stderr mismatch");
    assert_eq!(code, Some(0), "exit code mismatch");
}

#[test]
fn redirect_first_capture_battery_utf8_locale() {
    let (stdout, stderr, code) = rubash_file(CAPTURE_BATTERY, "C.UTF-8");
    assert_eq!(stdout, CAPTURE_EXPECTED_STDOUT, "stdout mismatch");
    assert_eq!(stderr, CAPTURE_EXPECTED_STDERR, "stderr mismatch");
    assert_eq!(code, Some(0), "exit code mismatch");
}

#[test]
fn redirect_first_capture_battery_c_locale() {
    let (stdout, stderr, code) = rubash_file(CAPTURE_BATTERY, "C");
    assert_eq!(stdout, CAPTURE_EXPECTED_STDOUT, "stdout mismatch");
    assert_eq!(stderr, CAPTURE_EXPECTED_STDERR, "stderr mismatch");
    assert_eq!(code, Some(0), "exit code mismatch");
}
