//! Breakpoint resolution (debugger design sections 5.1/5.2; WP-8).
//!
//! Resolution answers one question per entry — *can this
//! breakpoint fire?* — from the syntax views of the files the
//! engine has parsed so far ([`crate::api::syntax`]) plus the set
//! of function names the runtime has been seen to define. The
//! verdicts are the design's three states:
//!
//! - **Verified**: the target exists — a line the seam can stop on,
//!   or a function whose definition has appeared.
//! - **Unverified, with reason**: the target provably cannot fire.
//!   The two reason strings are the design's own (section 5.1):
//!   "no command on this line" and "DEBUG trap never fires on this
//!   line". A dead line is reported, never silently accepted.
//! - **Pending**: the question cannot be answered yet — the file
//!   has not been parsed, or the function has not been defined.
//!   Pending is the state of a function breakpoint set before its
//!   definition appears (section 5.2); it resolves the moment the
//!   definition shows up, in a parsed unit's syntax view or at the
//!   function's entry seam.
//!
//! Files parse incrementally, so a line verdict is only as strong
//! as the view behind it: "no command on this line" is pronounced
//! only for a line inside a parsed unit's span, or once the
//! file's reader has reached its end
//! ([`DebugSession::note_syntax_file_complete`]). A line beyond
//! every parsed unit is pending, not dead — its unit may not have
//! been read yet.
//!
//! Timing rules, chosen so a frontend always learns the truth
//! exactly once:
//!
//! - An entry inserted through [`DebugSession::set_line_breakpoint`]
//!   / [`DebugSession::set_function_breakpoint`] is adjudicated
//!   synchronously at insertion, against the views already
//!   registered. No event is emitted for the insertion verdict —
//!   the caller is holding the new entry and reads its state
//!   directly.
//! - Every *later* change — a view registration that proves or
//!   kills a pending entry, a runtime definition note — goes
//!   through [`DebugSession::apply_resolution`], which emits
//!   [`DebugEvent::BreakpointResolved`] (DE-11) exactly when the
//!   state actually changes.
//!
//! Resolution is reporting, not gating: an unverified or pending
//! entry still occupies its table slot, and the consult's hit
//! accounting does not consult resolution state. Views only grow
//! (a file's parsed units accumulate), so verdicts move
//! monotonically toward verified — except that a line first judged
//! "no command" can be re-judged if a later unit of the same file
//! attributes a command to it, which re-adjudication handles by
//! simply recomputing from the grown view.

use super::breakpoints::{BreakpointId, BreakpointResolution, BreakpointTarget};
use super::event::DebugEvent;
use super::session::DebugSession;
use crate::api::syntax::{LineClass, SyntaxView};

/// The one canonical file identity a spelling resolves to
/// (PLAN row 11; the row-10 DAP residual).
///
/// Breakpoint keys and syntax-view keys are identities, not
/// spellings: the same file reached through a symlink, a relative
/// path, or a Windows 8.3 short name must meet the registration
/// made under any other spelling of it. Every registration and
/// every query in the session core converges here — frontends
/// never normalize (and never guess by basename); the DAP
/// adapter's own bridge helper produces this same value for the
/// paths it canonicalizes, so the two sides meet by construction.
///
/// - A spelling that names an existing file canonicalizes
///   through the filesystem (symlinks and short names resolve;
///   the Windows extended-length prefix is stripped).
/// - A spelling whose file does not exist (yet) falls back to a
///   lexical absolutization against the process cwd with `.` /
///   `..` components collapsed, so a pending entry still has one
///   stable key. Caveat, shared with bashdb: a relative spelling
///   is anchored at the cwd in force when it is keyed; the
///   session's own cwd starts at the launch cwd and the process
///   cwd does not follow the debuggee's `cd` (G-02 model).
/// - Pseudo-sources (spellings starting with `<`, e.g.
///   `<debugger>`) are not files and pass through unchanged.
///
/// Idempotent: the identity of an identity is itself.
pub(crate) fn file_identity(spelling: &str) -> String {
    if spelling.starts_with('<') {
        return spelling.to_string();
    }
    if let Ok(path) = std::fs::canonicalize(spelling) {
        return identity_path_string(&path);
    }
    let path = std::path::Path::new(spelling);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(path),
            Err(_) => path.to_path_buf(),
        }
    };
    let mut out = std::path::PathBuf::new();
    for component in absolute.components() {
        use std::path::Component;
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::RootDir => out.push(std::path::MAIN_SEPARATOR.to_string()),
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::Normal(part) => out.push(part),
        }
    }
    identity_path_string(&out)
}

