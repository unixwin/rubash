use super::*;

thread_local! {
    /// Side-effect writes from arithmetic evaluation in array subscripts
    /// (e.g. `count++` in `${arr[$((count++))]}`). Applied by the mutable
    /// caller after `expand_braced_indexed_parameter` returns.
    pub(in crate::executor) static PENDING_SUBSCRIPT_WRITES:
        std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// env_vars overlaid with the queued subscript writes, so a deferred
/// subscript evaluation sees the side effects earlier subscripts in the
/// same word produced. GNU evaluates each subscript against the live
/// environment, left-to-right (arrayfunc.c:1353 array_expand_index ->
/// evalexp); the deferred evaluators clone the env for `&self` callers, so
/// the clone must start from the accumulated writes, and the returned
/// write list then holds only that evaluation's own deltas.
pub(in crate::executor) fn env_vars_with_pending_subscript_writes<'a>(
    env_vars: &'a HashMap<String, String>,
) -> std::borrow::Cow<'a, HashMap<String, String>> {
    PENDING_SUBSCRIPT_WRITES.with(|pending| {
        let pending = pending.borrow();
        if pending.is_empty() {
            std::borrow::Cow::Borrowed(env_vars)
        } else {
            let mut overlaid = env_vars.clone();
            for (name, value) in pending.iter() {
                overlaid.insert(name.clone(), value.clone());
            }
            std::borrow::Cow::Owned(overlaid)
        }
    })
}

thread_local! {
    /// Per-`${}` memo for array-element expansion. GNU param_expand resolves
    /// a subscript once per parameter expansion (subst.c array_variable_part
    /// -> array_expand_index), but the `&self` walkers re-fetch the element
    /// through several helper layers — `${a[i],,}` reaches
    /// `array_element_parameter_value` 3-4 times with the same `name[sub]`
    /// text. Each frame maps the raw `name[sub]` expression to its result,
    /// so subscript side effects (`$((i++))`) run exactly once per `${}`
    /// expansion; distinct `name[sub]` texts never share an entry.
    static AEPV_MEMO: std::cell::RefCell<Vec<HashMap<String, Option<String>>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// RAII frame: pushes a memo scope on construction, pops on drop. The frame
/// is only pushed when the stack is empty — one memo per top-level `${}`
/// fragment. Nested `${}` inside the body share the outer frame (their
/// `name[sub]` keys differ, so there are no false hits) while repeated
/// probes of the same `${}` all dedup against it.
pub(in crate::executor) struct AepvMemoFrame {
    pushed: bool,
}

impl AepvMemoFrame {
    pub(in crate::executor) fn new() -> Self {
        let pushed = AEPV_MEMO.with(|memo| {
            let mut memo = memo.borrow_mut();
            if memo.is_empty() {
                memo.push(HashMap::new());
                true
            } else {
                false
            }
        });
        Self { pushed }
    }
}

impl Drop for AepvMemoFrame {
    fn drop(&mut self) {
        if self.pushed {
            AEPV_MEMO.with(|memo| {
                memo.borrow_mut().pop();
            });
        }
    }
}

/// RAII guard for command-substitution boundaries: GNU command_substitute
/// runs the body in a subshell, so arithmetic writes made inside `$(...)`
/// never reach the parent environment. The deferred subscript-write queue is
/// thread-local and would otherwise carry inner writes into the parent's
/// next flush; save+clear on entry, restore on drop.
pub(in crate::executor) struct PendingSubscriptWritesGuard {
    saved: Vec<(String, String)>,
}

impl PendingSubscriptWritesGuard {
    pub(in crate::executor) fn new() -> Self {
        let saved = PENDING_SUBSCRIPT_WRITES.with(|w| std::mem::take(&mut *w.borrow_mut()));
        Self { saved }
    }
}

impl Drop for PendingSubscriptWritesGuard {
    fn drop(&mut self) {
        PENDING_SUBSCRIPT_WRITES.with(|w| {
            *w.borrow_mut() = std::mem::take(&mut self.saved);
        });
    }
}

/// Look up the top memo frame only — a different `${}` expansion may share
/// the same `name[sub]` text and must evaluate it independently.
pub(in crate::executor) fn aepv_memo_lookup(expression: &str) -> Option<Option<String>> {
    AEPV_MEMO.with(|memo| {
        memo.borrow()
            .last()
            .and_then(|frame| frame.get(expression).cloned())
    })
}

pub(in crate::executor) fn aepv_memo_store(expression: &str, value: Option<String>) {
    AEPV_MEMO.with(|memo| {
        if let Some(frame) = memo.borrow_mut().last_mut() {
            frame.insert(expression.to_string(), value);
        }
    });
}

/// Site of one `${}` occurrence: (word-context id, fragment path). The
/// path is the sequence of `${` ordinals at each scan level — the first
/// top-level `${` is [0], a `${}` nested inside it at index k is [0,k] —
/// so two identical `${}` fragments, a nested `${}` inside a body, and a
/// different word all key differently. GNU param_expand resolves a
/// subscript once per `${}` expansion (arrayfunc.c:1353
/// array_expand_index -> evalexp); this site key identifies that
/// occurrence across the layered validate/pre-scan/real passes, which all
/// scan the same fragments in the same order.
type SubSite = (u64, Vec<usize>);

thread_local! {
    static SUB_RES_XPASS: std::cell::RefCell<Vec<HashMap<(SubSite, String), String>>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static SUB_IDX_XPASS: std::cell::RefCell<
        Vec<HashMap<(SubSite, String), crate::executor::subscript_expansion::IndexedSubscript>>,
    > = const { std::cell::RefCell::new(Vec::new()) };
    /// Word-context id for site keys — one per command-word expansion (and
    /// per non-command word entry such as an assignment RHS), so sibling
    /// words and re-executions of the same text never share entries.
    static CURRENT_WORD_CTX: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static NEXT_WORD_CTX: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
    /// Fragment path of the `${` currently being evaluated (see SubSite).
    static CURRENT_FRAG_PATH: std::cell::RefCell<Option<Vec<usize>>> =
        const { std::cell::RefCell::new(None) };
}

/// Allocate a fresh word-context id for site keys.
pub(in crate::executor) fn next_word_ctx() -> u64 {
    NEXT_WORD_CTX.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    })
}

