use crate::executor::markers::STORAGE_WORD_PREFIX;

const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";
const UNSET_DYNAMIC_VARS: &str = "__RUBASH_UNSET_DYNAMIC_VARS";

/// GNU test.c:650 (unary `-v`): the operand goes through find_variable,
/// which follows nameref chains (variables.c:2011-2047
/// find_variable_nameref), so the verdict is the resolved TARGET's
/// set-ness, never the nameref cell's own existence (rubash#1707).
/// Returns Some(verdict) when the operand's base name is a nameref;
/// None when it is not (the caller's ordinary logic then applies —
/// including to the resolved name, which this helper substitutes by
/// recursing on the rewritten operand). Chain accounting mirrors the
/// executor's nameref_resolution: an empty/invalid/missing cell is
/// Unresolved, a chain that returns to its start or repeats a hop is
/// Circular, more than 8 hops is MaxDepth — all three read as unset.
/// A resolved target whose cell text itself carries a subscript
/// (`declare -n r='a[1]'`) is always unset here: -v tests the variable
/// find_variable returns for the target NAME and never decomposes the
/// target text into an element reference (oracle: `a=(x y);
/// declare -n r='a[1]'; [[ -v r ]]` is false even though a[1] is set),
/// while an operand subscript re-attaches to the resolved plain name
/// (`[[ -v r[1] ]]` with r->a tests a[1]).
fn dash_v_nameref_verdict(
    operand: &str,
    env_vars: &crate::shell::var_table::VarTable,
) -> Option<bool> {
    let (base, suffix) = match operand.find('[') {
        Some(index) => (&operand[..index], &operand[index..]),
        None => (operand, ""),
    };
    let namerefs = marked_vars(env_vars, NAMEREF_VARS);
    if !namerefs.iter().any(|marked| marked == base) {
        return None;
    }
    let mut current = base.to_string();
    for level in 1..=9 {
        if !namerefs.iter().any(|marked| marked == &current) {
            if current.contains('[') {
                return Some(false);
            }
            return Some(variable_is_set(&format!("{current}{suffix}"), env_vars));
        }
        if level > 8 {
            return Some(false);
        }
        let Some(cell) = env_vars.get(&current) else {
            return Some(false);
        };
        if cell.is_empty()
            || (!is_valid_variable_name(cell) && parse_array_subscript(cell).is_none())
        {
            return Some(false);
        }
        if cell == base || cell == &current {
            return Some(false);
        }
        current = cell.clone();
    }
    Some(false)
}

