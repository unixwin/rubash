//! The 1.5.0 Session API's external consumer test: this file
//! is an integration test, so it sees only the crate's public
//! surface — and it deliberately uses **only** `rubash::api`
//! (plus std), the way an embedding host would. It drives a
//! session end-to-end on the published contract alone:
//! build → run → state views → completion → parse → startup
//! sourcing → close.
//!
//! If a face a host needs is missing from `rubash::api`, this
//! file stops compiling — that is the point of putting it
//! outside the crate.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use rubash::api::{
    Candidate, CompletionProvider, Diagnostic, DiagnosticKind, DiagnosticSink,
    ExternalCommandHandler, Flow, HistoryProvider, HostExternalCommandOutput, PreparedLine,
    RcPolicy, RunOptions, Session, SessionEvents, SharedHistoryProvider, StartupClass,
    StartupContext,
};

fn temp_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("rubash-session-api-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn read(dir: &PathBuf, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).expect("output file written by the script")
}

fn opts() -> RunOptions {
    RunOptions::default()
}

// ---------------------------------------------------------------------------
// Q5: the threading contract is in the types.
// ---------------------------------------------------------------------------

mod send_sync_check {
    pub trait AmbiguousIfSend<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfSend<()> for T {}
    impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}

    pub trait AmbiguousIfSync<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfSync<()> for T {}
    impl<T: ?Sized + Sync> AmbiguousIfSync<u16> for T {}
}

#[test]
fn session_is_neither_send_nor_sync_by_construction() {
    <Session as send_sync_check::AmbiguousIfSend<_>>::check();
    <Session as send_sync_check::AmbiguousIfSync<_>>::check();
}

// ---------------------------------------------------------------------------
// Build + run + state views: the census operations, first-class.
// ---------------------------------------------------------------------------

#[test]
fn build_run_and_state_views_round_trip() {
    let dir = temp_dir("basic");
    let dir_text = dir.to_string_lossy().into_owned();
    let mut session = Session::builder()
        .env("OUT", dir_text.clone())
        .env("GREETING", "hello world")
        .working_directory(dir.clone())
        .argv0("consumer-test")
        .build()
        .expect("session builds");

    // Builder seeding is live shell state, and the shell's own
    // writes are visible through the views (one machinery).
    assert_eq!(
        session.variables().resolved_value("GREETING").as_deref(),
        Some("hello world")
    );
    let outcome = session
        .run(
            "printf '%s' \"$GREETING\" > \"$OUT/greeting.txt\"; FROM_SHELL=seen",
            &opts(),
        )
        .expect("run succeeds");
    assert_eq!(outcome.status, 0);
    assert_eq!(outcome.flow, Flow::Completed);
    assert_eq!(read(&dir, "greeting.txt"), "hello world");
    assert_eq!(
        session.variables().resolved_value("FROM_SHELL").as_deref(),
        Some("seen")
    );

    // Working directory: the builder entered it, `pwd` agrees,
    // and a view move goes through the cd machinery.
    assert_eq!(session.cwd(), dir.as_path());
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).expect("subdir");
    assert!(session.set_cwd(&sub));
    assert_eq!(
        session.cwd(),
        std::fs::canonicalize(&sub)
            .expect("canonical sub")
            .as_path()
    );
    let outcome = session.run("pwd > \"$OUT/pwd.txt\"", &opts()).expect("run");
    assert_eq!(outcome.status, 0);
    assert!(read(&dir, "pwd.txt").contains("sub"));

    // Options + last status + positional parameters.
    assert!(session.options().set("extglob", true));
    assert_eq!(session.options().get("extglob"), Some(true));
    session.set_last_status(7);
    assert_eq!(session.last_status(), 7);
    session.set_positional(vec!["one".to_string(), "two".to_string()]);
    assert_eq!(session.positional(), vec!["one", "two"]);
    let outcome = session
        .run("false", &opts())
        .expect("a failing command is an outcome, not an error");
    assert_eq!(outcome.status, 1);
    assert_eq!(session.last_status(), 1);

    // Aliases, functions, and the environment view.
    session.run("alias ll='ls -l'", &opts()).expect("alias run");
    assert!(session
        .aliases()
        .contains(&("ll".to_string(), "ls -l".to_string())));
    session
        .run("greet() { return 7; }", &opts())
        .expect("function definition run");
    assert!(session.functions().contains(&"greet".to_string()));
    assert_eq!(session.call_function("greet", &[] as &[&str]), Ok(7));
    assert!(session
        .call_function("no_such_function_xyz", &[] as &[&str])
        .is_err());
    session.variables().set_exported("GREETING", true);
    assert!(session
        .environment()
        .contains(&("GREETING".to_string(), "hello world".to_string())));

    // Word expansion against live state (the read-only entry).
    assert_eq!(session.expand_word("$GREETING"), "hello world");
}

