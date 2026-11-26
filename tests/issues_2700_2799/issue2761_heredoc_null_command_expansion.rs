//! rubash#2761 absorb (remainder of Uarz #3362 after #3715 merged):
//! the vanished-words arm of the empty-words dispatch passed `cmd`
//! with its raw words (`["$EMPTY"]`) to `execute_empty_words_command`,
//! so the heredoc fork classifier saw an effective word and expanded
//! a numbered-fd body in a fork, discarding `:=`. GNU's words==0 check
//! (execute_cmd.c:4626-4640) runs on the EXPANDED word list, which is
//! empty here — the node is a null command, and the force-fork rule
//! (execute_cmd.c:4210-4216: fd-0 / REDIR_VARASSIGN only) leaves a
//! numbered-fd body expanding in the shell, so `:=` persists.
//!
//! The rest of #2761's face (raw-empty null commands, the fd-0 vs
//! numbered-fd split, quoted delimiters, here-strings) is pinned by
//! #3715's tests/issue2761_heredoc_empty_words_regressions.rs. This
//! file keeps only the shapes that PR lacks: the discriminating
//! vanished-words numbered-fd `:=` pin (red on master, green after
//! the `words.clear()` hunk) plus eight coverage pins.
//!
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::process::Command;

fn run_script(name: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-issue2761-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script_path = dir.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn vanished_words_numbered_heredoc_assign_persists() {
    // DISCRIMINATING pin: `$EMPTY 3<<EOF` with `${v:=bound}` in the
    // body. GNU expands the body in the shell (expanded words empty
    // ⇒ null command; fd-3 does not force the fork), so `:=` binds
    // persist. Before the `words.clear()` hunk the classifier saw
    // the raw `["$EMPTY"]` word, forked the expansion, and `v` stayed
    // unset. #3715's vanished-words pin is fd-0 only and asserts
    // expansion, not numbered-fd `:=` persistence.
    let (stdout, _, code) = run_script(
        "vanished-numbered-assign",
        "EMPTY=\nunset v\n$EMPTY 3<<EOF\n${v:=bound}\nEOF\necho \"v=${v-unset}\"\n",
    );
    assert_eq!(stdout, "v=bound\n");
    assert_eq!(code, Some(0));
}

#[test]
fn null_heredoc_fd0_arithmetic_discarded() {
    let (stdout, _, code) = run_script(
        "fd0-arith-discarded",
        "n=10\n<<EOF\n$((n+=1))\nEOF\necho \"n=$n\"\n",
    );
    assert_eq!(stdout, "n=10\n");
    assert_eq!(code, Some(0));
}

#[test]
fn numbered_heredoc_arithmetic_persists() {
    let (stdout, _, code) = run_script(
        "numbered-arith",
        "n=10\n3<<EOF\n$((n+=1))\nEOF\necho \"n=$n\"\n",
    );
    assert_eq!(stdout, "n=11\n");
    assert_eq!(code, Some(0));
}

#[test]
fn two_heredocs_both_expand_in_order() {
    // GNU do_redirections walks the list in document order: both
    // bodies expand, earlier effects visible to later bodies.
    let (stdout, _, code) = run_script(
        "two-bodies-order",
        "<<A <<B\n$(echo first >> order.txt)\nA\n$(echo second >> order.txt)\nB\ncat order.txt\n",
    );
    assert_eq!(stdout, "first\nsecond\n");
    assert_eq!(code, Some(0));
}

#[test]
fn numbered_bodies_expand_in_order() {
    // Same ordering, in the shell context: the second body's
    // expansion sees the first body's binding.
    let (stdout, _, code) = run_script(
        "numbered-order",
        "unset b\n3<<A 4<<B\n${a:=fromA}\nA\n${b:=$a}\nB\necho \"b=${b-unset}\"\n",
    );
    assert_eq!(stdout, "b=fromA\n");
    assert_eq!(code, Some(0));
}

#[test]
fn arith_error_fd0_status_1_survives() {
    // An arithmetic error in the body is the funnel's latched
    // failure class: the fd-0 child fails, the parent reports
    // status 1 and the script continues (GNU 5.3.20 oracle P12).
    let (stdout, _, code) = run_script(
        "arith-error-fd0",
        "<<EOF\n$((1/0))\nEOF\necho \"rc=$? survived\"\n",
    );
    assert_eq!(stdout, "rc=1 survived\n");
    assert_eq!(code, Some(0));
}

#[test]
fn arith_error_numbered_status_1_survives() {
    // Same, in the shell context (GNU 5.3.20 oracle P13).
    let (stdout, _, code) = run_script(
        "arith-error-numbered",
        "3<<EOF\n$((1/0))\nEOF\necho \"rc=$? survived\"\n",
    );
    assert_eq!(stdout, "rc=1 survived\n");
    assert_eq!(code, Some(0));
}

#[test]
fn fd0_fatal_body_error_contained() {
    // A fatal `${var:?}` in an fd-0 null body is contained by the
    // force-fork: status 1, script continues (GNU 5.3.20 oracle P17).
    let (stdout, stderr, code) = run_script(
        "fd0-fatal-contained",
        "<<EOF\n${UNDEF_Z:?boom}\nEOF\necho \"rc=$? alive\"\n",
    );
    assert_eq!(stdout, "rc=1 alive\n");
    assert!(stderr.contains("UNDEF_Z: boom"), "{stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn post_alias_empty_heredoc_comsub_executes() {
    // The third empty-words arm: alias expansion empties the words
    // (execute_cmd.c:4626-4640), and the body still expands
    // (GNU 5.3.20 oracle P18).
    let (stdout, _, code) = run_script(
        "post-alias-comsub",
        "shopt -s expand_aliases\nalias z=''\nz <<EOF\n$(touch alias_ran.txt)\nEOF\nls alias_ran.txt 2>/dev/null || echo \"no-side-effect-file\"\n",
    );
    assert_eq!(stdout, "alias_ran.txt\n");
    assert_eq!(code, Some(0));
}
