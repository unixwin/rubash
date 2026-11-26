//! Issue rubash#321: the special variable `$_` kept its startup value
//! forever — `: "ARGVAL"; echo $_` printed the interpreter path instead of
//! ARGVAL. pure-bash-bible's trim_string/urlencode (sections 001/013) rely
//! on `: "${1#...}"; printf '%s\n' "$_"` and were corrupted class-wide.
//!
//! GNU semantics (vendored third_party/bash):
//! - execute_cmd.c:4746: `lastarg = lastword->word->word` — the LAST word of
//!   the EXPANDED word list.
//! - execute_cmd.c:4943 bind_lastarg(lastarg) at return_result — bound AFTER
//!   builtin/function/external dispatch; execute_cmd.c:4188-4193 bind_lastarg
//!   binds `_` and VUNSETATTRs the export attribute.
//! - execute_cmd.c:4625-4640: WORDS==0 (pure assignments, vanished words)
//!   → bind_lastarg(NULL) → `_` = "".
//! - `(( ))` (cm_arith) and `[[ ]]` (cm_cond) are NOT simple commands — no
//!   bind; compound-command BODIES bind through their inner simple commands.
//! - Subshells/pipeline members are forks — the parent's `_` is untouched.
//! - variables.c:543: at startup `_` = $0.
//!
//! Root cause: rubash's update_underscore_parameter bound only env_vars,
//! while parameter expansion resolves through the typed variable store
//! (variable_state.rs shell_variable_value), which shadows env_vars and
//! still held the startup value (`declare -p _` already showed the new
//! value — the two stores disagreed).
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28).

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

fn assert_clean(stderr: &str) {
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
}

/// `: "ARGVAL"; echo "[$_]"` — quotes removed (post-expansion word text).
#[test]
fn underscore_binds_expanded_last_word() {
    let (stdout, stderr, _) = rubash(": \"ARGVAL\"\necho \"1[$_]\"\n: x y z\necho \"2[$_]\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "1[ARGVAL]\n2[z]\n");
}

/// pure-bash-bible trim_string (section 001) end-to-end.
#[test]
fn pbb_trim_string_uses_underscore() {
    let (stdout, stderr, _) = rubash(
        "trim_string() {\n\
         : \"${1#\"${1%%[![:space:]]*}\"}\"\n\
         : \"${_%\"${1##*[![:space:]]}\"}\"\n\
         printf '%s\\n' \"$_\"\n\
         }\n\
         trim_string \"    Hello,    World    \"",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "Hello,    World\n");
}

/// pure-bash-bible urlencode (section 013) end-to-end: every iteration's
/// `: "${1:i:1}"` must rebind `$_` to the current character.
#[test]
fn pbb_urlencode_uses_underscore_per_char() {
    let (stdout, stderr, _) = rubash(
        "urlencode() {\n\
         local LC_ALL=C c='' i=''\n\
         for (( i=0; i<${#1}; i++ )); do\n\
         : \"${1:i:1}\"\n\
         case \"$_\" in\n\
         [a-zA-Z0-9.~_-]) printf '%s' \"$_\" ;;\n\
         *) printf '%%%02X' \"'$_\" ;;\n\
         esac\n\
         done\n\
         }\n\
         urlencode \"https://example.com/a b?c=d\"\necho",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "https%3A%2F%2Fexample.com%2Fa%20b%3Fc%3Dd\n");
}

/// `true $UNSET` — when expansion drops every word after the first, the
/// command word itself is the last word.
#[test]
fn underscore_binds_command_word_when_tail_vanishes() {
    let (stdout, stderr, _) = rubash("true $RUBASH_UNSET_VAR\necho \"[$_]\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "[true]\n");
}

/// Pure assignment commands (WORDS==0) bind `$_` to the null string —
/// including the command-substitution RHS form.
#[test]
fn pure_assignment_binds_empty_underscore() {
    let (stdout, stderr, _) =
        rubash(": one\nv=5\necho \"a[$_]\"\nx=$(: sub)\necho \"b[$_]\"\ny=1 z=2\necho \"c[$_]\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "a[]\nb[]\nc[]\n");
}

/// A function call rebinds `$_` to the CALL's last argument after the body
/// finished — the body's own last `$_` write does not survive the call.
#[test]
fn function_call_binds_call_last_word() {
    let (stdout, stderr, _) = rubash("f(){ : inner; }\nf arg1 arg2\necho \"[$_]\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "[arg2]\n");
}

/// `(( ))` and `[[ ]]` are not simple commands: no `$_` bind. (The
/// interleaved `echo` rebinds `$_` to its own last word — expected.)
#[test]
fn arith_and_conditional_do_not_bind_underscore() {
    let (stdout, stderr, _) =
        rubash(": one\n((v=1))\necho \"a[$_]\"\n[[ a == a ]]\necho \"b[$_]\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "a[one]\nb[a[one]]\n");
}

/// eval's own last ARGUMENT binds (eval is a builtin like any other); the
/// body command's bind happens first, then the call's last word wins.
#[test]
fn eval_binds_its_last_argument() {
    let (stdout, stderr, _) = rubash(": one\neval ': inner'\necho \"[$_]\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "[: inner]\n");
}

/// Subshells are forks: the parent's `$_` keeps the value from before;
/// inside the fork the inner commands bind normally.
#[test]
fn subshell_does_not_leak_underscore_to_parent() {
    let (stdout, stderr, _) =
        rubash(": outer\n( : insub; printf 'sub=[%s]\n' \"$_\" )\nprintf 'parent=[%s]\n' \"$_\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "sub=[insub]\nparent=[outer]\n");
}

#[test]
fn command_substitution_binds_inside_fork_only() {
    let (stdout, stderr, _) = rubash(
        r#": outer
v="$(printf '%s' "$_")"
printf 'in=[%s] after=[%s]\n' "$v" "$_""#,
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "in=[outer] after=[]\n");
}

/// `set -- p q; : "$@"` — the expanded positional words participate; the
/// last one binds.
#[test]
fn quoted_dollar_at_last_element_binds() {
    let (stdout, stderr, _) = rubash("set -- p q\n: \"$@\"\necho \"[$_]\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "[q]\n");
}

/// The word is bound AFTER execution, so `$_` inside the command's own
/// expansion sees the PREVIOUS value.
#[test]
fn underscore_visible_during_own_expansion_is_previous_value() {
    let (stdout, stderr, _) = rubash(": first\n: \"pre$_\"\necho \"[$_]\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "[prefirst]\n");
}

/// Single-quoted words are data: `: '${literal}'` binds the literal text
/// (no parameter re-expansion of the bound value).
#[test]
fn single_quoted_last_word_binds_verbatim() {
    let (stdout, stderr, _) = rubash(": '${not expanded}'\necho \"[$_]\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "[${not expanded}]\n");
}

/// Redirect targets are not command words: `: > /dev/null` binds `:`.
#[test]
fn redirect_target_is_not_the_bound_word() {
    let (stdout, stderr, _) = rubash(": > /dev/null\necho \"[$_]\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "[:]\n");
}
