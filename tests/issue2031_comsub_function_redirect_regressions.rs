//! Issue rubash#2031 (ecosuite audit DIV-G): output of a FUNCTION call
//! carrying a dup redirect (`>&2` / `2>&1`) was silently lost inside a
//! command substitution when the dup's source fd was bound by an
//! enclosing scope to the substitution capture.
//!
//! GNU semantics: a call's redirections apply once to the shell's real
//! descriptors for the body's duration (execute_cmd.c:5606
//! execute_builtin_or_function -> redir.c do_redirections, undone on
//! return), so `f 2>&1 >/dev/null` freezes fd 2 onto fd 1's object at
//! call time and a body's later `>&2` resolves against THAT binding.
//!
//! Root cause: an enclosing compound's stdout leg, spliced onto the
//! function-call node as an injected (GROUP_REDIRECT_INJECTED) append,
//! was re-spliced by apply_function_call_redirects into the function
//! body and re-applied at the writing leaf — after the call's own `>&2`
//! dup had rebound fd 1 to the capture (an FdWriteEndpoint::Stdout
//! endpoint, which injected_redirect_fd_is_bound does not recognize as
//! an in-force binding). The leaf re-opened the ancestor's /dev/null
//! over the dup binding and the message went there. Fix: the function
//! splice skips injected legs — the ancestor binding already lives on
//! the fd table and the call's with_compound_output_redirects wrap is
//! what keeps it in force (or lets genuine call legs displace it).
//!
//! Expected outputs are byte-verified against GNU Bash 5.3.20
//! (self-compiled oracle, script-file probes 2026-10-29). The defect
//! needs the capture endpoint, so most pins are comsub-shaped; the
//! file-shaped guards pin that plain descriptor bindings never
//! regressed.

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

fn assert_stdout(script: &str, expected: &str) {
    let (stdout, stderr, _) = rubash(script);
    assert_eq!(stdout, expected, "script: {script}\nstderr: {stderr}");
}

/// The issue's minimal reproducer, form 1: the callee's body is a
/// subshell whose `>&2` applies to a nested FUNCTION call.
#[test]
fn comsub_function_subshell_body_redirected_function_call() {
    assert_stdout(
        "a() { printf %s\\\\n \"$*\"; }\n\
         b() { (>&2 a \"$@\") || return 0; }\n\
         echo \"[$(b MSG 2>&1 >/dev/null)]\"",
        "[MSG]\n",
    );
}

/// Minimal reproducer, form 2: anonymous subshell carrying the call
/// redirects, function call inside carrying the dup.
#[test]
fn comsub_subshell_redirected_function_call_with_dup() {
    assert_stdout(
        "a() { printf %s\\\\n \"$*\"; }\n\
         echo \"[$( ( a MSG >&2 ) 2>&1 >/dev/null )]\"",
        "[MSG]\n",
    );
}

/// Minimal reproducer, form 3: no subshell — the dup sits on a function
/// call inside another function's body.
#[test]
fn comsub_nested_function_call_with_dup() {
    assert_stdout(
        "a() { printf %s\\\\n \"$*\"; }\n\
         c() { a \"$@\" >&2; }\n\
         echo \"[$(c MSG 2>&1 >/dev/null)]\"",
        "[MSG]\n",
    );
}

/// Same route through a brace group inside the function body (the
/// group splice also copies the ancestor leg onto the call node).
#[test]
fn comsub_function_brace_group_redirected_function_call() {
    assert_stdout(
        "a() { printf %s\\\\n \"$*\"; }\n\
         b() { { a \"$@\" >&2; }; }\n\
         echo \"[$(b BG 2>&1 >/dev/null)]\"",
        "[BG]\n",
    );
}

/// The nvm try_err signature (nvm test/fast: 21 same-source failures,
/// including nvm_err's own unit test): the redirected call is dispatched
/// through `"$@"` inside the substitution.
#[test]
fn comsub_try_err_shape_captures_function_stderr() {
    assert_stdout(
        "nvm_echo() { printf %s\\\\n \"$*\"; }\n\
         nvm_err() { (>&2 nvm_echo \"$@\") || return 0; }\n\
         try_err() { CAPTURED=\"$( \"$@\" 2>&1 >/dev/null )\"; }\n\
         try_err nvm_err OOPS\n\
         echo \"[$CAPTURED]\"",
        "[OOPS]\n",
    );
}

/// The substitution's exit status still comes from the body.
#[test]
fn comsub_redirected_function_call_keeps_status() {
    assert_stdout(
        "a() { printf %s\\\\n \"$*\"; }\n\
         b() { (>&2 a \"$@\") || return 0; }\n\
         out=$(b ST 2>&1 >/dev/null); echo \"[$out] rc=$?\"",
        "[ST] rc=0\n",
    );
}

