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
