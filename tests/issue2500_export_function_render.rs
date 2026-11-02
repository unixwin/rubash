//! rubash#2500 — `export -f` / `declare -f` must render the function
//! body as an AST reprint, not a source-text slice.
//!
//! GNU print_cmd.c named_function_string (print_cmd.c:1400) is the owner
//! of both renderings: variables.c:4875 make_env_array_from_var_list
//! builds every BASH_FUNC_<name>%% value with
//! named_function_string(NULL, body, 0) and variables.c:1158
//! print_var_function builds the `declare -f` text with
//! FUNC_MULTILINE|FUNC_EXTERNAL. Because the export string is reprinted
//! from the AST:
//!   * comments are dropped (they never survive the parse);
//!   * `$'..'` words are carried decoded and single-quoted at lex time
//!     (parse.y:5548-5575 read_token_word: ansiexpand + sh_single_quote),
//!     so `$'\t'` reprints as `'<TAB>'`, never as a bare control byte;
//!   * `$( ... )` bodies are normalized at lex time via print_comsub
//!     (parse.y:4632), so comments/indentation inside them are gone and
//!     newline connectors inside them stay newlines (print_cmd.c:288-320
//!     prints the connector char itself in comsub mode);
//!   * the export form (flags 0) sets indentation=1/indentation_amount=0
//!     (print_cmd.c:1428-1431), so nested keywords (`else`, `fi`) reprint
//!     at one space, unlike declare -f's 4-space form.
//!
//! A rubash child shell refuses a BASH_FUNC_%% payload that carries a
//! comment or a bare control byte (src/executor/init.rs ShellShock-style
//! guard, aligned with GNU variables.c:398-441), so the old source-slice
//! exportstr made every exported fzf `__fzf_history__` silently
//! unusable in children. Expectations byte-verified against GNU bash
//! 5.3.20 (WSL /usr/local/bin/bash) with the same script files
//! (niubash#797 / rubash#2500).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2500-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// The fzf `__fzf_history__` shape in miniature: a multi-line command
/// substitution whose lines carry comments and `$'\t'` spans, a
/// `${...#*$'\t'}` assignment and a nested if/else.
const PINNED_FUNCTION: &str = "\
__pin_hist__() {
    local output
    output=$(
      set +o pipefail # a comment to drop
      builtin fc -lnr -2147483648 2> /dev/null | # ( $'\t '<lines> )*
        FZF_DEFAULT_OPTS=\"--wrap-sign '\"$'\t'\"x ${FZF_CTRL_R_OPTS-}\" \
        $(__fzfcmd) --query \"$READLINE_LINE\"
    )
    [[ -n $output ]] || return
    READLINE_LINE=${output#*$'\t'}
    if [[ -z $READLINE_POINT ]]; then
      echo \"$READLINE_LINE\"
    else
      READLINE_POINT=0x7fffffff
    fi
  }
";

/// GNU `declare -f` for PINNED_FUNCTION: no comments, `$'\t'` reprinted
/// as `'<TAB>'` (decoded + single-quoted at lex time), 4-space indents.
/// (concat!, not `\<newline>` continuation — the latter eats the leading
/// spaces these lines must carry.)
const EXPECTED_DECLARE: &str = concat!(
    "__pin_hist__ () \n",
    "{ \n",
    "    local output;\n",
    "    output=$(set +o pipefail\n",
    "builtin fc -lnr -2147483648 2> /dev/null | FZF_DEFAULT_OPTS=\"--wrap-sign '\"'\t'\"x ${FZF_CTRL_R_OPTS-}\" $(__fzfcmd) --query \"$READLINE_LINE\");\n",
    "    [[ -n $output ]] || return;\n",
    "    READLINE_LINE=${output#*'\t'};\n",
    "    if [[ -z $READLINE_POINT ]]; then\n",
    "        echo \"$READLINE_LINE\";\n",
    "    else\n",
    "        READLINE_POINT=0x7fffffff;\n",
    "    fi\n",
    "}\n"
);

