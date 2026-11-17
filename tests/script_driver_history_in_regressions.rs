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
    // histsize None = unstifled (variables.c:6066 sv_histsize only stifles on
    // a set HISTSIZE). Passing Some(0) here would be HISTSIZE=0, which
    // stifle_history(0) (readline/history.c:675) turns into an EMPTY list —
    // GNU empties it too, so the seedmark preload must carry no limit.
    assert!(session
        .borrow_mut()
        .record("echo seedmark", "", "", None, true));

    let out = std::env::temp_dir().join(format!("rubash-s1-{}-out.txt", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let out_str = out.to_string_lossy().replace('\\', "/");
    // `!echo` resolves to the newest entry starting with "echo" — the
    // host-preloaded line — while `!!` would hit the script's own
    // `set -o histexpand` record instead.
    let script = format!("set -o history\nset -o histexpand\n!echo >{out_str}\n");

    let mut executor = Executor::new();
    // Hermetic: `set -o history` load_history (bashhist.c) must not read the
    // machine's real HISTFILE, and ambient HISTSIZE must not stifle the list.
    executor.set_env(
        "HISTFILE",
        &std::env::temp_dir()
            .join(format!("rubash-s1-{}-absent-histfile", std::process::id()))
            .to_string_lossy(),
    );
    executor.set_env("HISTSIZE", "500");
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

/// unixwin/niubash#508 (part B): an interactive host whose history list is
/// a HistoryProvider (niu's REPL wires reedline's file-backed list here)
/// must keep that list across a grouped-reader line batch. The driver
/// unconditionally installed a fresh SessionHistory over the provider
/// (script_driver's session swap), so `history`/`fc` — which read the
/// shell's ONE list per builtins/history.def — saw an empty list inside
/// the batch and forever after.
#[test]
fn provider_backed_list_survives_grouped_batch() {
    let provider = Rc::new(RefCell::new(VecProvider {
        entries: vec!["alias 1='cd -1'".to_string(), "HISTORY_MARKER".to_string()],
    }));
    let mut executor = Executor::new();
    executor.set_history_provider(provider.clone());
    executor.set_env("HISTSIZE", "500");

    let out = std::env::temp_dir().join(format!("rubash-508-{}-out.txt", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let out_str = out.to_string_lossy().replace('\\', "/");

    // The `;` group is the shape niubash's interactive reader routes
    // through this driver (a `history 1` line whose `1` matches an alias
    // takes the same route). `history 1` must still list the provider's
    // last entry.
    let code = rubash::script_driver::run_script_with_history_at_line_offset(
        &mut executor,
        &format!("alias 1='cd -1'\nhistory 1 >{out_str}\n"),
        0,
        None,
    );
    assert_eq!(code, 0);

    // The batch must not have installed a scratch list over the provider.
    assert!(
        executor.get_session_history().is_none(),
        "a provider-backed shell must not gain a shadow session history"
    );

    let body = std::fs::read_to_string(&out).expect("history listing output file");
    assert!(
        body.contains("HISTORY_MARKER"),
        "`history 1` inside the batch must list the provider's entry: {body:?}"
    );

    // A second batch still sees the provider's list — the wipe used to be
    // permanent because the installed scratch shadowed it.
    let _ = std::fs::remove_file(&out);
    let code = rubash::script_driver::run_script_with_history_at_line_offset(
        &mut executor,
        &format!("history >{out_str}\n"),
        0,
        None,
    );
    assert_eq!(code, 0);
    let body = std::fs::read_to_string(&out).expect("history listing output file");
    assert!(
        body.contains("HISTORY_MARKER"),
        "`history` after the batch must still list the provider's entries: {body:?}"
    );
    let _ = std::fs::remove_file(&out);
}

/// Without a host provider the fresh scratch still installs as the shell's
/// list (GNU's per-shell history list), so `history` inside the batch reads
/// the same list the group's recording writes to.
#[test]
fn providerless_batch_still_installs_fresh_session() {
    let mut executor = Executor::new();
    let code = rubash::script_driver::run_script_with_history_at_line_offset(
        &mut executor,
        "echo seed\n",
        0,
        None,
    );
    assert_eq!(code, 0);
    assert!(executor.get_session_history().is_some());
}

/// Same #508 root on the PROMPT_COMMAND path: the runner's empty-session
/// arm installed a per-prompt scratch over the host provider, emptying
/// `history`/`fc` for the rest of the session. With a provider installed
/// the scratch is used for expansion/record mechanics only and never
/// becomes the shell's list.
#[test]
fn prompt_command_does_not_shadow_provider_list() {
    let provider = Rc::new(RefCell::new(VecProvider {
        entries: vec!["HISTORY_MARKER".to_string()],
    }));
    let mut executor = Executor::new();
    executor.set_history_provider(provider.clone());
    executor.set_env("PROMPT_COMMAND", "__pc_ran=1");
    assert_eq!(executor.execute_prompt_command(), Some(0));
    assert_eq!(executor.get_env("__pc_ran"), Some("1"));
    assert!(
        executor.get_session_history().is_none(),
        "PROMPT_COMMAND must not leave a scratch session over the provider"
    );
}

/// Without a host provider the PROMPT_COMMAND scratch still installs, so
/// `history` inside the runner sees the shell's live list (bashhist.c:
/// the runner acts on the current shell's history list).
#[test]
fn prompt_command_scratch_installs_without_provider() {
    let mut executor = Executor::new();
    executor.set_env("PROMPT_COMMAND", "__pc_ran=1");
    assert_eq!(executor.execute_prompt_command(), Some(0));
    assert!(executor.get_session_history().is_some());
}

/// Without a host provider the engine's session history is the list
/// (rubash's own interactive stdin path).
#[test]
fn interactive_session_history_fallback_without_provider() {
    let session = Rc::new(RefCell::new(SessionHistory::new()));
    assert!(session
        .borrow_mut()
        .record("echo fallback", "", "", None, true));
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
