//! Executor side of the §5.3 state views (API design §5.3; PLAN
//! row 6): capture and host-write helpers behind
//! `crate::api::views`.
//!
//! The split of labor is the no-shadow-bookkeeping rule (debugger
//! design §7): the snapshot *types* and the live view objects live
//! in `crate::api`, but every capture reads the engine's own stores
//! and dynamic views, and every write delegates to the machinery the
//! shell itself uses — the host write path (`set_env`), the array
//! state stores, the `cd` bindpwd path, the unset builtin's leaf
//! sequence. Nothing here maintains a parallel copy of any state.

use super::*;
use crate::api::views::{
    DynamicStateSnapshot, FrameSnapshot, InspectionSnapshot, OptionFamily, OptionState,
    VariableAttrs, VariableSnapshot, VariableValue,
};
use crate::executor::markers::STORAGE_WORD_PREFIX;

/// Merge the table's structured attribute word for `name` with the
/// typed cell's flags into the API attribute set. The two sources
/// are kept in sync by the write paths; the merge is a belt over
/// both because each carries flags the other lacks (the table word
/// has array/assoc/trace, the typed cell has the declare-time bits
/// in field form).
fn merged_attrs(
    table: crate::shell::var_table::VarAttrs,
    typed: Option<&crate::shell::Variable>,
    capcase: bool,
) -> VariableAttrs {
    VariableAttrs {
        exported: table.exported || typed.is_some_and(|cell| cell.exported),
        readonly: table.readonly || typed.is_some_and(|cell| cell.readonly),
        integer: table.integer || typed.is_some_and(|cell| cell.integer),
        nameref: table.nameref || typed.is_some_and(|cell| cell.nameref.is_some()),
        trace: table.trace,
        uppercase: table.uppercase || typed.is_some_and(|cell| cell.uppercase),
        lowercase: table.lowercase || typed.is_some_and(|cell| cell.lowercase),
        capcase,
        array: table.array,
        assoc: table.assoc,
    }
}

/// The snapshot attribute set in the shared declaration printer's
/// own attribute form.
fn decl_attrs_from(attrs: &VariableAttrs) -> crate::builtins::declare::DeclarationAttrs {
    crate::builtins::declare::DeclarationAttrs {
        exported: attrs.exported,
        readonly: attrs.readonly,
        array: attrs.array,
        assoc: attrs.assoc,
        integer: attrs.integer,
        uppercase: attrs.uppercase,
        lowercase: attrs.lowercase,
        capcase: attrs.capcase,
        nameref: attrs.nameref,
        trace: attrs.trace,
    }
}

/// The scalar RHS of a rendered declaration line, unquoted — the
/// inverse of the `sh_double_quote_text` every scalar dynamic arm
/// renders with, so a dynamic snapshot's value and declaration are
/// the *same* read (a second `dynamic_parameter_value` call would
/// draw RANDOM again and disagree with the printed line).
fn declaration_scalar_rhs(text: &str, name: &str) -> Option<String> {
    let rest = text.strip_prefix("declare ")?;
    let (_, rhs) = rest.split_once(&format!(" {name}="))?;
    let inner = rhs.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            out.push(chars.next()?);
        } else {
            out.push(ch);
        }
    }
    Some(out)
}

/// Parse a mirror storage string into typed cells for a snapshot:
/// associative when the ASSOC mark is set, indexed when the value
/// carries the array storage form, scalar otherwise. Used for names
/// whose typed cell is absent (startup-seeded and materialized
/// arrays) — names with a typed cell snapshot from the cell itself.
fn value_from_mirror(
    env_vars: &crate::shell::var_table::VarTable,
    name: &str,
    mirror: &str,
    attrs: &VariableAttrs,
) -> VariableValue {
    if attrs.assoc {
        let entries = assoc_hash_ordered_entries(mirror, assoc_nbuckets(env_vars, name));
        let mut entries: Vec<(String, String)> = entries.into_iter().map(|(k, v)| (k, v)).collect();
        entries.sort();
        return VariableValue::Associative(entries);
    }
    if attrs.array || mirror.starts_with(STORAGE_WORD_PREFIX) || mirror.starts_with('(') {
        let entries = indexed_array_entries(mirror)
            .into_iter()
            .map(|(index, value)| (index as i64, value))
            .collect();
        return VariableValue::Indexed(entries);
    }
    VariableValue::Scalar(mirror.to_string())
}

