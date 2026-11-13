//! rubash#3387 — `times` in a subshell / command substitution /
//! pipeline element / process substitution must print that fork
//! face's own near-zero clocks, not the host process's accumulated
//! rusage.
//!
//! GNU anchor: builtins/times.def:65-84 — `times_builtin` reads
//! getrusage(RUSAGE_SELF) for the first line and
//! getrusage(RUSAGE_CHILDREN) for the second; every one of those
//! faces is a true fork in GNU (execute_cmd.c:1576
//! execute_in_subshell, subst.c:7143 command_substitute, subst.c:6362
//! process_substitute, execute_cmd.c:2620 execute_pipeline), so the
//! child's rusage starts near zero. rubash runs those faces
//! in-process, so the fix snapshots the process counters at each
//! fork-face entry and `times` reports current minus the innermost
//! baseline.
//!
//! The pins burn a bounded amount of CPU in the parent (50k
//! arithmetic iterations — far above the 20ms bar in either build,
//! per the #2932 burn note) and require each fork face's SELF user
//! time to be at most 20ms and below the parent's. The lastpipe
//! last element is the negative control: GNU runs it in the current
//! shell (execute_cmd.c:2758), so its `times` must still print the
//! accumulated parent value.
//!
//! The parent control prints its `times` directly (not through a
//! capture — any capture IS a fork face); the faces' readings are
//! tagged lines parsed here with the #2932 `timeval_millis`
//! convention.

use std::process::Command;

fn run_script(script: &str) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

/// `{mins}m{secs}.{millis:03}s` -> total milliseconds.
fn timeval_millis(field: &str) -> u64 {
    let (mins, rest) = field.split_once('m').expect("minutes separator");
    let (secs, frac) = rest.split_once('.').expect("decimal point");
    let frac = frac.strip_suffix('s').expect("seconds suffix");
    assert_eq!(frac.len(), 3, "field: {field}");
    mins.parse::<u64>().expect("minutes") * 60_000
        + secs.parse::<u64>().expect("seconds") * 1_000
        + frac.parse::<u64>().expect("millis")
}

fn self_user_millis(times_line: &str) -> u64 {
    let first = times_line.split(' ').next().expect("SELF user field");
    timeval_millis(first)
}

fn tagged_millis(stdout: &str, tag: &str) -> u64 {
    let line = stdout
        .lines()
        .find(|line| line.starts_with(tag))
        .unwrap_or_else(|| panic!("missing {tag} line, stdout: {stdout}"));
    let field = line
        .strip_prefix(tag)
        .expect("tag prefix")
        .trim_start();
    timeval_millis(field)
}

// One 50k-iteration burn shared by the parent control and all four
// fork faces (subshell, command substitution, pipeline element,
// process substitution).
const FACES_SCRIPT: &str = r#"
i=0; while ((i<50000)); do ((i+=1)); done
times
echo "subshell-begin"; ( times ); echo "subshell-end"
cout=$(times); echo "comsub ${cout%% *}"
times | { read -r a b; read -r c d; echo "pipeline $a"; }
{ read -r a b; read -r c d; echo "procsub $a"; } < <(times)
"#;

#[test]
fn fork_faces_report_near_zero_times() {
    let (ok, stdout) = run_script(FACES_SCRIPT);
    assert!(ok, "script failed, stdout: {stdout}");
    let lines: Vec<&str> = stdout.lines().collect();
    let parent_line = lines.first().expect("parent times line");
    let parent = self_user_millis(parent_line);
    assert!(
        parent > 20,
        "parent control must show a busy shell (SELF user > 20ms), got: {parent_line}"
    );
    let subshell_begin = lines
        .iter()
        .position(|line| *line == "subshell-begin")
        .expect("subshell-begin marker");
    let subshell_line = lines
        .get(subshell_begin + 1)
        .expect("subshell times line");
    let subshell = self_user_millis(subshell_line);
    assert!(
        subshell <= 20 && subshell < parent,
        "subshell SELF user must be near zero and below the parent, got subshell={subshell} parent={parent}"
    );
    let comsub = tagged_millis(&stdout, "comsub ");
    assert!(
        comsub <= 20 && comsub < parent,
        "comsub SELF user must be near zero and below the parent, got comsub={comsub} parent={parent}"
    );
    let pipeline = tagged_millis(&stdout, "pipeline ");
    assert!(
        pipeline <= 20 && pipeline < parent,
        "pipeline SELF user must be near zero and below the parent, got pipeline={pipeline} parent={parent}"
    );
    let procsub = tagged_millis(&stdout, "procsub ");
    assert!(
        procsub <= 20 && procsub < parent,
        "procsub SELF user must be near zero and below the parent, got procsub={procsub} parent={parent}"
    );
}

#[test]
fn lastpipe_last_stage_keeps_accumulated_times() {
    let script = r#"
shopt -s lastpipe
i=0; while ((i<50000)); do ((i+=1)); done
true | times
"#;
    let (ok, stdout) = run_script(script);
    assert!(ok, "script failed, stdout: {stdout}");
    let first = stdout.lines().next().expect("lastpipe times line");
    let lastpipe = self_user_millis(first);
    assert!(
        lastpipe > 20,
        "lastpipe last element runs in the current shell, so its SELF user must stay accumulated (> 20ms), got: {first}"
    );
}
