//! Structured variable attributes — the GNU `att_*` flag port.
//!
//! GNU reference: `variables.h:124-133` stores every declare-family
//! attribute as an `att_*` bit on the `SHELL_VAR` struct itself
//! (`att_exported`, `att_readonly`, `att_integer`, `att_uppercase`,
//! `att_lowercase`, `att_nameref`, `att_array`, `att_associative`,
//! `att_trace`, ...). Reading, setting, or clearing one is a bit test on
//! the variable's cell — there is no list to scan.
//!
//! Rubash historically encoded the same facts as membership in
//! `__RUBASH_*_VARS` marker strings inside the flat env map (a
//! DATA_DOLLAR-joined name list per attribute). Every query was a
//! split+scan and every write a collect+join+insert; `local a=$1 b=$2
//! c=$3` paid 77 list scans and 60 list rewrites per call (perf11:
//! 1.54M `is_marked_var` + 1.2M `set_marked_var` per 20000-iteration
//! run). `VarTable` ports the GNU model: a `HashMap<String, VarAttrs>`
//! beside the value map, one lookup per query, bit compares per write.
//!
//! ## The two forms and who owns them
//!
//! - `attrs` (the `VarAttrs` map) is AUTHORITATIVE for the ten
//!   declare-family keys listed in `attr_flag`. All reads of those keys
//!   go through `VarTable` methods.
//! - The marker strings in `values` remain as the SERIALIZED form and
//!   stay synchronized on every real flip, because they are still load
//!   bearing at the boundaries:
//!   - **Process boundary**: `Executor::new` imports the whole process
//!     environment verbatim, so a child rubash started by a parent
//!     rubash receives the attribute lists as env entries
//!     (`from_values` parses them back into `attrs` at construction).
//!   - **List enumeration** (child-env export lists, `declare -p`
//!     renderers) reads the strings, which preserve insertion order.
//! - Keys outside the ten (`__RUBASH_*_FUNCTIONS`,
//!   `__RUBASH_ASSOC_128_VARS`, `__RUBASH_CAPCASE_VARS`,
//!   `__RUBASH_UNSET_DYNAMIC_VARS`, ...) are list-shaped facts, not
//!   SHELL_VAR attributes; they live in the strings only and the
//!   dispatch helpers fall through to the string path for them.
//!
//! Direct writes of the ten marker strings through the `DerefMut` value
//! view are FORBIDDEN — they would desynchronize the two forms. All
//! attribute mutation goes through `mark_name` / `unmark_name` /
//! `set_attrs` / `replace_attr_list`.

use std::collections::{HashMap, HashSet};
use std::ops::Deref;

use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};
use crate::executor::{
    ARRAY_VARS, ASSOC_VARS, DECLARED_UNSET_VARS, EXPORTED_VARS, INTEGER_VARS, LOWERCASE_VARS,
    NAMEREF_VARS, READONLY_VARS, TRACE_VARS, UPPERCASE_VARS,
};

/// The ten declare-family SHELL_VAR attributes (variables.h:124-133
/// att_* port). Captured and restored as one unit by function frames
/// (`local_attr_scopes`), tempenv snapshots, and `declare -g` save/restore.
///
/// The listing underlay for the innermost tempenv frame (rubash#1569):
/// for each name bound by the frame, `None` when no binding existed
/// before the frame (listings must hide the name) or `Some((env text,
/// attrs))` for the pre-frame binding a listing must render instead of
/// the live tempvar cell. See Executor::tempenv_listing_underlay.
pub type TempenvListingUnderlay =
    std::collections::HashMap<String, Option<(Option<String>, VarAttrs)>>;

/// `pub` because builtin dispatch entries (`set::set`, `declare::execute`,
/// ...) take `&mut VarTable`; every method stays crate-visible.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VarAttrs {
    pub(crate) exported: bool,
    pub(crate) readonly: bool,
    pub(crate) integer: bool,
    pub(crate) uppercase: bool,
    pub(crate) lowercase: bool,
    pub(crate) nameref: bool,
    pub(crate) array: bool,
    pub(crate) assoc: bool,
    pub(crate) trace: bool,
    /// variables.c `unset` with att_invisible semantics: `local x` with
    /// no value leaves the name bound-but-valueless until the frame pops.
    pub(crate) declared_unset: bool,
}

