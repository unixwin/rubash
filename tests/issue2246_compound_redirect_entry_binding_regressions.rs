//! Issue rubash#2246 — compound-command redirections bind ONCE at entry
//! (GNU execute_cmd.c:854-883 do_redirections + :1762 dispose), they are
//! no longer deep-cloned and spliced into every leaf of the body.
//!
//! Before this change each compound wrapper (if / loop / for / select /
//! case / subshell / brace group) cloned the whole command node and the
//! body subtree, then rewrote an APPEND-mode copy of each stdio redirect
//! into every leaf command, so every leaf re-opened the target. The
//! wrappers now run only the entry side effects (parse-order expansion,
//! the one create/truncate per file target, noclobber adjudication) and
//! execute the original tree against the fd table bound by
//! with_command_input_redirects — GNU's model, where the body inherits
//! the compound's descriptor table.
//!
//! These pins lock the GNU-visible semantics the splice used to provide
//! implicitly, cell by cell: shared offsets across leaves, independent
//! offsets for repeated targets, dup ordering, nested shielding, the
//! entry-cwd anchor, pipeline last stages, fd-alias targets, heredoc /
//! herestring staging, procsub targets, restricted-shell refusal,
//! failure status + `!` non-flip, and create ordering on failure.
//! Expected outputs follow GNU bash 5.3 semantics (oracle-verified
//! shapes are the ones the #826/#940/#1068/#699 lines established).

use std::path::{Path, PathBuf};
use std::process::Command;

fn rubash_in(script: &str, cwd: &Path) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(cwd)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash(script: &str) -> (String, String, Option<i32>) {
    rubash_in(script, &std::env::temp_dir())
}

fn rubash_args(args: &[&str], cwd: &Path) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2246-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_else(|_| panic!("{name} exists"))
}

