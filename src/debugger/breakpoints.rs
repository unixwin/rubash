//! The breakpoint table (debugger design section 5).
//!
//! Four entry kinds share one table. Every entry carries the field
//! set the design fixes for the whole model: id, kind, target,
//! enabled flag, ignore count, hit count, optional condition,
//! optional command list, one-shot flag. Locations are keyed by the
//! file identity the engine itself reports, so a breakpoint set
//! against one file hits wherever that file's lines run.
//!
//! The design's conditional breakpoints (section 5.3) are not a
//! fifth kind: a condition is an orthogonal field any entry may
//! carry, and line and function entries compose with conditions and
//! ignore counts identically. Watchpoints (section 5.4) are two of
//! the four kinds — a tracked *name* and an arbitrary *expression* —
//! because their re-evaluation rules differ; both are stop-and-diff,
//! re-checked at stops rather than intercepted at writes, and their
//! previous-stop snapshot lives on the entry.
//!
//! This module is the data model only. Resolution against the parsed
//! tree (which lines can stop at all), condition evaluation, and the
//! stop decision itself belong to later work packages; the table
//! records resolution state and hit bookkeeping so those layers have
//! exactly one ledger to consult.

use std::collections::BTreeMap;
use std::fmt;

/// Session-stable breakpoint identifier.
///
/// Ids are assigned in creation order starting at 1 and are never
/// reused within a session, even after the entry is deleted: a
/// transcript that mentions an id always means one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BreakpointId(u32);

impl BreakpointId {
    /// The numeric id, as rendered in breakpoint listings.
    pub fn number(self) -> u32 {
        self.0
    }
}

impl fmt::Display for BreakpointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The four entry kinds of the one table (design section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointKind {
    /// Stop before the command at a (file, line) executes (5.1).
    Line,
    /// Stop at a function's entry, before its body (5.2).
    Function,
    /// Stop-and-diff on a tracked variable name (5.4, `watch`).
    Watch,
    /// Stop-and-diff on an arbitrary strict-subset expression
    /// (5.4, `watche`).
    WatchExpression,
}

impl BreakpointKind {
    /// Watch entries never fire at a seam; they are re-evaluated at
    /// stops. Line and function entries are seam-fired.
    pub fn is_watch(self) -> bool {
        matches!(
            self,
            BreakpointKind::Watch | BreakpointKind::WatchExpression
        )
    }
}

/// Where an entry fires, in the engine's own identity terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreakpointTarget {
    /// A line in a file, keyed by the file identity the engine
    /// reports for the running code.
    Line {
        /// File identity as reported by the engine.
        file: String,
        /// 1-based line number.
        line: u32,
    },
    /// A function name; may name a function not yet defined.
    Function {
        /// The function's name.
        name: String,
    },
    /// The watched text: a variable name for
    /// [`BreakpointKind::Watch`], an expression for
    /// [`BreakpointKind::WatchExpression`].
    Expression {
        /// The name or expression text as entered.
        text: String,
    },
}

/// Resolution state of an entry (design 5.1/5.2; event DE-11).
///
/// A line with no stoppable command is answered not-verified with
/// its reason, never silently; a function breakpoint on an undefined
/// name waits as pending and resolves when the definition appears.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreakpointResolution {
    /// The entry is bound to a real stop point.
    Verified,
    /// The entry cannot fire, with the reason recorded.
    Unverified {
        /// Why the entry cannot fire.
        reason: String,
    },
    /// Awaiting a definition that has not appeared yet.
    Pending,
}

/// The previous-stop snapshot a watch entry diffs against (5.4).
///
/// Tracked names may carry a freshness epoch from the engine's
/// variable table; when both snapshots carry epochs, the epoch
/// comparison decides and the rendered strings are not consulted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchSnapshot {
    /// The rendered value at the previous stop, if any.
    pub rendered: Option<String>,
    /// The freshness epoch at the previous stop, if tracked.
    pub epoch: Option<u64>,
}