impl Executor {
    /// Capture one variable as the debuggee resolves it (§7.2):
    /// dynamic parameters first (their getter IS the read), then an
    /// innermost local binding, then the global stores.
    pub(crate) fn capture_variable(&self, name: &str) -> Option<VariableSnapshot> {
        let env_vars = &self.shell_state.env_vars;
        // Dynamic parameters: the declaration view is the same text
        // `declare -p NAME` prints (declare_local.rs merges it into
        // both operand and listing forms), so snapshot text and
        // builtin text are one rendering.
        if let Some(text) = self.dynamic_declaration_view(name) {
            let mut attrs = merged_attrs(
                env_vars.attr_of(name),
                self.shell_state.variables.get(name),
                env_vars.is_marked(CAPCASE_VARS, name),
            );
            let value = match name {
                "PIPESTATUS" => VariableValue::Indexed(self.pipestatus_entries()),
                "GROUPS" => VariableValue::Indexed(
                    self.groups_words()
                        .into_iter()
                        .enumerate()
                        .map(|(index, value)| (index as i64, value))
                        .collect(),
                ),
                "BASH_SOURCE" | "BASH_LINENO" | "BASH_ARGC" | "BASH_ARGV" => {
                    VariableValue::Indexed(
                        self.dynamic_stack_array_values(name)
                            .unwrap_or_default()
                            .into_iter()
                            .enumerate()
                            .map(|(index, value)| (index as i64, value))
                            .collect(),
                    )
                }
                _ => match declaration_scalar_rhs(&text, name) {
                    Some(value) => VariableValue::Scalar(value),
                    None => VariableValue::Unset,
                },
            };
            if matches!(value, VariableValue::Indexed(_)) {
                attrs.array = true;
            }
            return Some(VariableSnapshot {
                name: name.to_string(),
                value,
                attrs,
                dynamic: true,
                scope_depth: None,
                declaration: Some(text),
            });
        }
        // FUNCNAME has no dynamic declaration view (its scalar getter
        // is element 0, so the declaration match falls through), but
        // inside a function it is a first-class inspection target:
        // snapshot it from the composed stack view with the same
        // declaration shape the BASH_ARGC arm renders. At depth 0 the
        // array is invisible (variables.c:1812) and the name is
        // unbound here, as everywhere else.
        if name == "FUNCNAME" && self.shell_state.function_depth > 0 {
            let values = self
                .dynamic_stack_array_values("FUNCNAME")
                .unwrap_or_default();
            let mut attrs = merged_attrs(
                env_vars.attr_of(name),
                None,
                env_vars.is_marked(CAPCASE_VARS, name),
            );
            attrs.array = true;
            // The declaration goes through the shared printer over
            // the storage form, like every other array line.
            let storage = format_indexed_array_storage(
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| (index, value.clone()))
                    .collect(),
            );
            let mut out: Vec<u8> = Vec::new();
            let declaration = crate::builtins::declare::print_declaration(
                "FUNCNAME",
                &storage,
                decl_attrs_from(&attrs),
                0,
                &mut out,
            )
            .ok()
            .and_then(|()| String::from_utf8(out).ok())
            .map(|text| text.trim_end_matches('\n').to_string());
            return Some(VariableSnapshot {
                name: name.to_string(),
                value: VariableValue::Indexed(
                    values
                        .into_iter()
                        .enumerate()
                        .map(|(index, value)| (index as i64, value))
                        .collect(),
                ),
                attrs,
                dynamic: true,
                scope_depth: None,
                declaration,
            });
        }
        // Local scope home. The scope maps hold the SAVED outer
        // bindings (value, attrs, typed cell) for restore at frame
        // return; while a frame is live, the local's own value and
        // attributes live in the global stores — which is why the
        // engine's read path (shell_variable_value) never consults
        // the scopes. The maps' keys are therefore the record of
        // WHICH names are local-bound in which live scope, and the
        // binding itself is captured from the global stores below,
        // exactly as the debuggee resolves it.
        let scope_count = self.shell_state.local_typed_scopes.len();
        let scope_depth = (0..scope_count).rev().find_map(|index| {
            if self.shell_state.local_typed_scopes[index].contains_key(name)
                || self.shell_state.local_var_scopes[index].contains_key(name)
            {
                Some(scope_count - 1 - index)
            } else {
                None
            }
        });
        // The visible binding (a live local's value sits in the
        // global stores; a global binding is the same read).
        let typed = self.shell_state.variables.get(name);
        let mirror = env_vars.get(name).cloned().or_else(|| {
            // Materialize-on-named-access cells (variables.c
            // get_dirstack and friends): the storage arms of
            // parameter_array_storage build them on demand.
            if matches!(name, "BASH_ALIASES" | "BASH_CMDS" | "DIRSTACK") {
                self.parameter_array_storage(name)
                    .filter(|storage| !storage.is_empty())
            } else {
                None
            }
        });
        if mirror.is_none() && typed.is_none() {
            // Declared-but-unset globals have no cell in either
            // store; the declaration printer's rule (an attribute or
            // the declared-unset mark) is what makes them visible.
            let declaration = crate::builtins::declare::declaration_line(name, env_vars, false);
            return declaration.map(|text| VariableSnapshot {
                name: name.to_string(),
                value: VariableValue::Unset,
                attrs: merged_attrs(
                    env_vars.attr_of(name),
                    None,
                    env_vars.is_marked(CAPCASE_VARS, name),
                ),
                dynamic: false,
                scope_depth,
                declaration: Some(text),
            });
        }
        let attrs = merged_attrs(
            env_vars.attr_of(name),
            typed,
            env_vars.is_marked(CAPCASE_VARS, name),
        );
        // The declared-unset guard, identical to the read path's
        // (variable_state.rs, rubash#409): a declared-unset name has
        // an empty scalar materialized in the typed store, and that
        // placeholder must not read as a bound empty value while no
        // mirror cell exists.
        let value = if env_vars.attr_of(name).declared_unset && mirror.is_none() {
            VariableValue::Unset
        } else {
            match (typed, &mirror) {
                (Some(variable), _) => value_from_typed(&variable.value),
                (None, Some(text)) => value_from_mirror(env_vars, name, text, &attrs),
                (None, None) => VariableValue::Unset,
            }
        };
        Some(VariableSnapshot {
            name: name.to_string(),
            value,
            attrs,
            dynamic: false,
            scope_depth,
            declaration: crate::builtins::declare::declaration_line(name, env_vars, false),
        })
    }

    /// Capture the visible variable set: the `declare -p` listing's
    /// own name set (which already excludes `__RUBASH_*` internals,
    /// the §7.2 filter) merged with the listing's dynamic views and
    /// any local bindings, each captured through
    /// [`Executor::capture_variable`] so listing and named lookup
    /// can never resolve differently.
    pub(crate) fn capture_variables(&self) -> Vec<VariableSnapshot> {
        let env_vars = &self.shell_state.env_vars;
        let empty = std::collections::HashSet::new();
        let mut names = crate::builtins::declare::declaration_names_to_print(
            env_vars, false, false, false, false, false, false, false, false, &empty, &empty,
            &empty,
        );
        // The listing merges the dynamic views whose cells live
        // outside the table (declare_local.rs's listing rule).
        for name in ["PIPESTATUS", "BASH_SOURCE", "BASH_LINENO"] {
            if !names.iter().any(|listed| listed == name)
                && self.dynamic_declaration_view(name).is_some()
            {
                names.push(name.to_string());
            }
        }
        // Declared-but-unset globals have no table cell, and the
        // builtin's bare listing (whose name set this is) currently
        // omits them — GNU's listing includes them (variables.c
        // print_var_list walks every shell_variable). The inspection
        // set is the visible set (§7.2), so the declared-unset marks
        // merge in here; each name still renders through the shared
        // printer. The builtin's own listing is unchanged (recorded
        // as a divergence in the cluster REPORT, not silently
        // "fixed" from the view side).
        for name in env_vars.marked_names(DECLARED_UNSET_VARS) {
            if !name.starts_with("__RUBASH_") && !names.iter().any(|listed| listed == &name) {
                names.push(name);
            }
        }
        for scope in self.shell_state.local_typed_scopes.iter() {
            for name in scope.keys() {
                if !names.iter().any(|listed| listed == name) {
                    names.push(name.clone());
                }
            }
        }
        for scope in self.shell_state.local_var_scopes.iter() {
            for name in scope.keys() {
                if !names.iter().any(|listed| listed == name) {
                    names.push(name.clone());
                }
            }
        }
        let mut snapshots: Vec<VariableSnapshot> = names
            .iter()
            .filter_map(|name| self.capture_variable(name))
            .collect();
        snapshots.sort_by(|left, right| left.name.cmp(&right.name));
        snapshots
    }

    /// Capture the call stack as frames (debugger design §7.1): the
    /// zip of the FUNCNAME / BASH_SOURCE / BASH_LINENO dynamic views,
    /// with the script-main-frame rule (`has_script_main_frame`) —
    /// the same composition `caller` reads, so the two agree.
    pub(crate) fn capture_frames(&self) -> Vec<FrameSnapshot> {
        let funcnames = self
            .dynamic_stack_array_values("FUNCNAME")
            .unwrap_or_default();
        let sources = self
            .dynamic_stack_array_values("BASH_SOURCE")
            .unwrap_or_default();
        let linenos = self
            .dynamic_stack_array_values("BASH_LINENO")
            .unwrap_or_default();
        let current_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .cloned()
            .unwrap_or_else(|| "0".to_string());
        // Per-frame args from the BASH_ARGC/BASH_ARGV views. The
        // counts are top-aligned by construction (BASH_ARGC[0] is
        // the innermost frame's count), and the views may cover
        // fewer frames than FUNCNAME lists — the arg arrays omit
        // the bottom frame in some states — so slice the innermost
        // `argc_view.len()` frames and leave the rest arg-less.
        // Without materialization (no extdebug) the views are
        // empty and every frame simply carries no args.
        let argc_view = self.bash_argc_view();
        let argv_view = self.bash_argv_view();
        let mut frame_args: Vec<Vec<String>> = vec![Vec::new(); funcnames.len()];
        {
            let mut offset = 0usize;
            for (index, count_text) in argc_view.iter().enumerate() {
                if index >= funcnames.len() {
                    break;
                }
                let count: usize = count_text.parse().unwrap_or(0);
                let mut args: Vec<String> =
                    argv_view.iter().skip(offset).take(count).cloned().collect();
                // BASH_ARGV stores each frame's parameters reversed
                // (last argument at the lower index); call order is
                // the reverse of the stored slice.
                args.reverse();
                frame_args[index] = args;
                offset += count;
            }
        }
        let mut frames: Vec<FrameSnapshot> = funcnames
            .iter()
            .enumerate()
            .map(|(index, function)| {
                let line = if index == 0 {
                    current_line.clone()
                } else {
                    linenos.get(index - 1).cloned().unwrap_or_default()
                };
                FrameSnapshot {
                    function: function.clone(),
                    source: sources.get(index).cloned().unwrap_or_default(),
                    line,
                    args: frame_args.get(index).cloned().unwrap_or_default(),
                }
            })
            .collect();
        if frames.is_empty() && self.has_script_main_frame() {
            // Top level of a script file: no function is executing,
            // but the main frame the shell itself pushed
            // (shell.c:1635-1650 shell_execscript) is the frame a
            // `where` renders.
            frames.push(FrameSnapshot {
                function: "main".to_string(),
                source: sources.first().cloned().unwrap_or_default(),
                line: current_line,
                args: self.shell_state.positional_window().to_vec(),
            });
        }
        frames
    }

    /// Capture the dynamic inspection state (§7.3).
    pub(crate) fn capture_dynamic_state(&self) -> DynamicStateSnapshot {
        DynamicStateSnapshot {
            last_status: self.last_exit_code(),
            pipestatus: self
                .pipestatus_entries()
                .into_iter()
                .map(|(index, value)| (index, value.parse::<i32>().unwrap_or(0)))
                .collect(),
            bash_command: self.dynamic_parameter_value("BASH_COMMAND"),
            current_line: self
                .shell_state
                .env_vars
                .get("__RUBASH_CURRENT_LINE")
                .and_then(|value| value.parse::<u32>().ok()),
        }
    }

    /// Capture the whole inspection snapshot (§7 / WP-6).
    pub(crate) fn capture_inspection(&self) -> InspectionSnapshot {
        InspectionSnapshot {
            frames: self.capture_frames(),
            variables: self.capture_variables(),
            dynamic: self.capture_dynamic_state(),
        }
    }

    /// The value a `$name` read produces: the dynamic getter layered
    /// over the engine's variable read, exactly as the direct-read
    /// callers layer it (variable_state.rs), with the mirror as the
    /// final fallback.
    pub(crate) fn view_resolved_value(&self, name: &str) -> Option<String> {
        if let Some(value) = self.dynamic_parameter_value(name) {
            return Some(value);
        }
        if let Some(value) = self.shell_variable_value(name) {
            return Some(value);
        }
        self.get_env(name).map(str::to_string)
    }

    /// Every option in both families with its current state.
    pub(crate) fn capture_options(&self) -> Vec<OptionState> {
        let env = self.shell_state.env_vars.values_map();
        let mut options: Vec<OptionState> = crate::builtins::set::shell_option_names()
            .map(|name| OptionState {
                name: name.to_string(),
                family: OptionFamily::Shell,
                enabled: crate::builtins::set::shell_option_enabled(env, name),
            })
            .collect();
        options.extend(
            crate::builtins::shopt::SHOPT_OPTIONS
                .iter()
                .map(|name| OptionState {
                    name: name.to_string(),
                    family: OptionFamily::Shopt,
                    enabled: crate::builtins::shopt::option_enabled(env, name),
                }),
        );
        options
    }

    /// One option's state by name across both families. Flag-only
    /// shell options (GNU flags.c shell_flags[], rubash#2040) read
    /// through the same key the executor's restricted-shell consumers
    /// consult, keeping get/set symmetric over the flag surface.
    pub(crate) fn view_option_enabled(&self, name: &str) -> Option<bool> {
        let env = self.shell_state.env_vars.values_map();
        if self.is_shell_option(name) {
            Some(crate::builtins::set::shell_option_enabled(env, name))
        } else if crate::builtins::set::is_flag_only_shell_option(name) {
            Some(crate::builtins::set::shell_option_enabled(env, name))
        } else if crate::builtins::shopt::is_supported_option(name) {
            Some(crate::builtins::shopt::option_enabled(env, name))
        } else {
            None
        }
    }

    /// What children inherit (§5.3): exported names through the
    /// spawn paths' own exportability filter.
    pub(crate) fn view_environment(&self) -> Vec<(String, String)> {
        let mut env: Vec<(String, String)> =
            marked_env_names(&self.shell_state.env_vars, EXPORTED_VARS)
                .into_iter()
                .filter(|name| is_exportable_env_name(&self.shell_state.env_vars, name))
                .filter_map(|name| {
                    self.shell_state
                        .env_vars
                        .get(&name)
                        .map(|value| (name, value.clone()))
                })
                .collect();
        env.sort();
        env
    }

    /// Set or clear the exported attribute via the mark machinery
    /// the setattr path uses (builtins/setattr/marks.rs).
    pub(crate) fn view_set_exported(&mut self, name: &str, exported: bool) {
        if exported {
            self.mark_exported(name);
        } else {
            self.shell_state.env_vars.unmark_name(EXPORTED_VARS, name);
        }
    }

    /// Whole-value indexed-array bind through the engine's array
    /// state store (the storage form + typed sync every array write
    /// converges on).
    pub(crate) fn view_set_indexed_array(&mut self, name: &str, cells: &[(i64, String)]) -> bool {
        if cells.iter().any(|(index, _)| *index < 0)
            || is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name)
        {
            return false;
        }
        let entries: std::collections::BTreeMap<usize, String> = cells
            .iter()
            .map(|(index, value)| (*index as usize, value.clone()))
            .collect();
        let storage = format_indexed_array_storage(entries);
        // Binding a value retires the declared-unset bit alone —
        // NOT remove_unset_marks, which is unset's bulk clear of
        // every attribute bit and would take the ARRAY bit just set
        // with it.
        self.shell_state
            .env_vars
            .unmark_name(DECLARED_UNSET_VARS, name);
        self.shell_state
            .env_vars
            .insert(name.to_string(), storage.clone());
        mark_env_name(&mut self.shell_state.env_vars, ARRAY_VARS, name);
        sync_typed_indexed_storage(&mut self.shell_state.variables, name, &storage);
        true
    }

    /// Whole-value associative-array bind; same store contract as
    /// [`Executor::view_set_indexed_array`].
    pub(crate) fn view_set_assoc_array(&mut self, name: &str, cells: &[(String, String)]) -> bool {
        if is_marked_var(&self.shell_state.env_vars, ARRAY_VARS, name)
            && !is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name)
        {
            return false;
        }
        let storage = format_assoc_storage(cells.to_vec());
        // Same declared-unset retirement as the indexed bind (see
        // there): a single-bit unmark, never the bulk clear.
        self.shell_state
            .env_vars
            .unmark_name(DECLARED_UNSET_VARS, name);
        self.shell_state
            .env_vars
            .insert(name.to_string(), storage.clone());
        mark_env_name(&mut self.shell_state.env_vars, ASSOC_VARS, name);
        sync_typed_assoc_storage(&mut self.shell_state.variables, name, &storage);
        true
    }

    /// Unbind a variable with the unset builtin's leaf sequence
    /// (unset_arrays.rs): local bindings first, the att_nounset
    /// stack arrays refuse, dynamic names record their unbind, the
    /// mirror and the typed store both drop the name, and the
    /// PIPESTATUS cell resets. The builtin's diagnostics have no
    /// host-API form; refusal is the bool.
    pub(crate) fn view_unset_variable(&mut self, name: &str) -> bool {
        if matches!(
            name,
            "BASH_SOURCE" | "BASH_LINENO" | "BASH_ARGC" | "BASH_ARGV"
        ) {
            return false;
        }
        if self.unset_outer_local_variable(name) {
            return true;
        }
        if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, name) {
            return false;
        }
        let existed = self.shell_state.env_vars.contains_key(name)
            || self.shell_state.variables.get(name).is_some()
            || self.dynamic_parameter_is_set(name)
            || is_marked_var(&self.shell_state.env_vars, DECLARED_UNSET_VARS, name);
        if !existed {
            return false;
        }
        if self.dynamic_parameter_is_set(name)
            // Same DIRSTACK unbind recording as the unset builtin's
            // leaf sequence (unset_arrays.rs, rubash#2434).
            || (name == "DIRSTACK" && !self.dirstack_shadowed())
        {
            mark_env_name(&mut self.shell_state.env_vars, UNSET_DYNAMIC_VARS, name);
        }
        // Nameref: unbinding removes the referenced variable and
        // keeps the nameref (set.def:995-1000) — drop the target
        // from the typed owner as the builtin does.
        if is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, name) {
            if let Some(cell) = self
                .shell_state
                .env_vars
                .get(name)
                .filter(|cell| is_shell_name(cell))
                .cloned()
            {
                self.shell_state.variables.remove(cell.as_str());
            }
        }
        let mut sink: Vec<u8> = Vec::new();
        let _ = crate::builtins::set::unset_with_stderr(
            [name].into_iter(),
            &mut self.shell_state.env_vars,
            &mut sink,
        );
        self.shell_state.variables.remove(name);
        if name == "PIPESTATUS" {
            self.shell_state.pipestatus_shape = crate::shell::state::PipestatusShape::Absent;
        }
        if matches!(name, "IGNOREEOF" | "ignoreeof") {
            crate::builtins::set::sync_shell_option_flag(
                &mut self.shell_state.env_vars,
                "ignoreeof",
                false,
            );
        }
        true
    }

    /// Define or replace an alias in the store the `alias` builtin
    /// writes; BASH_ALIASES re-syncs from it.
    pub(crate) fn view_define_alias(&mut self, name: &str, value: &str) {
        self.shell_state
            .aliases
            .insert(name.to_string(), crate::builtins::alias::Alias::new(value));
        self.sync_dynamic_assoc_vars();
    }

    /// Remove an alias; false when it did not exist.
    pub(crate) fn view_undefine_alias(&mut self, name: &str) -> bool {
        let removed = self.shell_state.aliases.remove(name).is_some();
        if removed {
            self.sync_dynamic_assoc_vars();
        }
        removed
    }

    /// Remove a function definition and its bookkeeping stores;
    /// false when it did not exist.
    pub(crate) fn view_undefine_function(&mut self, name: &str) -> bool {
        if !self.shell_state.functions.contains_key(name) {
            return false;
        }
        self.shell_state.functions.remove(name);
        self.shell_state.function_definition_redirects.remove(name);
        self.shell_state.function_def_infos.remove(name);
        self.shell_state.function_definition_locations.remove(name);
        self.shell_state
            .env_vars
            .unmark_name(EXPORTED_FUNCTIONS, name);
        true
    }

    /// Move the session cwd through the `cd` builtin's own machinery
    /// (builtins/cd.def bindpwd via `execute_with_io` on the session
    /// cwd slot), then run cd's one variable sync point. The
    /// builtin's diagnostics go to a sink: the bool is the answer.
    pub(crate) fn view_set_cwd(&mut self, path: &std::path::Path) -> bool {
        let arg = path.to_string_lossy().into_owned();
        let mut stdout: Vec<u8> = Vec::new();
        let mut stderr: Vec<u8> = Vec::new();
        let status = crate::builtins::cd::execute_with_io(
            [arg.as_str()].into_iter(),
            &mut self.shell_state.env_vars,
            &mut self.shell_state.cwd,
            &mut stdout,
            &mut stderr,
        );
        match status {
            Ok(0) => {
                self.sync_cd_variables();
                true
            }
            _ => false,
        }
    }
}

/// A typed cell's value in snapshot form.
fn value_from_typed(value: &crate::shell::ShellValue) -> VariableValue {
    match value {
        crate::shell::ShellValue::Scalar(text) => VariableValue::Scalar(text.clone()),
        crate::shell::ShellValue::IndexedArray(entries) => VariableValue::Indexed(
            entries
                .iter()
                .map(|(index, value)| (*index, value.clone()))
                .collect(),
        ),
        crate::shell::ShellValue::AssociativeArray(entries) => {
            let mut entries: Vec<(String, String)> = entries
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            entries.sort();
            VariableValue::Associative(entries)
        }
    }
}