fn is_valid_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// Set-ness for `test -v` / `[[ -v ]]`, over the shell variable table
/// ONLY. GNU test.c:650-677 (unary `-v`) is `find_variable(arg)`; an
/// environment entry the shell never imported is unset even while the
/// raw process environment still carries it — GNU's importer
/// (variables.c:368 initialize_shell_variables) binds such entries into
/// the separate `invalid_env` table (variables.c:126, :3307
/// bind_invalid_envvar) that find_variable never consults. The process
/// environment is therefore never a fallback here (rubash#2073): this
/// engine's ingest drops undecodable entries the same way (rubash#2064),
/// so consulting `env::var_os` reported entries the shell does not hold,
/// including entries a host adds after the session was built.
pub(crate) fn variable_is_set(operand: &str, env_vars: &crate::shell::var_table::VarTable) -> bool {
    if let Some(verdict) = dash_v_nameref_verdict(operand, env_vars) {
        return verdict;
    }
    if let Some(name) = operand
        .strip_suffix("[@]")
        .or_else(|| operand.strip_suffix("[*]"))
    {
        let arrays = marked_vars(env_vars, ARRAY_VARS);
        let assocs = marked_vars(env_vars, ASSOC_VARS);
        if assocs.iter().any(|marked| marked == name) {
            // GNU test.c:664-681 unary_operator `-v` (test.def -v): the
            // operand already went through word expansion once, so
            // `a[@]`/`a[*]` arrive with AV_ALLOWALL. Under the default
            // level (`shell_compatibility_level > 51`) AV_ATSTARKEYS is
            // added and the subscript is a LITERAL key — `test -v 'm[@]'`
            // checks key `@`. Under compat (`<= 51`,
            // `arrayfunc.c:1511-1545 array_value_internal`) the whole-array
            // branch runs instead: any-element (non-empty) semantics.
            // Verified oracle matrix: default non-empty-no-@-key is false,
            // compat is true; empty is false in both; literal `@` key is
            // true in both. `[[ -v ]]` shares this function
            // (conditional.rs) and GNU agrees there too.
            let key = if operand.ends_with("[@]") { "@" } else { "*" };
            if crate::builtins::shopt::compatibility_level(env_vars) > 51 {
                return env_vars
                    .get(name)
                    .map(|value| assoc_key_is_set(value, key))
                    .unwrap_or(false);
            }
            return env_vars
                .get(name)
                .map(|value| assoc_is_nonempty(value))
                .unwrap_or(false);
        }
        if arrays.iter().any(|marked| marked == name) {
            return env_vars
                .get(name)
                .map(|value| !array_entries(value).is_empty())
                .unwrap_or(false);
        }
        return env_vars.contains_key(name);
    }

    // GNU test.c:650-668: a `[`-shaped operand that fails
    // valid_array_reference is NOT an array reference at all and falls
    // through to find_variable(arg), the literal-name lookup below. The
    // flag set reaching valid_array_reference is flag-0 for BOTH callers:
    // `test -v` passes AV_NOEXPAND (0x020), which sets none of the VA_*
    // bits the function consumes (arrayfunc.h:62 vs :69-70), and `[[ -v ]]`
    // forces array_expand_once off via set_expand_once(0,0)
    // (execute_cmd.c:4027). `a[80's]` (unterminated quote) and `F[]]`
    // (empty subscript) therefore report unset in every option state.
    let valid_arrayref = !operand.contains('[')
        || crate::executor::subscript_expansion::valid_array_reference_env(
            operand, false, false, env_vars,
        );
    if !valid_arrayref {
        return env_vars.contains_key(operand);
    }

    if let Some((name, subscript)) = parse_array_subscript(operand) {
        let arrays = marked_vars(env_vars, ARRAY_VARS);
        let assocs = marked_vars(env_vars, ASSOC_VARS);
        let Some(value) = env_vars.get(name) else {
            return subscript == "0" && dynamic_parameter_is_set(name, env_vars);
        };

        if assocs.iter().any(|marked| marked == name) {
            // The executor's -v pre-pass (rewrite_operand_array_subscript)
            // hex-encodes the resolved key behind the \x1e carrier so `]`,
            // `=` or quoting inside a key cannot corrupt the re-parse.
            let key = crate::executor::arithmetic::decode_arithmetic_assoc_key(subscript)
                .unwrap_or_else(|| subscript.to_string());
            return assoc_key_is_set(value, &key);
        }

        if arrays.iter().any(|marked| marked == name) || is_array_storage(value) {
            return crate::executor::arithmetic::eval_conditional_arith_value(subscript, env_vars)
                .and_then(|index| resolve_array_index(value, index))
                .map(|index| array_index_is_set(value, index))
                .unwrap_or(false);
        }

        return subscript == "0" && (!value.is_empty() || env_vars.contains_key(name));
    }

    let arrays = marked_vars(env_vars, ARRAY_VARS);
    let assocs = marked_vars(env_vars, ASSOC_VARS);
    if assocs.iter().any(|marked| marked == operand) {
        return env_vars
            .get(operand)
            .map(|value| assoc_key_is_set(value, "0"))
            .unwrap_or(false);
    }
    if arrays.iter().any(|marked| marked == operand) {
        return env_vars
            .get(operand)
            .map(|value| array_index_is_set(value, 0))
            .unwrap_or(false);
    }

    env_vars.contains_key(operand) || dynamic_parameter_is_set(operand, env_vars)
}

// GNU builtins/set.def:928: `unset -v` unbinds a dynamic variable like an
// ordinary one; the UNSET_DYNAMIC_VARS mark records that unbind so `-v`
// reports the name as unset.
fn dynamic_parameter_is_set(name: &str, env_vars: &crate::shell::var_table::VarTable) -> bool {
    if marked_vars(env_vars, UNSET_DYNAMIC_VARS)
        .iter()
        .any(|marked| marked == name)
    {
        return false;
    }
    matches!(
        name,
        "EPOCHSECONDS"
            | "EPOCHREALTIME"
            | "BASH_MONOSECONDS"
            | "SECONDS"
            | "RANDOM"
            | "SRANDOM"
            | "BASHPID"
            | "BASH_SUBSHELL"
            | "BASH_ARGV0"
            | "FUNCNAME"
            | "GROUPS"
            | "LINENO"
            | "BASH_COMMAND"
            | "SHELLOPTS"
            | "BASHOPTS"
            | "PIPESTATUS"
    )
}