/// One breakpoint table entry: the design section 5 field set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Breakpoint {
    /// Session-stable id.
    pub id: BreakpointId,
    /// Which of the four kinds this entry is.
    pub kind: BreakpointKind,
    /// Where the entry fires.
    pub target: BreakpointTarget,
    /// Disabled entries are skipped by the consult entirely.
    pub enabled: bool,
    /// One-shot entries are deleted at their first committed stop.
    pub one_shot: bool,
    /// Hits still to pass over silently (design 5.5 `ignore`).
    /// Consulted before any condition.
    pub ignore_remaining: u32,
    /// Total hits counted, including ignored ones.
    pub hit_count: u64,
    /// Optional condition text (design 5.3): at a would-be hit the
    /// condition is evaluated in the stopped frame and the stop is
    /// committed only when it succeeds.
    pub condition: Option<String>,
    /// Optional command list (design 5.5), run at each committed
    /// hit.
    pub commands: Vec<String>,
    /// Resolution state of the entry.
    pub resolution: BreakpointResolution,
    /// Watch bookkeeping; `None` for non-watch entries and for watch
    /// entries that have not been evaluated at a stop yet.
    pub watch_snapshot: Option<WatchSnapshot>,
}

impl Breakpoint {
    fn new(id: BreakpointId, kind: BreakpointKind, target: BreakpointTarget) -> Self {
        Self {
            id,
            kind,
            target,
            enabled: true,
            one_shot: false,
            ignore_remaining: 0,
            hit_count: 0,
            condition: None,
            commands: Vec::new(),
            resolution: BreakpointResolution::Verified,
            watch_snapshot: None,
        }
    }

    /// A line entry (5.1). Resolution defaults to verified; the
    /// resolution layer may still mark it unverified with a reason
    /// before it is ever consulted.
    pub fn line(id: BreakpointId, file: impl Into<String>, line: u32) -> Self {
        Self::new(
            id,
            BreakpointKind::Line,
            BreakpointTarget::Line {
                file: file.into(),
                line,
            },
        )
    }

    /// A function entry (5.2). Starts pending: whether the name is
    /// already defined is the resolution layer's knowledge, and an
    /// undefined name is a pending entry, never an error.
    pub fn function(id: BreakpointId, name: impl Into<String>) -> Self {
        let mut entry = Self::new(
            id,
            BreakpointKind::Function,
            BreakpointTarget::Function { name: name.into() },
        );
        entry.resolution = BreakpointResolution::Pending;
        entry
    }

    /// A watch entry on a variable name (5.4).
    pub fn watch(id: BreakpointId, name: impl Into<String>) -> Self {
        Self::new(
            id,
            BreakpointKind::Watch,
            BreakpointTarget::Expression { text: name.into() },
        )
    }

    /// A watch entry on an arbitrary expression (5.4).
    pub fn watch_expression(id: BreakpointId, expr: impl Into<String>) -> Self {
        Self::new(
            id,
            BreakpointKind::WatchExpression,
            BreakpointTarget::Expression { text: expr.into() },
        )
    }

    /// Count one hit against this entry and decide its flow.
    ///
    /// A disabled entry counts nothing. A counted hit first spends
    /// ignore credit; a surviving hit with a condition defers to the
    /// consult's evaluation (see [`HitFlow::NeedsCondition`]). The
    /// table layer owns what a decided flow *does* — in particular
    /// deleting a one-shot entry whose stop committed.
    pub fn register_hit(&mut self) -> HitFlow {
        if !self.enabled {
            return HitFlow::Disabled;
        }
        self.hit_count += 1;
        if self.ignore_remaining > 0 {
            self.ignore_remaining -= 1;
            return HitFlow::Ignored;
        }
        if self.condition.is_some() {
            return HitFlow::NeedsCondition;
        }
        HitFlow::Stop
    }

    /// The command list's `silent` rule (5.5): a first line of
    /// `silent` suppresses the stop banner for this entry.
    pub fn commands_are_silent(&self) -> bool {
        self.commands
            .first()
            .is_some_and(|line| line.trim() == "silent")
    }