/// RAII guard installing a word-context id; nested uses restore the
/// enclosing context on drop.
pub(in crate::executor) struct WordCtxGuard {
    saved: u64,
}

impl WordCtxGuard {
    /// Install `ctx` unconditionally.
    pub(in crate::executor) fn new(ctx: u64) -> Self {
        let saved = CURRENT_WORD_CTX.with(|current| current.replace(ctx));
        Self { saved }
    }

    /// Install a fresh context only when none is active — used by word
    /// expansion entries that may run either top-level or inside an
    /// already-scoped word (`${}` body expansions keep the parent's).
    pub(in crate::executor) fn new_if_absent() -> Option<Self> {
        if CURRENT_WORD_CTX.with(|current| current.get()) != 0 {
            return None;
        }
        Some(Self::new(next_word_ctx()))
    }
}

impl Drop for WordCtxGuard {
    fn drop(&mut self) {
        CURRENT_WORD_CTX.with(|current| current.set(self.saved));
    }
}

/// RAII frame: one cross-pass memo scope per word-expansion entry. The
/// map lives only while the word is expanded, so a later command (or loop
/// iteration) re-expanding the same word text cannot hit stale entries.
pub(in crate::executor) struct SubXpassFrame;

impl SubXpassFrame {
    pub(in crate::executor) fn new() -> Self {
        SUB_RES_XPASS.with(|maps| maps.borrow_mut().push(HashMap::new()));
        SUB_IDX_XPASS.with(|maps| maps.borrow_mut().push(HashMap::new()));
        Self
    }
}

impl Drop for SubXpassFrame {
    fn drop(&mut self) {
        SUB_RES_XPASS.with(|maps| {
            maps.borrow_mut().pop();
        });
        SUB_IDX_XPASS.with(|maps| {
            maps.borrow_mut().pop();
        });
    }
}

/// RAII guard extending the current fragment path with the `${` ordinal
/// `frag_index` within the scan that found it, for the duration of that
/// fragment's evaluation.
pub(in crate::executor) struct SubSiteGuard {
    saved: Option<Vec<usize>>,
}

impl SubSiteGuard {
    pub(in crate::executor) fn new(frag_index: usize) -> Self {
        let saved = CURRENT_FRAG_PATH.with(|path| {
            let mut path = path.borrow_mut();
            let saved = path.clone();
            match path.as_mut() {
                Some(current) => current.push(frag_index),
                None => *path = Some(vec![frag_index]),
            }
            saved
        });
        Self { saved }
    }
}

impl Drop for SubSiteGuard {
    fn drop(&mut self) {
        CURRENT_FRAG_PATH.with(|path| {
            *path.borrow_mut() = self.saved.clone();
        });
    }
}