/// An identity string from a path: the native spelling, with the
/// Windows extended-length prefixes the canonicalizer adds
/// mapped back to their ordinary forms (`\\?\C:\…` → `C:\…`,
/// `\\?\UNC\…` → `\\…`).
fn identity_path_string(path: &std::path::Path) -> String {
    let text = path.to_string_lossy().into_owned();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    text
}

/// The design's verdict for a line that carries no command
/// (section 5.1).
pub(crate) const REASON_NO_COMMAND: &str = "no command on this line";
/// The design's verdict for a line whose command the seam never
/// fires at (section 5.1).
pub(crate) const REASON_NEVER_FIRES: &str = "DEBUG trap never fires on this line";

impl DebugSession {
    /// Set a line breakpoint (design 5.1), adjudicated immediately
    /// against the syntax views registered so far: verified for a
    /// stoppable line, unverified with the design's reason for a
    /// dead one, pending when the file has not been parsed yet.
    pub fn set_line_breakpoint(&mut self, file: impl Into<String>, line: u32) -> BreakpointId {
        let file = file_identity(&file.into());
        let id = self.breakpoints_mut().insert_line(&file, line);
        let verdict = self.line_verdict(&file, line);
        self.breakpoints_mut().set_resolution(id, verdict);
        id
    }

    /// Set a function breakpoint (design 5.2). A name already
    /// proven defined — by a registered syntax view or by a runtime
    /// definition note — verifies immediately; any other name is
    /// pending, never an error, and resolves when the definition
    /// appears.
    pub fn set_function_breakpoint(&mut self, name: impl Into<String>) -> BreakpointId {
        let name = name.into();
        let id = self.breakpoints_mut().insert_function(&name);
        if self.function_is_defined(&name) {
            self.breakpoints_mut()
                .set_resolution(id, BreakpointResolution::Verified);
        }
        id
    }

    /// Register the syntax view of one parsed reader unit (built
    /// by the executor at the two file-backed group sites) and
    /// re-adjudicate every entry the grown knowledge touches: all
    /// line entries for the view's file, and all function entries
    /// (any file's view may prove a name). Changes emit DE-11.
    pub fn register_syntax_view(&mut self, mut view: SyntaxView) {
        // Registration keys by canonical identity (row 11): a
        // unit parsed under one spelling serves the entries set
        // under every spelling of the same file.
        view.file = file_identity(&view.file);
        let file = view.file.clone();
        match self.syntax_views.get_mut(&file) {
            Some(existing) => existing.merge(view),
            None => {
                self.syntax_views.insert(file.clone(), view);
            }
        }
        self.readjudicate_lines_for_file(&file);
        let function_ids: Vec<BreakpointId> = self
            .breakpoints()
            .iter()
            .filter(|entry| matches!(entry.target, BreakpointTarget::Function { .. }))
            .map(|entry| entry.id)
            .collect();
        for id in function_ids {
            self.readjudicate_function(id);
        }
    }

    /// Note that a file's reader reached its end (called by the
    /// group drivers when their read loop finishes normally): from
    /// here on, a line no parsed unit covered is soundly "no
    /// command on this line" rather than pending. Line entries for
    /// the file re-adjudicate, with DE-11 on any change.
    pub fn note_syntax_file_complete(&mut self, file: &str) {
        let file = file_identity(file);
        let view = self
            .syntax_views
            .entry(file.clone())
            .or_insert_with(|| SyntaxView {
                file: file.clone(),
                ..SyntaxView::default()
            });
        if view.complete {
            return;
        }
        view.mark_complete();
        self.readjudicate_lines_for_file(&file);
    }

