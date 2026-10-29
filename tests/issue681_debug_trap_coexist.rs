//! niubash#681: a user DEBUG trap whose action ends in a function
//! call must not desynchronize the debugger consult.
//!
//! Design (docs/design/debugger-full-design.md): section 9.1 — the
//! debugger does not consume, replace, or suppress the user's DEBUG
//! trap; section 4.2 — at every seam the user trap runs in full
//! BEFORE the consult, and the consult decides about the seam's own
//! command. The consult therefore has to judge the seam by the
//! location the seam arrived with, not by ambient line state the
//! trap action's function calls leave behind (a function exit
//! deliberately restamps the ambient line to the function's
//! definition line for the RETURN trap's benefit). Before the fix,
//! a function-valued trap (`trap "omh" DEBUG` — the oh-my-niu
//! dispatch shape) silently lost every line breakpoint: stops were
//! adjudicated against the trap function's definition line, so
//! they either never matched or replayed at a stale location with
//! the current command's text (the pkg9 A2 B6 shape).

use std::io::{BufReader, Cursor, Write};
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

#[derive(Clone)]
struct SharedOut(std::sync::Arc<Mutex<Vec<u8>>>);

impl Write for SharedOut {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Write `source` to a fresh temp file; return (dir, program path
/// in the identity form the session core itself uses).
fn program_file(name: &str, source: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("rubash-issue681-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{name}.sh"));
    std::fs::write(&path, source).expect("write program");
    let canonical = std::fs::canonicalize(&path).expect("canonicalize");
    let text = canonical.to_string_lossy().into_owned();
    let identity = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        text
    };
    (dir, identity)
}

/// Run one scripted CLI session; return (status, transcript with
/// the program path normalized to `<PROG>`).
fn run_cli(program: &str, commands: &str) -> (i32, String) {
    let out = SharedOut(std::sync::Arc::new(Mutex::new(Vec::new())));
    let status = rubash::run_debug_cli_with_stream(
        rubash::DebugCliPreset {
            program: Some(program.to_string()),
            args: Vec::new(),
        },
        Box::new(BufReader::new(Cursor::new(commands.as_bytes().to_vec()))),
        Box::new(out.clone()),
    );
    let bytes = out.0.lock().unwrap().clone();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    (status, text.replace(program, "<PROG>"))
}

/// The issue's minimal form: the trap action is a bare function
/// name. The breakpoint must stop, at its own line, with its own
/// command text — and the user's trap must have run (its marker
/// file carries one line per body command).
#[test]
fn function_valued_debug_trap_keeps_line_breakpoints() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let (dir, program) = program_file("functrap", "");
    let marker = dir.join("trap-fired.txt");
    let source = format!(
        "omh(){{ echo fired >> '{}'; }}\ntrap \"omh\" DEBUG\necho one\necho two\necho three\n",
        marker.display()
    );
    std::fs::write(dir.join("functrap.sh"), source).expect("write program");
    let (status, transcript) = run_cli(&program, "break 4\ncontinue\ncontinue\n");
    assert_eq!(status, 0, "{transcript}");
    assert!(
        transcript.contains("Stopped (breakpoint 1) at <PROG>:4: echo two"),
        "the line-4 breakpoint must stop at its own seam:\n{transcript}"
    );
    // The user trap runs in full (design 9.1): it fired for each of
    // the three echo commands after its installation.
    let fired = std::fs::read_to_string(&marker).expect("the user trap ran");
    assert_eq!(fired.lines().count(), 3, "trap firings: {fired:?}");
    std::fs::remove_dir_all(&dir).ok();
}

/// The mechanism, not the bare-name shape: an action whose LAST
/// command is a function call (`true; omh`) desynchronized the
/// consult exactly like the bare-name form, while an action ending
/// in a builtin did not. Both must stop now.
#[test]
fn trap_action_ending_in_function_call_keeps_breakpoints() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let (dir, program) = program_file(
        "tailcall",
        "omh(){ :; }\ntrap \"true; omh\" DEBUG\necho one\necho two\necho three\n",
    );
    let (status, transcript) = run_cli(&program, "break 4\ncontinue\ncontinue\n");
    assert_eq!(status, 0, "{transcript}");
    assert!(
        transcript.contains("Stopped (breakpoint 1) at <PROG>:4: echo two"),
        "the line-4 breakpoint must stop at its own seam:\n{transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The B6 shape: with breakpoints near the trap function's
/// definition line, stops must not replay at the stale definition
/// location wearing another command's text. Every stop banner must
/// pair its line with that line's own command.
#[test]
fn stops_never_replay_at_the_trap_functions_definition_line() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let (dir, program) = program_file(
        "replay",
        "omh(){ :; }\ntrap \"omh\" DEBUG\necho one\necho two\necho three\n",
    );
    let (status, transcript) = run_cli(
        &program,
        "break 1\nbreak 3\nbreak 4\nbreak 5\ncontinue\ncontinue\ncontinue\ncontinue\n",
    );
    assert_eq!(status, 0, "{transcript}");
    // Line 1 (the definition) has no seam before the trap exists;
    // the stops that fire are the three echo seams, each at its own
    // line with its own text.
    for needle in [
        "at <PROG>:3: echo one",
        "at <PROG>:4: echo two",
        "at <PROG>:5: echo three",
    ] {
        assert!(
            transcript.contains(needle),
            "stop {needle:?} must appear:\n{transcript}"
        );
    }
    assert!(
        !transcript.contains("at <PROG>:1: echo"),
        "no stop may wear the definition line with an echo's text:\n{transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Control: a string trap whose action is a builtin already kept
/// its breakpoints before the fix; it must keep them after.
#[test]
fn string_builtin_debug_trap_still_keeps_breakpoints() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let (dir, program) = program_file(
        "strtrap",
        "trap 'echo TRAPFIRED' DEBUG\necho one\necho two\necho three\n",
    );
    let (status, transcript) = run_cli(&program, "break 3\ncontinue\ncontinue\n");
    assert_eq!(status, 0, "{transcript}");
    assert!(
        transcript.contains("Stopped (breakpoint 1) at <PROG>:3: echo one"),
        "the line-3 breakpoint must stop at its own seam:\n{transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}
