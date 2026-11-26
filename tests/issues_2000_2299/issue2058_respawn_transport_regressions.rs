//! rubash#2058 — respawn transport-set completeness. GNU's fork-shaped
//! children (subst.c:6362 process_substitute, execute_cmd.c:1576
//! execute_in_subshell, coproc make_child) inherit the parent's
//! dollar_vars and call-stack arrays verbatim (variables.c); a
//! respawned `rubash -c` child re-imports only its process
//! environment, so the fork state crosses through the
//! `__RUBASH_FORK_*` transport stamped at the four spawn sites and
//! consumed at Executor birth (docs/design/
//! respawn-transport-census.md). These pins cover the TRANSPORT
//! verdicts per item; expectations measured against GNU bash 5.3.20
//! with script files (2026-10-30).
//!
//! Unix-only: the shapes spell POSIX paths and rely on `sleep` /
//! procsub timing idioms the Windows persona exercises through the
//! goldens lane instead.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

struct Case {
    dir: PathBuf,
}

fn make_case(tag: &str) -> Case {
    let dir = std::env::temp_dir().join(format!("rubash2058-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    Case { dir }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Run `script` as ./case.sh with args A B inside the case dir and
/// return its stdout. The script gets two prelude lines binding $OUT
/// (an absolute path inside the dir) and a trailing `cat "$OUT"`;
/// respawn bodies append their readings to $OUT, so the returned
/// text is the accumulated readings in completion order.
fn run_script(case: &Case, script: &str) -> String {
    let path = case.dir.join("case.sh");
    let full = format!(
        "OUT=\"{}\"\n: > \"$OUT\"\n{}\ncat \"$OUT\"\n",
        case.dir.join("out.txt").display(),
        script
    );
    std::fs::write(&path, full).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&case.dir)
        .arg("case.sh")
        .arg("A")
        .arg("B")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    assert_eq!(
        output.status.code(),
        Some(0),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// 1-based line number of the first line containing `needle`, in the
/// full script layout run_script produces (two prelude lines + body).
fn line_of(body: &str, needle: &str) -> usize {
    format!("OUT=\n: >\n{body}")
        .lines()
        .position(|line| line.contains(needle))
        .map(|index| index + 1)
        .unwrap_or_else(|| panic!("needle {needle:?} not in script"))
}

#[test]
fn issue2058_procsub_child_inherits_extdebug_argv_frames() {
    // FX context: f's frame plus the real bottom frame (script args),
    // innermost first (variables.c push_args / init_bash_argv).
    let case = make_case("fxargv");
    let stdout = run_script(
        &case,
        "shopt -s extdebug\n\
         f() {\n\
         cat < <(echo \"argc=[${BASH_ARGC[*]}] argv=[${BASH_ARGV[*]}]\" >> \"$OUT\")\n\
         }\n\
         f x y\n",
    );
    assert_eq!(stdout, "argc=[2 2] argv=[y x B A]\n");
}

#[test]
fn issue2058_procsub_child_argv_stays_unmaterialized_in_function() {
    // F context without extdebug: the arrays never materialized, and
    // variables.c:1656 get_bashargcv initializes only at
    // variable_context == 0 — the fork child reads inside the
    // inherited function context and must stay empty (a bottom frame
    // synthesized from the transported window would read [y x]).
    let case = make_case("funmat");
    let stdout = run_script(
        &case,
        "f() {\n\
         cat < <(echo \"argc=[${BASH_ARGC[*]}] argv=[${BASH_ARGV[*]}]\" >> \"$OUT\")\n\
         }\n\
         f x y\n",
    );
    assert_eq!(stdout, "argc=[] argv=[]\n");
}

#[test]
fn issue2058_procsub_child_argv_bottom_frame_is_the_materialized_one() {
    // The bottom frame freezes at materialization time: a top-level
    // read installs (B A); a later `set --` must not move the frame
    // the fork child inherits.
    let case = make_case("frozen");
    let stdout = run_script(
        &case,
        "echo \"top=[${BASH_ARGV[*]}]\" >> \"$OUT\"\n\
         set -- C D\n\
         cat < <(echo \"child=[${BASH_ARGV[*]}]\" >> \"$OUT\")\n",
    );
    assert_eq!(stdout, "top=[B A]\nchild=[B A]\n");
}

#[test]
fn issue2058_procsub_child_inherits_call_stack_views() {
    // FUNCNAME / BASH_SOURCE / BASH_LINENO of the enclosing function
    // are fork state (variables.c): the child reads f's frame above
    // the main frame, with f's call line in BASH_LINENO[0].
    let case = make_case("stacks");
    let body = "f() {\n\
                cat < <(echo \"func=[${FUNCNAME[*]}] src=[${BASH_SOURCE[*]}] blineno=[${BASH_LINENO[*]}]\" >> \"$OUT\")\n\
                }\n\
                f x y\n";
    let stdout = run_script(&case, body);
    let call_line = line_of(body, "f x y");
    assert_eq!(
        stdout,
        format!("func=[f main] src=[case.sh case.sh] blineno=[{call_line} 0]\n")
    );
}

#[test]
fn issue2058_async_function_child_inherits_caller_frames() {
    // `g &` inside f: the respawned child re-enters g above the
    // transported f frame (GNU: FUNCNAME=(g f main), BASH_LINENO =
    // (g's call line, f's call line, 0)); under extdebug the argv
    // frames compose the same way.
    let case = make_case("asyncfn");
    let body = "shopt -s extdebug\n\
                g() {\n\
                echo \"func=[${FUNCNAME[*]}] blineno=[${BASH_LINENO[*]}] argc=[${BASH_ARGC[*]}] argv=[${BASH_ARGV[*]}]\" >> \"$OUT\"\n\
                }\n\
                f() {\n\
                g gq &\n\
                wait\n\
                }\n\
                f x y\n";
    let stdout = run_script(&case, body);
    let g_call = line_of(body, "g gq &");
    let f_call = line_of(body, "f x y");
    assert_eq!(
        stdout,
        format!("func=[g f main] blineno=[{g_call} {f_call} 0] argc=[1 2 2] argv=[gq y x B A]\n")
    );
}

#[test]
fn issue2058_respawn_children_keep_parent_shlvl() {
    // A fork is not a new shell: SHLVL does not increment across any
    // respawn shape (the fresh process's startup increment must not
    // leak in).
    let case = make_case("shlvl");
    let stdout = run_script(
        &case,
        "echo \"main=$SHLVL\" >> \"$OUT\"\n\
         cat < <(echo \"inps=$SHLVL\" >> \"$OUT\")\n\
         ( echo \"asub=$SHLVL\" >> \"$OUT\" ) &\n\
         wait\n\
         coproc CP { echo \"cop=$SHLVL\" >> \"$OUT\"; }\n\
         sleep 0.5\n",
    );
    let main: i64 = stdout
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("main="))
        .and_then(|value| value.parse().ok())
        .expect("main SHLVL line");
    assert_eq!(
        stdout,
        format!("main={main}\ninps={main}\nasub={main}\ncop={main}\n")
    );
}

#[test]
fn issue2058_procsub_child_inherits_pipestatus_at_fork_time() {
    // The PIPESTATUS cell is fork state: the child reads the value
    // the previous pipeline left, before its own commands run.
    let case = make_case("pipestatus");
    let stdout = run_script(
        &case,
        "false | true\n\
         cat < <(echo \"inps=[${PIPESTATUS[*]}]\" >> \"$OUT\")\n\
         false | true\n\
         true > >(echo \"outps=[${PIPESTATUS[*]}]\" >> \"$OUT\")\n\
         sleep 0.5\n",
    );
    assert_eq!(stdout, "inps=[1 0]\noutps=[1 0]\n");
}

#[test]
fn issue2058_coproc_child_dollar_dollar_projects_parent_shell() {
    // The #1582 projection every other spawn site already produced:
    // $$ in the coproc child is the parent shell's pid, BASHPID the
    // child's own. (The coproc site used to omit the
    // __RUBASH_SHELL_PID stamp entirely.)
    let case = make_case("coproc-pid");
    let stdout = run_script(
        &case,
        "echo \"main=$$\" >> \"$OUT\"\n\
         coproc CP { echo \"pid=$$ bashpid=$BASHPID\" >> \"$OUT\"; }\n\
         sleep 0.5\n",
    );
    let mut lines = stdout.lines();
    let main: u32 = lines
        .next()
        .and_then(|line| line.strip_prefix("main="))
        .and_then(|value| value.parse().ok())
        .expect("main pid line");
    let child = lines.next().expect("child pid line").to_string();
    let (pid, bashpid) = child
        .strip_prefix("pid=")
        .and_then(|rest| rest.split_once(" bashpid="))
        .map(|(pid, bashpid)| {
            (
                pid.parse::<u32>().expect("child pid"),
                bashpid.parse::<u32>().expect("child bashpid"),
            )
        })
        .expect("child pid fields");
    assert_eq!(pid, main, "$$ must project the parent shell");
    assert_ne!(bashpid, main, "BASHPID must be the child's own pid");
}

#[test]
fn issue2058_async_subshell_lineno_starts_at_first_body_command() {
    // The transported body collapses onto its seed line in the child,
    // so the seed must be the first body command's own line — not the
    // compound node's closing line (( ... ) & used to seed the `)`
    // line).
    let case = make_case("asub-line");
    let body = "( echo \"start=$LINENO\" >> \"$OUT\"\n\
                echo \"second\" >> \"$OUT\" ) &\n\
                wait\n";
    let stdout = run_script(&case, body);
    let start = line_of(body, "start=$LINENO");
    assert_eq!(stdout, format!("start={start}\nsecond\n"));
}

#[test]
fn issue2058_coproc_lineno_starts_at_first_body_command() {
    // Same start-point rule at the coproc site: the seed is the first
    // body command's line, not the `coproc` keyword's line.
    let case = make_case("coproc-line");
    let body = "coproc CP {\n\
                echo \"start=$LINENO\" >> \"$OUT\"\n\
                }\n\
                sleep 0.5\n";
    let stdout = run_script(&case, body);
    let start = line_of(body, "start=$LINENO");
    assert_eq!(stdout, format!("start={start}\n"));
}

#[test]
fn issue2058_nested_respawn_restamps_the_full_frame_chain() {
    // A respawn child that itself respawns re-stamps the composed
    // (own + inherited) views: the grandchild still sees f's frame.
    let case = make_case("nested");
    let stdout = run_script(
        &case,
        "f() {\n\
         cat < <( ( echo \"grand=[${FUNCNAME[*]}]\" >> \"$OUT\" ) & wait )\n\
         }\n\
         f x y\n",
    );
    assert_eq!(stdout, "grand=[f main]\n");
}

#[test]
fn issue2058_random_parent_sequence_untouched_by_respawn() {
    // DELIBERATELY-OMIT verdict, parent half: variables.c:1412
    // get_random_number reseeds the CHILD in a subshell environment
    // and never touches the parent's generator — after seeding
    // RANDOM=4242 and spawning a drawing procsub child, the parent's
    // draws are exactly the no-spawn GNU sequence.
    let case = make_case("random");
    let stdout = run_script(
        &case,
        "RANDOM=4242\n\
         cat < <(echo \"child=$RANDOM\" >> \"$OUT\")\n\
         echo \"parent=$RANDOM $RANDOM $RANDOM\" >> \"$OUT\"\n",
    );
    let parent = stdout
        .lines()
        .find(|line| line.starts_with("parent="))
        .expect("parent draws line");
    assert_eq!(parent, "parent=25857 21059 5065");
}
