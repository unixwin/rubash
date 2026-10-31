//! rubash#2369: the six variables GNU seeds with att_noassign
//! (variables.c:1839 FUNCNAME, :1886 GROUPS, :1889 BASH_ARGC,
//! :1890 BASH_ARGV, :1892 BASH_SOURCE, :1893 BASH_LINENO;
//! ASSIGN_DISALLOWED at variables.h:243) refuse assignment on every
//! path GNU enforces, each with its own shape:
//!
//! - statement / element / compound / append assigns and top-level
//!   declare are SILENT NO-OPS (status 0, value unchanged);
//! - env-prefix assigns are dropped entirely (no store, no export);
//! - an expansion `=`/`:=` that actually attempts the assignment
//!   DISCARDs the enclosing command silently (subst.c:8153-8166 —
//!   the POSIX fatal branch requires readonly_p, so noassign never
//!   fatals there): status 2 in a script file, folded to 1 inside
//!   parse_and_execute (-c / eval / source), preserved as 2 across a
//!   `()` region;
//! - local/declare/typeset inside a function print
//!   `{cmd}: {var}: variable may not be assigned value` and fail 1
//!   (make_local_variable, variables.c), while `declare -g` is a
//!   silent status-0 no-op;
//! - printf -v, read, read -a fail silently with status 1; mapfile
//!   refuses at setup before consuming any input; for/select loop
//!   variables fail 1 before the body runs; getopts' name variable
//!   fails 2; arithmetic assignment fails the evaluation silently
//!   with status 1; `{fd}>` redirection reports
//!   `cannot assign fd to variable` after creating the target file.
//!
//! PIPESTATUS is NOT a carrier (variables.c:6299-6301 plain array)
//! and must keep accepting ordinary stores.
//!
//! All expectations were verified against the vendored GNU bash
//! 5.3.20 oracle binary with script files, per the repo discipline.
//! Cases run as script FILES (the -c shapes have their own runner)
//! in a per-case sandbox directory.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn stamp() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    )
}

struct Run {
    stdout: String,
    stderr: String,
    rc: Option<i32>,
    /// Whether `probe_out.txt` exists in the case sandbox afterwards.
    probe_file_exists: bool,
}

fn run_script(script: &str, stdin: &[u8]) -> Run {
    let sandbox = std::env::temp_dir().join(format!("rubash-2369-{}", stamp()));
    std::fs::create_dir_all(&sandbox).expect("create sandbox");
    let path = sandbox.join("script.sh");
    std::fs::write(&path, script).expect("write script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.current_dir(&sandbox)
        .arg(&path)
        .env("RUBASH_BIN", env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    let probe_file_exists = sandbox.join("probe_out.txt").exists();
    let _ = std::fs::remove_dir_all(&sandbox);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        rc: output.status.code(),
        probe_file_exists,
    }
}

fn run_dash_c(text: &str) -> Run {
    let sandbox = std::env::temp_dir().join(format!("rubash-2369-c-{}", stamp()));
    std::fs::create_dir_all(&sandbox).expect("create sandbox");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&sandbox)
        .arg("-c")
        .arg(text)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash -c");
    let _ = std::fs::remove_dir_all(&sandbox);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        rc: output.status.code(),
        probe_file_exists: false,
    }
}

/// The ticket shape: under -c, BASH_SOURCE is null, so := attempts the
/// assignment, hits att_noassign, and the command is discarded
/// silently; parse_and_execute folds the status to 1.
#[test]
fn dash_c_colon_equals_on_seeded_bash_source_discards_silently() {
    let run = run_dash_c("echo \"val=[${BASH_SOURCE:=X}]\"; echo AFTER");
    assert_eq!(run.stdout, "", "stderr: {:?}", run.stderr);
    assert_eq!(run.stderr, "");
    assert_eq!(run.rc, Some(1));
}

/// Inside a -c string the discard is caught per command: the next
/// line still runs and observes status 1, not 2.
#[test]
fn dash_c_discard_folds_to_one_and_string_continues() {
    let run = run_dash_c("echo \"val=[${BASH_SOURCE:=X}]\"\necho st=$?");
    assert_eq!(run.stdout, "st=1\n", "stderr: {:?}", run.stderr);
    assert_eq!(run.stderr, "");
    assert_eq!(run.rc, Some(0));
}

