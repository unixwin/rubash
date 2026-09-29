//! Issue rubash#316 (partial, wt13/sweepfix lane): for-list words whose
//! glob character reaches the field only through an expansion boundary.
//!
//! Isolated chain (target/sweepfix/bit/, 2026-09-29): bash-it's
//! `scripts/reloader.bash` loads components with
//! `for f in "$BASH_IT/enabled"/*"${_bash_it_reloader_type}.bash"` —
//! the quoted EMPTY expansion glued between `*` and `.bash`. Under GNU
//! the glob matches `enabled/800---aliases.completion.bash` (which
//! defines `_bash-it-component-completion-callback-on-init-aliases`,
//! the 162nd function of the issue's sample); under rubash the loop
//! iterated over the literal `*.bash` and the component was never
//! sourced (FUNCS 161 vs 162).
//!
//! GNU contract: subst.c:13219 expand_word_list_internal splits each
//! for-list word on IFS and THEN pathname-expands every resulting field
//! (glob_expand_word_list). rubash's unquoted-expansion branch did the
//! field split but never the per-field pathname expansion, so any glob
//! character that surfaced only at an expansion boundary stayed literal.
//!
//! This file pins the now-fixed shapes, byte-verified against WSL GNU
//! Bash 5.3.0 (all six variants below glob on GNU):
//! - `$DIR/enabled/*"${empty}".bash` (unquoted head, quoted empty)
//! - `$DIR/enabled/*$empty.bash`     (unquoted empty expansion)
//!
//! Known residual (recorded in the commit, NOT pinned here as wrong
//! behavior): the fully-quoted-head shape
//! `"$DIR/enabled"/*"${empty}".bash` still globs correctly in a
//! two-line script but regresses to the literal when additional
//! for-loops with the same word appear later in the same script, and a
//! quoted-empty at word END (`"$DIR/enabled"/*"${empty}"`) stays
//! literal — a deeper cooked-word/metadata state interaction in the
//! quoted-word (non-split) branch, still open under this issue.

use std::process::Command;

fn rubash_for(pattern_line: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i316-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(dir.join("enabled")).expect("scratch dir");
    std::fs::write(dir.join("enabled/800---comp.completion.bash"), b"").expect("component");
    let script = format!(
        "cd '{}'\nempty=''\nfor f in {pattern_line}; do echo \"OUT=[$f]\"; done\n",
        dir.display()
    );
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        String::from_utf8_lossy(&output.stderr).is_empty(),
        "stderr not empty"
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The bash-it reloader shape with an unquoted head: the glob expands.
#[test]
fn for_list_glob_through_empty_expansion_boundary() {
    let out = rubash_for("$PWD/enabled/*\"${empty}\".bash");
    assert!(
        out.starts_with("OUT=[") && out.contains("800---comp.completion.bash"),
        "out: {out}"
    );
    assert_eq!(out.matches("OUT=[").count(), 1);
}

/// Unquoted empty expansion glued to the glob character.
#[test]
fn for_list_glob_through_unquoted_empty_expansion() {
    let out = rubash_for("$PWD/enabled/*$empty.bash");
    assert!(out.contains("800---comp.completion.bash"), "out: {out}");
}

/// Glob characters arriving FROM the expansion value also expand
/// (GNU: expansion results pathname-expand in unquoted for-list words).
#[test]
fn for_list_glob_from_expansion_value() {
    let out = rubash_for("enabled/$pat");
    // pat is unset here, so this is the literal-word arm; use a set var
    // instead by inline expansion of the pattern variable.
    let out2 = rubash_for("enabled/${pat:-*completion.bash}");
    assert!(out2.contains("800---comp.completion.bash"), "out2: {out2}");
    let _ = out;
}

/// Multiple fields each glob (split, then per-field expansion).
#[test]
fn for_list_split_fields_each_glob() {
    let out = rubash_for("enabled/$empty*completion.bash enabled/8*");
    assert_eq!(out.matches("OUT=[").count(), 2, "out: {out}");
    assert!(out.contains("800---comp.completion.bash"), "out: {out}");
}