pub(in crate::executor) fn sub_site_active() -> bool {
    CURRENT_FRAG_PATH.with(|path| path.borrow().is_some())
}

pub(in crate::executor) fn sub_site_key(text: &str) -> Option<(SubSite, String)> {
    let ctx = CURRENT_WORD_CTX.with(|current| current.get());
    if ctx == 0 {
        return None;
    }
    CURRENT_FRAG_PATH
        .with(|path| path.borrow().clone())
        .map(|path| ((ctx, path), text.to_string()))
}

pub(in crate::executor) fn sub_res_lookup(key: &(SubSite, String)) -> Option<String> {
    SUB_RES_XPASS.with(|maps| {
        maps.borrow()
            .iter()
            .rev()
            .find_map(|frame| frame.get(key).cloned())
    })
}

pub(in crate::executor) fn sub_res_store(key: (SubSite, String), resolved: String) {
    SUB_RES_XPASS.with(|maps| {
        if let Some(frame) = maps.borrow_mut().last_mut() {
            frame.insert(key, resolved);
        }
    });
}

pub(in crate::executor) fn sub_idx_lookup(
    key: &(SubSite, String),
) -> Option<crate::executor::subscript_expansion::IndexedSubscript> {
    SUB_IDX_XPASS.with(|maps| {
        maps.borrow()
            .iter()
            .rev()
            .find_map(|frame| frame.get(key).copied())
    })
}

pub(in crate::executor) fn sub_idx_store(
    key: (SubSite, String),
    result: crate::executor::subscript_expansion::IndexedSubscript,
) {
    SUB_IDX_XPASS.with(|maps| {
        if let Some(frame) = maps.borrow_mut().last_mut() {
            frame.insert(key, result);
        }
    });
}

impl Executor {
    /// Apply side-effect writes from arithmetic evaluation in array
    /// subscripts (e.g. `count++` in `${arr[$((count++))]}`) to the real
    /// env_vars. Called by the mutable parameter expansion path after
    /// `expand_braced_indexed_parameter` or `array_element_parameter_value`
    /// returns.
    pub(in crate::executor) fn apply_pending_subscript_writes(&mut self) {
        let writes = PENDING_SUBSCRIPT_WRITES.with(|w| std::mem::take(&mut *w.borrow_mut()));
        for (name, value) in writes {
            if self.shell_state.env_vars.get(&name) != Some(&value) {
                self.shell_state
                    .env_vars
                    .insert(name.clone(), value.clone());
                // Also sync to shell_state.variables, which is checked
                // first by shell_variable_value (variable_state.rs:178-184).
                if let Some(variable) = self.shell_state.variables.get_mut(&name) {
                    variable.value = crate::shell::ShellValue::Scalar(value);
                } else if !name.starts_with("__RUBASH_") {
                    let _ = self.shell_state.variables.set_scalar(&name, &value);
                }
            }
        }
    }

    pub(in crate::executor) fn expand_braced_indexed_parameter(
        &self,
        name: &str,
    ) -> Option<String> {
        if name == "DIRSTACK[@]" || name == "DIRSTACK[*]" {
            return Some(crate::builtins::pushd::stack_words(
                &self.shell_state.env_vars,
            ));
        }
        if let Some(index) = name
            .strip_prefix("DIRSTACK[")
            .and_then(|rest| rest.strip_suffix(']'))
            .and_then(|index| self.dirstack_subscript(index))
        {
            return Some(
                crate::builtins::pushd::stack_value(&self.shell_state.env_vars, index)
                    .unwrap_or_default(),
            );
        }
        if let Some(array_name) = name.strip_prefix('#').and_then(|name| {
            name.strip_suffix("[@]")
                .or_else(|| name.strip_suffix("[*]"))
        }) {
            if array_name == "GROUPS" {
                return Some(self.groups_words().len().to_string());
            }
            return Some(
                self.parameter_array_storage(array_name)
                    .map(|value| {
                        if is_marked_array_var(&self.shell_state.env_vars, array_name)
                            || is_array_storage(&value)
                        {
                            self.array_length(array_name)
                        } else {
                            1
                        }
                    })
                    .unwrap_or(0)
                    .to_string(),
            );
        }
        // `${#}` is the braced spelling of `$#`; `${##pat}` and `${#%pat}`
        // are prefix/suffix removal on `$#`, not length expansions. The
        // bare `${##}` however is the LENGTH of `$#` (exp.tests:376 prints
        // 1 with $#=5), like `${#!}` is the length of `$!`.
        if name == "#" {
            return Some(self.expand_parameter_named_value("#"));
        }
        if (name.starts_with("##") && name.len() > 2) || name.starts_with("#%") {
            return None;
        }
        if let Some(var_name) = name.strip_prefix('#') {
            return Some(self.expand_braced_length_parameter(var_name));
        }
        if let Some((var_name, offset, length)) = self.parse_parameter_substring(name) {
            return Some(self.expand_braced_substring_parameter(var_name, offset, length));
        }
        // GNU valid_array_reference only treats NAME[...] as an array
        // subscript when NAME is a valid identifier. A pattern-replacement
        // word like `z//[^;]` extracts array name "z//" here; without the
        // identifier check the `;` key failed arithmetic evaluation and
        // printed a spurious "z//: bad array subscript" (new-exp8.sub).
        if let Some((array_name, _)) = parse_array_subscript(name) {
            if is_shell_name(array_name) {
                return self.array_element_parameter_value(name);
            }
        }
        None
    }