    /// Stop-and-diff for watch entries (5.4): compare a freshly
    /// rendered value against the previous stop's snapshot and adopt
    /// the new snapshot. Returns the `(old, new)` rendered pair when
    /// the value changed, `None` when it did not — including the
    /// first evaluation, which only establishes the baseline.
    ///
    /// When both snapshots carry freshness epochs, the epoch decides:
    /// an unchanged epoch means unchanged, whatever the rendering
    /// says, and the rendering is not adopted.
    pub fn diff_watch(
        &mut self,
        rendered: Option<String>,
        epoch: Option<u64>,
    ) -> Option<(Option<String>, Option<String>)> {
        let Some(previous) = self.watch_snapshot.take() else {
            self.watch_snapshot = Some(WatchSnapshot { rendered, epoch });
            return None;
        };
        if let (Some(old_epoch), Some(new_epoch)) = (previous.epoch, epoch) {
            if old_epoch == new_epoch {
                self.watch_snapshot = Some(previous);
                return None;
            }
        }
        let changed = previous.rendered != rendered;
        let old = previous.rendered;
        let new = rendered.clone();
        self.watch_snapshot = Some(WatchSnapshot { rendered, epoch });
        if changed {
            Some((old, new))
        } else {
            None
        }
    }
}

/// What one registered hit means, decided by the entry alone.
///
/// Condition evaluation is deliberately not in here: it needs the
/// stopped frame and belongs to the consult. The flow therefore
/// separates *counting* a hit from *committing* its stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitFlow {
    /// The entry is disabled; the hit was not counted.
    Disabled,
    /// The hit was counted and an ignore credit consumed.
    Ignored,
    /// The hit was counted; a condition must be evaluated before
    /// the stop is committed (design 5.3).
    NeedsCondition,
    /// The hit was counted; stop now.
    Stop,
}

/// The table-level outcome of registering a hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitOutcome {
    /// No entry with that id exists.
    NoSuchEntry,
    /// The entry is disabled; nothing was counted.
    Disabled,
    /// The hit was counted and silently passed over.
    Ignored,
    /// The hit was counted; evaluate the entry's condition, then
    /// report the verdict through
    /// [`BreakpointTable::commit_conditional_stop`].
    NeedsCondition {
        /// The entry awaiting a condition verdict.
        id: BreakpointId,
    },
    /// Stop for this entry.
    Stop {
        /// The entry that stopped the session.
        id: BreakpointId,
        /// True when the entry was one-shot and has been deleted.
        consumed: bool,
    },
}

/// The one breakpoint table for a session.
///
/// Entries are keyed by id in a `BTreeMap`, so iteration order is
/// id order — the order breakpoint listings render in.
#[derive(Debug, Default)]
pub struct BreakpointTable {
    entries: BTreeMap<BreakpointId, Breakpoint>,
    next_number: u32,
}

