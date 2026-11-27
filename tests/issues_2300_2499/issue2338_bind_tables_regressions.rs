//! rubash#2338 — the `bind` key tables diverged from GNU readline:
//! `-q` listed invoking sequences in the wrong order and missed
//! sequences (`backward-char` lacked `\eOD`), `-l` omitted function
//! names GNU lists (`complete`, `history-search-backward`), the
//! `-m vi-insert` / `-m vi-command` keymap queries answered from the
//! emacs table, and the `bind -f` failure leaked std's `(os error 2)`
//! suffix into the diagnostic.
//!
//! GNU ground:
//! - builtins/bind.def:344-371 query_bindings — answers from
//!   rl_invoking_keyseqs over the CURRENT keymap; unbound is rc 1.
//! - lib/readline/bind.c rl_invoking_keyseqs_in_map — sequences are
//!   collected scanning key bytes 0..255, submaps recursed in place:
//!   byte order of the translated sequence is the listing order.
//! - lib/readline/bind.c rl_function_dumper / rl_print_keybinding —
//!   `-p`/`-P` iterate the funmap names in order after a leading blank
//!   line; unbound functions print `# name (not bound)` (readable) or
//!   `name is not bound to any keys` (plain, no period).
//! - lib/readline/funmap.c funmap[] + bashline.c rl_add_defun — the
//!   176 names `bind -l` lists (rl_list_funmap_names).
//! - builtins/bind.def:274-276 — the `-f` failure text is
//!   `FILE: cannot read: ` + strerror(errno), nothing else.
//!
//! All expectations byte-verified against GNU bash 5.3.20 (the hunt
//! r35 verdicts bd01/bd24/bd25/bd27/bd28 and the oracle dumps the
//! engine tables were captured from).

use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

fn run(script: &str) -> Run {
    // INPUTRC points at an empty file so the host's real inputrc can
    // never leak bindings into the bind world (initialize_readline
    // reads it on the first bind call).
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
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code().unwrap_or(-1),
    }
}

#[test]
fn issue2338_query_backward_char_lists_all_sequences_in_scan_order() {
    let r = run("bind -q backward-char");
    assert_eq!(r.code, 0);
    assert_eq!(
        r.stdout,
        "backward-char can be invoked via \"\\C-b\", \"\\eOD\", \"\\e[D\".\n"
    );
}

#[test]
fn issue2338_query_uses_the_named_keymaps_table() {
    let r = run("bind -m vi-insert -q backward-char");
    assert_eq!(
        r.stdout,
        "backward-char can be invoked via \"\\eOD\", \"\\e[D\".\n"
    );
    let r = run("bind -m vi-command -q backward-char");
    assert_eq!(
        r.stdout,
        "backward-char can be invoked via \"\\C-h\", \"\\eOD\", \"\\e[D\", \"h\".\n"
    );
    // The standalone meta map answers on its own sequences.
    let r = run("bind -m emacs-meta -q shell-expand-line");
    assert_eq!(
        r.stdout,
        "shell-expand-line can be invoked via \"\\C-e\".\n"
    );
}

#[test]
fn issue2338_query_more_default_sequences() {
    let r = run("bind -q complete");
    assert_eq!(
        r.stdout,
        "complete can be invoked via \"\\C-i\", \"\\e\\e\".\n"
    );
    let r = run("bind -q yank");
    assert_eq!(r.stdout, "yank can be invoked via \"\\C-y\".\n");
    let r = run("bind -q set-mark");
    assert_eq!(
        r.stdout,
        "set-mark can be invoked via \"\\C-@\", \"\\e \".\n"
    );
    let r = run("bind -q self-insert");
    assert_eq!(
        r.stdout,
        "self-insert can be invoked via \" \", \"!\", \"\\\"\", \"#\", \"$\", ...\n"
    );
}

#[test]
fn issue2338_function_list_is_gnus_176() {
    let r = run("bind -l");
    let names: Vec<&str> = r.stdout.lines().collect();
    assert_eq!(names.len(), 176);
    assert!(names.contains(&"complete"), "bind -l must list complete");
    assert!(
        names.contains(&"history-search-backward"),
        "bind -l must list history-search-backward"
    );
    assert_eq!(names.first().copied(), Some("abort"));
}

#[test]
fn issue2338_dump_plain_matches_rl_function_dumper() {
    let r = run("bind -P");
    let lines: Vec<&str> = r.stdout.lines().collect();
    // Leading blank line + one line per funmap name.
    assert_eq!(r.stdout.split('\n').next(), Some(""));
    assert_eq!(lines.len(), 177);
    assert!(r
        .stdout
        .contains("backward-char can be found on \"\\C-b\", \"\\eOD\", \"\\e[D\".\n"));
    assert!(r
        .stdout
        .contains("self-insert can be found on \" \", \"!\", \"\\\"\", \"#\", \"$\", ...\n"));
    // Unbound in this keymap: plain form, NO trailing period.
    assert!(r
        .stdout
        .contains("alias-expand-line is not bound to any keys\n"));
}

#[test]
fn issue2338_dump_readable_matches_rl_function_dumper() {
    let r = run("bind -p");
    let lines: Vec<&str> = r.stdout.lines().collect();
    assert_eq!(lines.len(), 497);
    assert!(r.stdout.contains("\"\\e[200~\": bracketed-paste-begin\n"));
    assert!(r.stdout.contains("\"\\C-b\": backward-char\n"));
    assert!(r.stdout.contains("# alias-expand-line (not bound)\n"));
}

#[test]
fn issue2338_unreadable_file_error_has_no_os_error_suffix() {
    let r = run("bind -f /nonexistent_rubash_2338");
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("cannot read: "), "stderr: {}", r.stderr);
    assert!(
        !r.stderr.contains("(os error"),
        "io error suffix leaked: {}",
        r.stderr
    );
    #[cfg(unix)]
    assert!(
        r.stderr
            .contains("/nonexistent_rubash_2338: cannot read: No such file or directory"),
        "stderr: {}",
        r.stderr
    );
}
