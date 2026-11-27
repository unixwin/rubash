//! rubash#1953 — nested `${a:-…}` default words whose text carries a
//! literal `name=` segment (the c14 construction
//! `x=${a:-x=${a:-…${a:-deep}}}`) blew up exponentially: the assignment
//! pre-scan split each `${...}` body on its first top-level `=`/`:=`,
//! read the `:-` operator's word as an assignment value, and expanded it
//! (nested fragment included) once per nesting level before discarding
//! it — depth 10 took 1778ms and depth 15 never finished, where GNU
//! bash 5.3.20 is millisecond-class at depth 400. The same phantom
//! evaluation also executed word side effects GNU never runs: a command
//! substitution inside an unselected default word ran twice, and three
//! times when the word was selected (pre-scan passes + the real pass).
//!
//! GNU anchor: parameter_brace_expand (subst.c:9777) extracts the
//! parameter name first and dispatches on the FIRST operator character,
//! and parameter_brace_expand_word (subst.c:7663) expands an operator's
//! word only when that operator selects it. All expectations below were
//! measured against GNU bash 5.3.20.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rubash-i1953-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    /// Run a script file under the built rubash with a hard deadline.
    /// Returns (stdout, stderr, rc, elapsed); rc is -1 when the deadline
    /// killed the child (the pre-fix blowup shape) so a regression fails
    /// the pin instead of hanging the suite.
    fn run(&self, script: &str, deadline: Duration) -> (String, String, i32, Duration) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out_path = self.dir.join("out.txt");
        let err_path = self.dir.join("err.txt");
        let out_file = fs::File::create(&out_path).unwrap();
        let err_file = fs::File::create(&err_path).unwrap();
        let start = Instant::now();
        let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::from(err_file))
            .spawn()
            .unwrap();
        let rc = loop {
            match child.try_wait().unwrap() {
                Some(status) => break status.code().unwrap_or(-1),
                None => {
                    if start.elapsed() > deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        break -1;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        };
        let elapsed = start.elapsed();
        let read = |p: &PathBuf| {
            let mut s = String::new();
            fs::File::open(p).unwrap().read_to_string(&mut s).unwrap();
            s
        };
        (read(&out_path), read(&err_path), rc, elapsed)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// The exact c14 construction from the RC audit evidence
/// (confirm-a probes/gen_f2d.py): `depth` levels of `${a:-`, the
/// depth-1 outer levels carrying an `x=` literal prefix in their
/// default word, innermost word `deep`.
fn c14_script(depth: usize) -> String {
    let mut s = String::from("deep");
    for _ in 0..depth - 1 {
        s = format!("x=${{a:-{s}}}");
    }
    s = format!("${{a:-{s}}}");
    format!("x={s}\necho \"len=${{#x}}\"\necho end\n")
}

#[test]
fn issue1953_nested_default_depth_ladder_matches_gnu() {
    let s = Scratch::new("ladder");
    for depth in [10usize, 15, 25] {
        let (out, err, rc, _) = s.run(&c14_script(depth), Duration::from_secs(20));
        assert_eq!(rc, 0, "depth {depth}: stderr: {err}");
        // GNU bash 5.3.20: the value gains one literal `x=` per level
        // over the innermost `deep`, so len = 2*depth + 2.
        assert_eq!(
            out,
            format!("len={}\nend\n", 2 * depth + 2),
            "depth {depth}"
        );
        assert_eq!(err, "", "depth {depth}");
    }
}

#[test]
fn issue1953_depth100_stays_gnu_order_of_magnitude() {
    let s = Scratch::new("d100");
    // Best-of-3 per depth damps process-spawn noise; the growth ratio
    // between depths is the assertion's substance (post-fix growth is
    // polynomial in the word size, the blowup grew by ~phi per level).
    let best = |depth: usize| {
        let mut min = Duration::MAX;
        for _ in 0..3 {
            let (out, err, rc, elapsed) = s.run(&c14_script(depth), Duration::from_secs(30));
            assert_eq!(rc, 0, "depth {depth}: stderr: {err}");
            assert_eq!(
                out,
                format!("len={}\nend\n", 2 * depth + 2),
                "depth {depth}"
            );
            min = min.min(elapsed);
        }
        min
    };
    let d25 = best(25);
    let d100 = best(100);
    assert!(
        d100 <= Duration::from_secs(30),
        "depth 100 took {d100:?} (absolute cap 30s)"
    );
    assert!(
        d100 <= d25 * 50 + Duration::from_secs(10),
        "depth 100 {d100:?} vs depth 25 {d25:?}: growth is not polynomial"
    );
}

#[test]
fn issue1953_default_word_side_effect_runs_once_when_selected() {
    let s = Scratch::new("sel");
    let script = "unset a\necho \"${a:-x=$(echo PHANTOM >&2)}\"\necho after\n";
    let (out, err, rc, _) = s.run(script, Duration::from_secs(20));
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "x=\nafter\n");
    // GNU runs the substitution in the selected word exactly once.
    assert_eq!(err, "PHANTOM\n");
}

#[test]
fn issue1953_default_word_side_effect_never_runs_when_unselected() {
    let s = Scratch::new("unsel");
    let script = "a=set\necho \"${a:-x=$(echo PHANTOM >&2)}\"\necho after\n";
    let (out, err, rc, _) = s.run(script, Duration::from_secs(20));
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "set\nafter\n");
    // GNU never expands an unselected operator word.
    assert_eq!(err, "");
}

#[test]
fn issue1953_assignment_shaped_text_in_default_word_is_not_a_bind() {
    let s = Scratch::new("text");
    // Both the `=` and `:=` spellings inside a `:-` word are literal
    // text: the result is the word itself and `x` stays unset (GNU).
    let script = "unset a\n\
                  echo \"e1=[${a:-x=1}]\"\n\
                  echo \"e2=[${a:-x:=1}]\"\n\
                  echo \"x=[${x-unset}]\"\n";
    let (out, err, rc, _) = s.run(script, Duration::from_secs(20));
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "e1=[x=1]\ne2=[x:=1]\nx=[unset]\n");
    assert_eq!(err, "");
}

#[test]
fn issue1953_genuine_assignment_fragments_still_bind() {
    let s = Scratch::new("bind");
    // The pre-scan's real purpose is untouched: plain, subscripted and
    // associative `=`/`:=` fragments still apply their bind, visible to
    // later fragments of the same word and to later commands (GNU).
    let script = "unset v\necho \"b1=[${v:=42}]\"\necho \"b1v=[$v]\"\n\
                  unset w\necho \"b2=[${w:=one} $w]\"\n\
                  unset u\necho \"b3=[${u=two}]\"\necho \"b3u=[$u]\"\n\
                  unset A\necho \"b4=[${A[1]:=z}]\"\necho \"b4a=[${A[1]}]\"\n\
                  declare -A M\necho \"b5=[${M[k]:=v6}]\"\necho \"b5m=[${M[k]}]\"\n";
    let (out, err, rc, _) = s.run(script, Duration::from_secs(20));
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(
        out,
        "b1=[42]\nb1v=[42]\nb2=[one one]\nb3=[two]\nb3u=[two]\n\
         b4=[z]\nb4a=[z]\nb5=[v6]\nb5m=[v6]\n"
    );
    assert_eq!(err, "");
}
