//! Issue rubash#2493 regressions: the modernish die trap's
//! `{ { ...; } & } 8<&0` body killed every respawned async child at
//! parse, and the die path died silently (rc 128, no diagnostic).
//!
//! Root cause (parse face): `bash_command_source_text_render`'s brace
//! arm (executor/command_text.rs) closed every brace group with a
//! literal `; }` — after a body whose last command printed `... &`
//! that is `{ ... &; }`, which GNU's list grammar rejects
//! (parse.y:1275/1290 admit only newline_list after `&`). GNU closes
//! through print_cmd.c:693-728 print_group_command's `semicolon()'
//! (print_cmd.c:1520-1530), which prints NOTHING after `... &' — the
//! same rule the sequence joiner, loop/if closers and the respawn
//! prelude already port (rubash#294). The coproc-body and nested
//! function-definition arms carried the identical latent `; }`/`; '
//! closers and are fixed by the same rule.
//!
//! Root cause (silent face): the corrupted `{ ... &; }` rode into the
//! respawned child's `-c` source through `background_function_prelude`
//! (any defined function containing the shape corrupts EVERY later
//! async respawn), so the child died at parse before running the die
//! message block — rc 128 with no output where GNU prints
//! `<prog> died: <msg>` from the forked trap body.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.20
//! (/usr/local/bin/bash, script-file probes 2026-11-02: `done`/exit 0
//! for the parse shape; `dielike-sleep.sh died: hard error`/exit 128
//! for the die chain, probed with a short pre-exit delay that mirrors
//! the real die's ps/awk sweep — rubash delivers the message without
//! the delay because the respawned child survives the parent's exit).

use std::process::Command;

fn run_script_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i2493-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("i2493.sh");
    std::fs::write(&path, script).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The minimal parse shape: a function body containing a
/// redirect-carrying brace group whose last command is backgrounded —
/// the modernish die trap's `{ { ...; } & } 8<&0` skeleton — plus any
/// compound async respawn to transport the function through the
/// prelude. Before the fix the respawned child died with
/// `syntax error near unexpected token `;'' on the serialized
/// `f() { { { echo inner; } &; } 8<&0; }; ...` prelude text; GNU runs
/// the same script with exit 0 and `done`.
#[test]
fn redirect_brace_group_ending_in_background_respawns_cleanly() {
    let script = concat!(
        "f() { { { echo inner; } & } 8<&0; }\n",
        "{ sleep 0; } &\n",
        "wait\n",
        "echo done\n",
    );
    let (stdout, stderr, code) = run_script_file(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "done\n", "stdout: {stdout}");
    assert!(
        !stderr.contains("syntax error"),
        "respawned child hit the `&; }}' serialization corruption: {stderr}"
    );
}

/// The die chain: the trap-shaped function (both `case` arms carrying
/// the `... 0<&8 & ;;` terminator from the issue, nested braces, fd-8
/// copy) must deliver its diagnostic from the respawned background
/// body and exit 128 — not die silently. GNU 5.3.20 probe (with the
/// ps/awk-equivalent pre-exit delay): `dielike-sleep.sh died: hard
/// error` on stderr, exit 128, empty stdout.
#[test]
fn die_shaped_trap_body_delivers_diagnostic_and_exits_128() {
    let script = concat!(
        "die_like() {\n",
        "    _Msh_E=127\n",
        "    { {\n",
        "        case ${NOSUCHVAR123+s} in\n",
        "        ( s ) while let \"(1-1) >= 0\"; do echo stacktrap 0<&8 & done ;;\n",
        "        esac\n",
        "        case ${NOSUCHVAR456+p} in\n",
        "        ( p ) eval \"echo evaltrap\" 0<&8 & ;;\n",
        "        esac\n",
        "        printf \"%s died: %s\\n\" \"${0##*/}\" \"$1\" >&2\n",
        "    } & } 8<&0\n",
        "    command exit 128\n",
        "}\n",
        "die_like \"hard error\"\n",
    );
    let (stdout, stderr, code) = run_script_file(script);
    assert_eq!(code, Some(128), "stderr: {stderr}");
    assert!(stdout.is_empty(), "stdout: {stdout}");
    assert!(
        stderr.contains("died: hard error"),
        "die path was silent (rc 128 without diagnostic): {stderr:?}"
    );
    assert!(
        !stderr.contains("syntax error"),
        "die trap body failed to re-parse in the respawned child: {stderr}"
    );
}

/// The modernish real-state chain (issue repro: source modernish, call
/// die) when the local fixture tree exists on this machine. The fixture
/// is not part of the repository, so the test self-skips elsewhere; on
/// machines that carry it, this pins the exact issue face: GNU 5.3.20
/// prints `<script> died: direct die call` and exits 128, rubash was
/// silent before the fix.
#[test]
fn modernish_die_trap_real_state_when_fixture_present() {
    let fixture = "/d/repo/pre15-modernish/bin/modernish";
    let fixture_path = std::path::Path::new("D:/repo/pre15-modernish/bin/modernish");
    if !fixture_path.exists() {
        eprintln!("skipping: {fixture} not present on this machine");
        return;
    }
    let script = format!(
        "export MSH_PREFIX=/d/repo/pre15-modernish\n. {fixture}\ndie \"direct die call\"\n"
    );
    let (stdout, stderr, code) = run_script_file(&script);
    assert_eq!(code, Some(128), "stderr: {stderr}");
    assert!(stdout.is_empty(), "stdout: {stdout}");
    assert!(
        stderr.contains("died: direct die call"),
        "modernish die path silent under rubash; stderr: {stderr:?}"
    );
}
