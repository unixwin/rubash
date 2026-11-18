//! Pipeline stage output must keep its cross-stream production order
//! when the stage's own redirections merge stderr into stdout
//! (rubash#3324; GNU bash 5.3.20 oracle-verified 2026-11-11).
//!
//! GNU `execute_pipeline` (execute_cmd.c:2620) forks each element with
//! the pipe bound to fd 1 and applies the element's own redirections
//! in the child (do_redirections, redir.c:237), so `trap -p BADSIG
//! INT 2>&1 | cat` writes the BADSIG diagnostic and the INT listing
//! onto the same pipe in `display_traps`' per-operand order
//! (builtins/trap.def:333). Rubash's stage result is a
//! (stdout_text, stderr_text) split, and the merge layer appended
//! the whole stderr block after the whole stdout block, so the
//! listing overtook the diagnostic. The pins drive script FILES
//! (diagnostic prefixes carry the script path; the harness
//! normalizes it to `S`) and assert stdout/stderr byte-for-byte.
//!
//! The `type` shapes run in a script WITHOUT an EXIT trap: a
//! builtin stage that goes through the subshell executor fires the
//! parent's inherited EXIT trap at stage exit (missing
//! reset_for_subshell_child in execute_builtin_pipeline_stage —
//! a separate pre-existing defect, not this issue's mechanism),
//! which would pollute the expected output.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    dir: PathBuf,
}

/// Run a script file; return normalized stdout/stderr, exit code, and
/// the temp dir (for asserting files the script created).
fn run_script(tag: &str, body: &str) -> Run {
    let dir = std::env::temp_dir().join(format!("issue3324-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path: PathBuf = dir.join("case.sh");
    fs::write(&path, body).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&path)
        .env("LC_ALL", "C.UTF-8")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let normalize = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes).replace(path.to_str().expect("utf8 path"), "S")
    };
    Run {
        stdout: normalize(&output.stdout),
        stderr: normalize(&output.stderr),
        code: output.status.code(),
        dir,
    }
}

const DIAG: &str = "S: line 2: trap: BADSIG: invalid signal specification\n";
const LISTING: &str = "trap -- 'echo hi' SIGINT\n";