#[test]
fn build_refuses_an_unenterable_working_directory() {
    let result = Session::builder()
        .working_directory("/nonexistent-session-api-dir-xyz")
        .build();
    assert!(result.is_err(), "a bad cwd fails the build, per §5.1");
}

// ---------------------------------------------------------------------------
// Flow as a value, and the event observer.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct EventLog {
    before: usize,
    after: Vec<i32>,
    cwd_changes: Vec<PathBuf>,
    exported_changes: Vec<(String, Option<String>)>,
    prompts: Vec<String>,
    exit_requests: Vec<i32>,
}

struct Observer {
    log: Rc<RefCell<EventLog>>,
}

impl SessionEvents for Observer {
    fn before_command(&mut self, _event: &rubash::api::CommandEvent) {
        self.log.borrow_mut().before += 1;
    }
    fn after_command(
        &mut self,
        _event: &rubash::api::CommandEvent,
        status: i32,
        _elapsed: std::time::Duration,
    ) {
        self.log.borrow_mut().after.push(status);
    }
    fn cwd_changed(&mut self, new: &std::path::Path) {
        self.log.borrow_mut().cwd_changes.push(new.to_path_buf());
    }
    fn exported_variable_changed(&mut self, name: &str, value: Option<&str>) {
        self.log
            .borrow_mut()
            .exported_changes
            .push((name.to_string(), value.map(str::to_string)));
    }
    fn prompt_ready(&mut self, prompt: &str) {
        self.log.borrow_mut().prompts.push(prompt.to_string());
    }
    fn exit_requested(&mut self, status: i32) {
        self.log.borrow_mut().exit_requests.push(status);
    }
}

#[test]
fn exit_is_a_value_and_events_fire_at_run_boundaries() {
    let dir = temp_dir("events");
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).expect("subdir");
    let log = Rc::new(RefCell::new(EventLog::default()));
    let mut session = Session::builder()
        .working_directory(dir.clone())
        .events(Box::new(Observer { log: log.clone() }))
        .build()
        .expect("session builds");

    let outcome = session
        .run(
            "export SESSION_API_NEWVAR=hello; PLAIN_NOT_EXPORTED=1",
            &opts(),
        )
        .expect("run");
    assert_eq!(outcome.flow, Flow::Completed);
    session.run("cd sub", &opts()).expect("cd run");
    let prompt = session.expand_prompt("P:\\w");
    assert!(
        prompt.contains("sub"),
        "prompt expands against the session: {prompt}"
    );

    {
        let log = log.borrow();
        assert_eq!(log.before, 2, "one before per run submission");
        assert_eq!(log.after, vec![0, 0]);
        assert_eq!(log.cwd_changes.len(), 1, "the cd fired cwd_changed");
        assert!(log.cwd_changes[0].ends_with("sub"));
        assert!(
            log.exported_changes
                .contains(&("SESSION_API_NEWVAR".to_string(), Some("hello".to_string()))),
            "the export fired exported_variable_changed: {:?}",
            log.exported_changes
        );
        assert!(
            !log.exported_changes
                .iter()
                .any(|(name, _)| name == "PLAIN_NOT_EXPORTED"),
            "an unexported write is not an environment event"
        );
        assert_eq!(log.prompts, vec![prompt.clone()]);
        assert!(log.exit_requests.is_empty());
    }

    // `exit` is a value the host acts on (§5.2), and the
    // observer hears about it.
    let outcome = session.run("exit 3", &opts()).expect("exit run");
    assert_eq!(outcome.status, 3);
    assert_eq!(outcome.flow, Flow::ExitRequested(3));
    assert_eq!(log.borrow().exit_requests, vec![3]);
}

