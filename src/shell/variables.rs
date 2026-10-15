//! Typed shell variable storage.
//!
//! GNU Bash references: `variables.c`, `array.c`, `assoc.c`, and
//! `builtins/declare.def`. The executor's legacy environment map remains an
//! adapter while callers migrate to this owner.

use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellValue {
    Scalar(String),
    IndexedArray(BTreeMap<i64, String>),
    AssociativeArray(HashMap<String, String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub value: ShellValue,
    pub exported: bool,
    pub readonly: bool,
    pub integer: bool,
    pub nameref: Option<String>,
    pub uppercase: bool,
    pub lowercase: bool,
}

impl Variable {
    pub fn scalar(value: impl Into<String>) -> Self {
        Self {
            value: ShellValue::Scalar(value.into()),
            exported: false,
            readonly: false,
            integer: false,
            nameref: None,
            uppercase: false,
            lowercase: false,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VariableStore {
    variables: BTreeMap<String, Variable>,
}

impl VariableStore {
    pub fn get(&self, name: &str) -> Option<&Variable> {
        self.variables.get(name)
    }
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Variable> {
        self.variables.get_mut(name)
    }

    pub(crate) fn remove(&mut self, name: &str) -> Option<Variable> {
        self.variables.remove(name)
    }

    pub fn set(&mut self, name: impl Into<String>, variable: Variable) -> Result<(), &'static str> {
        let name = name.into();
        if self.get(&name).map_or(false, |old| old.readonly) {
            return Err("readonly variable");
        }
        self.variables.insert(name, variable);
        Ok(())
    }

    pub fn set_scalar(
        &mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), &'static str> {
        self.set(name, Variable::scalar(value))
    }

    pub fn indexed_element(&self, name: &str, index: i64) -> Option<&str> {
        match &self.get(name)?.value {
            ShellValue::IndexedArray(values) => values.get(&index).map(String::as_str),
            _ => None,
        }
    }

    pub fn associative_element(&self, name: &str, key: &str) -> Option<&str> {
        match &self.get(name)?.value {
            ShellValue::AssociativeArray(values) => values.get(key).map(String::as_str),
            _ => None,
        }
    }

    pub fn set_indexed_element(
        &mut self,
        name: &str,
        index: i64,
        value: impl Into<String>,
    ) -> Result<(), &'static str> {
        let variable = self
            .variables
            .entry(name.to_string())
            .or_insert_with(|| Variable {
                value: ShellValue::IndexedArray(BTreeMap::new()),
                ..Variable::scalar("")
            });
        if variable.readonly {
            return Err("readonly variable");
        }
        if matches!(variable.value, ShellValue::Scalar(_)) {
            variable.value = ShellValue::IndexedArray(BTreeMap::new());
        }
        match &mut variable.value {
            ShellValue::IndexedArray(values) => {
                values.insert(index, value.into());
                Ok(())
            }
            ShellValue::AssociativeArray(_) => {
                Err("cannot assign indexed element to associative array")
            }
            ShellValue::Scalar(_) => unreachable!(),
        }
    }

    pub fn replace_indexed_array<I, V>(&mut self, name: &str, values: I) -> Result<(), &'static str>
    where
        I: IntoIterator<Item = V>,
        V: Into<String>,
    {
        let variable = self
            .variables
            .entry(name.to_string())
            .or_insert_with(|| Variable {
                value: ShellValue::IndexedArray(BTreeMap::new()),
                ..Variable::scalar("")
            });
        if variable.readonly {
            return Err("readonly variable");
        }
        if !matches!(variable.value, ShellValue::IndexedArray(_)) {
            variable.value = ShellValue::IndexedArray(BTreeMap::new());
        }
        let ShellValue::IndexedArray(entries) = &mut variable.value else {
            unreachable!();
        };
        entries.clear();
        for (index, value) in values.into_iter().enumerate() {
            entries.insert(index as i64, value.into());
        }
        Ok(())
    }

    /// rubash#616 (plan Phase 1a): replace the structured indexed cell while
    /// PRESERVING the caller's keys. `replace_indexed_array` is dense by
    /// position (it enumerates the values), which cannot carry a sparse
    /// array across — `a=([5]=x [2]=y)` must stay `{2,5}`, not `{0,1}`
    /// (GNU array.c array_insert keeps the real index). Same readonly
    /// refusal as the dense form.
    pub fn replace_indexed_array_entries<I, V>(
        &mut self,
        name: &str,
        entries: I,
    ) -> Result<(), &'static str>
    where
        I: IntoIterator<Item = (i64, V)>,
        V: Into<String>,
    {
        let variable = self
            .variables
            .entry(name.to_string())
            .or_insert_with(|| Variable {
                value: ShellValue::IndexedArray(BTreeMap::new()),
                ..Variable::scalar("")
            });
        if variable.readonly {
            return Err("readonly variable");
        }
        if !matches!(variable.value, ShellValue::IndexedArray(_)) {
            variable.value = ShellValue::IndexedArray(BTreeMap::new());
        }
        let ShellValue::IndexedArray(values) = &mut variable.value else {
            unreachable!();
        };
        values.clear();
        for (index, value) in entries {
            values.insert(index, value.into());
        }
        Ok(())
    }

    /// rubash#616 (plan Phase 1a): extend an ALREADY-structured indexed cell
    /// by one element without rebuilding the map. Returns `false` when the
    /// cell is absent, of another shape, readonly, or when the resulting
    /// size disagrees with `expected_len` (the caller's own parse of the
    /// mirror it is about to render); the caller then performs the full
    /// structured sync, which drops a refused cell. The size check keeps the
    /// "complete or absent" invariant self-healing instead of trusting the
    /// cell blindly: the element-write loop is the array battery's hot path
    /// and a full `BTreeMap` rebuild per element made it measurably slower
    /// than the pre-#616 baseline (GNU array.c:516 `array_insert` appends to
    /// the ARRAY object in O(1)).
    pub fn extend_indexed_element(
        &mut self,
        name: &str,
        index: i64,
        value: &str,
        expected_len: usize,
    ) -> bool {
        let Some(variable) = self.variables.get_mut(name) else {
            return false;
        };
        if variable.readonly {
            return false;
        }
        let ShellValue::IndexedArray(entries) = &mut variable.value else {
            return false;
        };
        entries.insert(index, value.to_string());
        entries.len() == expected_len
    }

    /// rubash#616 (plan Phase 1a): refresh a complete indexed cell from a
    /// freshly parsed mirror WITHOUT rebuilding it, when the new map is an
    /// extension of the current one. This is the append-shaped case the
    /// compound write path (`a+=(...)`) hits once per iteration; a full
    /// rebuild there copies every value per write, which made the
    /// compound-append battery measurably slower than the pre-#616 baseline
    /// (GNU `array.c:516 array_insert` appends to the ARRAY object and
    /// `array.h:121` counts it — nothing is re-serialised).
    ///
    /// The retained prefix is **verified key-by-key**, not assumed: a cell
    /// that is not an exact prefix of `parsed` returns `false` and the caller
    /// performs the full sync. That keeps this a cache-refresh, never a
    /// second source of truth. Returns `true` only when the cell is now
    /// exactly `parsed`.
    pub fn extend_indexed_array_from_parsed(
        &mut self,
        name: &str,
        parsed: &BTreeMap<usize, String>,
    ) -> bool {
        let Some(variable) = self.variables.get_mut(name) else {
            return false;
        };
        if variable.readonly {
            return false;
        }
        let ShellValue::IndexedArray(cell) = &mut variable.value else {
            return false;
        };
        if parsed.len() < cell.len() {
            return false;
        }
        if !cell.iter().zip(parsed.iter()).all(|((a, va), (b, vb))| {
            i64::try_from(*b).map(|b| b == *a).unwrap_or(false) && va == vb
        }) {
            return false;
        }
        let tail = parsed
            .iter()
            .skip(cell.len())
            .map(|(index, value)| (i64::try_from(*index).unwrap_or(i64::MAX), value.clone()))
            .collect::<Vec<_>>();
        for (index, value) in tail {
            cell.insert(index, value);
        }
        true
    }

    /// rubash#616 (plan Phase 1b): `(len, max_index)` of a complete indexed
    /// cell, or `None` when `name` is not an indexed cell. This is GNU's
    /// `ARRAY` shape — `num_elements` and `max_index` (array.h:30-41) — read
    /// without touching the mirror render, so the element-write path can
    /// decide whether a write is an append (index == max_index + 1) in
    /// O(log n) instead of re-parsing the whole array.
    pub fn indexed_array_shape(&self, name: &str) -> Option<(usize, Option<i64>)> {
        match self.get(name).map(|variable| &variable.value) {
            Some(ShellValue::IndexedArray(values)) => {
                Some((values.len(), values.keys().next_back().copied()))
            }
            _ => None,
        }
    }

    pub fn replace_associative_array<I, K, V>(
        &mut self,
        name: &str,
        values: I,
    ) -> Result<(), &'static str>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let variable = self
            .variables
            .entry(name.to_string())
            .or_insert_with(|| Variable {
                value: ShellValue::AssociativeArray(HashMap::new()),
                ..Variable::scalar("")
            });
        if variable.readonly {
            return Err("readonly variable");
        }
        if !matches!(variable.value, ShellValue::AssociativeArray(_)) {
            variable.value = ShellValue::AssociativeArray(HashMap::new());
        }
        let ShellValue::AssociativeArray(entries) = &mut variable.value else {
            unreachable!();
        };
        entries.clear();
        for (key, value) in values {
            entries.insert(key.into(), value.into());
        }
        Ok(())
    }

    pub fn set_associative_element(
        &mut self,
        name: &str,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), &'static str> {
        let variable = self
            .variables
            .entry(name.to_string())
            .or_insert_with(|| Variable {
                value: ShellValue::AssociativeArray(HashMap::new()),
                ..Variable::scalar("")
            });
        if variable.readonly {
            return Err("readonly variable");
        }
        if matches!(variable.value, ShellValue::Scalar(_)) {
            variable.value = ShellValue::AssociativeArray(HashMap::new());
        }
        match &mut variable.value {
            ShellValue::AssociativeArray(values) => {
                values.insert(key.into(), value.into());
                Ok(())
            }
            ShellValue::IndexedArray(_) => {
                Err("cannot assign associative element to indexed array")
            }
            ShellValue::Scalar(_) => unreachable!(),
        }
    }

    pub fn remove_indexed_element(
        &mut self,
        name: &str,
        index: i64,
    ) -> Result<Option<String>, &'static str> {
        let Some(variable) = self.get_mut(name) else {
            return Ok(None);
        };
        if variable.readonly {
            return Err("readonly variable");
        }
        match &mut variable.value {
            ShellValue::IndexedArray(values) => Ok(values.remove(&index)),
            _ => Ok(None),
        }
    }

    pub fn remove_associative_element(
        &mut self,
        name: &str,
        key: &str,
    ) -> Result<Option<String>, &'static str> {
        let Some(variable) = self.get_mut(name) else {
            return Ok(None);
        };
        if variable.readonly {
            return Err("readonly variable");
        }
        match &mut variable.value {
            ShellValue::AssociativeArray(values) => Ok(values.remove(key)),
            _ => Ok(None),
        }
    }

    pub fn unset(&mut self, name: &str) -> Result<Option<Variable>, &'static str> {
        if self.get(name).map_or(false, |variable| variable.readonly) {
            return Err("readonly variable");
        }
        Ok(self.variables.remove(name))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Variable)> {
        self.variables.iter()
    }

    /// Import the process environment as exported scalar shell variables.
    ///
    /// This is the initial migration boundary: executor writes still update the
    /// legacy environment mirror, while typed array and attribute state can be
    /// introduced without changing child-process setup in one step.
    pub fn from_environment(environment: &HashMap<String, String>) -> Self {
        let mut store = Self::default();
        for (name, value) in environment {
            // GNU variables.c:511-526 (initialize_shell_variables): only
            // valid identifiers are bound as shell variables; entries with
            // invalid names (Windows inherits names like
            // `CommonProgramFiles(x86)`) go into the invisible invalid_env
            // table (variables.c:3307 bind_invalid_envvar) — exported to
            // children, never expandable or listable. Rubash keeps those
            // entries in the executor's env_vars mirror for child-process
            // passthrough; the typed store only mirrors shell variables.
            if !is_valid_identifier(name) {
                continue;
            }
            let mut variable = Variable::scalar(value.clone());
            variable.exported = true;
            store.variables.insert(name.clone(), variable);
        }
        store
    }

    /// Serialize only exported scalar values for a child process.
    pub fn export_environment(&self) -> HashMap<String, String> {
        self.variables
            .iter()
            .filter_map(|(name, variable)| {
                if !variable.exported {
                    return None;
                }
                let ShellValue::Scalar(value) = &variable.value else {
                    return None;
                };
                let value = if variable.uppercase {
                    value.to_uppercase()
                } else if variable.lowercase {
                    value.to_lowercase()
                } else {
                    value.clone()
                };
                Some((name.clone(), value))
            })
            .collect()
    }
}