impl VarAttrs {
    pub(crate) fn is_empty(&self) -> bool {
        *self == VarAttrs::default()
    }

    /// Flag state for one of the ten attribute keys, or `None` for a key
    /// that is not a SHELL_VAR attribute (function marks, capcase, ...).
    pub(crate) fn flag(&self, key: &str) -> Option<bool> {
        Some(match key {
            EXPORTED_VARS => self.exported,
            READONLY_VARS => self.readonly,
            INTEGER_VARS => self.integer,
            UPPERCASE_VARS => self.uppercase,
            LOWERCASE_VARS => self.lowercase,
            NAMEREF_VARS => self.nameref,
            ARRAY_VARS => self.array,
            ASSOC_VARS => self.assoc,
            TRACE_VARS => self.trace,
            DECLARED_UNSET_VARS => self.declared_unset,
            _ => return None,
        })
    }

    /// Mutable flag slot for one of the ten attribute keys.
    pub(crate) fn flag_mut(&mut self, key: &str) -> Option<&mut bool> {
        Some(match key {
            EXPORTED_VARS => &mut self.exported,
            READONLY_VARS => &mut self.readonly,
            INTEGER_VARS => &mut self.integer,
            UPPERCASE_VARS => &mut self.uppercase,
            LOWERCASE_VARS => &mut self.lowercase,
            NAMEREF_VARS => &mut self.nameref,
            ARRAY_VARS => &mut self.array,
            ASSOC_VARS => &mut self.assoc,
            TRACE_VARS => &mut self.trace,
            DECLARED_UNSET_VARS => &mut self.declared_unset,
            _ => return None,
        })
    }
}

/// The ten attribute keys, in `VarAttrs` declaration order.
pub(crate) const ATTR_KEYS: [&str; 10] = [
    EXPORTED_VARS,
    READONLY_VARS,
    INTEGER_VARS,
    UPPERCASE_VARS,
    LOWERCASE_VARS,
    NAMEREF_VARS,
    ARRAY_VARS,
    ASSOC_VARS,
    TRACE_VARS,
    DECLARED_UNSET_VARS,
];

/// The value map plus the structured attribute map.
///
/// `Deref` exposes the value map for the hundreds of existing read-only
/// call sites (plain `get`/`contains_key`/iteration). There is deliberately
/// NO `DerefMut`: every mirror write must pass through the inherent
/// choke-point methods below (`insert`/`remove`/`get_mut`/`entry`/`retain`),
/// which call `note_mirror_write` so a write to a tracked name marks its
/// typed cell stale. Attribute operations MUST use the methods below.
#[derive(Debug, Default, Clone)]
pub struct VarTable {
    values: HashMap<String, String>,
    attrs: HashMap<String, VarAttrs>,
    /// Set when a mirror write bypassed the typed-cell sync (declare-family
    /// builtins write through this table but cannot reach the executor's
    /// structured store). Readers consult it before trusting a typed cell;
    /// see `take_array_cells_dirty`.
    array_cells_dirty: bool,
    /// Names with a live typed cell in the executor's structured store
    /// (`shell_state.variables`). Only tracked names participate in the
    /// per-name freshness protocol; untracked writes skip the stale set
    /// entirely so marker/housekeeping keys never pollute it.
    tracked: HashSet<String>,
    /// Tracked names with an unsynced mirror write pending. A name is
    /// fresh when it is tracked and absent here (plus epoch equality, see
    /// below). Cleared per name by `mark_cell_synced`.
    stale: HashSet<String>,
    /// Per-name mirror-write epoch, bumped by every choked write to a
    /// tracked name. Lets read gates compare versions instead of only set
    /// membership (P1b/P1c).
    mirror_epoch: HashMap<String, u64>,
    /// Epoch the typed cell was last synced at (P1b sync helpers record
    /// here via `mark_cell_synced`).
    synced_epoch: HashMap<String, u64>,
}