#[test]
fn issue3324_trap_p_pipeline_diag_first() {
    // The issue's canonical shape: the diagnostic for the first
    // operand must precede the listing for the second.
    let run = run_script(
        "canon",
        "trap \"echo hi\" INT\ntrap -p BADSIG INT 2>&1 | cat; echo rc=$?\n",
    );
    assert_eq!(run.stdout, format!("{DIAG}{LISTING}rc=0\n"));
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3324_trap_p_pipeline_list_first() {
    // The reverse order guards against an "always diagnostic first"
    // hack: production order is the operand order, either way.
    let run = run_script(
        "list-first",
        "trap \"echo hi\" INT\ntrap -p INT BADSIG 2>&1 | cat\n",
    );
    assert_eq!(run.stdout, format!("{LISTING}{DIAG}"));
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3324_trap_p_pipeline_sandwich() {
    let run = run_script(
        "sandwich",
        "trap \"echo hi\" INT\ntrap \"echo bye\" EXIT\ntrap -p BADSIG INT EXIT 2>&1 | cat\n",
    );
    // The `trap -p` is on line 3 of this script (the installs are
    // lines 1-2), so the diagnostic names line 3, not line 2.
    assert_eq!(
        run.stdout,
        "S: line 3: trap: BADSIG: invalid signal specification\n".to_string()
            + LISTING
            + "trap -- 'echo bye' EXIT\nbye\n"
    );
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3324_trap_p_pipeline_multi_stage() {
    let run = run_script(
        "multi",
        "trap \"echo hi\" INT\ntrap -p BADSIG INT 2>&1 | cat | cat\n",
    );
    assert_eq!(run.stdout, format!("{DIAG}{LISTING}"));
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3324_trap_cap_p_pipeline_diag_first() {
    // -P shares the same display_traps loop (DISP_ACTIONONLY): the
    // action text prints where the listing would, and the script's
    // own EXIT trap still fires once at the end.
    let run = run_script(
        "cap-diag",
        "trap \"echo bye\" EXIT\ntrap -P BADSIG EXIT 2>&1 | cat\n",
    );
    assert_eq!(run.stdout, format!("{DIAG}echo bye\nbye\n"));
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3324_trap_cap_p_pipeline_action_first() {
    let run = run_script(
        "cap-action",
        "trap \"echo bye\" EXIT\ntrap -P EXIT BADSIG 2>&1 | cat\n",
    );
    assert_eq!(run.stdout, format!("echo bye\n{DIAG}bye\n"));
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3324_printf_pipeline_mixed() {
    // printf formats and diagnoses as it goes (builtin_error is
    // unbuffered, printf.def): the bad-number diagnostic for `bad`
    // sits between the `5` and the `0`/`7` outputs.
    let run = run_script("printf", "printf '%d\\n' 5 bad 7 2>&1 | cat\n");
    assert_eq!(
        run.stdout,
        "5\nS: line 1: printf: bad: invalid number\n0\n7\n"
    );
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3324_printf_direct_merged_order() {
    // The direct form shares the same #1134 root cause (the old
    // stderr-first pair route): `printf '%d\n' 5 bad 7 2>&1` with
    // no pipe must also interleave in production order. GNU oracle:
    // 5, diagnostic, 0, 7.
    let run = run_script("printf-direct", "printf '%d\\n' 5 bad 7 2>&1\n");
    assert_eq!(
        run.stdout,
        "5\nS: line 1: printf: bad: invalid number\n0\n7\n"
    );
    // The conversion error makes printf itself exit 1 (no pipeline
    // to mask it, unlike the piped shape).
    assert_eq!(run.code, Some(1));
}

#[test]
fn issue3324_trap_p_pipeline_stderr_merge() {
    // `1>&2` merges both streams the other way: the listing and the
    // diagnostic both land on the shell's stderr, in production order.
    let run = run_script(
        "stderr-merge",
        "trap \"echo hi\" INT\ntrap -p BADSIG INT 1>&2 | cat\n",
    );
    assert_eq!(run.stdout, "");
    assert_eq!(run.stderr, format!("{DIAG}{LISTING}"));
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3324_trap_p_subshell_control() {
    // The subshell form was already correct (the redirect applies
    // inside the subshell, so the chunk router merges in order).
    // Regression lock.
    let run = run_script(
        "subshell",
        "trap \"echo hi\" INT\n( trap -p BADSIG INT 2>&1 ) | cat\n",
    );
    assert_eq!(run.stdout, format!("{DIAG}{LISTING}"));
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3324_trap_p_direct_control() {
    // The direct form was already correct (#2699 OrderedOutputLog).
    // Regression lock.
    let run = run_script("direct", "trap \"echo hi\" INT\ntrap -p BADSIG INT 2>&1\n");
    assert_eq!(run.stdout, format!("{DIAG}{LISTING}"));
    // The bare builtin (no pipeline) reports its own status: 1 for
    // the invalid operand.
    assert_eq!(run.code, Some(1));
}

#[test]
fn issue3324_type_pipeline_diag_first() {
    // The subshell-stage chunks path (type uses OrderedOutputLog
    // like trap): the diagnostic for the first operand must precede
    // the listing. No EXIT trap installed in this script (see the
    // file header: the builtin stage leaks the parent's EXIT trap).
    let run = run_script("type-diag", "type -a nosuchcmd123 echo 2>&1 | cat\n");
    assert_eq!(
        run.stdout,
        "S: line 1: type: nosuchcmd123: not found\necho is a shell builtin\necho is /usr/bin/echo\necho is /bin/echo\n"
    );
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3324_type_pipeline_list_first() {
    // stdout-first order is already right on HEAD (the block merge
    // coincides with production order); lock it against regressions
    // from the merge-layer change.
    let run = run_script("type-list", "type -a echo nosuchcmd123 2>&1 | cat\n");
    assert_eq!(
        run.stdout,
        "echo is a shell builtin\necho is /usr/bin/echo\necho is /bin/echo\nS: line 1: type: nosuchcmd123: not found\n"
    );
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3324_trap_p_file_merge() {
    // `>f 2>&1`: both streams land in the same file, in production
    // order; the pipe (and cat) gets nothing.
    let run = run_script(
        "file-merge",
        "trap \"echo hi\" INT\ntrap -p BADSIG INT > out.txt 2>&1 | cat\n",
    );
    assert_eq!(run.stdout, "");
    assert_eq!(run.code, Some(0));
    let merged = fs::read_to_string(run.dir.join("out.txt")).expect("read out.txt");
    // The file content carries the real script path in the diagnostic;
    // normalize it exactly like the run helper normalizes the streams.
    let script = format!("{}/case.sh", run.dir.display());
    let merged = merged.replace(&script, "S");
    assert_eq!(merged, format!("{DIAG}{LISTING}"));
}

#[test]
fn issue3324_trap_p_non_merge_order_dependent() {
    // `2>&1 >f` is order-dependent the other way: fd 2 was duped to
    // the pipe BEFORE fd 1 went to the file, so the diagnostic goes
    // down the pipe and only the listing lands in the file. Guards
    // against over-merging.
    let run = run_script(
        "non-merge",
        "trap \"echo hi\" INT\ntrap -p BADSIG INT 2>&1 > out2.txt | cat\n",
    );
    assert_eq!(run.stdout, DIAG);
    assert_eq!(run.code, Some(0));
    let filed = fs::read_to_string(run.dir.join("out2.txt")).expect("read out2.txt");
    assert_eq!(filed, LISTING);
}

#[test]
fn issue3324_trap_p_enclosing_group() {
    // The group's `2>&1` binds fd 2 in the fd table before the
    // trap runs, so the chunk router must merge in order even
    // though the trap command itself carries no redirect.
    let run = run_script(
        "group",
        "trap \"echo hi\" INT\n{ trap -p BADSIG INT; } 2>&1 | cat\n",
    );
    assert_eq!(run.stdout, format!("{DIAG}{LISTING}"));
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3324_trap_p_pipe_ampersand() {
    // `|&` normalizes to a trailing `2>&1`: same merged order.
    let run = run_script(
        "pipe-amp",
        "trap \"echo hi\" INT\ntrap -p BADSIG INT |& cat\n",
    );
    assert_eq!(run.stdout, format!("{DIAG}{LISTING}"));
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3324_trap_p_pipeline_no_redirect_split() {
    // Without a merge the streams stay split: the listing goes down
    // the pipe, the diagnostic to the shell's stderr.
    let run = run_script("split", "trap \"echo hi\" INT\ntrap -p BADSIG INT | cat\n");
    assert_eq!(run.stdout, LISTING);
    assert_eq!(run.stderr, DIAG);
    assert_eq!(run.code, Some(0));
}