// ---------------------------------------------------------------------------
// Completion: engine candidates + the host provider's, merged.
// ---------------------------------------------------------------------------

struct HostCommands;

impl CompletionProvider for HostCommands {
    fn candidates(&mut self, _line: &str, _cursor: usize) -> Vec<Candidate> {
        vec![Candidate::described("hostonly", "only the host knows")]
    }
}

#[test]
fn completion_merges_engine_and_provider_candidates() {
    let mut session = Session::builder()
        .completion_provider(Box::new(HostCommands))
        .build()
        .expect("session builds");

    let engine = session.complete("ech", 3);
    assert!(
        engine.iter().any(|candidate| candidate.text == "echo"),
        "the engine completes its own builtin: {engine:?}"
    );

    let merged = session.complete("hos", 3);
    let host = merged
        .iter()
        .find(|candidate| candidate.text == "hostonly")
        .expect("the provider's candidate is merged in");
    assert_eq!(host.description.as_deref(), Some("only the host knows"));

    assert!(!session.has_compspec("no_such_command_xyz"));
    session
        .run("complete -W 'alpha alpine' mycmd", &opts())
        .expect("compspec registration run");
    assert!(session.has_compspec("mycmd"));
    let words = session.complete("mycmd al", 8);
    assert!(
        words.iter().any(|candidate| candidate.text == "alpha"),
        "the compspec's word list completes: {words:?}"
    );
}

// ---------------------------------------------------------------------------
// Parse: the tier-B syntax view, and diagnostics as values.
// ---------------------------------------------------------------------------

struct Sink {
    seen: Rc<RefCell<Vec<Diagnostic>>>,
}

impl DiagnosticSink for Sink {
    fn diagnostic(&mut self, diagnostic: &Diagnostic) {
        self.seen.borrow_mut().push(diagnostic.clone());
    }
}

#[test]
fn parse_returns_owned_syntax_views_and_value_diagnostics() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut session = Session::builder()
        .diagnostic_sink(Box::new(Sink { seen: seen.clone() }))
        .build()
        .expect("session builds");

    let unit = session
        .parse("f() {\n  echo hi\n}\n", Some("probe.sh"))
        .expect("a clean unit parses");
    assert_eq!(unit.classify_line(1), rubash::api::LineClass::Stoppable);
    assert_eq!(unit.classify_line(2), rubash::api::LineClass::Stoppable);
    let site = unit
        .function_site("f")
        .expect("the definition is in the view");
    assert_eq!(site.definition_line, 1);
    assert!(!unit.needs_more_input());
    assert!(session.needs_more_input("echo 'unclosed"));
    assert!(!session.needs_more_input("echo closed"));

    let failure = session
        .parse("if true; then echo x", Some("bad.sh"))
        .expect_err("an unclosed compound is a parse failure");
    let diagnostic = failure.diagnostic();
    assert_eq!(diagnostic.kind, DiagnosticKind::Parse);
    assert!(
        diagnostic.message.contains("unexpected end of file"),
        "the value carries the GNU-worded message: {}",
        diagnostic.message
    );
    assert_eq!(diagnostic.source_name.as_deref(), Some("bad.sh"));
    assert_eq!(seen.borrow().len(), 1, "the sink received the same value");
    assert_eq!(seen.borrow()[0], *diagnostic);
}

// ---------------------------------------------------------------------------
// The history capability: builder injection reaches the builtins, and
// history expansion in prepare_line resolves against the provider's list.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct ListProvider {
    entries: Vec<String>,
}