/// A `()` region is not a parse_and_execute catch: the discard status
/// stays 2 for the enclosing -c string.
#[test]
fn dash_c_subshell_region_preserves_discard_status_two() {
    let run = run_dash_c("( echo \"val=[${BASH_SOURCE:=X}]\" ); echo st=$?");
    assert_eq!(run.stdout, "st=2\n", "stderr: {:?}", run.stderr);
    assert_eq!(run.rc, Some(0));
}

/// In a script file the same discard keeps EX_BADUSAGE: the next line
/// observes 2 and the script continues.
#[test]
fn script_element_colon_equals_discards_with_status_two() {
    let run = run_script(
        "echo \"val=[${BASH_SOURCE[9]:=X}]\"\necho \"st=$? n=${#BASH_SOURCE[@]} elem9=[${BASH_SOURCE[9]-UNSET}]\"\n",
        b"",
    );
    assert_eq!(
        run.stdout, "st=2 n=1 elem9=[UNSET]\n",
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.stderr, "");
    assert_eq!(run.rc, Some(0));
}

/// A single-line script whose only command is discarded exits 2.
#[test]
fn script_single_discarded_command_exits_two() {
    let run = run_script("echo \"val=[${BASH_ARGV:=X}]\"\n", b"");
    assert_eq!(run.stdout, "", "stderr: {:?}", run.stderr);
    assert_eq!(run.stderr, "");
    assert_eq!(run.rc, Some(2));
}

/// FUNCNAME is a carrier too: at top level it is unset, so :=
/// attempts the bind and discards.
#[test]
fn script_funcname_colon_equals_discards() {
    let run = run_script(
        "echo \"val=[${FUNCNAME:=X}]\"\necho \"st=$?\"\n",
        b"",
    );
    assert_eq!(run.stdout, "st=2\n", "stderr: {:?}", run.stderr);
    assert_eq!(run.stderr, "");
}

/// A nameref-mediated := resolves to the carrier and discards the
/// same way; a nameref-mediated plain statement assign binds through
/// the funnel and is swallowed silently (status 0, value unchanged).
#[test]
fn nameref_mediated_assignments_hit_the_carrier() {
    let run = run_script(
        "declare -n r=BASH_SOURCE\nr=zzz\necho \"stmt st=$? base=[${BASH_SOURCE[0]##*/}]\"\necho \"val=[${r[9]:=X}]\"\necho \"after st=$?\"\n",
        b"",
    );
    assert_eq!(
        run.stdout, "stmt st=0 base=[script.sh]\nafter st=2\n",
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.stderr, "");
}

/// The non-enforced faces are silent no-ops with status 0 and the
/// seeded value untouched: plain, element, compound, append, and
/// top-level declare with a value.
#[test]
fn statement_assign_family_is_silent_noop() {
    let run = run_script(
        "BASH_SOURCE=zzz; echo \"plain=$?\"\nBASH_SOURCE[0]=zzz; echo \"elem=$?\"\nBASH_SOURCE=(a b); echo \"compound=$?\"\nBASH_SOURCE+=x; echo \"append=$?\"\ndeclare BASH_SOURCE=zzz; echo \"declare=$?\"\necho \"n=${#BASH_SOURCE[@]} base=[${BASH_SOURCE[0]##*/}]\"\n",
        b"",
    );
    assert_eq!(
        run.stdout,
        "plain=0\nelem=0\ncompound=0\nappend=0\ndeclare=0\nn=1 base=[script.sh]\n",
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.stderr, "");
    assert_eq!(run.rc, Some(0));
}