impl<'a> IntoIterator for &'a VarTable {
    type Item = (&'a String, &'a String);
    type IntoIter = std::collections::hash_map::Iter<'a, String, String>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}

impl Deref for VarTable {
    type Target = HashMap<String, String>;
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

// NOTE: no `DerefMut` by design (P1a dirty choke-point). Mutable access
// goes through the inherent methods below so every write is observed.

impl From<HashMap<String, String>> for VarTable {
    fn from(values: HashMap<String, String>) -> Self {
        let mut table = VarTable {
            values,
            attrs: HashMap::new(),
            array_cells_dirty: false,
            tracked: HashSet::new(),
            stale: HashSet::new(),
            mirror_epoch: HashMap::new(),
            synced_epoch: HashMap::new(),
        };
        table.import_attr_strings();
        table
    }
}

impl VarTable {
    /// Build from a plain value map, importing the ten attribute marker
    /// strings (the cross-process serialization) into the structured map.
    /// The strings stay in `values` — they are the transport encoding.
    pub(crate) fn from_values(values: HashMap<String, String>) -> Self {
        VarTable::from(values)
    }

    // ---- P1a dirty choke-point: every mirror write observes here. ----

    /// Record a mirror write to `name`: when the name has a live typed
    /// cell, mark it stale and bump its mirror epoch. Untracked names are
    /// a no-op beyond this call (the global `array_cells_dirty` bit keeps
    /// its existing owners and semantics; P1b wires the sync helpers to
    /// the per-name protocol).
    pub(crate) fn note_mirror_write(&mut self, name: &str) {
        if !self.tracked.contains(name) {
            return;
        }
        self.stale.insert(name.to_string());
        self.mirror_epoch
            .entry(name.to_string())
            .and_modify(|epoch| *epoch = epoch.wrapping_add(1))
            .or_insert(1);
    }

    /// Mirror `HashMap::insert` through the choke-point.
    pub(crate) fn insert(&mut self, key: String, value: String) -> Option<String> {
        self.note_mirror_write(&key);
        self.values.insert(key, value)
    }

    /// Mirror `HashMap::remove` through the choke-point. A mirror removal
    /// also invalidates the typed cell (the verifier skips absent mirrors,
    /// but the read gates must not trust a cell with no mirror behind it).
    pub(crate) fn remove(&mut self, key: &str) -> Option<String> {
        self.note_mirror_write(key);
        self.values.remove(key)
    }

    /// Mirror `HashMap::get_mut` through the choke-point. Conservative:
    /// the returned reference can mutate without further observation, so
    /// the name is marked at call time (all current call sites mutate).
    pub(crate) fn get_mut(&mut self, key: &str) -> Option<&mut String> {
        self.note_mirror_write(key);
        self.values.get_mut(key)
    }

    /// Mirror `HashMap::entry` through the choke-point. Conservative like
    /// `get_mut`: a tracked key is marked at call time, so any subsequent
    /// `or_insert`/`and_modify` mutation is already observed. Returns the
    /// plain `HashMap` entry so all existing combinators keep working.
    pub(crate) fn entry(
        &mut self,
        key: String,
    ) -> std::collections::hash_map::Entry<'_, String, String> {
        self.note_mirror_write(&key);
        self.values.entry(key)
    }

    /// Mirror `HashMap::retain` through the choke-point. The predicate
    /// receives `(&String, &mut String)` exactly as before and may mutate
    /// values in place, so every tracked name present before the call is
    /// marked afterwards (current call sites only touch `__RUBASH_*`
    /// housekeeping keys, which are never tracked, so this stays a no-op
    /// in practice).
    pub(crate) fn retain<F>(&mut self, f: F)
    where
        F: FnMut(&String, &mut String) -> bool,
    {
        if self.tracked.is_empty() {
            self.values.retain(f);
            return;
        }
        let maybe_touched: Vec<String> = self
            .tracked
            .iter()
            .filter(|name| self.values.contains_key(*name))
            .cloned()
            .collect();
        self.values.retain(f);
        for name in maybe_touched {
            self.note_mirror_write(&name);
        }
    }

    // ---- Per-name freshness protocol (read gates consume this in P1c). ----

    /// Register a live typed cell for `name`. Starts fresh at the current
    /// mirror epoch (a just-created cell agrees with the mirror by
    /// construction; the writer synced it).
    pub(crate) fn track_cell(&mut self, name: &str) {
        self.tracked.insert(name.to_string());
        self.stale.remove(name);
        let mirror = self.mirror_epoch.get(name).copied().unwrap_or(0);
        self.synced_epoch.insert(name.to_string(), mirror);
    }

