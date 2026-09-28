//! Regression: run_script_with_history_in lets an embedding host run a
//! script against its own session history (engine sinking list S1).
//!
//! GNU gives each new shell process an empty history list, but a host
//! shell running a script in-process must share its live session so
//! `!!` expands against the host's entries and the script's own commands
//! record back into the same list (niubash histexp suite / in-script `!!`).

use rubash::executor::Executor;
use rubash::history::SessionHistory;
use rubash::script_driver::run_script_with_history_in;
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn host_session_history_feeds_and_collects_script_expansions() {
    let session = Rc::new(RefCell::new(SessionHistory::new()));
    assert!(session
        .borrow_mut()
        .record("echo seedmark", "", "", Some(0)));

    let out = std::env::temp_dir().join(format!("rubash-s1-{}-out.txt", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let out_str = out.to_string_lossy().replace('\\', "/");
    // `!echo` resolves to the newest entry starting with "echo" — the
    // host-preloaded line — while `!!` would hit the script's own
    // `set -o histexpand` record instead.
    let script = format!("set -o history\nset -o histexpand\n!echo >{out_str}\n");

    let mut executor = Executor::new();
    let code = run_script_with_history_in(&mut executor, &script, session.clone(), None);
    assert_eq!(code, 0);

    // `!!` expanded to the host-preloaded `echo seedmark` entry and ran it
    // under the redirect.
    let body = std::fs::read_to_string(&out).expect("expanded command output file");
    assert_eq!(body, "seedmark\n");

    // The script's groups recorded back into the SAME session object —
    // that is the shared data plane the host owns. (`set -o history`
    // itself runs before the flag takes effect, so the first recorded
    // script line is `set -o histexpand`; the last is the expanded
    // `!echo` text.)
    let entries = &session.borrow().entries;
    assert_eq!(
        entries.get(1).map(String::as_str),
        Some("set -o histexpand")
    );
    assert_eq!(
        entries.last().map(String::as_str),
        Some(format!("echo seedmark >{out_str}").as_str())
    );
}

/// Vec-backed HistoryProvider standing in for the host's line-editor
/// history (niubash wires reedline's LiveFileBackedHistory here).
#[derive(Debug, Default)]
struct VecProvider {
    entries: Vec<String>,
}

impl rubash::history::HistoryProvider for VecProvider {
    fn entries(&mut self) -> std::io::Result<Vec<String>> {
        Ok(self.entries.clone())
    }
    fn clear(&mut self) -> std::io::Result<()> {
        self.entries.clear();
        Ok(())
    }
    fn append(&mut self, command: String) -> std::io::Result<()> {
        self.entries.push(command);
        Ok(())
    }
    fn replace(&mut self, entries: Vec<String>) -> std::io::Result<()> {
        self.entries = entries;
        Ok(())
    }
    fn write_history(&mut self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn read_history(&mut self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn append_history(&mut self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn read_new_history(&mut self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }
}

/// bashhist.c:562 pre_process_line on one interactive line with a host
/// provider: `!!` resolves to the previous entry (the pre-added raw line is
/// hidden, bashhist.c:576-580 history_length--) and the recorded entry
/// becomes the EXPANDED text (maybe_add_history(return_value)).
#[test]
fn interactive_bang_bang_expands_and_records_expanded_line() {
    let provider = Rc::new(RefCell::new(VecProvider {
        entries: vec![
            "echo bang-target".to_string(),
            "!!".to_string(), // reedline added the accepted raw line
        ],
    }));
    let mut executor = Executor::new();
    executor.set_history_provider(provider.clone());

    let outcome = rubash::script_driver::pre_process_interactive_line(&mut executor, "!!");
    assert_eq!(
        outcome,
        rubash::script_driver::InteractiveExpansion::Execute("echo bang-target".to_string())
    );
    let entries = provider.borrow().entries.clone();
    assert_eq!(entries.last().map(String::as_str), Some("echo bang-target"));
}

/// `!$` reuses the last argument of the previous command (histexpand.c
/// history_arg_extract with the DOLLAR designator).
#[test]
fn interactive_bang_dollar_reuses_last_argument() {
    let provider = Rc::new(RefCell::new(VecProvider {
        entries: vec![
            "echo first-arg second-arg".to_string(),
            "echo got:!$".to_string(),
        ],
    }));
    let mut executor = Executor::new();
    executor.set_history_provider(provider.clone());

    let outcome = rubash::script_driver::pre_process_interactive_line(&mut executor, "echo got:!$");
    assert_eq!(
        outcome,
        rubash::script_driver::InteractiveExpansion::Execute("echo got:second-arg".to_string())
    );
}

/// A failed expansion (event not found) discards the line and un-records
/// the raw entry: GNU never runs maybe_add_history on that path.
#[test]
fn interactive_failed_expansion_discards_and_unrecords() {
    let provider = Rc::new(RefCell::new(VecProvider {
        entries: vec!["echo one".to_string(), "!9".to_string()],
    }));
    let mut executor = Executor::new();
    executor.set_history_provider(provider.clone());

    let outcome = rubash::script_driver::pre_process_interactive_line(&mut executor, "!9");
    assert_eq!(
        outcome,
        rubash::script_driver::InteractiveExpansion::Discard
    );
    let entries = provider.borrow().entries.clone();
    assert_eq!(entries.last().map(String::as_str), Some("echo one"));
}

/// `set +H` (histexpand off) passes the line through untouched.
#[test]
fn interactive_set_plus_h_disables_expansion() {
    let provider = Rc::new(RefCell::new(VecProvider {
        entries: vec![
            "echo flip-target".to_string(),
            "echo no-expand-!!-mark".to_string(),
        ],
    }));
    let mut executor = Executor::new();
    executor.set_history_provider(provider.clone());
    executor.set_env("__RUBASH_SETOPT_histexpand", "0");

    let line = "echo no-expand-!!-mark";
    let outcome = rubash::script_driver::pre_process_interactive_line(&mut executor, line);
    assert_eq!(
        outcome,
        rubash::script_driver::InteractiveExpansion::Execute(line.to_string())
    );
    // The raw line stays recorded verbatim when nothing expanded.
    let entries = provider.borrow().entries.clone();
    assert_eq!(
        entries.last().map(String::as_str),
        Some("echo no-expand-!!-mark")
    );
}

/// The print-only `:p` modifier prints the expansion and records it without
/// executing (bashhist.c:598-606: maybe_add_history(history_value)).
#[test]
fn interactive_print_only_modifier_records_without_executing() {
    let provider = Rc::new(RefCell::new(VecProvider {
        entries: vec!["echo p-target".to_string(), "!!:p".to_string()],
    }));
    let mut executor = Executor::new();
    executor.set_history_provider(provider.clone());

    let outcome = rubash::script_driver::pre_process_interactive_line(&mut executor, "!!:p");
    assert_eq!(
        outcome,
        rubash::script_driver::InteractiveExpansion::Discard
    );
    let entries = provider.borrow().entries.clone();
    assert_eq!(entries.last().map(String::as_str), Some("echo p-target"));
}

/// An unset histexpand flag is GNU's interactive HISTEXPAND_DEFAULT
/// (bashhist.c:288): expansion is on without any `set -H`.
/// (Covered implicitly by every test above; this one pins the flag to `1`
/// explicitly to document the enabled spelling.)
#[test]
fn interactive_set_minus_h_explicitly_enables() {
    let provider = Rc::new(RefCell::new(VecProvider {
        entries: vec!["echo again".to_string(), "!!".to_string()],
    }));
    let mut executor = Executor::new();
    executor.set_history_provider(provider.clone());
    executor.set_env("__RUBASH_SETOPT_histexpand", "1");

    let outcome = rubash::script_driver::pre_process_interactive_line(&mut executor, "!!");
    assert_eq!(
        outcome,
        rubash::script_driver::InteractiveExpansion::Execute("echo again".to_string())
    );
}

/// Without a host provider the engine's session history is the list
/// (rubash's own interactive stdin path).
#[test]
fn interactive_session_history_fallback_without_provider() {
    let session = Rc::new(RefCell::new(SessionHistory::new()));
    assert!(session.borrow_mut().record("echo fallback", "", "", None));
    let mut executor = Executor::new();
    executor.set_session_history(Some(session.clone()));

    let outcome = rubash::script_driver::pre_process_interactive_line(&mut executor, "!!");
    assert_eq!(
        outcome,
        rubash::script_driver::InteractiveExpansion::Execute("echo fallback".to_string())
    );
    // The session list is untouched by the interactive path: recording is
    // the reader loop's job there (run_history_group).
    assert_eq!(
        session.borrow().entries.last().map(String::as_str),
        Some("echo fallback")
    );
}
