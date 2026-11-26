//! Issue rubash#1586: `mapfile -C callback` must EVALUATE the callback
//! as a shell command string (GNU builtins/mapfile.def run_callback
//! appends ` <index> <quoted line>` to the string and evals it), not
//! split it on whitespace; and the callback must fire per quantum
//! DURING the read, with the array bound up to (not including) the
//! current record, not after the whole input has been slurped.
//!
//! GNU bash 5.3.20 oracle behavior (script-file probes):
//! - `printf 'L\n' | mapfile -c 1 -C 'f "a b"' A` with
//!   `f(){ echo "argc=$# a2=$2 a3=$3"; }` prints `argc=3 a2=0 a3=L`
//!   (whitespace-splitting printed argc=4 with the arguments shifted).
//! - Quotes, redirections and `;` lists inside the callback all work,
//!   because the string is eval'd whole.
//! - During the callback for record k, the target array already holds
//!   records 0..k-1 and not yet record k.
//! - The line argument keeps its delimiter unless -t is given; -d
//!   changes the delimiter; -O shifts the index passed to the callback.

use std::path::PathBuf;
use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1586-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// The issue seed: the callback string is parsed as a command, so the
/// quoted "a b" stays one argument and index/line land in $2/$3.
#[test]
fn mapfile_callback_is_evaluated_not_whitespace_split() {
    let (stdout, stderr, _code) = rubash(
        "f(){ echo \"argc=$# a2=$2 a3=$3\"; }\nprintf 'L\\n' | mapfile -c 1 -C 'f \"a b\"' A",
    );
    assert!(
        stdout.contains("argc=3 a2=0 a3=L"),
        "callback arguments wrong: stdout={stdout:?} stderr={stderr:?}"
    );
}

/// A redirection inside the callback string takes effect (only
/// possible if the string is really parsed as a command).
#[test]
fn mapfile_callback_supports_redirection_and_lists() {
    let dir = tempdir("redir");
    let (stdout, stderr, _code) = rubash_in(
        &dir,
        "f(){ echo \"argc=$# a2=$2\"; }\nprintf 'q\\n' | mapfile -c 1 -C 'f \"p q\" > out.txt' A\n\
         printf 'q\\n' | mapfile -c 1 -C 'echo CB; f' A",
    );
    let out =
        std::fs::read_to_string(dir.join("out.txt")).expect("callback redirect wrote out.txt");
    assert!(
        out.contains("argc=3 a2=0"),
        "redirected callback output wrong: {out:?} (stderr={stderr:?})"
    );
    assert!(
        stdout.contains("CB") && stdout.contains("argc=2"),
        "`;` list in callback not evaluated: stdout={stdout:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Per-quantum firing during the read: while the callback for record
/// k runs, the array holds records 0..k-1 only (GNU binds the current
/// record after its callback).
#[test]
fn mapfile_callback_sees_array_bound_up_to_previous_record() {
    let (stdout, stderr, _code) =
        rubash("cb(){ echo \"see$1=[${A[*]}]\"; }\nprintf 'l1\\nl2\\n' | mapfile -t -c 1 -C cb A");
    assert!(
        stdout.contains("see0=[]"),
        "first callback should see an empty array: stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        stdout.contains("see1=[l1]"),
        "second callback should see only l1 bound: stdout={stdout:?} stderr={stderr:?}"
    );
}

/// The line argument keeps the delimiter unless -t trims it; -d picks
/// the delimiter; both forms eval the callback the same way.
#[test]
fn mapfile_callback_line_argument_delimiter_forms() {
    let (stdout, stderr, _code) = rubash(
        "f(){ printf 'arg=[%s]\\n' \"$3\"; }\n\
         printf 'x:y:' | mapfile -d : -c 1 -C 'f a' A\n\
         printf 'x:y:' | mapfile -t -d : -c 1 -C 'f a' A",
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        ["arg=[x:]", "arg=[y:]", "arg=[x]", "arg=[y]"],
        "delimiter forms wrong: stdout={stdout:?} stderr={stderr:?}"
    );
}

/// With -O the callback receives the target (origin-shifted) index,
/// and -s skipped records never reach the callback.
#[test]
fn mapfile_callback_index_includes_origin_and_skips() {
    let dir = tempdir("origin");
    std::fs::write(dir.join("in.txt"), "n1\nn2\n").expect("write input");
    let (stdout, stderr, _code) = rubash_in(
        &dir,
        "f(){ echo \"idx=$1\"; }\nA=(old0 old1)\nmapfile -O 5 -c 1 -C 'f' A < in.txt\necho \"A5=[${A[5]}] A0=[${A[0]}]\"",
    );
    assert!(
        stdout.contains("idx=5") && stdout.contains("idx=6"),
        "origin indices wrong: stdout={stdout:?} stderr={stderr:?}"
    );
    // No -t here, so the element keeps its delimiter: GNU prints
    // `A5=[n1<LF>]` (byte-verified against bash 5.3.20) — assert that
    // exact shape, not the trimmed one.
    assert!(
        stdout.contains("A5=[n1\n]") && stdout.contains("A0=[old0]"),
        "origin binding wrong: stdout={stdout:?}"
    );
    let (stdout, stderr, _code) = rubash(
        "f(){ echo \"idx=$1 line=[$2]\"; }\nprintf 's1\\ns2\\nk1\\n' | mapfile -t -s 2 -c 1 -C f A",
    );
    assert!(
        stdout.contains("idx=0 line=[k1]"),
        "skip handling wrong: stdout={stdout:?} stderr={stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Streaming: the first callback must fire while the producer is
/// still holding back the second record — if mapfile slurped to EOF
/// first, the producer's bounded wait expires and `produced2` lands
/// in the log before `cb0`. The producer runs as a coproc and
/// mapfile pulls from its stdout fd (-u): a genuinely concurrent
/// source. (The `prod | mapfile` shape cannot observe read-time
/// interleaving on this tree — a builtin pipeline stage is fed from
/// the executor's parked FUNCTION_STDIN mirror, drained before the
/// builtin starts; same staging seam as rubash#1581. GNU output for
/// this coproc shape is byte-identical: cb0/produced2/cb1.)
#[test]
fn mapfile_callback_fires_during_read_not_after_eof() {
    let dir = tempdir("stream");
    std::fs::write(
        dir.join("prod.sh"),
        "printf 'l1\\n'\nn=0\nwhile [ ! -e go.flag ] && [ $n -lt 120 ]; do sleep 0.05; n=$((n+1)); done\necho produced2 >> order.log\nprintf 'l2\\n'\n",
    )
    .expect("write producer script");
    let script = "\
cb(){ echo \"cb$1\" >> order.log; [ \"$1\" = 0 ] && touch go.flag; }
coproc sh ./prod.sh
mapfile -u \"${COPROC[0]}\" -c 1 -C cb A
cat order.log";
    let (stdout, stderr, _code) = rubash_in(&dir, script);
    let cb0 = stdout.find("cb0");
    let produced = stdout.find("produced2");
    assert!(
        cb0.is_some() && produced.is_some(),
        "streaming log incomplete: stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        cb0 < produced,
        "callback fired only after the producer finished (slurp): stdout={stdout:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