    /// Drop all per-name state for `name` (typed cell destroyed).
    pub(crate) fn untrack_cell(&mut self, name: &str) {
        self.tracked.remove(name);
        self.stale.remove(name);
        self.mirror_epoch.remove(name);
        self.synced_epoch.remove(name);
    }

    /// Record that the typed cell was (re)synced with the mirror: clears
    /// the stale mark and pins the synced epoch to the current mirror
    /// epoch. No-op for untracked names.
    pub(crate) fn mark_cell_synced(&mut self, name: &str) {
        if !self.tracked.contains(name) {
            return;
        }
        self.stale.remove(name);
        let mirror = self.mirror_epoch.get(name).copied().unwrap_or(0);
        self.synced_epoch.insert(name.to_string(), mirror);
    }

    /// Whether the typed cell for `name` may be trusted: tracked, not
    /// stale, and epochs equal. Untracked names report stale (`false`) so
    /// read gates fall through to the mirror path.
    pub(crate) fn cell_is_fresh(&self, name: &str) -> bool {
        if !self.tracked.contains(name) || self.stale.contains(name) {
            return false;
        }
        self.mirror_epoch.get(name).copied().unwrap_or(0)
            == self.synced_epoch.get(name).copied().unwrap_or(0)
    }

    /// The tracked names a choked mirror write has marked since their
    /// last proof — one half of the boundary verifier's candidate set
    /// (rubash#1956; the other half is the store's cell registry).
    pub(crate) fn stale_names(&self) -> &HashSet<String> {
        &self.stale
    }

    /// The current mirror epoch of a tracked name, or `None` when
    /// the name is untracked. This is the freshness coordinate the
    /// debugger's stop-and-diff watch compares across stops
    /// (debugger design section 5.4): equal epochs prove the value
    /// did not move, without comparing rendered text. An untracked
    /// name reports `None`, so the watch falls back to comparing
    /// rendered values.
    pub(crate) fn mirror_epoch_of(&self, name: &str) -> Option<u64> {
        if !self.tracked.contains(name) {
            return None;
        }
        Some(self.mirror_epoch.get(name).copied().unwrap_or(0))
    }

    fn import_attr_strings(&mut self) {
        for key in ATTR_KEYS {
            for name in self.string_marked_names(key) {
                let attrs = self.attrs.entry(name).or_default();
                if let Some(flag) = attrs.flag_mut(key) {
                    *flag = true;
                }
            }
        }
        self.attrs.retain(|_, attrs| !attrs.is_empty());
    }

    /// The value map itself (the `Deref` target, named for call sites
    /// that need an explicit `&HashMap`, e.g. returning it).
    pub(crate) fn values_map(&self) -> &HashMap<String, String> {
        &self.values
    }

    /// The full attribute set of one variable (GNU: the SHELL_VAR's flag
    /// word). Absent name = no attributes.
    pub(crate) fn attr_of(&self, name: &str) -> VarAttrs {
        self.attrs.get(name).copied().unwrap_or_default()
    }

    /// Mark typed indexed-array cells potentially stale: a mirror write
    /// happened outside the executor's syncing paths (declare-family
    /// builtins write through VarTable and cannot reach the structured
    /// store). Readers check the flag before trusting a typed cell;
    /// executor sync points clear it again.
    pub(crate) fn mark_array_cells_dirty(&mut self) {
        self.array_cells_dirty = true;
    }

    /// Whether an unsynced mirror write happened since the last executor
    /// sync. Read-only; only `clear_array_cells_dirty` resets it.
    pub(crate) fn array_cells_dirty(&self) -> bool {
        self.array_cells_dirty
    }

    /// Clear the dirty bit: the caller just re-synced every typed cell
    /// with the mirror (or dropped the stale ones).
    pub(crate) fn clear_array_cells_dirty(&mut self) {
        self.array_cells_dirty = false;
    }