/// Guard: a dup on the WRITING command itself (no function splice in
/// between) already worked and must keep working.
#[test]
fn comsub_builtin_subshell_dup_still_captures() {
    assert_stdout(
        "echo \"[$( ( printf 'P1\\n' >&2 ) 2>&1 >/dev/null )]\"",
        "[P1]\n",
    );
}

/// Guard: the redirected function whose body writes through its own
/// `>&2` on the writing builtin already worked.
#[test]
fn comsub_redirected_function_body_own_dup_still_captures() {
    assert_stdout(
        "g() { printf 'P2\\n' >&2; }\n\
         echo \"[$(g 2>&1 >/dev/null)]\"",
        "[P2]\n",
    );
}

/// Guard: the ancestor compound's binding must still reach a function
/// body when no nearer scope rebinds the fd — skipping the re-splice
/// must not lose the group redirect itself.
#[test]
fn comsub_group_redirect_still_suppresses_function_output() {
    assert_stdout(
        "f() { printf 'X\\n'; }\n\
         echo \"[$( { f; } >/dev/null )]\"",
        "[]\n",
    );
}

/// Sibling face of the same capture contract (found by the ecosuite
/// nvm fast re-run, `nvm_get_default_packages`): when the writing leaf
/// is a SPAWNED EXTERNAL command, the stdio plan binds its fd 2 to the
/// active capture through a shared pipe, and the post-exit drain must
/// deliver those bytes to the capture the pipe was materialized for.
/// The drain resolved through fd 1's CURRENT endpoint instead, so a
/// call wrap's later `>/dev/null` (or `>file`) rebind of fd 1 swallowed
/// the merged stream (or diverted it into the file). GNU: the child's
/// fd 2 IS the substitution pipe (subst.c:7143 command_substitute).
#[cfg(unix)]
#[test]
fn comsub_redirected_function_external_stderr_leaf_captures() {
    assert_stdout(
        "h() { /bin/sh -c 'echo CEE >&2'; }\n\
         echo \"[$(h 2>&1 >/dev/null)]\"",
        "[CEE]\n",
    );
}

/// The diverted-to-file variant: with `>cap.txt` as the call's fd-1
/// leg, the external leaf's stderr still belongs to the capture and
/// only its stdout may land in the file.
#[cfg(unix)]
#[test]
fn comsub_redirected_function_external_leaf_file_target_keeps_stderr_in_capture() {
    let dir = std::env::temp_dir().join(format!("rubash-issue2031-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script = format!(
        "cd {} || exit 1\n\
         w() {{ /bin/sh -c 'echo WERR >&2'; }}\n\
         echo \"cap=[$(w 2>&1 >cap.txt)] file=[$(cat cap.txt)]\"",
        dir.display()
    );
    assert_stdout(&script, "cap=[WERR] file=[]\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The nvm_get_default_packages shape exactly: an external writer
/// addressing /dev/stderr inside the function (awk prints the error
/// there), captured through the call's `2>&1 >/dev/null`.
#[cfg(unix)]
#[test]
fn comsub_redirected_function_dev_stderr_writer_captures() {
    assert_stdout(
        "v() { awk 'BEGIN{ print \"AWKERR\" > \"/dev/stderr\" }'; }\n\
         echo \"[$(v 2>&1 >/dev/null)]\"",
        "[AWKERR]\n",
    );
}

/// Guards for the external leaf: dup-only still captures, and the
/// null-first order still empties (fd 2 follows fd 1 into /dev/null).
#[cfg(unix)]
#[test]
fn comsub_redirected_function_external_leaf_order_guards() {
    assert_stdout(
        "h() { /bin/sh -c 'echo CEE >&2'; }\n\
         echo \"A:[$(h 2>&1)] B:[$(h >/dev/null 2>&1)]\"",
        "A:[CEE] B:[]\n",
    );
}

/// Guard (file-shaped): a call's genuine redirect wins over the
/// enclosing group's, and the group's alone reaches the body — the
/// plain-descriptor forms of the same bindings.
#[test]
fn group_and_call_file_redirects_keep_precedence() {
    let dir = std::env::temp_dir().join(format!("rubash-issue2031-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script = format!(
        "cd {} || exit 1\n\
         f() {{ printf 'X\\n'; }}\n\
         {{ f >inner.txt; }} >outer.txt\n\
         {{ f; }} >outer2.txt\n\
         echo \"inner=[$(cat inner.txt)] outer=[$(cat outer.txt)] outer2=[$(cat outer2.txt)]\"",
        dir.display()
    );
    assert_stdout(&script, "inner=[X] outer=[] outer2=[X]\n");
    let _ = std::fs::remove_dir_all(&dir);
}
