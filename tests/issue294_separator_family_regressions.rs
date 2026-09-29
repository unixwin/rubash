//! rubash#294 `&;' separator-family regressions (wt13/fixpack lane, Misc A).
//!
//! The #294 fix covered three joiners; the audit (ctrlcfix lane) left the
//! same unconditional `"; "' join at every other serialization site. GNU
//! print_cmd.c:1519-1529 `semicolon()` suppresses the `;' separator when
//! the printed command ends with `" &"` or a newline (after `&` the list
//! grammar admits only newline_list — parse.y:1275/1290; a `;' at list
//! start is the parse.y:1326 error arm), and every compound-closer print
//! runs `semicolon(); newline("done"/"fi"/...)' (print_cmd.c:626-631 for
//! loops, 800-832 while/until, print_if_command for elif/else/fi).
//!
//! Fixed sites:
//! - executor/alias_reparse.rs: all seven alias-reparse splice joins
//!   (incl. the close-word join) now use the shared
//!   `push_command_separator`.
//! - executor/command_text.rs: `push_command_separator` shared helper;
//!   `command_body_source_text`, `loop_command_source_text` and
//!   `if_command_source_text` join their closers through it — a function
//!   body ending in `... &' serialized as `... &; done'/`... &; fi'
//!   corrupted every respawned background child (background_command_source
//!   re-serializes exported functions into the child's `-c' source).
//!
//! Probes verified byte-for-byte against WSL GNU Bash 5.3.0 (2026-09-28,
//! artifacts under target/issue-suites/results/fixpackmisca/): master
//! printed only `ok' plus `syntax error near unexpected token `;'' and
//! lost every async tick; the fixed build matches GNU exactly.

use std::process::Command;

/// Run a script FILE in its own scratch directory; returns (stdout lines
/// sorted — async children may interleave — stderr, code).
fn rubash_file_sorted(script: &str) -> (Vec<String>, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-misca-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(String::from)
        .collect();
    lines.sort();
    (
        lines,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Core repro: a while body whose LAST command is async — the serialized
/// closer must be `\ndone', never `; done' (the `&;' corrupted the
/// respawned child and both async ticks were lost with a `-c' syntax
/// error).
#[test]
fn loop_closer_after_async_body_command() {
    let (out, err, code) = rubash_file_sorted(
        "f() {\n\ti=0\n\twhile [ \"$i\" -lt 2 ]; do\n\t\ti=$((i+1))\n\t\techo tick$i &\n\tdone\n}\nf\nwait\necho ok\n",
    );
    assert_eq!(
        out,
        vec!["ok", "tick1", "tick2"],
        "async ticks must survive"
    );
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Same shape through the `fi' closer of an if body.
#[test]
fn if_closer_after_async_body_command() {
    let (out, err, code) =
        rubash_file_sorted("h() {\n\tif true; then\n\t\techo a &\n\tfi\n}\nh\nwait\necho ok\n");
    assert_eq!(out, vec!["a", "ok"]);
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Alias-spliced loop (the alias_reparse path): the splice joins and the
/// closer both go through the separator rule.
#[test]
fn alias_spliced_loop_with_async_body() {
    let (out, err, code) = rubash_file_sorted(
        "shopt -s expand_aliases\nalias wbegin='while true; do'\nalias wend='done'\ng() {\n\twbegin\n\techo tick &\n\tbreak\n\twend\n}\ng\nwait\necho done\n",
    );
    assert_eq!(out, vec!["done", "tick"]);
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Non-async bodies keep the plain `; closer' spelling (regression guard
/// for the guard itself).
#[test]
fn normal_closer_still_uses_semicolon() {
    let (out, err, code) = rubash_file_sorted(
        "f() {\n\twhile true; do\n\t\techo tick\n\t\tbreak\n\tdone\n}\nf\necho ok\n",
    );
    assert_eq!(out, vec!["ok", "tick"]);
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}