    fn expand_braced_length_parameter(&self, var_name: &str) -> String {
        if matches!(var_name, "@" | "*") {
            return self.shell_state.positional_params.len().to_string();
        }
        if is_special_parameter_name(var_name) || var_name.parse::<usize>().is_ok() {
            return self
                .expand_parameter_named_value(var_name)
                .chars()
                .count()
                .to_string();
        }
        if let Some((array_name, index)) = parse_array_integer_subscript(var_name) {
            // GNU subst.c:7561-7569: negative subscripts to indexed arrays
            // count back from end; if still negative, err_badarraysub is
            // called and the expansion returns empty (array.tests:232-240
            // ${#xpath[-10]} with max_index=5 -> -4 -> bad array subscript,
            // echo produces an empty line, not "0").
            if let Some(value) = self.shell_state.env_vars.get(array_name) {
                if let Some(resolved) = resolve_indexed_array_subscript(value, index) {
                    if let Some(element) = array_value_at(value, resolved) {
                        return parameter_char_length(&element).to_string();
                    }
                } else if index < 0 {
                    eprintln!(
                        "{}[{}]: bad array subscript",
                        self.diagnostic_prefix(),
                        index
                    );
                    self.shell_state.arithmetic_nonfatal_error.set(true);
                    return String::new();
                }
            }
            return "0".to_string();
        }
        if let Some((array_name, index)) = parse_array_numeric_subscript(var_name) {
            return self
                .shell_state
                .env_vars
                .get(array_name)
                .and_then(|value| array_value_at(value, index))
                .map(|value| parameter_char_length(&value).to_string())
                .unwrap_or_else(|| "0".to_string());
        }
        if let Some((array_name, raw_key)) = parse_array_subscript(var_name) {
            if self.is_assoc_parameter_array(array_name) {
                let key = self.assoc_subscript_key(raw_key);
                // GNU subst.c:7545-7552: for assoc arrays, if the expanded
                // key is empty, report `bad array subscript` and return empty
                // (assoc.tests:99 ${#wheat[$unset]} with $unset empty).
                // GNU reports the raw (unexpanded) subscript text in the
                // error: `[$unset]: bad array subscript`.
                if key.is_empty() {
                    eprintln!(
                        "{}[{}]: bad array subscript",
                        self.diagnostic_prefix(),
                        raw_key
                    );
                    self.shell_state.arithmetic_nonfatal_error.set(true);
                    return String::new();
                }
                return self
                    .parameter_array_storage(array_name)
                    .and_then(|value| assoc_value_at(&value, &key))
                    .map(|value| parameter_char_length(&value).to_string())
                    .unwrap_or_else(|| "0".to_string());
            }
            // GNU subst.c param_expand: a subscript that is not a plain
            // integer is evaluated as an arithmetic expression
            // (array4.sub: ${#LNAME[$(( 0 ))]} -> length of LNAME[0]).
            // parse_array_integer_subscript / parse_array_numeric_subscript
            // only accept bare digit strings, so evaluate the subscript
            // here for scalar vars and indexed arrays alike.
            let index = if let Some(expr) = raw_key
                .strip_prefix("$((")
                .and_then(|e| e.strip_suffix("))"))
            {
                // `$((...))` keeps its own writes-capturing evaluation so
                // `count++` side effects survive the cloned env.
                let overlaid = env_vars_with_pending_subscript_writes(&self.shell_state.env_vars);
                let (result, writes) =
                    eval_conditional_arith_value_with_writes(expr.trim(), &overlaid);
                if !writes.is_empty() {
                    PENDING_SUBSCRIPT_WRITES.with(|w| {
                        w.borrow_mut().extend(writes);
                    });
                }
                result
            } else {
                // GNU array_expand_index -> evalexp (arrayfunc.c:1353-1391):
                // one expand_subscript_string pass (Raw), then no-expand
                // evaluation — a surviving $name/$(...) is "operand
                // expected" and evalerror aborts the command list.
                match self.eval_indexed_subscript_deferred(SubscriptSource::Raw(raw_key)) {
                    IndexedSubscript::Index(index) => Some(index),
                    // Empty (${#a[]}) and Error both abandon the expansion;
                    // GNU reports the error / drops the word and produces
                    // empty output (bad substitution / operand expected).
                    IndexedSubscript::Empty | IndexedSubscript::Error => None,
                }
            };
            if let Some(index) = index {
                if let Some(value) = self.shell_state.env_vars.get(array_name) {
                    if let Some(resolved) = resolve_indexed_array_subscript(value, index) {
                        if let Some(element) = array_value_at(value, resolved) {
                            return parameter_char_length(&element).to_string();
                        }
                    }
                    // Scalar variable: ${#scalar[$(( 0 ))]} is the length
                    // of the scalar value (GNU variables.c: a scalar is a
                    // one-element array at index 0).
                    if !is_array_storage(value) && index == 0 {
                        if let Some(scalar) = self.scalar_parameter_value(array_name, value) {
                            return parameter_char_length(&scalar).to_string();
                        }
                    }
                }
            }
            return "0".to_string();
        }
        if let Some(value) = self.dynamic_parameter_value(var_name) {
            return parameter_char_length(&value).to_string();
        }
        // GNU subst.c ${#name} over a nameref: the length of the value the
        // reference resolves to. A cell naming a variable resolves through
        // it (an array contributes element 0); a cell naming an array
        // element contributes that element; an invalid cell falls back to
        // the length of the cell string itself
        // (nameref24.sub: name4 -> 'aa&bb' prints 5, name2 -> unset prints 0).
        if is_marked_var(&self.shell_state.env_vars, NAMEREF_VARS, var_name) {
            let cell = self
                .shell_state
                .env_vars
                .get(var_name)
                .cloned()
                .unwrap_or_default();
            if is_shell_name(&cell) {
                if let Some(target_value) = self.shell_state.env_vars.get(&cell) {
                    if is_array_storage(target_value) {
                        let element_zero = if self.is_assoc_parameter_array(&cell) {
                            assoc_value_at(target_value, "0")
                        } else {
                            array_value_at(target_value, 0)
                        };
                        return element_zero
                            .map(|value| parameter_char_length(&value).to_string())
                            .unwrap_or_else(|| "0".to_string());
                    }
                    return parameter_char_length(&target_value).to_string();
                }
                if let Some(crate::shell::Variable {
                    value: crate::shell::ShellValue::Scalar(scalar),
                    ..
                }) = self.shell_state.variables.get(&cell)
                {
                    return parameter_char_length(&scalar).to_string();
                }
                return "0".to_string();
            }
            if let Some((array_name, key)) = parse_array_subscript(&cell) {
                if self.is_assoc_parameter_array(array_name) {
                    let key = self.assoc_subscript_key(key);
                    return self
                        .parameter_array_storage(array_name)
                        .and_then(|value| assoc_value_at(&value, &key))
                        .map(|value| parameter_char_length(&value).to_string())
                        .unwrap_or_else(|| "0".to_string());
                }
                if let Some(index) = key.parse::<usize>().ok() {
                    return self
                        .shell_state
                        .env_vars
                        .get(array_name)
                        .and_then(|value| array_value_at(value, index))
                        .map(|value| parameter_char_length(&value).to_string())
                        .unwrap_or_else(|| "0".to_string());
                }
                return "0".to_string();
            }
            return parameter_char_length(&cell).to_string();
        }
        self.shell_state
            .env_vars
            .get(var_name)
            .map(|value| {
                if is_array_storage(value) {
                    let element_zero = if self.is_assoc_parameter_array(var_name) {
                        assoc_value_at(value, "0")
                    } else {
                        array_value_at(value, 0)
                    };
                    element_zero
                        .map(|value| parameter_char_length(&value).to_string())
                        .unwrap_or_else(|| "0".to_string())
                } else {
                    parameter_char_length(&value).to_string()
                }
            })
            .unwrap_or_else(|| "0".to_string())
    }