    /// Recompute the verdict of every line entry bound to `file`
    /// and apply the results (DE-11 on change).
    fn readjudicate_lines_for_file(&mut self, file: &str) {
        let line_ids: Vec<BreakpointId> = self
            .breakpoints()
            .iter()
            .filter(|entry| {
                matches!(
                    &entry.target,
                    BreakpointTarget::Line { file: f, .. } if f == file
                )
            })
            .map(|entry| entry.id)
            .collect();
        for id in line_ids {
            if let Some(entry) = self.breakpoints().get(id) {
                if let BreakpointTarget::Line { file, line } = &entry.target {
                    let verdict = self.line_verdict(file, *line);
                    self.apply_resolution(id, verdict);
                }
            }
        }
    }

    /// Note that a function definition has been observed at
    /// runtime (the entry seam consult calls this for the function
    /// being entered: a name with no syntax view — defined by
    /// `eval`, say — is proven defined by its first entry).
    /// Pending entries for the name verify, with DE-11.
    pub fn note_function_defined(&mut self, name: &str) {
        if !self.defined_functions.insert(name.to_string()) {
            return;
        }
        let ids: Vec<BreakpointId> = self
            .breakpoints()
            .iter()
            .filter(|entry| {
                matches!(
                    &entry.target,
                    BreakpointTarget::Function { name: n } if n == name
                )
            })
            .map(|entry| entry.id)
            .collect();
        for id in ids {
            self.readjudicate_function(id);
        }
    }

    /// Whether any registered view or runtime note proves `name`
    /// is defined.
    fn function_is_defined(&self, name: &str) -> bool {
        self.defined_functions.contains(name)
            || self
                .syntax_views
                .values()
                .any(|view| view.function_site(name).is_some())
    }

    /// The verdict for one line of one file, from the views alone.
    fn line_verdict(&self, file: &str, line: u32) -> BreakpointResolution {
        let Some(view) = self.syntax_views.get(file) else {
            return BreakpointResolution::Pending;
        };
        match view.classify_line(line) {
            LineClass::Stoppable => BreakpointResolution::Verified,
            LineClass::CommandOnly => BreakpointResolution::Unverified {
                reason: REASON_NEVER_FIRES.to_string(),
            },
            LineClass::NoCommand => {
                // Sound only where the view's knowledge reaches:
                // inside a parsed unit's span, or anywhere once
                // the file has been read to its end. Beyond that
                // the line's unit may simply not be parsed yet.
                if view.complete || view.covers_line(line) {
                    BreakpointResolution::Unverified {
                        reason: REASON_NO_COMMAND.to_string(),
                    }
                } else {
                    BreakpointResolution::Pending
                }
            }
        }
    }

    /// Recompute one function entry's verdict and apply it.
    fn readjudicate_function(&mut self, id: BreakpointId) {
        let verdict = match self.breakpoints().get(id).map(|entry| &entry.target) {
            Some(BreakpointTarget::Function { name }) => {
                if self.function_is_defined(name) {
                    BreakpointResolution::Verified
                } else {
                    BreakpointResolution::Pending
                }
            }
            _ => return,
        };
        self.apply_resolution(id, verdict);
    }