/// GNU exportstr (named_function_string flags 0): `() {  ` prefix,
/// body commands at one space, comsub newline connector preserved,
/// nested keywords at one space.
const EXPECTED_EXPORTSTR: &str = concat!(
    "() {  local output;\n",
    " output=$(set +o pipefail\n",
    "builtin fc -lnr -2147483648 2> /dev/null | FZF_DEFAULT_OPTS=\"--wrap-sign '\"'\t'\"x ${FZF_CTRL_R_OPTS-}\" $(__fzfcmd) --query \"$READLINE_LINE\");\n",
    " [[ -n $output ]] || return;\n",
    " READLINE_LINE=${output#*'\t'};\n",
    " if [[ -z $READLINE_POINT ]]; then\n",
    " echo \"$READLINE_LINE\";\n",
    " else\n",
    " READLINE_POINT=0x7fffffff;\n",
    " fi\n",
    "}\n"
);

fn run_rubash(dir: &PathBuf, script: &str, args: &[&str]) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn declare_f_drops_comments_and_requotes_ansi_c() {
    let dir = tempdir("decl");
    std::fs::write(
        dir.join("case.sh"),
        format!("{PINNED_FUNCTION}declare -f __pin_hist__\n"),
    )
    .expect("write case");
    let (stdout, stderr, code) = run_rubash(&dir, "", &["case.sh"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stderr.is_empty(), "stderr: {stderr}");
    // Byte equality with the GNU text is the comment check: EXPECTED_DECLARE
    // carries no comment text (the `#` in `${output#*...}` is the parameter
    // expansion operator, present in GNU output too).
    assert_eq!(stdout, EXPECTED_DECLARE);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exportstr_matches_gnu_export_form() {
    let dir = tempdir("env");
    std::fs::write(
        dir.join("case.sh"),
        format!(
            "{PINNED_FUNCTION}\
             export -f __pin_hist__\n\
             printenv \"BASH_FUNC___pin_hist__%%\"\n"
        ),
    )
    .expect("write case");
    let (stdout, stderr, code) = run_rubash(&dir, "", &["case.sh"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stderr.is_empty(), "stderr: {stderr}");
    assert_eq!(stdout, EXPECTED_EXPORTSTR);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exported_function_imports_and_runs_in_child_shell() {
    let dir = tempdir("child");
    // cf() exercises both leak classes and is callable without fzf's
    // external helpers: a body comment plus `$'\t'` words.
    std::fs::write(
        dir.join("stage1.sh"),
        "cf() { # body comment\n  local v=$'\\t'\n  printf '%s' \"x${v}y\" # tail\n}\n\
         export -f cf\n\
         \"$SELF\" -c 'type -t cf; cf'\n",
    )
    .expect("write stage1");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .env("SELF", env!("CARGO_BIN_EXE_rubash"))
        .arg("stage1.sh")
        .output()
        .expect("run rubash stage1");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    // The child's import guard (src/executor/init.rs:481) must accept the
    // payload: no warning, and the function is both visible and callable.
    assert!(stderr.is_empty(), "child refused the import: {stderr}");
    assert_eq!(stdout, "function\nx\ty");
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ansi_c_newline_and_hex_escapes_reprint_quoted() {
    let dir = tempdir("ansi");
    std::fs::write(
        dir.join("case.sh"),
        "g() { echo $'\\t' \"a\"$'\\n'b $'\\x41'; }\ndeclare -f g\n",
    )
    .expect("write case");
    let (stdout, stderr, code) = run_rubash(&dir, "", &["case.sh"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "g () \n{ \n    echo '\t' \"a\"'\n'b 'A'\n}\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn comsub_connectors_reprint_newline_vs_semicolon() {
    let dir = tempdir("conn");
    std::fs::write(
        dir.join("case.sh"),
        "q1() { echo $(true\nfalse); }\n\
         declare -f q1\n\
         q2() { echo $(true; false); }\n\
         declare -f q2\n\
         q3() { x=$(echo hi # comment\n); }\n\
         declare -f q3\n",
    )
    .expect("write case");
    let (stdout, stderr, code) = run_rubash(&dir, "", &["case.sh"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    // print_cmd.c:288-320: inside a comsub the connector char itself is
    // reprinted — newline stays a newline, `;` stays `; ` — and comments
    // inside the body are gone (parse-time print_comsub normalization).
    assert_eq!(
        stdout,
        "q1 () \n{ \n    echo $(true\nfalse)\n}\n\
         q2 () \n{ \n    echo $(true; false)\n}\n\
         q3 () \n{ \n    x=$(echo hi)\n}\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