fn parse_array_subscript(value: &str) -> Option<(&str, &str)> {
    let (name, subscript) = value.split_once('[')?;
    Some((name, subscript.strip_suffix(']')?))
}

fn is_array_storage(value: &str) -> bool {
    value.starts_with('(') && value.ends_with(')') || value.starts_with(STORAGE_WORD_PREFIX)
}

fn array_index_is_set(value: &str, index: usize) -> bool {
    array_entries(value)
        .into_iter()
        .any(|(entry_index, _)| entry_index == index)
}

fn resolve_array_index(value: &str, index: i128) -> Option<usize> {
    if index >= 0 {
        return usize::try_from(index).ok();
    }

    let max_index = array_entries(value)
        .into_iter()
        .map(|(entry_index, _)| entry_index)
        .max()?;
    let resolved = i128::try_from(max_index)
        .ok()?
        .checked_add(1)?
        .checked_add(index)?;
    usize::try_from(resolved).ok()
}

fn array_entries(value: &str) -> Vec<(usize, String)> {
    let value = value.strip_prefix(STORAGE_WORD_PREFIX).unwrap_or(value);
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Vec::new();
    };

    inner
        .split_whitespace()
        .enumerate()
        .filter_map(|(default_index, part)| {
            if let Some((left, right)) = part.split_once('=') {
                let index = left
                    .strip_prefix('[')
                    .and_then(|left| left.strip_suffix(']'))
                    .and_then(|index| index.parse::<usize>().ok())?;
                return Some((index, strip_array_value_quotes(right).to_string()));
            }
            Some((default_index, strip_array_value_quotes(part).to_string()))
        })
        .collect()
}

/// GNU `arrayfunc.c:1536-1545 array_value_internal()` compat branch
/// (`test -v 'm[@]'` with `shell_compatibility_level <= 51`): the
/// whole-assoc word list is non-empty iff the assoc has any entry.
fn assoc_is_nonempty(value: &str) -> bool {
    let value = value.strip_prefix(STORAGE_WORD_PREFIX).unwrap_or(value);
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return false;
    };

    split_storage_words(inner).any(|part| part.contains('='))
}

fn assoc_key_is_set(value: &str, key: &str) -> bool {
    let value = value.strip_prefix(STORAGE_WORD_PREFIX).unwrap_or(value);
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return false;
    };

    split_storage_words(inner).any(|part| {
        let Some((left, _)) = part.split_once('=') else {
            return false;
        };
        left.strip_prefix('[')
            .and_then(|left| left.strip_suffix(']'))
            .map(unquote_storage_value)
            .as_deref()
            == Some(key)
    })
}

fn split_storage_words(value: &str) -> impl Iterator<Item = String> + '_ {
    StorageWordIter {
        input: value,
        offset: 0,
    }
}

struct StorageWordIter<'a> {
    input: &'a str,
    offset: usize,
}

impl Iterator for StorageWordIter<'_> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(ch) = self.input.get(self.offset..)?.chars().next() {
            if !ch.is_ascii_whitespace() {
                break;
            }
            self.offset += ch.len_utf8();
        }

        let mut word = String::new();
        let mut in_double = false;
        let mut escaped = false;
        for (relative, ch) in self.input[self.offset..].char_indices() {
            if escaped {
                word.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' && in_double {
                word.push(ch);
                escaped = true;
                continue;
            }
            if ch == '"' {
                in_double = !in_double;
                word.push(ch);
                continue;
            }
            if ch.is_ascii_whitespace() && !in_double {
                self.offset += relative + ch.len_utf8();
                return Some(word);
            }
            word.push(ch);
        }
        self.offset = self.input.len();
        (!word.is_empty()).then_some(word)
    }
}

fn unquote_storage_value(value: &str) -> String {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        return value.to_string();
    };

    let mut unquoted = String::new();
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            unquoted.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            unquoted.push(ch);
        }
    }
    if escaped {
        unquoted.push('\\');
    }
    unquoted
}

fn strip_array_value_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
}

fn marked_vars(env_vars: &crate::shell::var_table::VarTable, key: &str) -> Vec<String> {
    // Rendered from the structured flags, not the serialized string
    // (writers stopped maintaining it per mark; #916).
    env_vars.marked_names(key)
}
