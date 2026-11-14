//! rubash#3404 — a `${var:?}`-family diagnostic that names a subscript
//! holding a `$(...)` re-printed the command substitution from its raw
//! source text (`>&2`), where GNU prints the canonical print_cmd.c form
//! (`1>&2`).
//!
//! GNU never keeps the raw `$()` body in a parser-read word: parse.y:4451
//! `parse_comsub` yyparse()s the body and replaces it with the
//! `print_comsub` serialization (parse.y:4632 -> print_cmd.c:152
//! make_command_string), whose redirections come from print_cmd.c:1150
//! `print_redirection`: r_duplicating_output prints both fds (`>&2` ->
//! `1>&2`), r_close_this prints `%d>&-` for `<&-` and `>&-` alike
//! (parse.y:754-789 build one instruction), `<` omits a 0 redirector,
//! `&>` prints `&> word`, r_move_*_word prints the redirector (`>&$x-`
//! -> `1>&$x-`). subst.c:8216 parameter_brace_expand_error then names
//! the parameter with that stored text. Here-document bodies and
//! backquote bodies never pass parse_comsub, so they stay raw (guards).
//!
//! Expectations are the GNU bash 5.3.20 oracle's, probed from script
//! files (script-mode fatal expansion status is 1).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3404-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("t.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("t.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue's shape: the diagnostic re-print carries `1>&2`, the comsub
/// side effect runs once, and the fatal `:?` ends the script with 1.
#[test]
fn issue_shape_reprints_explicit_fd_duplication() {
    let script = "a=(x)\n: ${a[$(printf 'H%sT' I >&2; echo 5)]:?err}\necho alive\n";
    let (stdout, stderr, code) = rubash_file("shape", script);
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "HITt.sh: line 2: a[$(printf 'H%sT' I 1>&2; echo 5)]: err\n"
    );
    assert_eq!(code, Some(1));
}

/// Sibling redirection forms and word contexts, each in its own subshell
/// so the fatal `:?` is contained (GNU: `alive` still prints, rc 0).
#[test]
fn sibling_forms_and_contexts_match_print_redirection() {
    let script = concat!(
        "a=(x)\n",
        "( : ${a[$(echo E >&- ; echo 5)]:?err} ) 2>&1 | grep -v 'write error'\n",
        "( : ${a[$(echo E &>/dev/null; echo 5)]?} )\n",
        "( : \"${a[$(cat 0</dev/null <&0; echo 5)]:?err}\" )\n",
        "( : ${a[$(echo    E    >&2  ;  echo 5)]:?err} ) 2>&1 | tail -1\n",
        "( f() { : ${a[$(echo 5 2>&1)]:?err}; }; f )\n",
        "( eval ': ${a[$(echo 5 >>/dev/null 2>&1; echo 5)]:?err}' )\n",
        "echo alive\n",
    );
    let (stdout, stderr, code) = rubash_file("siblings", script);
    assert_eq!(
        stdout,
        concat!(
            "t.sh: line 2: a[$(echo E 1>&-; echo 5)]: err\n",
            "t.sh: line 5: a[$(echo E 1>&2; echo 5)]: err\n",
            "alive\n",
        )
    );
    assert_eq!(
        stderr,
        concat!(
            "t.sh: line 3: a[$(echo E &> /dev/null; echo 5)]: parameter not set\n",
            "t.sh: line 4: a[$(cat < /dev/null 0<&0; echo 5)]: err\n",
            "t.sh: line 6: a[$(echo 5 2>&1)]: err\n",
            "t.sh: line 7: a[$(echo 5 >> /dev/null 2>&1; echo 5)]: err\n",
        )
    );
    assert_eq!(code, Some(0));
}

/// Guards: a backquote body (parse_matched_pair, verbatim) and a
/// here-document body (redir.c heredoc_expand, run-time expand_string)
/// never pass parse_comsub — GNU reports their text raw.
#[test]
fn backquote_and_heredoc_bodies_stay_raw() {
    let script = concat!(
        "a=(x)\n",
        "( : ${a[`echo E >&2; echo 5`]:?err} ) 2>&1 | tail -1\n",
        "( cat <<EOT\n",
        "${a[$(echo E >&2; echo 5)]:?err}\n",
        "EOT\n",
        ") 2>&1 | tail -1\n",
        "echo alive\n",
    );
    let (stdout, stderr, code) = rubash_file("raw", script);
    assert_eq!(
        stdout,
        concat!(
            "t.sh: line 2: a[`echo E >&2; echo 5`]: err\n",
            "t.sh: line 3: a[$(echo E >&2; echo 5)]: err\n",
            "alive\n",
        )
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// The shared print_redirection owner also drives `declare -f` and the
/// `$BASH_COMMAND` text of a command substitution (print_cmd.c:1150):
/// r_close_this is `%d>&-` for both directions, r_move_*_word prints its
/// redirector, and the comsub reprint carries the same forms.
#[test]
fn shared_print_redirection_owner_faces() {
    let script = concat!(
        "f() {\n",
        "echo b >&-; echo c <&-; echo q 2<&-; echo E >&$x-; echo F <&$x-; ",
        "echo G 3<&$x-; echo H {fd}<&-; echo a >&2; echo e <&0; echo u <&3-\n",
        "}\n",
        "declare -f f\n",
        "trap 'echo \"BC=[$BASH_COMMAND]\"' DEBUG\n",
        ": $(echo a >&2) $(echo b >&-) $(echo c <&-) $(echo l &>/dev/null) $(cat 0</dev/null)\n",
        "trap - DEBUG\n",
    );
    let (stdout, _stderr, code) = rubash_file("owner", script);
    assert_eq!(
        stdout,
        concat!(
            "f () \n",
            "{ \n",
            "    echo b 1>&-;\n",
            "    echo c 0>&-;\n",
            "    echo q 2>&-;\n",
            "    echo E 1>&$x-;\n",
            "    echo F 0<&$x-;\n",
            "    echo G 3<&$x-;\n",
            "    echo H {fd}>&-;\n",
            "    echo a 1>&2;\n",
            "    echo e 0<&0;\n",
            "    echo u 0<&3-\n",
            "}\n",
            "BC=[: $(echo a 1>&2) $(echo b 1>&-) $(echo c 0>&-) $(echo l &> /dev/null) ",
            "$(cat < /dev/null)]\n",
            "BC=[trap - DEBUG]\n",
        )
    );
    assert_eq!(code, Some(0));
}