/// Non-trigger controls: `:-` never assigns, and `:=` on a set,
/// non-null element does not attempt a bind — no discard either way.
#[test]
fn expansion_non_triggers_do_not_discard() {
    let run = run_script(
        "v=${BASH_SOURCE[0]:-def}; echo \"dash st=$? tail=[${v: -9}]\"\nv=${BASH_SOURCE[0]:=X}; echo \"set st=$? tail=[${v: -9}]\"\n",
        b"",
    );
    assert_eq!(
        run.stdout, "dash st=0 tail=[script.sh]\nset st=0 tail=[script.sh]\n",
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.stderr, "");
    assert_eq!(run.rc, Some(0));
}

/// Env-prefix: the binding is dropped entirely — the command runs
/// with its own status and a child shell does NOT inherit the value,
/// unlike an ordinary variable.
#[test]
fn env_prefix_assignment_is_dropped_not_exported() {
    let run = run_script(
        "BASH_SOURCE=INJECTED true; echo \"st=$?\"\nBASH_SOURCE=INJECTED \"$RUBASH_BIN\" -c 'printf \"child=<%s>\\n\" \"$BASH_SOURCE\"'\nMYV=INJECTED \"$RUBASH_BIN\" -c 'printf \"myv=<%s>\\n\" \"$MYV\"'\n",
        b"",
    );
    assert_eq!(
        run.stdout, "st=0\nchild=<>\nmyv=<INJECTED>\n",
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.stderr, "");
}

/// local/declare inside a function: every operand form fails 1 with
/// the `variable may not be assigned value` diagnostic naming the
/// variable; `declare -g` is the silent status-0 exception.
#[test]
fn local_and_declare_in_function_report_and_fail() {
    let run = run_script(
        "f() {\n  local BASH_SOURCE; echo \"bare=$?\"\n  local BASH_SOURCE=x; echo \"valued=$?\"\n  local 'BASH_SOURCE[0]=x'; echo \"elem=$?\"\n  declare BASH_LINENO=1; echo \"declare=$?\"\n  declare -g BASH_SOURCE=x; echo \"global=$?\"\n}\nf\n",
        b"",
    );
    assert_eq!(
        run.stdout, "bare=1\nvalued=1\nelem=1\ndeclare=1\nglobal=0\n",
        "stderr: {:?}",
        run.stderr
    );
    assert!(
        run.stderr
            .contains("BASH_SOURCE: variable may not be assigned value"),
        "stderr: {:?}",
        run.stderr
    );
    assert!(
        run.stderr
            .contains("BASH_LINENO: variable may not be assigned value"),
        "stderr: {:?}",
        run.stderr
    );
}

/// printf -v refuses silently (status 1), scalar and element forms,
/// on any carrier — here BASH_SOURCE and GROUPS.
#[test]
fn printf_v_refuses_silently() {
    let run = run_script(
        "printf -v BASH_SOURCE x; echo \"scalar=$?\"\nprintf -v 'BASH_SOURCE[0]' x; echo \"elem=$?\"\nprintf -v 'GROUPS[0]' x; echo \"groups=$?\"\necho \"base=[${BASH_SOURCE[0]##*/}]\"\n",
        b"",
    );
    assert_eq!(
        run.stdout, "scalar=1\nelem=1\ngroups=1\nbase=[script.sh]\n",
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.stderr, "");
}

/// read and read -a refuse silently with status 1 and leave the
/// carrier untouched.
#[test]
fn read_family_refuses_silently() {
    let run = run_script(
        "printf 'v\\n' | { read BASH_SOURCE; echo \"read=$?\"; }\nprintf 'a b\\n' | { read -a BASH_SOURCE; echo \"reada=$? n=${#BASH_SOURCE[@]}\"; }\nprintf 'v\\n' | { read GROUPS; echo \"groups=$?\"; }\n",
        b"",
    );
    assert_eq!(
        run.stdout, "read=1\nreada=1 n=1\ngroups=1\n",
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.stderr, "");
}

/// mapfile refuses at setup: status 1 and NO input is consumed, so a
/// following read still sees the first line.
#[test]
fn mapfile_refuses_before_consuming_input() {
    let run = run_script(
        "printf 'l1\\nl2\\n' | { mapfile BASH_SOURCE; echo \"map=$?\"; read x; echo \"x=[$x]\"; }\n",
        b"",
    );
    assert_eq!(
        run.stdout, "map=1\nx=[l1]\n",
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.stderr, "");
}