    pub(in crate::executor) fn expand_braced_substring_parameter(
        &self,
        var_name: &str,
        offset: isize,
        length: Option<isize>,
    ) -> String {
        if let Some(value) = self.indirect_substring_parameter(var_name, offset, length) {
            return value;
        }
        if matches!(var_name, "@" | "*") {
            let separator = if var_name == "*" {
                self.ifs_first_char_separator()
            } else {
                " ".to_string()
            };
            return positional_parameter_substring_with_zero(
                &self.shell_state.positional_params,
                &self.script_name_value(),
                offset,
                length,
            )
            .join(&separator);
        }
        if let Some(array_name) = var_name
            .strip_suffix("[@]")
            .or_else(|| var_name.strip_suffix("[*]"))
        {
            return self
                .array_subscript_range_values(
                    array_name,
                    offset,
                    length.and_then(|length| usize::try_from(length).ok()),
                )
                .map(|values| {
                    let values = values
                        .into_iter()
                        .map(normalize_array_expanded_value)
                        .collect::<Vec<_>>();
                    self.join_expanded_array_values(values, var_name)
                })
                .unwrap_or_default();
        }
        if let Some(value) = self.array_element_parameter_value(var_name) {
            return parameter_substring(&value, offset, length);
        }
        if is_shell_name(var_name) {
            // GNU variables.c get_string_value returns element [0] when a
            // bare array name is expanded without a subscript, so the slice
            // must operate on element 0 rather than the raw typed array
            // storage (probe: av=(abcd efgh); ${av:1:2} -> "bc").
            return self
                .parameter_pattern_scalar_value(var_name)
                .map(|value| parameter_substring(&value, offset, length))
                .unwrap_or_default();
        }
        String::new()
    }

