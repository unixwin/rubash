//! rubash#2914 — old-style keyname specs (`bind 'Control-t: undo'`)
//! were silently dropped: rl_parse_and_bind's old-style arm
//! (lib/readline/bind.c:1594, the tail after the new-style branch)
//! was not implemented, so the binding never landed and no
//! diagnostic fired. GNU ground: the keyname is the part after the
//! last `-` (bind.c:1803), translated by glean_key_from_name
//! (bind.c:2387: DEL/ESC/Escape/LFD/Newline/RET/Return/Rubout/SPC/
//! Space/Tab, matched case-insensitively, else the first byte);
//! Control-/Meta- prefixes are found by a case-insensitive substring
//! scan of the whole spec (`_rl_strindex`, util.c:274) over
//! _rl_possible_control_prefixes / _rl_possible_meta_prefixes
//! (bind.c:1213); a `-` spec matching neither prefix reports
//! `%s: unknown key modifier` (bind.c:1829). An unknown function
//! name binds the key to a NULL slot (rl_bind_key, bind.c:140):
//! the previous binding is gone and nothing lists the key. Every
//! probe below binds a NON-DEFAULT target — the default binding
//! would mask a dropped bind.

use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

struct Run {
    stdout: String,
    stderr: String,
}

fn run(script: &str) -> Run {
    #[cfg(unix)]
    const EMPTY_INPUTRC: &str = "/dev/null";
    #[cfg(windows)]
    const EMPTY_INPUTRC: &str = "NUL";
    let out = Command::new(RUBASH)
        .args(["-c", script])
        .env("INPUTRC", EMPTY_INPUTRC)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    let stderr = String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|line| !line.contains("line editing not enabled"))
        .map(|line| format!("{line}\n"))
        .collect();
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr,
    }
}

#[test]
fn issue2914_control_prefix_binds_the_key() {
    let r = run("bind 'Control-t: undo'; bind -p | grep -F '\"\\C-t\"'; bind -p | grep transpose");
    assert_eq!(r.stdout, "\"\\C-t\": undo\n# transpose-chars (not bound)\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2914_meta_prefix_binds_the_key() {
    let r = run("bind 'Meta-x: undo'; bind -p | grep -F '\"\\ex\"'; bind -p | grep execute-named");
    assert_eq!(
        r.stdout,
        "\"\\ex\": undo\n# execute-named-command (not bound)\n"
    );
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2914_bare_keyname_without_value_reports_unknown_key_modifier() {
    // `backward-char ` carries a separator (the trailing space), so
    // the spec reaches the old-style arm, where the `-` in the name
    // matches no Control-/Meta- prefix (bind.c:1827-1831).
    let r = run("bind 'backward-char '; echo rc=$?");
    assert_eq!(r.stdout, "rc=0\n");
    assert_eq!(r.stderr, "readline: backward-char: unknown key modifier\n");
}

#[test]
fn issue2914_prefix_spellings_and_case_match_gnu() {
    // _rl_strindex is case-insensitive: every prefix spelling binds.
    let r = run("bind 'C-t: yank'; bind -p | grep -F '\"\\C-t\"'");
    assert_eq!(r.stdout, "\"\\C-t\": yank\n");
    let r = run("bind 'CTRL-t: yank'; bind -p | grep -F '\"\\C-t\"'");
    assert_eq!(r.stdout, "\"\\C-t\": yank\n");
    let r = run("bind 'control-t: kill-line'; bind -p | grep -F '\"\\C-t\"'");
    assert_eq!(r.stdout, "\"\\C-t\": kill-line\n");
    let r = run("bind 'M-x: yank'; bind -p | grep -F '\"\\ex\"'");
    assert_eq!(r.stdout, "\"\\ex\": yank\n");
    // Control and Meta compose: META (CTRL (key)).
    let r = run("bind 'Meta-Control-t: undo'; bind -p | grep -F '\"\\e\\C-t\"'");
    assert_eq!(r.stdout, "\"\\e\\C-t\": undo\n");
}

#[test]
fn issue2914_bare_and_named_keys_bind() {
    // No `-` at all: the whole spec is the key name (bind.c:1804).
    let r = run("bind 't: undo'; bind -p | grep -F '\"t\":'");
    assert_eq!(r.stdout, "\"t\": undo\n");
    // glean_key_from_name's table (bind.c:2371).
    let r = run("bind 'DEL: undo'; bind -p | grep -F '\"\\C-?\"'");
    assert_eq!(r.stdout, "\"\\C-?\": undo\n");
    let r = run("bind 'Tab: undo'; bind -p | grep -F '\"\\C-i\"'");
    assert_eq!(r.stdout, "\"\\C-i\": undo\n");
    let r = run("bind 'Space: undo'; bind -p | grep -F '\" \":'");
    assert_eq!(r.stdout, "\" \": undo\n");
}

#[test]
fn issue2914_unknown_function_binds_a_null_slot() {
    // rl_named_function fails, rl_bind_key stores a NULL function:
    // the default binding of the key is gone, nothing lists it.
    let r =
        run("bind 'Control-t: ins-macro'; bind -p | grep transpose; bind -p | grep -F '\"\\C-t\"'");
    assert_eq!(r.stdout, "# transpose-chars (not bound)\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2914_equivalency_is_a_no_op() {
    // bind.c:1759: `key:=value` returns without binding.
    let r = run("bind '\"\\C-t\": yank'; bind 'Control-t:=undo'; bind -p | grep -F '\"\\C-t\"'");
    assert_eq!(r.stdout, "\"\\C-t\": yank\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2914_old_style_macro_binds() {
    // A quoted value in the old-style arm is a macro (bind.c:1834).
    let r = run("bind 'Control-y: \"echo hi\"'; bind -s | grep -F '\"\\C-y\"'");
    assert_eq!(r.stdout, "\"\\C-y\": \"echo hi\"\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2914_unterminated_macro_reports_missing_closing_quote() {
    let r = run("bind 'Meta-x: \"abc'; echo rc=$?");
    assert_eq!(r.stdout, "rc=0\n");
    assert_eq!(
        r.stderr,
        "readline: `\"abc': missing closing quote for macro\n"
    );
}

#[test]
fn issue2914_separator_carrying_absorbed_spec_still_binds_its_key() {
    // `bind 'ls -l'` is silent (rubash#2873) — but rl_parse_and_bind
    // still runs it through the old-style arm: key `l` (the part
    // after the last `-`... here the keyname is `ls`, gleaned to
    // `l`) bound to rl_named_function("-l") == NULL, so the default
    // self-insert slot for `l` is gone from the dump.
    let r = run("bind 'ls -l'; bind -p | grep -F '\"l\":'; echo done");
    assert_eq!(r.stdout, "done\n");
    assert_eq!(r.stderr, "");
}