    /// Membership query for any marker key. The ten attribute keys read
    /// the structured map (one lookup); other keys scan their list.
    pub(crate) fn is_marked(&self, key: &str, name: &str) -> bool {
        if let Some(flag) = self.attr_of(name).flag(key) {
            return flag;
        }
        self.string_is_marked(key, name)
    }

    /// Append `name` to a marker list. For the ten attribute keys the
    /// structured map is authoritative AND the whole store: no serialized
    /// string is maintained (GNU keeps them as flag words on the SHELL_VAR;
    /// serializing per mark cost O(n) and dominated export/unset loops).
    /// Readers use `marked_names` (rendered on demand from the flags) or
    /// `is_marked`, never the raw string.
    pub(crate) fn mark_name(&mut self, key: &str, name: &str) {
        let current = self.attr_of(name);
        if let Some(flag) = current.flag(key) {
            if !flag {
                self.write_flag(key, name, true);
            }
            return;
        }
        self.string_append(key, name);
    }

    /// Bulk clear for `unset` (#916): one HashMap removal drops all SHELL_VAR
    /// attribute bits instead of ten attr_of + write_flag round-trips. The
    /// ASSOC_128 bucket-count list is non-attribute bookkeeping and is
    /// dropped through its string path; TRACE goes with the rest (the
    /// variable is gone, GNU keeps no trace without a variable).
    pub(crate) fn remove_unset_marks(&mut self, name: &str) {
        self.attrs.remove(name);
        self.string_remove(crate::executor::types::ASSOC_128_VARS, name);
    }

    /// Remove `name` from a marker list (same contract as `mark_name`:
    /// attribute keys touch only the structured map).
    pub(crate) fn unmark_name(&mut self, key: &str, name: &str) {
        let current = self.attr_of(name);
        if let Some(flag) = current.flag(key) {
            if flag {
                self.write_flag(key, name, false);
            }
            return;
        }
        self.string_remove(key, name);
    }

    /// Replace the whole attribute set of one variable (frame restore,
    /// `declare -g` save/restore). Structured flags are the store; the
    /// serialized string is not maintained (see `mark_name`). An
    /// identical set is a no-op bit compare.
    pub(crate) fn set_attrs(&mut self, name: &str, new: VarAttrs) {
        let old = self.attr_of(name);
        if new == old {
            return;
        }
        if new.is_empty() {
            self.attrs.remove(name);
        } else {
            self.attrs.insert(name.to_string(), new);
        }
    }

    /// Rewrite one attribute list wholesale (startup import, fresh-shell
    /// rebuild): `names` becomes both the serialized string and the
    /// structured bits.
    pub(crate) fn replace_attr_list<I, S>(&mut self, key: &str, names: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let names: Vec<String> = names
            .into_iter()
            .map(|name| name.as_ref().to_string())
            .filter(|name| !name.is_empty())
            .collect();
        self.note_mirror_write(key);
        if names.is_empty() {
            self.values.remove(key);
        } else {
            self.values
                .insert(key.to_string(), names.join(DATA_DOLLAR_STR));
        }
        if let Some(_) = VarAttrs::default().flag(key) {
            // Ten-key family: rebuild the structured bits. Clear the bit
            // for every name not in the new list, set it for every name
            // that is.
            let members: std::collections::HashSet<&str> =
                names.iter().map(String::as_str).collect();
            let flagged: Vec<String> = self
                .attrs
                .iter()
                .filter(|(_, attrs)| attrs.flag(key) == Some(true))
                .map(|(name, _)| name.clone())
                .collect();
            for name in flagged {
                if !members.contains(name.as_str()) {
                    if let Some(flag) = self.attrs.get_mut(&name).and_then(|a| a.flag_mut(key)) {
                        *flag = false;
                    }
                }
            }
            for name in &members {
                let attrs = self.attrs.entry(name.to_string()).or_default();
                if let Some(flag) = attrs.flag_mut(key) {
                    *flag = true;
                }
            }
            self.attrs.retain(|_, attrs| !attrs.is_empty());
        }
    }

    /// Full marker-list enumeration (list-shaped consumers: child-env
    /// export lists, `declare -p` renderers). Attribute keys render on
    /// demand from the structured flags (sorted for determinism);
    /// non-attribute keys read their maintained string.
    pub(crate) fn marked_names(&self, key: &str) -> Vec<String> {
        if VarAttrs::default().flag(key).is_some() {
            return self.render_attr_names(key);
        }
        self.string_marked_names(key)
    }