/// GNU general.c valid_identifier: `[A-Za-z_][A-Za-z0-9_]*` (with the POSIX
/// function-name restriction compiled out of the 5.3 baseline — see
/// executor/support_names.rs valid_function_identifier).
fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_and_associative_values_are_typed() {
        let mut store = VariableStore::default();
        let mut indexed = BTreeMap::new();
        indexed.insert(4, "four".into());
        store
            .set(
                "indexed",
                Variable {
                    value: ShellValue::IndexedArray(indexed),
                    ..Variable::scalar("")
                },
            )
            .unwrap();
        let mut associative = HashMap::new();
        associative.insert("key".into(), "value".into());
        store
            .set(
                "assoc",
                Variable {
                    value: ShellValue::AssociativeArray(associative),
                    ..Variable::scalar("")
                },
            )
            .unwrap();
        assert!(matches!(
            store.get("indexed").unwrap().value,
            ShellValue::IndexedArray(_)
        ));
        assert!(matches!(
            store.get("assoc").unwrap().value,
            ShellValue::AssociativeArray(_)
        ));
    }

    #[test]
    fn export_is_an_adapter_not_array_storage() {
        let mut store = VariableStore::default();
        let mut variable = Variable::scalar("mixed");
        variable.exported = true;
        variable.uppercase = true;
        store.set("value", variable).unwrap();
        assert_eq!(store.export_environment()["value"], "MIXED");
    }

    // GNU variables.c:511-526: inherited entries whose names are not valid
    // identifiers (Windows `CommonProgramFiles(x86)`) never become shell
    // variables (niubash issue #102).
    #[test]
    fn from_environment_skips_invalid_identifier_names() {
        let mut environment = HashMap::new();
        environment.insert("VALID_NAME".to_string(), "kept".to_string());
        environment.insert("CommonProgramFiles(x86)".to_string(), "dropped".to_string());
        environment.insert(
            "BASH_FUNC_rm%%".to_string(),
            "() { rm \"$@\"; }".to_string(),
        );
        environment.insert("WITH SPACE".to_string(), "dropped".to_string());
        environment.insert("1STARTS_DIGIT".to_string(), "dropped".to_string());
        let store = VariableStore::from_environment(&environment);
        assert_eq!(store.get("VALID_NAME").map(|v| v.exported), Some(true));
        assert!(store.get("CommonProgramFiles(x86)").is_none());
        assert!(store.get("BASH_FUNC_rm%%").is_none());
        assert!(store.get("WITH SPACE").is_none());
        assert!(store.get("1STARTS_DIGIT").is_none());
    }

    #[test]
    fn element_apis_preserve_sparse_and_readonly_semantics() {
        let mut store = VariableStore::default();
        store.set_indexed_element("items", 3, "three").unwrap();
        store.set_indexed_element("items", 9, "nine").unwrap();
        assert_eq!(store.indexed_element("items", 3), Some("three"));
        assert_eq!(store.indexed_element("items", 4), None);
        assert_eq!(
            store.remove_indexed_element("items", 3).unwrap(),
            Some("three".into())
        );
        assert_eq!(store.indexed_element("items", 3), None);

        store
            .set_associative_element("map", "a b]", "value")
            .unwrap();
        assert_eq!(store.associative_element("map", "a b]"), Some("value"));

        store.get_mut("items").unwrap().readonly = true;
        assert_eq!(
            store.set_indexed_element("items", 1, "blocked"),
            Err("readonly variable")
        );
        assert_eq!(
            store.remove_indexed_element("items", 9),
            Err("readonly variable")
        );
    }
}
