//! rubash#2480 — modernish LOOP/DO/DONE: alias values that introduce
//! `{` reserved words must re-enter reserved-word recognition after
//! alias expansion.
//!
//! GNU contract: the reader is incremental — builtins/evalstring.c:359
//! `while (*(bash_input.location.string) || parser_expanding_alias ())`
//! parses one complete command only after the previous commands
//! EXECUTED — and read_token_word consults the alias gate live on every
//! token (parse.y:5759 `if (expand_aliases && quoted == 0)`). After an
//! alias expands (`alias_expand_token` parse.y:3249 pushes the value
//! back onto the input stream), every token of the pushed text goes
//! through `special_case_tokens` (parse.y:3366) and
//! `CHECK_FOR_RESERVED_WORD` (parse.y:3168), where `reserved_word_
//! acceptable` (parse.y:5899) accepts `last_read_token == '{'` — so
//! `{ { { cmd` from an alias value parses as three nested groups, and a
//! later `}` from another alias closes them (parse.y:3465
//! open_brace_count rule).
//!
//! The modernish trigger: `adj/std.sh` enables alias expansion WHILE
//! `bin/modernish` is being sourced (`command shopt -s expand_aliases`,
//! shopt.def:630 shopt_set_expaliases), so the CALLING script's own
//! text never mentions the option. The whole-file fast path parsed the
//! script with the gate off and handed `LOOP` to the executor's
//! word-level alias fallback, which flattened the alias-introduced
//! reserved words into plain words — `LOOP for i in 1 2 3; DO ...; DONE`
//! became one simple command with command name `{` ("{: command not
//! found"). The fix routes scripts that can run foreign text (`.` /
//! `source` / `eval`) through the grouped driver
//! (`script_driver.rs::script_runs_foreign_text`).
//!
//! Expectations byte-verified against GNU bash 5.3.20 (WSL, script
//! files). The full triplet reproduces the loop.mm:53-67 alias machine
//! (DO's value ends in `{ ` — trailing-blank next-word expansion) with
//! the FD8 producer on the PARENT side (`exec 8< <(...)` inside the
//! block, like _Msh_loopgen but without forking): the producer-child
//! ambient-fd inheritance gap that still blocks the unmodified
//! `bin/modernish` generator is a separate follow-up, not pinned here.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2480-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// inner.sh = the modernish adj/std.sh trigger reduced to one line:
/// alias expansion turned on from INSIDE a sourced file, so the main
/// script's own text never mentions `expand_aliases`.
const INNER: &str = "shopt -s expand_aliases\n";

#[test]
fn nested_brace_alias_after_sourced_expand_aliases() {
    let dir = tempdir("nested");
    std::fs::write(dir.join("inner.sh"), INNER).expect("write inner");
    let script = ". ./inner.sh\n\
                  alias LOOP='{ { { echo L3'\n\
                  alias DONE='} } }'\n\
                  LOOP x\n\
                  DONE\n\
                  echo \"c1 rc=$?\"\n";
    std::fs::write(dir.join("probe.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("probe.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    // GNU 5.3.20 (script file): `L3 x`, rc 0, empty stderr.
    assert_eq!(stdout, "L3 x\nc1 rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn modernish_loop_triplet_end_to_end() {
    let dir = tempdir("triplet");
    std::fs::write(dir.join("inner.sh"), INNER).expect("write inner");
    // The loop.mm:53-67 alias triplet verbatim (BUG_SCLOSEDFD branch),
    // with _Msh_loop opening the block-local FD8 from the parent — the
    // parse shape under test (three nested `{` groups + `}` closers +
    // do/done while loop reading FD8) is exactly the modernish one.
    let script = ". ./inner.sh\n\
                  alias LOOP='{ { { _Msh_loop'\n\
                  alias DO='}; _Msh_loop_c && while _loop_E=0; IFS= read -r _loop_i <&8 && eval \" ${_loop_i}\"; do { '\n\
                  alias DONE='} 8<&-; done; } 8<&-; _Msh_loop_setE; }'\n\
                  _Msh_loop() { exec 8< <(printf 'i=1\\ni=2\\ni=3\\n'); }\n\
                  _Msh_loop_c() { return 0; }\n\
                  _Msh_loop_setE() { _loop_E=0; }\n\
                  LOOP for i in 1 2 3; DO echo \"body i=$i\"; DONE\n\
                  echo \"RC=$? loop_E=${_loop_E-unset}\"\n";
    std::fs::write(dir.join("probe.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("probe.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    // GNU 5.3.20 (script file) prints the three iterations, rc 0.
    assert_eq!(stdout, "body i=1\nbody i=2\nbody i=3\nRC=0 loop_E=0\n");
    assert_eq!(stderr, "");
    assert_eq!(output.status.code(), Some(0));
}

/// GNU parses one PHYSICAL LINE at a time (shell_getc fills
/// shell_input_line): a sourced `shopt -s expand_aliases` cannot affect
/// commands that follow it on the SAME line through `;` — they were
/// parsed as one list while the gate was still off, so the alias word
/// runs as a plain command ("G: command not found"). Pin that
/// line-granularity contract so a future driver change cannot silently
/// diverge from GNU in the other direction.
#[test]
fn same_line_after_sourced_shopt_stays_unexpanded_like_gnu() {
    let dir = tempdir("sameline");
    std::fs::write(dir.join("inner.sh"), INNER).expect("write inner");
    let script = ". ./inner.sh; alias G='{ { { echo L3'; alias Z='} } }'; G x; Z; echo \"rc=$?\"\n";
    std::fs::write(dir.join("probe.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("probe.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    // GNU 5.3.20: `rc=127` on stdout, `line N: G: command not found`
    // and `line N: Z: command not found` on stderr.
    assert_eq!(stdout, "rc=127\n");
    assert_eq!(
        stderr.matches(": G: command not found").count()
            + stderr.matches(": Z: command not found").count(),
        2,
        "stderr: {stderr}"
    );
    assert_eq!(output.status.code(), Some(0));
}

/// The trailing-blank rule (mk_alexpansion keeps the alias's final
/// blank; parse.y:2098 sets PST_ALEXPNEXT when the pushed string pops):
/// an alias value ending in a blank makes the NEXT word an alias
/// candidate even in argument position — the modernish DO value ends
/// `{ `. GNU expands the chained word there.
#[test]
fn trailing_blank_alias_chain_expands_next_word() {
    let dir = tempdir("chain2");
    std::fs::write(dir.join("inner.sh"), INNER).expect("write inner");
    let script = ". ./inner.sh\n\
                  alias L='echo LEAD '\n\
                  alias EMIT='echo EMITTED'\n\
                  L EMIT\n";
    std::fs::write(dir.join("probe.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("probe.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    // GNU 5.3.20: L expands (`echo LEAD `), the trailing blank makes
    // EMIT expand too (`echo EMITTED`), leaving `echo LEAD echo
    // EMITTED`.
    assert_eq!(stdout, "LEAD echo EMITTED\n");
    assert_eq!(stderr, "");
    assert_eq!(output.status.code(), Some(0));
}