/// The if condition AND every branch leaf write through the one entry
/// binding, in execution order (GNU: the condition runs with the
/// compound's redirections in force).
#[test]
fn if_redirect_covers_condition_and_leaves() {
    let dir = scratch_dir("if-basic");
    let (stdout, stderr, code) = rubash_in(
        "if echo cond; then echo then1; echo then2; fi >f\necho \"f=[$(cat f)]\"\necho rc=$?",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[cond\nthen1\nthen2]\nrc=0\n");
    assert_eq!(code, Some(0));
}

/// Shared open file description across leaves: two sequential writes
/// advance ONE offset (no per-leaf truncate, no per-leaf append reopen
/// needed for correctness — the offset itself is shared).
#[test]
fn brace_leaves_share_one_offset() {
    let dir = scratch_dir("shared-offset");
    let (stdout, stderr, code) = rubash_in(
        "{ printf 'aaa'; printf 'bbb'; } >f\necho \"f=[$(cat f)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[aaabbb]\n");
    assert_eq!(code, Some(0));
}

/// GNU #1068: `>f 2>f` is TWO independent opens — the fd-2 write starts
/// at offset 0 of its own description and overwrites the fd-1 bytes.
/// (A single shared description would yield "out\nerr\n".)
#[test]
fn repeated_target_opens_independently() {
    let dir = scratch_dir("two-opens");
    let (stdout, stderr, code) = rubash_in(
        "{ echo out; echo err >&2; } >f 2>f\necho \"f=[$(cat f)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[err]\n");
    assert_eq!(code, Some(0));
}

/// Dup ordering (#699): `2>&1 >f` binds fd 2 to the stdout in force
/// BEFORE the `>f` entry — the leaf's stderr escapes to the real
/// stdout and f is created empty.
#[test]
fn dup_before_file_keeps_old_stdout() {
    let dir = scratch_dir("dup-order-a");
    let (stdout, stderr, code) = rubash_in("{ echo e >&2; } 2>&1 >f\necho rc=$?", &dir);
    let f = read(&dir, "f");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stdout, "e\nrc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(f, "");
    assert_eq!(code, Some(0));
}

/// The reverse order routes the leaf's stderr into the file through
/// the dup of the just-bound fd 1.
#[test]
fn file_before_dup_routes_stderr_into_file() {
    let dir = scratch_dir("dup-order-b");
    let (stdout, stderr, code) = rubash_in("{ echo e >&2; } >f 2>&1\necho \"f=[$(cat f)]\"", &dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[e]\n");
    assert_eq!(code, Some(0));
}

/// `>f 2>&1` shares ONE description: interleaved fd-1/fd-2 leaf writes
/// land in write order.
#[test]
fn combined_dup_shares_one_description() {
    let dir = scratch_dir("combined");
    let (stdout, stderr, code) = rubash_in(
        "{ echo o; echo e >&2; } >f 2>&1\necho \"f=[$(cat f)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[o\ne]\n");
    assert_eq!(code, Some(0));
}

/// Nested shielding: an inner compound with its own redirect binds its
/// own entry; the outer binding resumes afterwards (save/restore
/// stacking, not per-leaf shielding marks).
#[test]
fn nested_compound_shields_and_restores() {
    let dir = scratch_dir("nested");
    let (stdout, stderr, code) = rubash_in(
        "{ echo outer; { echo inner; } >g; echo outer2; } >f\necho \"f=[$(cat f)]\"\necho \"g=[$(cat g)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[outer\nouter2]\ng=[inner]\n");
    assert_eq!(code, Some(0));
}

/// A nested compound WITHOUT its own redirect inherits the outer
/// binding (no entry of its own to stack).
#[test]
fn nested_compound_inherits_outer_binding() {
    let dir = scratch_dir("nested-inherit");
    let (stdout, stderr, code) = rubash_in(
        "{ echo a; { echo b; }; echo c; } >f\necho \"f=[$(cat f)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[a\nb\nc]\n");
    assert_eq!(code, Some(0));
}

/// The target opens against the cwd in force at compound ENTRY: a
/// body `cd` must not relocate it (the anchor the splice used to bake
/// into every leaf is now the binder's entry-time resolution).
#[test]
fn body_cd_does_not_relocate_target() {
    let dir = scratch_dir("cd-anchor");
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).expect("sub dir");
    let (stdout, stderr, code) = rubash_in("cd sub\n{ cd ..; echo x; } >out.txt\necho rc=$?", &dir);
    let in_sub = sub.join("out.txt").exists();
    let in_root = dir.join("out.txt").exists();
    let content = if in_sub {
        read(&sub, "out.txt")
    } else {
        String::new()
    };
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=0\n");
    assert!(in_sub, "target created in the entry cwd (sub/)");
    assert!(!in_root, "target NOT relocated by the body's cd");
    assert_eq!(content, "x\n");
    assert_eq!(code, Some(0));
}

/// #532 shape: a pipeline in the middle of a redirected body — the
/// pipeline's last stage writes into the compound's file, in body
/// order (the stage's capture flush lands in the bound fd 1).
#[test]
fn pipeline_stage_inside_redirected_subshell() {
    let dir = scratch_dir("pipe-sub");
    let (stdout, stderr, code) = rubash_in(
        "( echo A; printf 'B\\n' | cat; echo C ) >f\necho \"f=[$(cat f)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[A\nB\nC]\n");
    assert_eq!(code, Some(0));
}

/// Same shape through the brace-group wrapper.
#[test]
fn pipeline_stage_inside_redirected_brace_group() {
    let dir = scratch_dir("pipe-brace");
    let (stdout, stderr, code) = rubash_in(
        "{ echo A; printf 'B\\n' | cat; echo C; } >f\necho \"f=[$(cat f)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[A\nB\nC]\n");
    assert_eq!(code, Some(0));
}

/// An fd-alias leaf target (`>>/dev/stdout`) resolves against the fd
/// table the compound bound (niubash#118 class): the leaf's append
/// lands in the group's file.
#[test]
fn fd_alias_leaf_target_resolves_against_binding() {
    let dir = scratch_dir("dev-stdout");
    let (stdout, stderr, code) = rubash_in(
        "{ printf 'x\\n' >>/dev/stdout; } >>so\necho \"so=[$(cat so)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "so=[x]\n");
    assert_eq!(code, Some(0));
}

/// Heredoc on the compound feeds the body's reader while the output
/// redirect binds at the same entry.
#[test]
fn heredoc_on_redirected_if() {
    let dir = scratch_dir("heredoc");
    let (stdout, stderr, code) = rubash_in(
        "if :; then cat; fi <<EOF >f\nhello\nEOF\necho \"f=[$(cat f)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[hello]\n");
    assert_eq!(code, Some(0));
}

/// Herestring on a redirected loop.
#[test]
fn herestring_on_redirected_while() {
    let dir = scratch_dir("herestring");
    let (stdout, stderr, code) = rubash_in(
        "while read -r l; do echo \"got:$l\"; done <<< \"one\" >f\necho \"f=[$(cat f)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[got:one]\n");
    assert_eq!(code, Some(0));
}

/// A procsub output target on the compound still materializes (the
/// one mutation that keeps the cmd clone) and receives the body. The
/// target is spelled as a redirect TO a process substitution
/// (`> >(...)`): a bare `fi >(...)` is not a redirection at all — GNU
/// rejects it as a syntax error (verified against GNU bash 5.3.20).
#[test]
fn procsub_target_on_if() {
    let dir = scratch_dir("procsub");
    let (stdout, stderr, code) = rubash_in(
        "if :; then echo hi; fi > >(cat > cap.txt)\necho \"cap=[$(cat cap.txt)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "cap=[hi]\n");
    assert_eq!(code, Some(0));
}

/// Restricted shell (#804): the refusal precedes any target creation
/// or body execution on the prepare path too.
#[test]
fn restricted_shell_refuses_before_create() {
    let dir = scratch_dir("restricted");
    let (stdout, stderr, code) = rubash_args(
        &["-r", "-c", "if :; then echo body; fi >f\necho \"rc=$?\""],
        &dir,
    );
    let created = dir.join("f").exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stdout, "rc=1\n");
    assert!(stderr.contains("restricted"), "stderr: {stderr}");
    assert!(!created, "no target file created under restricted shell");
    assert_eq!(code, Some(0));
}

/// Redirect failure on an if: status 1, the diagnostic exactly once,
/// and an enclosing `!` does NOT flip it (#846 — the failure prepared
/// at entry is the compound's own).
#[test]
fn if_redirect_failure_not_flipped() {
    let (stdout, stderr, code) = rubash("! if :; then :; fi > /nonexistent_dir/f\necho \"rc=$?\"");
    assert_eq!(
        stderr,
        "bash: line 1: /nonexistent_dir/f: No such file or directory\n"
    );
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(code, Some(0));
}

/// Create ordering on failure (#940): the `2>cap.txt` create lands
/// BEFORE the failing `>` open, and the diagnostic routes into
/// cap.txt — for the if wrapper's prepare walk as for the others.
#[test]
fn if_create_order_and_diag_route_on_failure() {
    let dir = scratch_dir("if-940");
    let (stdout, stderr, code) = rubash_in(
        "if :; then :; fi 2>cap.txt > /nonexistent_dir/f\necho \"rc=$?\"\necho \"cap=[$(cat cap.txt)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        "rc=1\ncap=[bash: line 1: /nonexistent_dir/f: No such file or directory]\n"
    );
    assert_eq!(code, Some(0));
}

/// A function DEFINITION inside a redirected compound is not itself
/// redirected as a call (#161 class): the file is created empty and
/// the later call writes to the real stdout.
#[test]
fn function_definition_not_captured() {
    let dir = scratch_dir("funcdef");
    let (stdout, stderr, code) = rubash_in("{ f() { echo in; }; } >f\nf", &dir);
    let f = read(&dir, "f");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "in\n");
    assert_eq!(f, "");
    assert_eq!(code, Some(0));
}

/// `exec` inside a subshell body retargets only the child's streams
/// (#170/#726 class): the subshell's own `>g` receives nothing.
#[test]
fn exec_inside_subshell_body() {
    let dir = scratch_dir("exec-sub");
    let (stdout, stderr, code) = rubash_in(
        "( exec >f; echo x; ) >g\necho \"f=[$(cat f)]\"\necho \"g=[$(cat g)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[x]\ng=[]\n");
    assert_eq!(code, Some(0));
}

/// Loop iterations all write through the one entry binding.
#[test]
fn for_loop_iterations_share_binding() {
    let dir = scratch_dir("for");
    let (stdout, stderr, code) = rubash_in(
        "for i in 1 2 3; do echo $i; done >f\necho \"f=[$(cat f)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[1\n2\n3]\n");
    assert_eq!(code, Some(0));
}

/// While loop with a herestring-free counter body.
#[test]
fn while_loop_iterations_share_binding() {
    let dir = scratch_dir("while");
    let (stdout, stderr, code) = rubash_in(
        "n=0; while [ $n -lt 2 ]; do echo $n; n=$((n+1)); done >f\necho \"f=[$(cat f)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[0\n1]\n");
    assert_eq!(code, Some(0));
}

/// Case clauses write through the entry binding (stdout and stderr
/// arms).
#[test]
fn case_clause_redirects() {
    let dir = scratch_dir("case");
    let (stdout, stderr, code) = rubash_in(
        "case x in x) echo hit;; *) echo miss;; esac >f\ncase x in x) echo e >&2;; esac 2>ef\necho \"f=[$(cat f)]\"\necho \"ef=[$(cat ef)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[hit]\nef=[e]\n");
    assert_eq!(code, Some(0));
}

/// Select body writes through the entry binding (menu goes to the
/// real stderr; the picked branch's echo lands in the file).
#[test]
fn select_body_redirect() {
    use std::io::Write as _;
    let dir = scratch_dir("select");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args([
            "-c",
            "select s in a b; do echo \"picked:$s\"; break; done >f",
        ])
        .current_dir(&dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("run rubash");
    let mut stdin = child.stdin.take().expect("stdin pipe");
    let _ = stdin.write_all(b"1\n");
    drop(stdin);
    let output = child.wait_with_output().expect("wait for shell");
    let f = read(&dir, "f");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(f, "picked:a\n");
    assert_eq!(output.status.code(), Some(0));
}

/// Append form: the entry binding opens in append mode — pre-seeded
/// content survives and leaves append after it.
#[test]
fn append_form_compound() {
    let dir = scratch_dir("append");
    std::fs::write(dir.join("f"), "seed\n").expect("seed f");
    let (stdout, stderr, code) = rubash_in(
        "{ echo a; } >>f\nif :; then echo b; fi >>f\necho \"f=[$(cat f)]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[seed\na\nb]\n");
    assert_eq!(code, Some(0));
}

/// An empty body still creates the target at entry (GNU
/// do_redirections runs before the body, even a `:` body).
#[test]
fn empty_body_still_creates() {
    let dir = scratch_dir("empty");
    let (stdout, stderr, code) = rubash_in(
        "if :; then :; fi >f\n{ :; } >g\necho \"f=$([ -f f ] && echo yes)\"\necho \"g=$([ -f g ] && echo yes)\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=yes\ng=yes\n");
    assert_eq!(code, Some(0));
}

/// Noclobber: an existing target refuses at entry — status 1, body
/// never runs, file untouched.
#[test]
fn noclobber_refuses_at_entry() {
    let dir = scratch_dir("noclobber");
    let (stdout, stderr, code) = rubash_in(
        "set -C\necho seed > f\nif :; then echo body; fi >f\necho \"rc=$?\"\necho \"f=[$(cat f)]\"",
        &dir,
    );
    let f = read(&dir, "f");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stdout, "rc=1\nf=[seed]\n");
    assert!(
        stderr.contains("cannot overwrite existing file"),
        "stderr: {stderr}"
    );
    assert_eq!(f, "seed\n");
    assert_eq!(code, Some(0));
}

/// A numbered fd target on the compound binds for the body's
/// duration and leaves resolve it through the table.
#[test]
fn numbered_fd_binding() {
    let dir = scratch_dir("fd3");
    let (stdout, stderr, code) = rubash_in("{ echo via3 >&3; } 3>f\necho \"f=[$(cat f)]\"", &dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[via3]\n");
    assert_eq!(code, Some(0));
}

/// An external leaf inside a redirected compound writes through the
/// bound fd 1 (the external stdio plan consults the binding instead
/// of re-applying the leaf's injected entry — there is none anymore).
#[test]
fn external_leaf_uses_binding() {
    let dir = scratch_dir("external");
    std::fs::write(dir.join("seed.txt"), "seed-data\n").expect("seed");
    let (stdout, stderr, code) = rubash_in("{ cat seed.txt; } >f\necho \"f=[$(cat f)]\"", &dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "f=[seed-data]\n");
    assert_eq!(code, Some(0));
}

/// The measured shape from the issue stays correct: a redirected
/// empty if is a no-op that creates/truncates via the entry binding
/// and reports success.
#[test]
fn measured_shape_devnull() {
    let (stdout, stderr, code) = rubash("if :; then :; fi >/dev/null\necho rc=$?");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(code, Some(0));
}