impl BreakpointTable {
    /// An empty table; the first entry gets id 1.
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            next_number: 1,
        }
    }

    fn alloc_id(&mut self) -> BreakpointId {
        let id = BreakpointId(self.next_number);
        self.next_number += 1;
        id
    }

    /// Insert a line entry (5.1) and return its id.
    ///
    /// The stored key is the file's canonical identity
    /// (`super::resolution::file_identity`, PLAN row 11), never
    /// the caller's spelling: entries set through a symlink, a
    /// relative path, or a short name are the same entry.
    pub fn insert_line(&mut self, file: impl Into<String>, line: u32) -> BreakpointId {
        let id = self.alloc_id();
        let file = super::resolution::file_identity(&file.into());
        self.entries.insert(id, Breakpoint::line(id, file, line));
        id
    }

    /// Insert a function entry (5.2) and return its id.
    pub fn insert_function(&mut self, name: impl Into<String>) -> BreakpointId {
        let id = self.alloc_id();
        self.entries.insert(id, Breakpoint::function(id, name));
        id
    }

    /// Insert a watch entry on a variable name (5.4).
    pub fn insert_watch(&mut self, name: impl Into<String>) -> BreakpointId {
        let id = self.alloc_id();
        self.entries.insert(id, Breakpoint::watch(id, name));
        id
    }

    /// Insert a watch entry on an arbitrary expression (5.4).
    pub fn insert_watch_expression(&mut self, expr: impl Into<String>) -> BreakpointId {
        let id = self.alloc_id();
        self.entries
            .insert(id, Breakpoint::watch_expression(id, expr));
        id
    }

    /// Look an entry up by id.
    pub fn get(&self, id: BreakpointId) -> Option<&Breakpoint> {
        self.entries.get(&id)
    }

    /// Look an entry up by id, mutably.
    pub fn get_mut(&mut self, id: BreakpointId) -> Option<&mut Breakpoint> {
        self.entries.get_mut(&id)
    }

    /// Delete an entry (`delete`), returning it if it existed.
    /// Its id is never handed out again.
    pub fn remove(&mut self, id: BreakpointId) -> Option<Breakpoint> {
        self.entries.remove(&id)
    }

    /// Delete every entry. Ids still keep advancing afterwards.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Delete the line entries at one (file, line) — the no-argument
    /// `clear` rule of design 5.5 — and return how many were removed.
    pub fn clear_at(&mut self, file: &str, line: u32) -> usize {
        let file = super::resolution::file_identity(file);
        let doomed: Vec<BreakpointId> = self
            .entries
            .values()
            .filter(|entry| {
                matches!(
                    &entry.target,
                    BreakpointTarget::Line { file: f, line: l } if f == &file && *l == line
                )
            })
            .map(|entry| entry.id)
            .collect();
        let removed = doomed.len();
        for id in doomed {
            self.entries.remove(&id);
        }
        removed
    }

    /// Number of entries currently in the table.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// All entries, in id order (the listing order).
    pub fn iter(&self) -> impl Iterator<Item = &Breakpoint> {
        self.entries.values()
    }

    /// Enabled seam-fired entries bound to one (file, line).
    ///
    /// The query spelling is normalized to the canonical
    /// identity here, at the same convergence point as
    /// registration (`insert_line`), so the engine's reported
    /// spelling and the user's spelling meet on one key.
    pub fn line_entries_at<'a>(
        &'a self,
        file: &'a str,
        line: u32,
    ) -> impl Iterator<Item = &'a Breakpoint> + 'a {
        let identity = super::resolution::file_identity(file);
        self.entries.values().filter(move |entry| {
            entry.enabled
                && matches!(
                    &entry.target,
                    BreakpointTarget::Line { file: f, line: l } if f == &identity && *l == line
                )
        })
    }

    /// Enabled entries bound to one function name.
    pub fn function_entries_named<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a Breakpoint> + 'a {
        self.entries.values().filter(move |entry| {
            entry.enabled
                && matches!(
                    &entry.target,
                    BreakpointTarget::Function { name: n } if n == name
                )
        })
    }

    /// All watch entries, enabled or not: the stop loop re-checks
    /// enabled watches and needs the whole set in one pass.
    pub fn watch_entries(&self) -> impl Iterator<Item = &Breakpoint> {
        self.entries.values().filter(|entry| entry.kind.is_watch())
    }

    /// Whether any entry could stop the debuggee at a seam: an
    /// enabled line or function entry. Watch entries never stop
    /// at a seam (their stop-and-diff is serviced at stops the
    /// other kinds cause), and disabled entries are skipped by
    /// the consult entirely — neither keeps the consult awake.
    /// The funnel's inert fast exit (design 4.4) asks this
    /// before its stop-decision machinery.
    pub fn has_enabled_seam_entries(&self) -> bool {
        self.entries.values().any(|entry| {
            entry.enabled && matches!(entry.kind, BreakpointKind::Line | BreakpointKind::Function)
        })
    }

    /// Enable or disable an entry; false when the id is unknown.
    pub fn set_enabled(&mut self, id: BreakpointId, enabled: bool) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        entry.enabled = enabled;
        true
    }

    /// Set the remaining ignore count (`ignore id n`, 5.5).
    pub fn set_ignore(&mut self, id: BreakpointId, count: u32) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        entry.ignore_remaining = count;
        true
    }

    /// Set or clear the condition text (5.3).
    pub fn set_condition(&mut self, id: BreakpointId, condition: Option<String>) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        entry.condition = condition;
        true
    }

    /// Replace the command list (5.5).
    pub fn set_commands(&mut self, id: BreakpointId, commands: Vec<String>) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        entry.commands = commands;
        true
    }

    /// Record a resolution verdict (5.1/5.2).
    pub fn set_resolution(&mut self, id: BreakpointId, resolution: BreakpointResolution) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        entry.resolution = resolution;
        true
    }

    /// Count a hit against an entry and decide its flow.
    ///
    /// Order of account, per the design: a disabled entry counts
    /// nothing; a counted hit first spends ignore credit (5.5 says
    /// the ignore rule is evaluated before conditions); a surviving
    /// hit with a condition defers to the consult; otherwise it
    /// stops. A one-shot entry that stops is deleted here — at the
    /// hit, before its command list would run (5.1).
    pub fn register_hit(&mut self, id: BreakpointId) -> HitOutcome {
        let Some(entry) = self.entries.get_mut(&id) else {
            return HitOutcome::NoSuchEntry;
        };
        let one_shot = entry.one_shot;
        match entry.register_hit() {
            HitFlow::Disabled => HitOutcome::Disabled,
            HitFlow::Ignored => HitOutcome::Ignored,
            HitFlow::NeedsCondition => HitOutcome::NeedsCondition { id },
            HitFlow::Stop => {
                if one_shot {
                    self.entries.remove(&id);
                }
                HitOutcome::Stop {
                    id,
                    consumed: one_shot,
                }
            }
        }
    }

    /// Commit a stop whose condition the consult evaluated as true.
    ///
    /// A condition that fails is not a committed hit: the entry —
    /// including a one-shot entry — survives a false verdict, and is
    /// deleted only when a stop actually commits. Returns whether
    /// the entry was consumed (one-shot, now deleted).
    pub fn commit_conditional_stop(&mut self, id: BreakpointId) -> bool {
        let consumed = self.entries.get(&id).is_some_and(|entry| entry.one_shot);
        if consumed {
            self.entries.remove(&id);
        }
        consumed
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Breakpoint, BreakpointId, BreakpointKind, BreakpointResolution, BreakpointTable,
        BreakpointTarget, HitOutcome,
    };

    #[test]
    fn has_enabled_seam_entries_counts_only_enabled_line_and_function() {
        let mut table = BreakpointTable::new();
        assert!(!table.has_enabled_seam_entries(), "empty table");
        let watch = table.insert_watch("x");
        assert!(
            !table.has_enabled_seam_entries(),
            "watch is not a seam stop"
        );
        let line = table.insert_line("a.sh", 10);
        assert!(table.has_enabled_seam_entries(), "enabled line entry");
        table.set_enabled(line, false);
        assert!(
            !table.has_enabled_seam_entries(),
            "a disabled line entry is inert"
        );
        table.set_enabled(line, true);
        let _ = table.remove(line);
        let func = table.insert_function("main");
        assert!(table.has_enabled_seam_entries(), "enabled function entry");
        table.set_enabled(func, false);
        assert!(!table.has_enabled_seam_entries());
        let _ = watch;
    }

    #[test]
    fn ids_start_at_one_and_advance_in_creation_order() {
        let mut table = BreakpointTable::new();
        let a = table.insert_line("a.sh", 10);
        let b = table.insert_function("main");
        let c = table.insert_watch("x");
        assert_eq!(a.number(), 1);
        assert_eq!(b.number(), 2);
        assert_eq!(c.number(), 3);
        assert_eq!(table.len(), 3);
    }

    #[test]
    fn ids_are_never_reused_after_delete_or_clear() {
        let mut table = BreakpointTable::new();
        let a = table.insert_line("a.sh", 1);
        assert!(table.remove(a).is_some());
        let b = table.insert_line("a.sh", 2);
        assert_eq!(b.number(), 2, "deleted id 1 must not come back");
        table.clear();
        assert!(table.is_empty());
        let c = table.insert_line("a.sh", 3);
        assert_eq!(c.number(), 3, "clear does not reset the counter");
    }

    #[test]
    fn entry_defaults_follow_the_design_field_set() {
        let mut table = BreakpointTable::new();
        let id = table.insert_line("lib.sh", 20);
        let entry = table.get(id).expect("just inserted");
        assert_eq!(entry.kind, BreakpointKind::Line);
        assert_eq!(
            entry.target,
            BreakpointTarget::Line {
                // The stored key is the canonical identity of the
                // spelling, per row 11's keying convergence.
                file: crate::debugger::resolution::file_identity("lib.sh"),
                line: 20
            }
        );
        assert!(entry.enabled);
        assert!(!entry.one_shot);
        assert_eq!(entry.ignore_remaining, 0);
        assert_eq!(entry.hit_count, 0);
        assert_eq!(entry.condition, None);
        assert!(entry.commands.is_empty());
        assert_eq!(entry.resolution, BreakpointResolution::Verified);
        assert_eq!(entry.watch_snapshot, None);
    }

    #[test]
    fn function_entries_start_pending() {
        let mut table = BreakpointTable::new();
        let id = table.insert_function("helper");
        let entry = table.get(id).expect("just inserted");
        assert_eq!(entry.kind, BreakpointKind::Function);
        assert_eq!(entry.resolution, BreakpointResolution::Pending);
        assert!(table.set_resolution(id, BreakpointResolution::Verified));
        assert_eq!(
            table.get(id).expect("entry").resolution,
            BreakpointResolution::Verified
        );
    }

    #[test]
    fn watch_kinds_are_distinct_and_flagged_as_watches() {
        let mut table = BreakpointTable::new();
        let by_name = table.insert_watch("counter");
        let by_expr = table.insert_watch_expression("counter + step");
        assert_eq!(
            table.get(by_name).expect("entry").kind,
            BreakpointKind::Watch
        );
        assert_eq!(
            table.get(by_expr).expect("entry").kind,
            BreakpointKind::WatchExpression
        );
        assert!(BreakpointKind::Watch.is_watch());
        assert!(BreakpointKind::WatchExpression.is_watch());
        assert!(!BreakpointKind::Line.is_watch());
        let watches: Vec<_> = table.watch_entries().map(|entry| entry.id).collect();
        assert_eq!(watches, vec![by_name, by_expr]);
    }

    #[test]
    fn disabled_entries_count_nothing() {
        let mut table = BreakpointTable::new();
        let id = table.insert_line("a.sh", 5);
        assert!(table.set_enabled(id, false));
        assert_eq!(table.register_hit(id), HitOutcome::Disabled);
        assert_eq!(table.get(id).expect("entry").hit_count, 0);
    }

    #[test]
    fn ignore_credits_are_spent_before_stopping_but_hits_still_count() {
        let mut table = BreakpointTable::new();
        let id = table.insert_line("a.sh", 5);
        assert!(table.set_ignore(id, 2));
        assert_eq!(table.register_hit(id), HitOutcome::Ignored);
        assert_eq!(table.register_hit(id), HitOutcome::Ignored);
        assert_eq!(
            table.register_hit(id),
            HitOutcome::Stop {
                id,
                consumed: false
            }
        );
        let entry = table.get(id).expect("entry");
        assert_eq!(entry.hit_count, 3, "ignored hits still increment");
        assert_eq!(entry.ignore_remaining, 0);
    }

    #[test]
    fn ignore_is_evaluated_before_conditions() {
        let mut table = BreakpointTable::new();
        let id = table.insert_line("a.sh", 5);
        assert!(table.set_ignore(id, 1));
        assert!(table.set_condition(id, Some("(( x > 1 ))".to_string())));
        assert_eq!(
            table.register_hit(id),
            HitOutcome::Ignored,
            "the ignore rule runs before the condition (design 5.5)"
        );
        assert_eq!(table.register_hit(id), HitOutcome::NeedsCondition { id });
    }

    #[test]
    fn one_shot_is_deleted_at_its_committed_hit() {
        let mut table = BreakpointTable::new();
        let id = table.insert_line("a.sh", 5);
        table.get_mut(id).expect("entry").one_shot = true;
        assert_eq!(
            table.register_hit(id),
            HitOutcome::Stop { id, consumed: true }
        );
        assert!(table.get(id).is_none(), "one-shot entry deleted at the hit");
        assert_eq!(table.register_hit(id), HitOutcome::NoSuchEntry);
    }

    #[test]
    fn one_shot_survives_a_false_condition_and_dies_on_the_true_one() {
        let mut table = BreakpointTable::new();
        let id = table.insert_line("a.sh", 5);
        table.get_mut(id).expect("entry").one_shot = true;
        assert!(table.set_condition(id, Some("[[ -n $go ]]".to_string())));
        assert_eq!(table.register_hit(id), HitOutcome::NeedsCondition { id });
        // A false verdict commits nothing; the harness simply does
        // not call commit. The entry must still be there.
        assert!(table.get(id).is_some());
        assert_eq!(table.register_hit(id), HitOutcome::NeedsCondition { id });
        assert!(table.commit_conditional_stop(id));
        assert!(table.get(id).is_none());
    }

    #[test]
    fn plain_conditional_commit_reports_not_consumed() {
        let mut table = BreakpointTable::new();
        let id = table.insert_line("a.sh", 5);
        assert!(table.set_condition(id, Some("true".to_string())));
        assert_eq!(table.register_hit(id), HitOutcome::NeedsCondition { id });
        assert!(!table.commit_conditional_stop(id));
        assert!(table.get(id).is_some());
    }

    #[test]
    fn clear_at_removes_only_the_matching_line() {
        let mut table = BreakpointTable::new();
        let a = table.insert_line("a.sh", 7);
        let b = table.insert_line("a.sh", 7);
        let c = table.insert_line("a.sh", 8);
        let d = table.insert_line("b.sh", 7);
        assert_eq!(table.clear_at("a.sh", 7), 2);
        assert!(table.get(a).is_none() && table.get(b).is_none());
        assert!(table.get(c).is_some() && table.get(d).is_some());
        assert_eq!(table.clear_at("a.sh", 7), 0);
    }

    #[test]
    fn location_lookups_skip_disabled_entries() {
        let mut table = BreakpointTable::new();
        let on = table.insert_line("a.sh", 3);
        let off = table.insert_line("a.sh", 3);
        let fun = table.insert_function("worker");
        assert!(table.set_enabled(off, false));
        let at: Vec<_> = table.line_entries_at("a.sh", 3).map(|e| e.id).collect();
        assert_eq!(at, vec![on]);
        let named: Vec<_> = table
            .function_entries_named("worker")
            .map(|e| e.id)
            .collect();
        assert_eq!(named, vec![fun]);
        assert_eq!(table.line_entries_at("a.sh", 4).count(), 0);
        assert_eq!(table.function_entries_named("other").count(), 0);
    }

    #[test]
    fn administration_on_unknown_ids_is_a_quiet_false() {
        let mut table = BreakpointTable::new();
        let id = table.insert_line("a.sh", 1);
        let missing = BreakpointId(999);
        assert!(!table.set_enabled(missing, true));
        assert!(!table.set_ignore(missing, 1));
        assert!(!table.set_condition(missing, None));
        assert!(!table.set_commands(missing, Vec::new()));
        assert!(!table.set_resolution(missing, BreakpointResolution::Verified));
        assert!(table.get(id).is_some());
    }

    #[test]
    fn silent_first_command_line_suppresses_the_banner() {
        let mut table = BreakpointTable::new();
        let id = table.insert_line("a.sh", 1);
        assert!(!table.get(id).expect("entry").commands_are_silent());
        assert!(table.set_commands(id, vec!["silent".to_string(), "print x".to_string()]));
        assert!(table.get(id).expect("entry").commands_are_silent());
        assert!(table.set_commands(id, vec!["print x".to_string()]));
        assert!(!table.get(id).expect("entry").commands_are_silent());
    }

    #[test]
    fn watch_diff_first_evaluation_only_sets_the_baseline() {
        let mut entry = Breakpoint::watch(BreakpointId(1), "counter");
        assert_eq!(entry.diff_watch(Some("1".to_string()), None), None);
        assert_eq!(
            entry.diff_watch(Some("2".to_string()), None),
            Some((Some("1".to_string()), Some("2".to_string())))
        );
        assert_eq!(entry.diff_watch(Some("2".to_string()), None), None);
        assert_eq!(
            entry.diff_watch(None, None),
            Some((Some("2".to_string()), None)),
            "becoming unset is a change too"
        );
    }

    #[test]
    fn watch_diff_epoch_equality_short_circuits_the_rendering() {
        let mut entry = Breakpoint::watch(BreakpointId(1), "counter");
        assert_eq!(entry.diff_watch(Some("same".to_string()), Some(7)), None);
        assert_eq!(
            entry.diff_watch(Some("different text".to_string()), Some(7)),
            None,
            "equal epochs decide: the rendering is not consulted"
        );
        assert_eq!(
            entry.diff_watch(Some("different text".to_string()), Some(8)),
            Some((Some("same".to_string()), Some("different text".to_string()))),
            "a moved epoch falls back to the rendered comparison"
        );
    }

    #[test]
    fn iteration_is_in_id_order() {
        let mut table = BreakpointTable::new();
        let ids: Vec<_> = (0..5).map(|i| table.insert_line("a.sh", i + 1)).collect();
        let seen: Vec<_> = table.iter().map(|entry| entry.id).collect();
        assert_eq!(seen, ids);
    }
}
