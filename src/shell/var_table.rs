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

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};
use crate::executor::{
    ARRAY_VARS, ASSOC_VARS, DECLARED_UNSET_VARS, EXPORTED_VARS, INTEGER_VARS, LOWERCASE_VARS,
    NAMEREF_VARS, READONLY_VARS, TRACE_VARS, UPPERCASE_VARS,
};

/// The ten declare-family SHELL_VAR attributes (variables.h:124-133
/// att_* port). Captured and restored as one unit by function frames
/// (`local_attr_scopes`), tempenv snapshots, and `declare -g` save/restore.
///
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
/// `Deref`/`DerefMut` expose the value map so the hundreds of existing
/// value-only call sites (plain `get`/`insert`/`remove`/iteration)
/// compile unchanged; attribute operations MUST use the methods below.
#[derive(Debug, Default, Clone)]
pub struct VarTable {
    values: HashMap<String, String>,
    attrs: HashMap<String, VarAttrs>,
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

impl DerefMut for VarTable {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.values
    }
}

impl From<HashMap<String, String>> for VarTable {
    fn from(values: HashMap<String, String>) -> Self {
        let mut table = VarTable {
            values,
            attrs: HashMap::new(),
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

    /// Membership query for any marker key. The ten attribute keys read
    /// the structured map (one lookup); other keys scan their list.
    pub(crate) fn is_marked(&self, key: &str, name: &str) -> bool {
        if let Some(flag) = self.attr_of(name).flag(key) {
            return flag;
        }
        self.string_is_marked(key, name)
    }

    /// Append `name` to a marker list. For the ten attribute keys the
    /// structured map is authoritative; the serialized string is
    /// rewritten only when the bit actually flips (GNU: setting an
    /// already-set att_* bit touches nothing).
    pub(crate) fn mark_name(&mut self, key: &str, name: &str) {
        let current = self.attr_of(name);
        if let Some(flag) = current.flag(key) {
            if !flag {
                self.string_append(key, name);
                self.write_flag(key, name, true);
            }
            return;
        }
        self.string_append(key, name);
    }

    /// Remove `name` from a marker list (same contract as `mark_name`).
    pub(crate) fn unmark_name(&mut self, key: &str, name: &str) {
        let current = self.attr_of(name);
        if let Some(flag) = current.flag(key) {
            if flag {
                self.string_remove(key, name);
                self.write_flag(key, name, false);
            }
            return;
        }
        self.string_remove(key, name);
    }

    /// Replace the whole attribute set of one variable (frame restore,
    /// `declare -g` save/restore). Strings are rewritten only for the
    /// bits that changed; an identical set is a no-op bit compare.
    pub(crate) fn set_attrs(&mut self, name: &str, new: VarAttrs) {
        let old = self.attr_of(name);
        if new == old {
            return;
        }
        for key in ATTR_KEYS {
            let (Some(old_flag), Some(new_flag)) = (old.flag(key), new.flag(key)) else {
                continue;
            };
            if old_flag == new_flag {
                continue;
            }
            if new_flag {
                self.string_append(key, name);
            } else {
                self.string_remove(key, name);
            }
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
    /// export lists, `declare -p` renderers). Reads the serialized
    /// string so insertion order is preserved.
    pub(crate) fn marked_names(&self, key: &str) -> Vec<String> {
        self.string_marked_names(key)
    }

    /// Restore a whole serialized marker list from a snapshot (tempenv
    /// save/restore of the `__RUBASH_*_VARS` entries). Rebuilds the
    /// structured bits for the ten attribute keys so a restored list and
    /// the flags map cannot diverge; `None` means the key was absent.
    pub(crate) fn restore_attr_string(&mut self, key: &str, value: Option<String>) {
        let Some(value) = value else {
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
        assert!(t.string_is_marked(EXPORTED_VARS, "a"));
        // No-op mark leaves both forms alone.
        let before = t.values.get(EXPORTED_VARS).cloned();
        t.mark_name(EXPORTED_VARS, "a");
        assert_eq!(
            before.as_deref(),
            t.values.get(EXPORTED_VARS).map(String::as_str)
        );
        t.unmark_name(EXPORTED_VARS, "a");
        assert!(!t.is_marked(EXPORTED_VARS, "a"));
        assert!(!t.string_is_marked(EXPORTED_VARS, "a"));
        assert!(t.attr_of("a").is_empty());
    }

    #[test]
    fn set_attrs_only_rewrites_flipped_strings() {
        let mut t = table();
        t.mark_name(EXPORTED_VARS, "a");
        t.mark_name(READONLY_VARS, "a");
        let mut attrs = t.attr_of("a");
        attrs.integer = true;
        t.set_attrs("a", attrs);
        assert!(t.string_is_marked(INTEGER_VARS, "a"));
        assert!(t.string_is_marked(EXPORTED_VARS, "a"));
        assert!(t.string_is_marked(READONLY_VARS, "a"));
        // Restoring the previous set is a pure no-op.
        attrs.integer = false;
        t.set_attrs("a", attrs);
        assert!(!t.string_is_marked(INTEGER_VARS, "a"));
        assert_eq!(
            t.values.get(EXPORTED_VARS).map(String::as_str),
            Some("a"),
            "exported string untouched by integer flip"
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
}