impl HistoryProvider for ListProvider {
    fn entries(&mut self) -> io::Result<Vec<String>> {
        Ok(self.entries.clone())
    }
    fn clear(&mut self) -> io::Result<()> {
        self.entries.clear();
        Ok(())
    }
    fn append(&mut self, command: String) -> io::Result<()> {
        self.entries.push(command);
        Ok(())
    }
    fn replace(&mut self, entries: Vec<String>) -> io::Result<()> {
        self.entries = entries;
        Ok(())
    }
    fn write_history(&mut self, path: &str) -> io::Result<()> {
        std::fs::write(path, self.entries.join("\n"))
    }
    fn read_history(&mut self, path: &str) -> io::Result<()> {
        let text = std::fs::read_to_string(path)?;
        self.entries = text.lines().map(str::to_string).collect();
        Ok(())
    }
    fn append_history(&mut self, path: &str) -> io::Result<()> {
        let mut text = std::fs::read_to_string(path).unwrap_or_default();
        for entry in &self.entries {
            text.push_str(entry);
            text.push('\n');
        }
        std::fs::write(path, text)
    }
    fn read_new_history(&mut self, path: &str) -> io::Result<()> {
        let text = std::fs::read_to_string(path)?;
        for line in text.lines() {
            if !self.entries.iter().any(|entry| entry == line) {
                self.entries.push(line.to_string());
            }
        }
        Ok(())
    }
}

#[test]
fn history_provider_is_the_sessions_history_list() {
    let dir = temp_dir("history");
    let dir_text = dir.to_string_lossy().into_owned();
    let provider: SharedHistoryProvider = Rc::new(RefCell::new(ListProvider {
        entries: vec!["echo alpha".to_string(), "echo beta".to_string()],
    }));
    let mut session = Session::builder()
        .env("OUT", dir_text)
        .history_provider(provider)
        .build()
        .expect("session builds");

    // `!!` resolves against the provider's list (the newest entry).
    session.run("set -H", &opts()).expect("histexpand on");
    assert_eq!(
        session.prepare_line("!!"),
        PreparedLine::Execute("echo beta".to_string())
    );

    // A script that uses `history` is routed to the history
    // driver, and the builtin reads the provider's list.
    let outcome = session
        .run("history > \"$OUT/hist.txt\"", &opts())
        .expect("history run");
    assert_eq!(outcome.status, 0);
    let text = read(&dir, "hist.txt");
    assert!(
        text.contains("echo alpha"),
        "provider entry listed: {text:?}"
    );
    assert!(
        text.contains("echo beta"),
        "provider entry listed: {text:?}"
    );

    // The session-history view is the distinct list face.
    assert!(session.history_entries().is_none());
    session.set_history_entries(vec!["one".to_string(), "two".to_string()]);
    assert_eq!(
        session.history_entries(),
        Some(vec!["one".to_string(), "two".to_string()])
    );
}

// ---------------------------------------------------------------------------
// The external-command capability: the handler's decision is the result.
// ---------------------------------------------------------------------------

struct Intercept;

impl ExternalCommandHandler for Intercept {
    fn handle_command(
        &mut self,
        argv: &[String],
        _env: &HashMap<String, String>,
    ) -> Option<HostExternalCommandOutput> {
        if argv.first().map(String::as_str) == Some("cat") {
            Some(HostExternalCommandOutput {
                stdout: b"from the host\n".to_vec(),
                stderr: Vec::new(),
                status: 42,
            })
        } else {
            None
        }
    }
}

#[test]
fn external_command_handler_decides_for_its_commands() {
    let dir = temp_dir("extcmd");
    let dir_text = dir.to_string_lossy().into_owned();
    let mut session = Session::builder()
        .env("OUT", dir_text)
        .external_command_handler(Box::new(Intercept))
        .build()
        .expect("session builds");
    let outcome = session
        .run(
            "cat /dev/null > \"$OUT/host.txt\"; printf '%s' \"$?\" > \"$OUT/host.rc\"",
            &opts(),
        )
        .expect("host command run");
    assert_eq!(outcome.status, 0);
    assert_eq!(read(&dir, "host.txt"), "from the host\n");
    assert_eq!(read(&dir, "host.rc"), "42");
}

// ---------------------------------------------------------------------------
// Startup sourcing: the RcPolicy face, custom and default.
// ---------------------------------------------------------------------------

struct OneFile {
    file: PathBuf,
    classes: Rc<RefCell<Vec<StartupClass>>>,
}

impl RcPolicy for OneFile {
    fn startup_files(&mut self, class: StartupClass, _context: &StartupContext) -> Vec<PathBuf> {
        self.classes.borrow_mut().push(class);
        vec![self.file.clone()]
    }
}