    fn indirect_substring_parameter(
        &self,
        var_name: &str,
        offset: isize,
        length: Option<isize>,
    ) -> Option<String> {
        let indirect_name = var_name.strip_prefix('!')?;
        if let Some(target_name) = self.nameref_target_name(indirect_name) {
            return Some(parameter_substring(&target_name, offset, length));
        }

        let target_expr = self.shell_state.env_vars.get(indirect_name)?;
        if target_expr.ends_with("[@]") || target_expr.ends_with("[*]") {
            let values = slice_array_values(
                self.indirect_target_values(target_expr),
                offset,
                length.and_then(|length| usize::try_from(length).ok()),
            );
            return Some(self.join_expanded_array_values(values, target_expr));
        }

        let value = self
            .indirect_target_values(target_expr)
            .into_iter()
            .next()?;
        Some(parameter_substring(&value, offset, length))
    }
}

/// GNU subst.c ${#name} length: MB_STRLEN over the value (subst.c:8308) with
/// the locale byte walk of lib/sh/utf8.c:167-184 utf8_mbstrlen.
///
/// In a multibyte locale this is one character per valid multibyte sequence,
/// one per byte of an invalid sequence (MB_INVALIDCH -> clen = 1). In a
/// single-byte locale MB_CUR_MAX is 1 and every byte is one character,
/// including a byte that only starts a multibyte sequence, so intl1.sub
/// reports 31/30 instead of 16/15 once setlocale() has fallen back to the C
/// locale.
///
/// Raw bytes >= 0x80 travel inside rubash words as U+E000 marker pairs
/// (substitution_metadata), so the value must be decoded to its byte view
/// before counting (intl.tests: a=$'\303\251' is 1, not the 4 chars of two
/// marker pairs).
fn parameter_char_length(value: &str) -> usize {
    let sentinel = char::from_u32(crate::executor::substitution_metadata::RAW_BYTE_MARKER_ESCAPE)
        .expect("raw-byte sentinel is a valid char");
    let bytes = if value.contains(sentinel) {
        crate::executor::substitution_metadata::decode_raw_byte_markers(value.as_bytes())
    } else {
        value.as_bytes().to_vec()
    };

    if !crate::locale::is_multi_byte() {
        return bytes.len();
    }

    let mut count = 0usize;
    let mut rest: &[u8] = &bytes;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                count += text.chars().count();
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if let Ok(text) = std::str::from_utf8(&rest[..valid]) {
                    count += text.chars().count();
                }
                count += 1; // utf8.c:177-178: an invalid byte counts as one
                rest = &rest[valid + 1..];
            }
        }
    }
    count
}