/// for/select loop variables fail 1 before the body ever runs (the
/// select menu still prints, to stderr).
#[test]
fn loop_variables_fail_before_body() {
    let run = run_script(
        "for BASH_SOURCE in a; do echo BODY; done; echo \"for=$?\"\nfor GROUPS in a; do echo BODY; done; echo \"forgroups=$?\"\n",
        b"",
    );
    assert_eq!(
        run.stdout, "for=1\nforgroups=1\n",
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.stderr, "");

    let run = run_script(
        "select BASH_SOURCE in one two; do echo BODY; break; done; echo \"sel=$?\"\n",
        b"1\n",
    );
    assert_eq!(run.stdout, "sel=1\n", "stderr: {:?}", run.stderr);
}

/// getopts' name variable is refused with status 2, silently. (A
/// `while getopts ...` loop over it therefore never iterates, and
/// the loop itself reports 0 — plain while semantics, same in GNU.)
#[test]
fn getopts_name_variable_fails_with_status_two() {
    let run = run_script(
        "set -- -a\ngetopts \"a\" BASH_SOURCE; echo \"st=$?\"\n",
        b"",
    );
    assert_eq!(run.stdout, "st=2\n", "stderr: {:?}", run.stderr);
    assert_eq!(run.stderr, "");
}

/// Arithmetic assignment to a carrier — scalar, element, via let,
/// via a nameref — fails the whole evaluation silently with 1.
#[test]
fn arithmetic_assignment_fails_silently() {
    let run = run_script(
        "((BASH_ARGC = 3)); echo \"arith=$?\"\n((BASH_SOURCE[0] = 3)); echo \"elem=$?\"\nlet 'BASH_ARGC=3'; echo \"let=$?\"\ndeclare -n r=BASH_ARGC; ((r = 3)); echo \"nameref=$?\"\necho \"argc=[${BASH_ARGC}]\"\n",
        b"",
    );
    assert_eq!(
        run.stdout, "arith=1\nelem=1\nlet=1\nnameref=1\nargc=[0]\n",
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.stderr, "");
}

/// `{fd}>` on a carrier reports `cannot assign fd to variable`,
/// fails 1, and the target file has still been created.
#[test]
fn dynamic_fd_redirect_refuses_after_creating_file() {
    let run = run_script(
        "exec {BASH_ARGC}>probe_out.txt; echo \"fd=$?\"\n",
        b"",
    );
    assert_eq!(run.stdout, "fd=1\n", "stderr: {:?}", run.stderr);
    assert!(
        run.stderr.contains("cannot assign fd to variable"),
        "stderr: {:?}",
        run.stderr
    );
    assert!(run.probe_file_exists);
}

/// PIPESTATUS is not a noassign carrier (variables.c:6299-6301
/// plain array): the faces that REFUSE a carrier accept it — an
/// element := assigns instead of discarding the command, and
/// printf -v stores instead of failing 1. (The stored values are
/// not asserted: GNU and rubash both rewrite PIPESTATUS after every
/// pipeline, so a store is only observable inside its own command.)
#[test]
fn pipestatus_is_not_a_carrier() {
    let run = run_script(
        "printf -v 'PIPESTATUS[0]' 5; echo \"printf=$?\"\necho \"v=[${PIPESTATUS[9]:=5}]\"\n",
        b"",
    );
    assert_eq!(
        run.stdout, "printf=0\nv=[5]\n",
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.stderr, "");
}

/// GROUPS is a carrier (variables.c:1886): it cannot be unset-proof
/// like BASH_SOURCE (that is att_nounset, a different attribute) —
/// `unset GROUPS` succeeds — but its elements refuse := discards
/// like the other carriers.
#[test]
fn groups_unset_succeeds_but_element_assign_discards() {
    let run = run_script(
        "echo \"val=[${GROUPS[99]:=X}]\"\necho \"discard st=$?\"\nunset GROUPS; echo \"unset=$?\"\n",
        b"",
    );
    assert_eq!(
        run.stdout, "discard st=2\nunset=0\n",
        "stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.stderr, "");
}