#[test]
fn rc_policy_decides_and_default_lists_source() {
    // Custom policy: its file is sourced with startup semantics.
    let dir = temp_dir("rcpolicy");
    let rc = dir.join("myrc");
    std::fs::write(&rc, "RC_MARKER=from-custom-policy\n").expect("rc file");
    let classes = Rc::new(RefCell::new(Vec::new()));
    let mut session = Session::builder()
        .rc_policy(Box::new(OneFile {
            file: rc,
            classes: classes.clone(),
        }))
        .build()
        .expect("session builds");
    let outcome = session
        .source_startup_files(StartupClass::InteractiveRc)
        .expect("sourcing succeeds");
    assert_eq!(outcome.flow, Flow::Completed);
    assert_eq!(
        session.variables().resolved_value("RC_MARKER").as_deref(),
        Some("from-custom-policy")
    );
    assert_eq!(classes.borrow().as_slice(), [StartupClass::InteractiveRc]);

    // Default policy: the engine's own invocation lists — a
    // $HOME/.bashrc in a seeded HOME is the personal rc.
    let home = temp_dir("default-rc");
    std::fs::write(home.join(".bashrc"), "DEFAULT_RC_MARKER=from-bashrc\n").expect("bashrc");
    let mut session = Session::builder()
        .env("HOME", home.to_string_lossy().into_owned())
        .build()
        .expect("session builds");
    let outcome = session
        .source_startup_files(StartupClass::InteractiveRc)
        .expect("default sourcing succeeds");
    assert_eq!(outcome.flow, Flow::Completed);
    assert_eq!(
        session
            .variables()
            .resolved_value("DEFAULT_RC_MARKER")
            .as_deref(),
        Some("from-bashrc")
    );
}

// ---------------------------------------------------------------------------
// Interactive line running: prepare + run in one call.
// ---------------------------------------------------------------------------

#[test]
fn run_line_records_and_runs() {
    let mut session = Session::builder().build().expect("session builds");
    let outcome = session.run_line("LINE_VAR=via-run-line").expect("run_line");
    assert_eq!(outcome.status, 0);
    assert_eq!(
        session.variables().resolved_value("LINE_VAR").as_deref(),
        Some("via-run-line")
    );
}

// ---------------------------------------------------------------------------
// Close: the finish machinery's duties are the contract's (§5.1).
// ---------------------------------------------------------------------------

#[test]
fn close_runs_exit_trap_and_persistent_procsub_delivery() {
    let dir = temp_dir("close");
    let dir_text = dir.to_string_lossy().into_owned();
    let mut session = Session::builder()
        .env("OUT", dir_text)
        .build()
        .expect("session builds");
    session
        .run("trap 'echo trapped > \"$OUT/trap.txt\"' EXIT", &opts())
        .expect("trap run");
    // The consumption audit's data-loss shape: a persistent
    // output process substitution whose consumer only runs
    // when the finish machinery closes it at close().
    session
        .run(
            "exec > >(cat > \"$OUT/proc.txt\"); echo hello-procsub",
            &opts(),
        )
        .expect("procsub run");
    let status = session.close();
    assert_eq!(status, 0);
    assert_eq!(read(&dir, "trap.txt"), "trapped\n");
    assert_eq!(read(&dir, "proc.txt"), "hello-procsub\n");
}

// ---------------------------------------------------------------------------
// niubash#627: the fd-table enumeration face host completion reads.
// ---------------------------------------------------------------------------

#[test]
fn open_fds_reflect_the_session_fd_table() {
    let mut session = Session::builder().build().expect("session builds");
    // A fresh session holds exactly the standard trio.
    assert_eq!(session.executor().open_fds(), vec![0, 1, 2]);
    // `exec 3>` is a persistent redirection: the fd joins the table
    // (and therefore the /dev/fd namespace completion enumerates).
    let outcome = session
        .run("exec 3>/dev/null", &opts())
        .expect("exec run succeeds");
    assert_eq!(outcome.status, 0);
    assert_eq!(session.executor().open_fds(), vec![0, 1, 2, 3]);
    // Closing it removes it again — the dynamic tier tracks the
    // table, not a name list.
    let outcome = session
        .run("exec 3>&-", &opts())
        .expect("close run succeeds");
    assert_eq!(outcome.status, 0);
    assert_eq!(session.executor().open_fds(), vec![0, 1, 2]);
}
