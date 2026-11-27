//! Regression tests for GitHub issue unixwin/rubash#3183:
//! Diagnostic line-number accounting across suite shapes from #2665 D1:
//! - `run-func` (`func5.sub`): `break()` in posix mode attributed to closing `}` line
//! - `run-exportfunc` (`exportfunc.tests`): `eval` syntax error / unexpected EOF line numbers
//! - `run-heredoc` (`heredoc3.sub`, `heredoc7.sub`, `heredoc.tests`): here-doc EOF warning lines
//! - `run-braces` (`braces.tests`): command substitution unexpected EOF line inside `${...}`
//! - `run-redir` (`redir.tests`): ambiguous redirect & invalid option line numbers
//! - `run-nquote` (`nquote4.sub`): multiline `$'...'` alias expansion attribution line

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Run a script file through `rubash` and return (stdout, stderr, exit_code).
fn run_script_file(script: &str) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("rubash_issue3183_{}_{}.sh", std::process::id(), n));
    std::fs::write(&path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue3183_func_posix_special_builtin_break_line() {
    // Matches third_party/bash/tests/func5.sub lines 88-94:
    // `( set -o posix; break() { ... }; ... )` attributes the special-builtin
    // error to the closing `)` line of the subshell (line 7 here, line 94 in func5.sub).
    let script = "\
( set -o posix
break()
{
        echo FUNCNAME: $FUNCNAME
}
echo after
)
";
    let (_out, err, _rc) = run_script_file(script);
    assert!(
        err.contains("line 7: `break': is a special builtin"),
        "expected line 7 special builtin error, got stderr:\n{err}"
    );
}

#[test]
fn issue3183_exportfunc_eval_lines() {
    // Matches third_party/bash/tests/exportfunc.tests lines 42-44 and 82:
    // 1) `eval 'X() { (a)>\\'`: syntax error at line 3 (eval starts at line 1,
    //    trailing backslash continues to line 3) from `{' command on line 1.
    let script1 = "\
eval 'X() { (a)>\\'
";
    let (_out, err1, _rc) = run_script_file(script1);
    assert!(
        err1.contains(
            "eval: line 3: syntax error: unexpected end of file from `{' command on line 1"
        ),
        "expected eval line 3 / command on line 1, got stderr:\n{err1}"
    );

    // 2) `eval 'foo() { _; } >_[${ $() }] ;{ echo eval ok; }'`:
    //    unexpected EOF attributed to eval: line 2 (eval starts at line 1).
    let script2 = "\
eval 'foo() { _; } >_[${ $() }] ;{ echo eval ok; }'
";
    let (_out, err2, _rc) = run_script_file(script2);
    assert!(
        err2.contains("eval: line 2: unexpected EOF while looking for matching `}'"),
        "expected eval line 2 unexpected EOF, got stderr:\n{err2}"
    );
}

#[test]
fn issue3183_heredoc_warning_lines() {
    // 1) Matches heredoc3.sub lines 96-98: `(cat <<EOF\n...\nEOF)`
    let script1 = "\
(cat <<EOF
still more text in a subshell
EOF)
";
    let (_out, err1, _rc) = run_script_file(script1);
    assert!(
        err1.contains(
            "line 3: warning: here-document at line 1 delimited by end-of-file (wanted `EOF')"
        ),
        "expected line 3 warning for line 1 heredoc, got stderr:\n{err1}"
    );
    assert!(
        err1.contains("line 4: syntax error: unexpected end of file from `(' command on line 1"),
        "expected line 4 syntax error from line 1 `(`, got stderr:\n{err1}"
    );

    // 2) Matches heredoc7.sub lines 26-29: `cat <<EOF && grep $(\n foobar\nEOF\necho notthereanywhere)`
    let script2 = "\
cat <<EOF && grep $(
 foobar
EOF
echo notthereanywhere)
";
    let (_out, err2, _rc) = run_script_file(script2);
    assert!(
        err2.contains(
            "line 4: warning: here-document at line 4 delimited by end-of-file (wanted `EOF')"
        ),
        "expected line 4 warning for line 4 heredoc, got stderr:\n{err2}"
    );

    // 3) Matches heredoc.tests lines 181-184: `cat <<''\nhi\nthere\n''`
    let script3 = "\
cat <<''
hi
there
''
";
    let (_out, err3, _rc) = run_script_file(script3);
    assert!(
        err3.contains(
            "line 4: warning: here-document at line 1 delimited by end-of-file (wanted `')"
        ),
        "expected line 4 warning for line 1 empty-delim heredoc, got stderr:\n{err3}"
    );
}

#[test]
fn issue3183_braces_comsub_eof_line() {
    // Matches third_party/bash/tests/braces.tests lines 54-57:
    // `echo "${a+'$('\'}"` triggers command substitution EOF at line 6.
    let script = "\
a=4
echo \"${a#'$('\\'}\"
echo \"${a-'$('\\'}\"
echo \"${a+'$('\\'}\"
";
    let (_out, err, _rc) = run_script_file(script);
    assert!(
        err.contains("command substitution: line 6: unexpected EOF while looking for matching `)'"),
        "expected command substitution line 6 unexpected EOF, got stderr:\n{err}"
    );
}

#[test]
fn issue3183_redir_ambiguous_and_exec_option_lines() {
    // Matches third_party/bash/tests/redir.tests lines 51-53:
    let script = "\
fd=-1
exec <&$fd
exec $fd</dev/null
";
    let (_out, err, _rc) = run_script_file(script);
    assert!(
        err.contains("line 2: -1: ambiguous redirect"),
        "expected line 2 ambiguous redirect, got stderr:\n{err}"
    );
    assert!(
        err.contains("line 3: exec: -1: invalid option"),
        "expected line 3 exec invalid option, got stderr:\n{err}"
    );
}

#[test]
fn issue3183_nquote_multiline_alias_attribution_line() {
    // Matches third_party/bash/tests/nquote4.sub lines 3-6:
    // Multiline `$'...'` alias expansion attributes `quux: command not found`
    // to the call-site line (line 4 here, line 6 in nquote4.sub).
    let script = "\
shopt -s expand_aliases
alias foo=$'echo foo\\n\\nquux\\n'

foo
";
    let (out, err, _rc) = run_script_file(script);
    assert_eq!(out.trim(), "foo");
    assert!(
        err.contains("line 4: quux: command not found"),
        "expected line 4 quux command not found, got stderr:\n{err}"
    );
}