    /// Render one attribute list from the structured flags. The ten
    /// attribute keys never consult the serialized string here: writers
    /// stopped maintaining it (see `mark_name`), so a parse would be
    /// stale. Sorted: `export -p` sorts anyway and every other consumer
    /// is order-insensitive.
    fn render_attr_names(&self, key: &str) -> Vec<String> {
        let mut names: Vec<String> = self
            .attrs
            .iter()
            .filter(|(_, attrs)| attrs.flag(key) == Some(true))
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names
    }

    /// Serialized form of one attribute list, for snapshots that persist
    /// the string (tempenv save/restore). Rendered from the structured
    /// flags so a write-behind list still round-trips; None when empty,
    /// matching the "emptied list drops the key" storage rule.
    pub(crate) fn rendered_attr_list(&self, key: &str) -> Option<String> {
        let names = self.marked_names(key);
        if names.is_empty() {
            return None;
        }
        Some(names.join(crate::executor::markers::DATA_DOLLAR_STR))
    }

    /// Restore a whole serialized marker list from a snapshot (tempenv
    /// save/restore of the `__RUBASH_*_VARS` entries). Rebuilds the
    /// structured bits for the ten attribute keys so a restored list and
    /// the flags map cannot diverge; `None` means the key was absent.
    pub(crate) fn restore_attr_string(&mut self, key: &str, value: Option<String>) {
        let Some(value) = value else {
            self.note_mirror_write(key);
            self.values.remove(key);
            if VarAttrs::default().flag(key).is_some() {
                let flagged: Vec<String> = self
                    .attrs
                    .iter()
                    .filter(|(_, attrs)| attrs.flag(key) == Some(true))
                    .map(|(name, _)| name.clone())
                    .collect();
                for name in flagged {
                    if let Some(flag) = self.attrs.get_mut(&name).and_then(|a| a.flag_mut(key)) {
                        *flag = false;
                    }
                }
                self.attrs.retain(|_, attrs| !attrs.is_empty());
            }
            return;
        };
        let names: Vec<String> = value
            .split(DATA_DOLLAR)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect();
        self.replace_attr_list(key, names);
    }

    fn write_flag(&mut self, key: &str, name: &str, value: bool) {
        let mut attrs = self.attr_of(name);
        if let Some(flag) = attrs.flag_mut(key) {
            *flag = value;
        }
        if attrs.is_empty() {
            self.attrs.remove(name);
        } else {
            self.attrs.insert(name.to_string(), attrs);
        }
    }

    fn string_is_marked(&self, key: &str, name: &str) -> bool {
        self.values
            .get(key)
            .is_some_and(|value| value.split(DATA_DOLLAR).any(|marked| marked == name))
    }