    /// Install a verdict, emitting DE-11 when — and only when —
    /// the state actually changes.
    fn apply_resolution(&mut self, id: BreakpointId, verdict: BreakpointResolution) {
        let changed = self
            .breakpoints()
            .get(id)
            .is_some_and(|entry| entry.resolution != verdict);
        if changed {
            self.breakpoints_mut().set_resolution(id, verdict.clone());
            self.emit(DebugEvent::BreakpointResolved {
                id,
                resolution: verdict,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{REASON_NEVER_FIRES, REASON_NO_COMMAND};
    use crate::api::syntax::SyntaxView;
    use crate::debugger::breakpoints::{BreakpointResolution, BreakpointTarget};
    use crate::debugger::event::DebugEvent;
    use crate::debugger::session::DebugSession;

    fn view_of(file: &str, source: &str) -> SyntaxView {
        let tokens = crate::lexer::tokenize(source);
        let ast = crate::parser::parse(&tokens);
        SyntaxView::from_ast(file, &ast)
    }

    fn resolution(
        session: &DebugSession,
        id: crate::debugger::BreakpointId,
    ) -> BreakpointResolution {
        session
            .breakpoints()
            .get(id)
            .expect("entry exists")
            .resolution
            .clone()
    }

    #[test]
    fn line_breakpoint_on_an_unparsed_file_is_pending_then_resolves() {
        let mut session = DebugSession::new();
        let id = session.set_line_breakpoint("a.sh", 2);
        assert_eq!(resolution(&session, id), BreakpointResolution::Pending);
        session.drain_events();
        session.register_syntax_view(view_of("a.sh", "echo one\necho two\n"));
        assert_eq!(resolution(&session, id), BreakpointResolution::Verified);
        let events = session.drain_events();
        assert_eq!(
            events,
            vec![DebugEvent::BreakpointResolved {
                id,
                resolution: BreakpointResolution::Verified
            }],
            "the resolution change is reported exactly once (DE-11)"
        );
    }

    #[test]
    fn line_breakpoint_set_after_the_view_verifies_immediately() {
        let mut session = DebugSession::new();
        session.register_syntax_view(view_of("a.sh", "echo one\necho two\n"));
        session.drain_events();
        let id = session.set_line_breakpoint("a.sh", 1);
        assert_eq!(resolution(&session, id), BreakpointResolution::Verified);
        assert!(
            session.drain_events().is_empty(),
            "insertion verdicts are synchronous: no DE-11 is emitted"
        );
    }

    #[test]
    fn dead_lines_carry_the_designs_two_reasons() {
        let mut session = DebugSession::new();
        session.register_syntax_view(view_of(
            "a.sh",
            "# comment\nif\n  test -f x\nthen\n  echo yes\nfi\n",
        ));
        // The comment on line 1 precedes the unit's first command
        // line, so only the completed read convicts it.
        session.note_syntax_file_complete("a.sh");
        let comment = session.set_line_breakpoint("a.sh", 1);
        assert_eq!(
            resolution(&session, comment),
            BreakpointResolution::Unverified {
                reason: REASON_NO_COMMAND.to_string()
            }
        );
        let bare_if = session.set_line_breakpoint("a.sh", 2);
        assert_eq!(
            resolution(&session, bare_if),
            BreakpointResolution::Unverified {
                reason: REASON_NEVER_FIRES.to_string()
            },
            "a command node starts on the bare `if` line but no seam fires at it"
        );
        let then_line = session.set_line_breakpoint("a.sh", 4);
        assert_eq!(
            resolution(&session, then_line),
            BreakpointResolution::Unverified {
                reason: REASON_NO_COMMAND.to_string()
            }
        );
        let body = session.set_line_breakpoint("a.sh", 5);
        assert_eq!(resolution(&session, body), BreakpointResolution::Verified);
    }

    #[test]
    fn a_pending_dead_line_reports_unverified_when_its_file_loads() {
        let mut session = DebugSession::new();
        let id = session.set_line_breakpoint("a.sh", 3);
        assert_eq!(resolution(&session, id), BreakpointResolution::Pending);
        session.drain_events();
        session.register_syntax_view(view_of("a.sh", "echo one\n# note\necho three\n"));
        assert_eq!(
            resolution(&session, id),
            BreakpointResolution::Verified,
            "line 3 carries the second echo in this fixture"
        );
        let id = session.set_line_breakpoint("a.sh", 2);
        assert_eq!(
            resolution(&session, id),
            BreakpointResolution::Unverified {
                reason: REASON_NO_COMMAND.to_string()
            }
        );
    }

    #[test]
    fn function_breakpoint_pends_until_a_view_proves_the_definition() {
        let mut session = DebugSession::new();
        let id = session.set_function_breakpoint("helper");
        assert_eq!(resolution(&session, id), BreakpointResolution::Pending);
        session.drain_events();
        session.register_syntax_view(view_of("lib.sh", "echo top\n"));
        assert_eq!(
            resolution(&session, id),
            BreakpointResolution::Pending,
            "a view of some other file's functions proves nothing"
        );
        assert!(session.drain_events().is_empty(), "no change, no event");
        session.register_syntax_view(view_of("lib.sh", "helper() {\n  echo h\n}\n"));
        assert_eq!(resolution(&session, id), BreakpointResolution::Verified);
        let events = session.drain_events();
        assert_eq!(
            events,
            vec![DebugEvent::BreakpointResolved {
                id,
                resolution: BreakpointResolution::Verified
            }]
        );
    }

    #[test]
    fn function_breakpoint_verifies_immediately_when_already_defined() {
        let mut session = DebugSession::new();
        session.register_syntax_view(view_of("lib.sh", "helper() {\n  echo h\n}\n"));
        let id = session.set_function_breakpoint("helper");
        assert_eq!(resolution(&session, id), BreakpointResolution::Verified);
    }

    #[test]
    fn runtime_definition_note_resolves_a_pending_function_breakpoint() {
        let mut session = DebugSession::new();
        let id = session.set_function_breakpoint("eval_made");
        assert_eq!(resolution(&session, id), BreakpointResolution::Pending);
        session.drain_events();
        session.note_function_defined("eval_made");
        assert_eq!(resolution(&session, id), BreakpointResolution::Verified);
        let events = session.drain_events();
        assert_eq!(events.len(), 1, "exactly one DE-11");
        session.note_function_defined("eval_made");
        assert!(
            session.drain_events().is_empty(),
            "a repeated note changes nothing and reports nothing"
        );
    }

    #[test]
    fn a_line_in_an_unparsed_unit_stays_pending_until_its_unit_registers() {
        let mut session = DebugSession::new();
        session.register_syntax_view(view_of("a.sh", "echo first\n"));
        let id = session.set_line_breakpoint("a.sh", 5);
        assert_eq!(
            resolution(&session, id),
            BreakpointResolution::Pending,
            "line 5 is beyond the only parsed unit: not dead, just unread"
        );
        session.drain_events();
        session.register_syntax_view(view_of("a.sh", "\n\n\n\necho fifth\n"));
        assert_eq!(resolution(&session, id), BreakpointResolution::Verified);
        assert_eq!(session.drain_events().len(), 1, "one DE-11 for the flip");
    }

    #[test]
    fn a_line_past_the_end_is_pending_until_the_read_completes() {
        let mut session = DebugSession::new();
        session.register_syntax_view(view_of("a.sh", "echo only\n"));
        let id = session.set_line_breakpoint("a.sh", 40);
        assert_eq!(resolution(&session, id), BreakpointResolution::Pending);
        session.drain_events();
        session.note_syntax_file_complete("a.sh");
        assert_eq!(
            resolution(&session, id),
            BreakpointResolution::Unverified {
                reason: REASON_NO_COMMAND.to_string()
            },
            "the read is complete: line 40 has no command because the file has no line 40"
        );
        let events = session.drain_events();
        assert_eq!(events.len(), 1, "the completion flip emits DE-11");
        session.note_syntax_file_complete("a.sh");
        assert!(
            session.drain_events().is_empty(),
            "completion is idempotent"
        );
    }

    #[test]
    fn canonical_keying_merges_spellings_of_one_file() {
        // Row 11: registration and query converge on the canonical
        // identity. A view registered under a relative spelling
        // adjudicates an entry set under the absolute spelling,
        // and the stored key is the identity itself.
        let mut session = DebugSession::new();
        session.register_syntax_view(view_of("a.sh", "echo one\necho two\n"));
        let absolute = super::file_identity("a.sh");
        assert_ne!(absolute, "a.sh", "the identity is absolute");
        let id = session.set_line_breakpoint(&absolute, 2);
        assert_eq!(resolution(&session, id), BreakpointResolution::Verified);
        let entry = session.breakpoints().get(id).expect("entry");
        match &entry.target {
            BreakpointTarget::Line { file, .. } => {
                assert_eq!(file, &absolute, "the stored key is the identity")
            }
            other => panic!("line entry expected, got {other:?}"),
        }
        // The table query converges too: the relative spelling
        // finds the entry the absolute spelling set.
        assert_eq!(session.breakpoints().line_entries_at("a.sh", 2).count(), 1);
        assert_eq!(
            session.breakpoints().line_entries_at(&absolute, 2).count(),
            1
        );
    }

    #[test]
    fn canonical_keying_does_not_merge_different_files() {
        // The counter-example: two distinct files in one directory
        // keep distinct keys — identity convergence is not
        // basename matching.
        let dir = std::env::temp_dir().join(format!("rubash-row11-keying-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let one = dir.join("one.sh");
        let two = dir.join("two.sh");
        std::fs::write(&one, "echo one\n").expect("write one");
        std::fs::write(&two, "echo two\n").expect("write two");
        let mut session = DebugSession::new();
        session.register_syntax_view(view_of(one.to_str().unwrap(), "echo one\n"));
        let in_two = session.set_line_breakpoint(two.to_str().unwrap(), 1);
        assert_eq!(
            resolution(&session, in_two),
            BreakpointResolution::Pending,
            "one.sh's view proves nothing about two.sh"
        );
        let in_one = session.set_line_breakpoint(one.to_str().unwrap(), 1);
        assert_eq!(resolution(&session, in_one), BreakpointResolution::Verified);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn canonical_keying_resolves_a_symlink_spelling() {
        // The row-10 residual in its original shape: a breakpoint
        // set on the symlink spelling must verify against the view
        // the engine registered under the real path (and the hit
        // query from either spelling finds it).
        let dir = std::env::temp_dir().join(format!("rubash-row11-symlink-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let real = dir.join("real.sh");
        let link = dir.join("link.sh");
        std::fs::write(&real, "echo one\necho two\n").expect("write real");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        let mut session = DebugSession::new();
        session.register_syntax_view(view_of(real.to_str().unwrap(), "echo one\necho two\n"));
        let id = session.set_line_breakpoint(link.to_str().unwrap(), 2);
        assert_eq!(resolution(&session, id), BreakpointResolution::Verified);
        assert_eq!(
            session
                .breakpoints()
                .line_entries_at(link.to_str().unwrap(), 2)
                .count(),
            1
        );
        assert_eq!(
            session
                .breakpoints()
                .line_entries_at(real.to_str().unwrap(), 2)
                .count(),
            1
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn canonical_keying_passes_pseudo_sources_through() {
        assert_eq!(super::file_identity("<debugger>"), "<debugger>");
        assert_eq!(super::file_identity("<stdin>"), "<stdin>");
        // Idempotence: the identity of an identity is itself.
        let once = super::file_identity("a.sh");
        assert_eq!(super::file_identity(&once), once);
    }

    #[test]
    fn re_registering_a_view_emits_no_duplicate_events() {
        let mut session = DebugSession::new();
        let id = session.set_line_breakpoint("a.sh", 1);
        session.register_syntax_view(view_of("a.sh", "echo one\n"));
        session.drain_events();
        session.register_syntax_view(view_of("a.sh", "echo one\n"));
        assert_eq!(resolution(&session, id), BreakpointResolution::Verified);
        assert!(session.drain_events().is_empty());
    }
}