    fn string_marked_names(&self, key: &str) -> Vec<String> {
        self.values
            .get(key)
            .map(|value| {
                value
                    .split(DATA_DOLLAR)
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn string_append(&mut self, key: &str, name: &str) {
        // Membership pre-check: appending an already-present name must
        // leave the string byte-identical (perf11's no-op fast path,
        // kept for the non-attribute keys that still route here).
        if self.string_is_marked(key, name) {
            return;
        }
        let mut names = self.string_marked_names(key);
        names.push(name.to_string());
        self.note_mirror_write(key);
        self.values
            .insert(key.to_string(), names.join(DATA_DOLLAR_STR));
    }

    fn string_remove(&mut self, key: &str, name: &str) {
        if !self.string_is_marked(key, name) {
            return;
        }
        let names: Vec<String> = self
            .string_marked_names(key)
            .into_iter()
            .filter(|current| current != name)
            .collect();
        // An emptied list drops the key (unset.rs's whole-family unbind
        // removes the entry; no consumer distinguishes "" from absent).
        self.note_mirror_write(key);
        if names.is_empty() {
            self.values.remove(key);
        } else {
            self.values
                .insert(key.to_string(), names.join(DATA_DOLLAR_STR));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> VarTable {
        VarTable::default()
    }

    #[test]
    fn attr_roundtrip_and_flip_sync() {
        let mut t = table();
        t.mark_name(EXPORTED_VARS, "a");
        assert!(t.is_marked(EXPORTED_VARS, "a"));
        // Write-behind: the serialized string is not maintained per mark;
        // readers use is_marked/marked_names instead.
        assert!(t.values.get(EXPORTED_VARS).is_none());
        assert!(t.marked_names(EXPORTED_VARS).contains(&"a".to_string()));
        // No-op mark leaves everything alone.
        t.mark_name(EXPORTED_VARS, "a");
        assert!(t.values.get(EXPORTED_VARS).is_none());
        t.unmark_name(EXPORTED_VARS, "a");
        assert!(!t.is_marked(EXPORTED_VARS, "a"));
        assert!(t.marked_names(EXPORTED_VARS).is_empty());
        assert!(t.attr_of("a").is_empty());
    }

    #[test]
    fn set_attrs_updates_flags_not_strings() {
        let mut t = table();
        t.mark_name(EXPORTED_VARS, "a");
        t.mark_name(READONLY_VARS, "a");
        let mut attrs = t.attr_of("a");
        attrs.integer = true;
        t.set_attrs("a", attrs);
        assert!(t.is_marked(INTEGER_VARS, "a"));
        assert!(t.is_marked(EXPORTED_VARS, "a"));
        assert!(t.is_marked(READONLY_VARS, "a"));
        // No serialized strings are written for attribute keys.
        assert!(t.values.get(INTEGER_VARS).is_none());
        // Restoring the previous set is a pure no-op.
        attrs.integer = false;
        t.set_attrs("a", attrs);
        assert!(!t.is_marked(INTEGER_VARS, "a"));
        assert_eq!(
            t.values.get(EXPORTED_VARS),
            None,
            "no exported string maintained"
        );
    }

    #[test]
    fn from_values_imports_strings_into_flags() {
        let mut values = HashMap::new();
        values.insert(
            EXPORTED_VARS.to_string(),
            format!("x{sep}y", sep = DATA_DOLLAR),
        );
        values.insert(NAMEREF_VARS.to_string(), "z".to_string());
        let t = VarTable::from_values(values);
        assert!(t.is_marked(EXPORTED_VARS, "x"));
        assert!(t.is_marked(EXPORTED_VARS, "y"));
        assert!(t.is_marked(NAMEREF_VARS, "z"));
        assert!(!t.is_marked(EXPORTED_VARS, "z"));
        // Non-attribute keys keep working through the same helpers.
        assert!(t.string_is_marked("__RUBASH_EXPORTED_FUNCTIONS", "f") == false);
    }

    #[test]
    fn non_attribute_keys_stay_string_only() {
        let mut t = table();
        t.mark_name("__RUBASH_EXPORTED_FUNCTIONS", "fn");
        assert!(t.is_marked("__RUBASH_EXPORTED_FUNCTIONS", "fn"));
        assert!(t.attr_of("fn").is_empty());
        t.unmark_name("__RUBASH_EXPORTED_FUNCTIONS", "fn");
        assert!(!t.is_marked("__RUBASH_EXPORTED_FUNCTIONS", "fn"));
    }

    #[test]
    fn replace_attr_list_rebuilds_both_forms() {
        let mut t = table();
        t.mark_name(EXPORTED_VARS, "a");
        t.mark_name(EXPORTED_VARS, "b");
        t.replace_attr_list(EXPORTED_VARS, ["b", "c"]);
        assert!(!t.is_marked(EXPORTED_VARS, "a"));
        assert!(t.is_marked(EXPORTED_VARS, "b"));
        assert!(t.is_marked(EXPORTED_VARS, "c"));
        assert_eq!(
            t.values.get(EXPORTED_VARS).map(String::as_str),
            Some(format!("b{sep}c", sep = DATA_DOLLAR).as_str())
        );
        t.replace_attr_list(EXPORTED_VARS, Vec::<&str>::new());
        assert!(t.values.get(EXPORTED_VARS).is_none());
        assert!(!t.is_marked(EXPORTED_VARS, "b"));
    }

    #[test]
    fn values_still_reachable_through_deref() {
        let mut t = table();
        t.insert("PATH".to_string(), "/bin".to_string());
        assert_eq!(t.get("PATH").map(String::as_str), Some("/bin"));
        t.remove("PATH");
        assert!(t.get("PATH").is_none());
    }

    #[test]
    fn dirty_flag_mark_clear_cycle() {
        let mut t = table();
        assert!(!t.array_cells_dirty());
        t.mark_array_cells_dirty();
        assert!(t.array_cells_dirty());
        t.clear_array_cells_dirty();
        assert!(!t.array_cells_dirty());
    }

    #[test]
    fn attr_flag_ops_do_not_touch_dirty_bit() {
        let mut t = table();
        // Attribute writes on a clean table leave it clean.
        t.mark_name(EXPORTED_VARS, "a");
        t.unmark_name(EXPORTED_VARS, "a");
        t.set_attrs("a", t.attr_of("a"));
        t.remove_unset_marks("a");
        assert!(!t.is_marked(EXPORTED_VARS, "a"));
        assert!(!t.array_cells_dirty());
        // The same writes on a dirty table leave it dirty.
        t.mark_name(READONLY_VARS, "b");
        t.mark_array_cells_dirty();
        let mut attrs = t.attr_of("b");
        attrs.integer = true;
        t.set_attrs("b", attrs);
        assert!(t.is_marked(INTEGER_VARS, "b"));
        t.unmark_name(READONLY_VARS, "b");
        assert!(!t.is_marked(READONLY_VARS, "b"));
        t.remove_unset_marks("b");
        assert!(!t.is_marked(INTEGER_VARS, "b"));
        assert!(t.array_cells_dirty());
        t.clear_array_cells_dirty();
        assert!(!t.array_cells_dirty());
    }

    #[test]
    fn untracked_writes_stay_out_of_stale_set() {
        let mut t = table();
        t.insert("PWD".to_string(), "/tmp".to_string());
        t.remove("OLDPWD");
        let _ = t.get_mut("PWD");
        t.entry("HOME".to_string()).or_insert("/root".to_string());
        t.retain(|_, _| true);
        assert!(!t.cell_is_fresh("PWD"));
        assert!(!t.cell_is_fresh("HOME"));
    }

    #[test]
    fn tracked_write_marks_stale_and_sync_clears() {
        let mut t = table();
        t.insert("a".to_string(), "(1 2)".to_string());
        // Untracked: no freshness either way.
        assert!(!t.cell_is_fresh("a"));
        t.track_cell("a");
        assert!(t.cell_is_fresh("a"));
        // Each choked write path marks stale + bumps the epoch.
        t.insert("a".to_string(), "(1 2 3)".to_string());
        assert!(!t.cell_is_fresh("a"));
        t.mark_cell_synced("a");
        assert!(t.cell_is_fresh("a"));
        t.remove("a");
        assert!(!t.cell_is_fresh("a"));
        t.mark_cell_synced("a");
        assert!(t.cell_is_fresh("a"));
        t.untrack_cell("a");
        assert!(!t.cell_is_fresh("a"));
    }

    #[test]
    fn get_mut_entry_retain_mark_tracked_stale() {
        let mut t = table();
        t.insert("a".to_string(), "(1)".to_string());
        t.insert("b".to_string(), "(2)".to_string());
        t.track_cell("a");
        t.track_cell("b");
        assert!(t.cell_is_fresh("a"));
        if let Some(slot) = t.get_mut("a") {
            slot.push_str("x");
        }
        assert!(!t.cell_is_fresh("a"));
        assert!(t.cell_is_fresh("b"));
        t.mark_cell_synced("a");
        t.entry("b".to_string()).or_insert("v".to_string());
        assert!(!t.cell_is_fresh("b"));
        assert!(t.cell_is_fresh("a"));
        t.mark_cell_synced("b");
        t.retain(|_, _| true);
        assert!(!t.cell_is_fresh("a"));
        assert!(!t.cell_is_fresh("b"));
        // Epochs advance monotonically per write; sync pins them equal.
        t.mark_cell_synced("a");
        t.mark_cell_synced("b");
        assert!(t.cell_is_fresh("a"));
        assert!(t.cell_is_fresh("b"));
        t.untrack_cell("a");
        t.untrack_cell("b");
    }
}
