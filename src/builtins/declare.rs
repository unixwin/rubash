//! declare module.
//!
//! GNU Bash source ownership:
// - builtins/declare.def

use std::io::{self, Write};

mod assign;
mod attrs;
mod marks;
mod names;
mod output;
mod print;
pub(in crate::builtins) mod storage;

pub(crate) use output::{print_declaration, DeclarationAttrs};
pub(crate) use print::{declaration_line, declaration_names_to_print};

/// A dynamic variable's declaration rendering, supplied by the executor
/// for a name whose value lives outside the variable table
/// (rubash#1363). `integer`/`array` carry the attribute class the
/// listing filters adjudicate — in GNU the INIT_DYNAMIC_VAR cell's own
/// attributes (variables.c:1844 initialize_dynamic_variables), which
/// the mark sets cannot express for provider-owned names: the integer
/// dynamics (RANDOM/SRANDOM/SECONDS/BASHPID/HISTCMD) carry no
/// INTEGER_VARS mark and the array dynamics no ARRAY_VARS mark, so a
/// mark-driven listing silently dropped them from `declare -i` /
/// `declare -a` (rubash#2700).
#[derive(Debug, Clone)]
pub(crate) struct DynamicDeclarationView {
    pub name: String,
    pub text: String,
    pub integer: bool,
    pub array: bool,
}

use crate::executor::builtin_diag::{Diag, DiagKind, PrologParts};
use crate::executor::builtin_opt::{BuiltinOpt, OptEvent};
use crate::executor::builtin_table::builtin_table_row;
use crate::shell::VariableStore;
use assign::{assign_declare_names, LocaleFunnelRecord};
use attrs::{apply_declare_attrs, DeclareOptions};
use marks::unmark_typed;
use names::{
    check_selfref, declare_base_name, valid_array_reference, valid_declare_name, valid_identifier,
    valid_nameref_value,
};
use storage::{format_array_value, format_assoc_value, indexed_array_entries, parse_assoc_words};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;

/// Format an associative array storage value for `set` output
/// (variables.c:1096 print_assignment -> arrayfunc.c:1257 print_assoc_assignment).
/// Used by the `set` builtin to print assoc arrays as `name=(["key"]="value" )`
/// instead of `name='(...)'`.
pub(crate) fn format_assoc_for_output(value: &str, nbuckets: usize) -> String {
    format_assoc_value(value, nbuckets)
}

/// Format an indexed array storage value for `set` output
/// (variables.c:1096 print_assignment -> arrayfunc.c:1239 print_array_assignment).
/// Used by the `set` builtin to print indexed arrays as `name=([0]="value" )`
/// instead of `name='(...)'`.
pub(crate) fn format_array_for_output(value: &str) -> String {
    format_array_value(value)
}

/// Create an associative array from a compound assignment value
/// (arrayfunc.c:630 assign_assoc_from_kvlist). Used by `readonly -A` and
/// `export` when they need to create an associative array from a kvpair
/// compound assignment like `( one 1 two 2 three 3 )`.
pub(crate) fn create_assoc_from_compound(value: &str) -> String {
    storage::append_assoc_value(
        "()",
        value,
        false,
        &crate::shell::var_table::VarTable::default(),
    )
}
const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";
const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
const ASSOC_128_VARS: &str = "__RUBASH_ASSOC_128_VARS";
const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
const UPPERCASE_VARS: &str = "__RUBASH_UPPERCASE_VARS";
const LOWERCASE_VARS: &str = "__RUBASH_LOWERCASE_VARS";
const CAPCASE_VARS: &str = "__RUBASH_CAPCASE_VARS";
const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";
const TRACE_VARS: &str = "__RUBASH_TRACE_VARS";
const DECLARED_UNSET_VARS: &str = "__RUBASH_DECLARED_UNSET_VARS";
use crate::executor::markers::STORAGE_WORD_PREFIX;
use crate::executor::types::COMPOUND_ASSIGNMENT_MARKER;
const EX_USAGE: i32 = 2;

/// The flush prolog for this family's `Diag` handles is composed
/// from the variable table at the diagnostic site
/// (`PrologParts::build`): the executor crossings hand in
/// `PrologParts::Executor` (their own flavor, deferred), this
/// module's non-executor entry hands in `PrologParts::Lib`, so
/// every line of one call carries one prolog and a
/// diagnostic-free call builds none (E4).

/// A diagnostic line that carries the prolog but NO builtin name —
/// the `error.c` report family (`err_badarraysub`, the unqualified
/// `warning:` lines). GNU writes these through `error.c`'s own
/// prefix, not `builtin_error`. Recorded diagnostics are flushed
/// first so the line keeps its position relative to the standard
/// `{prolog}{name}:` skeleton lines queued on the same handle.
pub(super) fn bare_diag_line(
    diag: &mut Diag,
    prolog: &PrologParts,
    variables: &crate::shell::var_table::VarTable,
    text: &str,
) {
    let built = prolog.build(variables);
    diag.flush_with(|| built.clone());
    let head = built.render();
    diag.sink().write_str(&format!("{head}{text}\n"));
}

/// Written spellings of declare operands whose subscripts the executor's
/// declare pass already evaluated, keyed by the operand's REWRITTEN
/// `name[subscript]` prefix (see `element_word_prefix`) with one queue
/// entry per occurrence. GNU's bad-subscript diagnostic (error.c:440-443
/// err_badarraysub) prints the operand as the user wrote it (`a[abc]`),
/// but by the time the builtin sees the operand the subscript has been
/// replaced by its value (`a[-5]`). The executor is the one place that
/// still holds both forms (positionally aligned right after the
/// rewrite), so it hands the pairing down and the builtin pops the
/// written form for exactly the operand it is failing on. Keying by the
/// `name[subscript]` prefix rather than the whole operand keeps the
/// pairing valid across the executor's later value-side rewrites
/// (integer RHS evaluation replaces the text after the `=`).
pub(crate) type WrittenOperandHints =
    std::collections::HashMap<String, std::collections::VecDeque<String>>;

/// The `name[...]` prefix of an operand: scan from the first `[` to its
/// matching `]`, counting nested pairs and skipping quoted/escaped
/// characters (the same span rule as GNU parse_matched_pair P_ARRAYSUB,
/// parse.y:3906). The append `+` and the `=value` tail are not part of
/// the prefix (`a[abc]+=x` keys and reports as `a[abc]`).
pub(crate) fn element_word_prefix(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let open = text.find('[')?;
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 1,
            b'\'' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'\'' {
                    i += 1;
                }
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += 1;
                }
            }
            b'[' => depth += 1,
            b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(text[..=i].to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Synchronize indexed declarations into the typed variable owner after the
/// legacy builtin path has applied its attribute and encoding rules.
pub(crate) fn sync_typed_assignments(
    args: &[String],
    variables: &crate::shell::var_table::VarTable,
    store: &mut VariableStore,
) {
    let mut parse_options = true;
    for arg in args {
        if parse_options && arg == "--" {
            parse_options = false;
            continue;
        }
        let Some((raw_name, _)) = arg.split_once('=') else {
            continue;
        };
        let name = raw_name.strip_suffix('+').unwrap_or(raw_name);
        let Some(base) = declare_base_name(name) else {
            continue;
        };
        let Some(value) = variables.get(base) else {
            continue;
        };
        if variables.is_marked(DECLARED_UNSET_VARS, base) {
            // A declared-but-unset variable has no value: its mirror
            // entry is an empty placeholder, and parsing it as an
            // implicit scalar would seed a phantom [0]="" element the
            // mirror does not have (GNU: the variable simply has no
            // cell contents — ${#a[@]} is 0). It reaches this sync when
            // an element bind FAILED and left it unset (rubash#2462);
            // any typed cell is stale by definition, so drop it.
            if variables.is_marked(ARRAY_VARS, base) {
                crate::executor::drop_typed_indexed_array(store, base);
            }
            continue;
        }
        if variables.is_marked(ARRAY_VARS, base) {
            let entries = indexed_array_entries(value);
            // rubash#616: preserve sparse keys — a values-only replace
            // enumerates densely, so `declare -a a=([5]=x [2]=y)` seeded
            // {0,1} and the element-write append fast path misfired on
            // index==max+1, duplicating the segment (GNU array.c
            // array_insert keeps the real index).
            let keyed = entries
                .into_iter()
                .map(|(index, value)| (i64::try_from(index).unwrap_or(i64::MAX), value));
            if store.replace_indexed_array_entries(base, keyed).is_err() {
                crate::executor::drop_typed_indexed_array(store, base);
            }
        } else if variables.is_marked(ASSOC_VARS, base) {
            let _ = store.replace_associative_array(base, parse_assoc_words(value));
        }
    }
}

/// Synchronize the final declare attribute markers into the typed variable owner.
pub(crate) fn sync_typed_attributes(
    args: &[String],
    variables: &crate::shell::var_table::VarTable,
    store: &mut VariableStore,
) {
    // A case attribute in THIS command rewrites stored values in the
    // mirror (attrs.rs apply_varcasemod pass), not just the marks: an
    // already-shaped typed cell must be refreshed from the mirror or
    // the two owners disagree (rubash#1398: `declare -A h=([k]=v)` in
    // an earlier command, then `declare -u h`, left the cell holding
    // the unfolded values).
    let case_touched = {
        let mut parse_options = true;
        let mut found = false;
        for arg in args {
            if parse_options && arg == "--" {
                parse_options = false;
                continue;
            }
            if parse_options
                && arg.starts_with('-')
                && arg.len() > 1
                && arg[1..].chars().all(|c| c.is_ascii_alphabetic())
            {
                if arg[1..].contains(['u', 'l', 'c']) {
                    found = true;
                }
                continue;
            }
            break;
        }
        found
    };

    for arg in args {
        let raw_name = arg.split_once('=').map(|(name, _)| name).unwrap_or(arg);
        let name = raw_name.strip_suffix('+').unwrap_or(raw_name);
        let Some(base) = declare_base_name(name) else {
            continue;
        };
        // If the variable doesn't exist in the typed store, create it from env_vars.
        if store.get(base).is_none() {
            let value = variables.get(base).cloned().unwrap_or_default();
            let _ = store.set_scalar(base, value);
        } else if case_touched
            && !variables.is_marked(ARRAY_VARS, base)
            && !variables.is_marked(ASSOC_VARS, base)
            && matches!(
                store.get(base).map(|v| &v.value),
                Some(crate::shell::ShellValue::Scalar(_))
            )
        {
            // Case conversion rewrote the scalar's mirror value without
            // an assignment operand (rubash#1398 scalar leg).
            if let Some(value) = variables.get(base) {
                if let Some(variable) = store.get_mut(base) {
                    variable.value = crate::shell::ShellValue::Scalar(value.clone());
                }
            }
        } else if arg.contains('=') {
            // When there's an assignment in a declare/typed command and the
            // variable already exists in the typed store, update its value
            // to match env_vars (this handles local shadowing of parent vars).
            if let Some(value) = variables.get(base) {
                if let Some(variable) = store.get_mut(base) {
                    // Update the scalar value while preserving the variable's type state.
                    variable.value = crate::shell::ShellValue::Scalar(value.clone());
                }
            }
        }
        if variables.is_marked(ARRAY_VARS, base)
            && (case_touched
                || !matches!(
                    store.get(base).map(|v| &v.value),
                    Some(crate::shell::ShellValue::IndexedArray(_))
                ))
        {
            // GNU arrayfunc.c convert_var_to_array: a scalar's value moves to
            // element 0 when the variable gains att_array — with
            // localvar_inherit that scalar is the inherited caller's value
            // (`declare -a v` on v=7 keeps [0]="7"). Serialized storage text
            // is parsed back into entries.
            let env_value = variables.get(base).cloned().unwrap_or_default();
            if env_value.starts_with(STORAGE_WORD_PREFIX)
                || (env_value.starts_with('(') && env_value.ends_with(')'))
            {
                // Parsed back with keys intact (same sparse-cell reason as
                // above: a values-only replace would seed {0,1} for
                // `([5]=x [2]=y)`). The scalar/empty arms below stay dense,
                // which is correct there (a scalar moves to element 0).
                let keyed = indexed_array_entries(&env_value)
                    .into_iter()
                    .map(|(index, value)| (i64::try_from(index).unwrap_or(i64::MAX), value));
                if store.replace_indexed_array_entries(base, keyed).is_err() {
                    crate::executor::drop_typed_indexed_array(store, base);
                }
            } else {
                let elements: Vec<String> = if env_value.is_empty() {
                    Vec::new()
                } else {
                    vec![env_value]
                };
                crate::executor::replace_typed_indexed_array(store, base, elements);
            }
        } else if variables.is_marked(ASSOC_VARS, base)
            && (case_touched
                || !matches!(
                    store.get(base).map(|v| &v.value),
                    Some(crate::shell::ShellValue::AssociativeArray(_))
                ))
        {
            // GNU arrayfunc.c:111-140 convert_var_to_assoc: a scalar's value
            // moves to element "0" when the variable gains att_assoc —
            // `declare -A v` on an inherited scalar v=7 keeps [0]="7" so a
            // later `v+=(1 one)` merges instead of replacing. Serialized
            // assoc storage text is parsed back into entries.
            let env_value = variables.get(base).cloned().unwrap_or_default();
            let entries: Vec<(String, String)> = if env_value.starts_with(STORAGE_WORD_PREFIX)
                || (env_value.starts_with('(') && env_value.ends_with(')'))
            {
                parse_assoc_words(
                    env_value
                        .strip_prefix(STORAGE_WORD_PREFIX)
                        .unwrap_or(&env_value),
                )
            } else if env_value.is_empty() {
                Vec::new()
            } else {
                vec![("0".to_string(), env_value)]
            };
            let _ = store.replace_associative_array(base, entries);
        }
        if let Some(variable) = store.get_mut(base) {
            variable.exported = variables.is_marked(EXPORTED_VARS, base);
            variable.readonly = variables.is_marked(READONLY_VARS, base);
            variable.integer = variables.is_marked(INTEGER_VARS, base);
            variable.uppercase = variables.is_marked(UPPERCASE_VARS, base);
            variable.lowercase = variables.is_marked(LOWERCASE_VARS, base);
            variable.nameref = if variables.is_marked(NAMEREF_VARS, base) {
                arg.split_once('=').map(|(_, value)| value.to_string())
            } else {
                None
            };
        }
    }
}

pub fn execute(
    args: &[String],
    variables: &mut crate::shell::var_table::VarTable,
) -> io::Result<i32> {
    let mut stdout = crate::executor::GlobalStdout;
    let mut stderr = io::stderr();
    let mut oob_word = None;
    let status = execute_with_io(args, variables, &mut stdout, &mut stderr, &mut oob_word)?;
    // No abort channel here (test-only path in practice): preserve the
    // failure as status so a future caller cannot mistake it for success.
    if oob_word.is_some() {
        return Ok(EXECUTION_FAILURE);
    }
    Ok(status)
}

/// Minimal flag scan used by the executor to decide whether a declare/typeset
/// invocation follows nameref chains (declare.def:704-806 applies when the
/// command has neither -n nor +n among its options).
pub(crate) fn declare_nameref_flags(args: &[String]) -> (bool, bool) {
    let mut nameref = false;
    let mut unset_nameref = false;
    let mut parse_options = true;
    for arg in args {
        if parse_options && arg == "--" {
            parse_options = false;
            continue;
        }
        if parse_options
            && (arg.starts_with('-') || arg.starts_with('+'))
            && arg != "-"
            && arg != "+"
        {
            let set_attr = arg.starts_with('-');
            for option in arg[1..].chars() {
                match option {
                    'n' if set_attr => nameref = true,
                    'n' => unset_nameref = true,
                    _ => {}
                }
            }
            continue;
        }
        break;
    }
    (nameref, unset_nameref)
}

/// Resolve the nameref assignment targets for an upcoming declare invocation:
/// for every `name=value` argument whose base is a marked nameref, returns
/// `(base, final_target)` from declare_nameref_chain so the executor can
/// mirror the new target value into the typed owner (variables.c:2051
/// find_variable_last_nameref traversal).
pub(crate) fn nameref_assignment_targets(
    args: &[String],
    variables: &crate::shell::var_table::VarTable,
) -> Vec<(String, String)> {
    let mut targets = Vec::new();
    for arg in args {
        let Some((raw_lhs, _)) = arg.split_once('=') else {
            continue;
        };
        let lhs = raw_lhs.strip_suffix('+').unwrap_or(raw_lhs);
        let Some(base) = declare_base_name(lhs) else {
            continue;
        };
        if let Some((_, target)) = declare_nameref_chain(variables, base) {
            targets.push((base.to_string(), target));
        }
    }
    targets
}

/// GNU variables.c:2051 find_variable_last_nameref: walk the nameref chain
/// while each cell is itself a nameref value (identifier or array reference)
/// and return `(last_nameref_name, final_target_name)`. Returns None when
/// NAME is not a nameref or the chain does not resolve to a variable-like
/// cell (GNU then keeps operating on the nameref itself).
fn declare_nameref_chain(
    variables: &crate::shell::var_table::VarTable,
    name: &str,
) -> Option<(String, String)> {
    if !variables.is_marked(NAMEREF_VARS, name) {
        return None;
    }
    let mut last = name.to_string();
    let mut current = name.to_string();
    for _ in 0..16 {
        if !variables.is_marked(NAMEREF_VARS, &current) {
            break;
        }
        let cell = variables.get(&current)?.clone();
        if !names::valid_nameref_value(&cell) {
            return None;
        }
        last = current;
        current = cell;
    }
    if last == current {
        return None;
    }
    Some((last, current))
}

/// GNU declare.def:623-640 declare_transform_name: at function scope every
/// operand (bare or `name=value`) resolves through namerefs to the name that
/// make_local_variable actually localizes -- `declare -a ref` on ref->var
/// creates a local array `var`, and a bare `declare ref` creates local `var`.
/// Returns the resolved operand names (bare names of `x[i]` cells included).
pub(crate) fn nameref_resolved_operand_names(
    args: &[String],
    variables: &crate::shell::var_table::VarTable,
) -> Vec<String> {
    let mut resolved = Vec::new();
    for arg in args {
        if arg == "--" || arg == "-" || arg == "+" {
            continue;
        }
        if arg.starts_with('-') || arg.starts_with('+') {
            continue;
        }
        let (raw_lhs, _) = arg.split_once('=').unwrap_or((arg.as_str(), ""));
        let lhs = raw_lhs.strip_suffix('+').unwrap_or(raw_lhs);
        let Some(base) = declare_base_name(lhs) else {
            continue;
        };
        if let Some((_, target)) = declare_nameref_chain(variables, base) {
            resolved.push(target);
        }
    }
    resolved
}

pub(crate) fn execute_with_io<W, E>(
    args: &[String],
    variables: &mut crate::shell::var_table::VarTable,
    stdout: &mut W,
    stderr: &mut E,
    oob_word: &mut Option<String>,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    execute_with_io_named("declare", args, variables, stdout, stderr, oob_word)
}

pub(crate) fn execute_with_io_named<W, E>(
    command_name: &str,
    args: &[String],
    variables: &mut crate::shell::var_table::VarTable,
    stdout: &mut W,
    stderr: &mut E,
    oob_word: &mut Option<String>,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    // The Diag's sink binds here, at the family's dispatch point, to
    // the stderr the caller supplied (Stage-2 ④ Phase 4).
    let row = builtin_table_row(command_name)
        .unwrap_or_else(|| builtin_table_row("declare").expect("declare row exists"));
    let prolog = PrologParts::Lib;
    let mut diag = Diag::new(row, stderr);
    let status = execute_with_io_named_in_context(
        command_name,
        args,
        variables,
        stdout,
        &mut diag,
        &prolog,
        false,
        &[],
        &[],
        &[],
        &crate::shell::var_table::TempenvListingUnderlay::new(),
        oob_word,
        &mut WrittenOperandHints::default(),
    )?;
    diag.flush_with(|| prolog.build(variables));
    Ok(status)
}

/// GNU builtins/declare.def decides between the global-scope self-reference
/// error and the function-scope circular-reference warning with
/// `variable_context` (declare.def:565). Rubash threads the executor's
/// function depth through here so `typeset -n` inside a function warns and
/// continues instead of erroring.
pub(crate) fn execute_with_io_named_in_context<'a, W>(
    command_name: &str,
    args: &'a [String],
    variables: &mut crate::shell::var_table::VarTable,
    stdout: &mut W,
    diag: &mut Diag<'a>,
    prolog: &PrologParts,
    in_function: bool,
    frame_locals: &[String],
    dynamic_views: &[DynamicDeclarationView],
    dynamic_set_views: &[crate::builtins::set::DynamicSetView],
    underlay: &crate::shell::var_table::TempenvListingUnderlay,
    oob_word: &mut Option<String>,
    written_hints: &mut WrittenOperandHints,
) -> io::Result<i32>
where
    W: Write,
{
    let mut print = false;
    let mut export = false;
    let mut array = false;
    let mut assoc = false;
    let mut integer = false;
    let mut uppercase = false;
    let mut lowercase = false;
    let mut capcase = false;
    let mut nameref = false;
    let mut trace = false;
    let mut unset_trace = false;
    let mut readonly = false;
    let mut unset_export = false;
    let mut unset_array = false;
    let mut unset_assoc = false;
    let mut unset_integer = false;
    let mut unset_uppercase = false;
    let mut unset_lowercase = false;
    let mut unset_capcase = false;
    let mut unset_nameref = false;
    let mut unset_readonly = false;
    // GNU builtins/declare.def:273-274 + bashgetopt.c: the option scan
    // is internal_getopt over DECLARE_OPTS "+acfgilnprtuxAFGI"
    // (declare.def:145) — it stops at the first non-option word, so an
    // option letter after an operand is an operand name, and `--help`
    // in option position prints the long help (CASE_HELPOPT). The
    // single BuiltinOpt machine replaces the hand scan here (Stage-2
    // ④ Phase 4); the row's spec IS the table (D1), so `typeset`
    // diagnostics carry the invoked name instead of "declare".
    let mut function = false;
    let mut names: Vec<&str> = {
        let row = diag.row();
        let mut opt = BuiltinOpt::new(row, args, diag, false);
        loop {
            match opt.next() {
                OptEvent::Help => {
                    crate::builtins::help::print_builtin_help(row.name, stdout)?;
                    return Ok(EX_USAGE);
                }
                OptEvent::End => break,
                OptEvent::Opt(c) => {
                    let set_attr = opt.opttype() == b'-';
                    match c {
                        b'p' => print = true,
                        b'x' if set_attr => export = true,
                        b'x' => unset_export = true,
                        b'a' if set_attr => array = true,
                        b'a' => unset_array = true,
                        b'A' if set_attr => assoc = true,
                        b'A' => unset_assoc = true,
                        b'i' if set_attr => integer = true,
                        b'i' => unset_integer = true,
                        // GNU builtins/declare.def:341-356: each case option
                        // sets its own bit in flags_on AND (only when setting,
                        // i.e. flags == &flags_on) adds the other two case
                        // attributes to flags_off. The bits accumulate across
                        // the whole option scan — `declare -lu` therefore ends
                        // with on={lower,upper} and off={upper,capcase,lower,
                        // capcase}, and the later VSETATTR(flags_on) +
                        // VUNSETATTR(flags_off) application (declare.def:
                        // 525-527) leaves NO case attribute at all, instead of
                        // the last option winning (rubash#1458).
                        b'u' => {
                            if set_attr {
                                uppercase = true;
                                unset_lowercase = true;
                                unset_capcase = true;
                            } else {
                                unset_uppercase = true;
                            }
                        }
                        b'l' => {
                            if set_attr {
                                lowercase = true;
                                unset_uppercase = true;
                                unset_capcase = true;
                            } else {
                                unset_lowercase = true;
                            }
                        }
                        b'c' if set_attr => {
                            capcase = true;
                            unset_uppercase = true;
                            unset_lowercase = true;
                        }
                        b'c' => unset_capcase = true,
                        b'n' if set_attr => nameref = true,
                        b'n' => unset_nameref = true,
                        b'r' if set_attr => readonly = true,
                        b'r' => unset_readonly = true,
                        // GNU declare.def option string "aAfFgiIlnrtux":
                        // -t sets att_trace (printed by declare -p); +t
                        // clears it.
                        b't' if set_attr => trace = true,
                        b't' => unset_trace = true,
                        b'g' | b'G' | b'I' => {
                            // TODO(variables.c/builtins/declare.def): `-g` forces
                            // global scope inside functions. Rubash has one
                            // variable table for now. `-I` is a local inheritance
                            // attribute; outside local it is accepted but does not
                            // add a printable variable attribute.
                        }
                        // declare.def:145: -f/-F set att_function (the
                        // function face owns the listing/definition work;
                        // a valued operand errors in the name loop below,
                        // declare.def:465-476). +f/+F clear att_function —
                        // VUNSETATTR on a variable is a no-op.
                        b'f' if set_attr => function = true,
                        b'f' => {}
                        // -F also sets att_function; the nodefs bit only
                        // shapes function LISTING, which the function face
                        // owns — the core keeps just the function flag.
                        b'F' if set_attr => function = true,
                        b'F' => {}
                        // Every other spec letter is covered above; the
                        // machine only yields spec letters.
                        _ => {}
                    }
                }
                // No spec letter takes an argument.
                OptEvent::OptArg(_, _) => {}
            }
        }
        // operands() borrows the args slice ('a), not the machine, so
        // the names outlive it; drop the machine first to release the
        // &mut Diag borrow before reading the scan status.
        let operands: Vec<&str> = opt.operands().iter().map(String::as_str).collect();
        drop(opt);
        if diag.status() != 0 {
            diag.usage();
            return Ok(diag.status());
        }
        operands
    };

    // GNU declare.def:389-406: the -p branch prints the operand words
    // verbatim (show_name_attributes(list->word->word)) — a valued
    // operand is a name to look up, never a bind (`declare -p dv=6`
    // reports `dv=6: not found`, E1). The attr pipeline below rewrites
    // operands (base split, subscript strip), so snapshot the raw
    // words here for the print call.
    // The snapshot is consumed only by the print call at the end
    // of this function, which is reachable with a non-empty name
    // list only when the scan itself set `print` (the listing-mode
    // flip further down fires only when `names` is empty, where
    // the snapshot is empty either way). A plain bind therefore
    // skips the clone (E4, PR #2968).
    let print_names: Vec<&str> = if print { names.clone() } else { Vec::new() };

    let had_name_args = !names.is_empty();
    // GNU builtins/declare.def:222 declare_invalid_opts (bash 5.3): `-a'
    // together with `-A' — combined (`-aA', `+aA') or separate (`-a -A') —
    // is a usage error reported by naming `-a' as the invalid option, with
    // EX_BADUSAGE and NO usage text, and no variable is created
    // (`declare -aA x; declare -p x` -> "x: not found"). The same check
    // rejects an attribute set AND unset on one command line: `-A +A' names
    // `-A', `-a +a' names `-a'. The display path returns before the check
    // in GNU (declare.def:330-406 -> 408), so `-paA' stays a pure display,
    // and with no name operands declare returns before it too (`declare
    // -aA` alone is rc 0). 5.2 accepted `-aA' and failed later at the
    // assignment ("cannot convert associative to indexed array"); 5.3 made
    // the combination a usage error — rubash follows 5.3.
    if !print && !names.is_empty() {
        let conflict: Option<char> = if assoc && unset_assoc {
            Some('A')
        } else if array && unset_array {
            Some('a')
        } else if assoc && array {
            Some('a')
        } else {
            None
        };
        if let Some(option) = conflict {
            diag.error(DiagKind::InvalidOpt(format!("-{option}")));
            diag.flush_with(|| prolog.build(variables));
            return Ok(EX_USAGE);
        }
        // GNU declare.def:222 declare_invalid_opts, first branch:
        // att_function together with att_array/att_assoc/att_integer/
        // att_nameref is a usage error naming the conflicting letter
        // (priority -n, -i, -A, -a) with EXECUTION_FAILURE and NO usage
        // text. The function face owns the binary path, so this arm
        // only fires on direct core entries (lib/test) — it keeps the
        // core honest when -f reaches it.
        if function {
            let conflict: Option<char> = if nameref {
                Some('n')
            } else if integer {
                Some('i')
            } else if assoc {
                Some('A')
            } else if array {
                Some('a')
            } else {
                None
            };
            if let Some(option) = conflict {
                diag.error(DiagKind::InvalidOpt(format!("-{option}")));
                diag.flush_with(|| prolog.build(variables));
                return Ok(EXECUTION_FAILURE);
            }
        }
    }
    // GNU declare.def: an operand name[subscript] without = declares or
    // re-declares the array; the subscript is only a size hint and is
    // discarded ("declare -a b[256]" declares b, and "declare -p b" prints
    // "declare -a b"). Strip it before any downstream name processing.
    if array || assoc {
        for name in names.iter_mut() {
            if name.contains('=') {
                continue;
            }
            if let Some((base, _subscript)) = name.split_once('[') {
                if !base.is_empty() && name.ends_with(']') && !base.contains('[') {
                    *name = base;
                }
            }
        }
    }
    let mut assign_names = Vec::new();
    let mut drop_nameref_attrs: Vec<String> = Vec::new();
    let mut attr_status = EXECUTION_SUCCESS;
    // GNU drops the -n when the same command requests an array shape: the
    // nameref attribute is lifted out of the set mask and cleared whenever
    // the variable is or becomes an array (declare.def:962-981), so the
    // attribute marks and the assignment routing must run with the nameref
    // bit off.
    let mut nameref_marks = nameref;
    for name in &names {
        // GNU declare.def:465-476: a valued operand with att_function
        // cannot make a function — one diagnostic, and the command
        // stops here (later operands are discarded). The binary path
        // never reaches this arm (the executor's function face owns
        // -f); it fires only on direct core entries.
        if function && name.contains('=') {
            diag.free(format_args!("cannot use `-f' to make functions"));
            diag.flush_with(|| prolog.build(variables));
            return Ok(EXECUTION_FAILURE);
        }
        // GNU builtins/declare.def:549-580 runs the nameref-specific lexical
        // checks on the LHS and RHS before the generic identifier check, and
        // reports with the invoked builtin name (declare or typeset).
        let (lhs, value) = match name.split_once('=') {
            Some((lhs, value)) => (lhs, value),
            None => (*name, ""),
        };
        let append = lhs.strip_suffix('+').is_some();
        let lhs = lhs.strip_suffix('+').unwrap_or(lhs);
        // GNU 5.2.21 (declare.def:651 invisible-husk creation + ksh93 onref
        // deferral): `declare -in name=value` -- integer and nameref together
        // WITH an assignment -- leaves nothing observable: no value, no
        // attributes, no message (probe: `declare -p a b` afterwards reports
        // "b: not found" and a later `b+=1` builds a plain b).
        // GNU 5.3 (variables.c bind_variable): the value bound to a cell
        // carrying BOTH att_integer and att_nameref is arithmetic-evaluated
        // first, and the numeric result is never a valid nameref target, so
        // the declare FAILS (status 1) while still creating nothing and
        // printing nothing (rubash#1243: the skip was right, the status was
        // left at success).
        if nameref && integer && !value.is_empty() {
            attr_status = EXECUTION_FAILURE;
            continue;
        }
        // GNU declare.def:554-558 + 966-980: a real compound assignment
        // operand under -n reports `name: reference variable cannot be an
        // array`, the array attribute wins so the compound still assigns,
        // and onref is dropped so the nameref attribute is never set
        // (nameref22.sub:74 `declare -n array=(one two three)` errors yet
        // leaves `declare -a array=([0]="one" ...)`).
        if nameref && value.starts_with(COMPOUND_ASSIGNMENT_MARKER) {
            diag.free(format_args!("{lhs}: reference variable cannot be an array"));
            attr_status = EXECUTION_FAILURE;
            drop_nameref_attrs.push(lhs.to_string());
            assign_names.push(*name);
            continue;
        }
        if nameref {
            // declare.def:554: a nameref cannot be declared as an array
            // reference name (x[3]).
            if valid_array_reference(lhs) {
                diag.free(format_args!("{lhs}: reference variable cannot be an array"));
                attr_status = EXECUTION_FAILURE;
                continue;
            }
            // declare.def:562-573: disallow self references at global scope,
            // warn at function scope and continue creating the nameref.
            if check_selfref(lhs, value) {
                if in_function {
                    diag.free(format_args!("warning: {lhs}: circular name reference"));
                    // bind_variable_value's ASS_NAMEREF check_selfref
                    // (variables.c:3324) reports the same self-reference via
                    // internal_warning — a second, unqualified diagnostic —
                    // then still stores the cell.
                    bare_diag_line(
                        diag,
                        prolog,
                        variables,
                        &format!("warning: {lhs}: circular name reference"),
                    );
                } else {
                    diag.free(format_args!(
                        "{lhs}: nameref variable self references not allowed"
                    ));
                    attr_status = EXECUTION_FAILURE;
                    continue;
                }
            }
            // declare.def:574-579: the value must be a valid identifier o
            // array reference when it will be used as a nameref target.
            if !value.is_empty()
                && !append
                && !value.starts_with(COMPOUND_ASSIGNMENT_MARKER)
                && !valid_nameref_value(value)
            {
                // GNU declare.def:576 prints the RAW assignment value. When
                // the operand reached the declare path through the array
                // assignment expansion (the target already carries -a), the
                // value here is the typed storage serialization
                // `("one"  "two" "three")'; decode the element words back
                // to their text for the diagnostic (rubash#356; GNU prints
                // `(one two three)').
                let display =
                    if value.starts_with('(') && value.ends_with(')') && value.contains('"') {
                        let inner = &value[1..value.len() - 1];
                        let words: Vec<String> =
                            crate::builtins::declare::storage::split_storage_words(inner)
                                .map(|word| {
                                    crate::builtins::declare::storage::unquote_storage_value(
                                        word.trim_start_matches(
                                            crate::executor::markers::ARRAY_FIELD_SPLIT_MARKER,
                                        ),
                                    )
                                })
                                .collect();
                        if words.is_empty() {
                            value.to_string()
                        } else {
                            format!("({})", words.join(" "))
                        }
                    } else {
                        value.to_string()
                    };
                diag.free(format_args!(
                    "`{display}': invalid variable name for name reference"
                ));
                attr_status = EXECUTION_FAILURE;
                continue;
            }
            // declare.def:854-862: converting an existing non-nameref variable
            // to a nameref without an assignment fails when its current value
            // is not a valid nameref value. An array/assoc variable is exempt:
            // declare.def:841 reports it as a conflicting array instead (an
            // empty array cell is not an invalid nameref value), so the guard
            // below must be the one to fire.
            if !append
                && value.is_empty()
                && !variables.is_marked(NAMEREF_VARS, lhs)
                && !variables.is_marked(ARRAY_VARS, lhs)
                && !variables.is_marked(ASSOC_VARS, lhs)
                && variables.get(lhs).is_some_and(|current| {
                    // A compound/array-storage cell is not a nameref value at
                    // all -- GNU reaches the array-rejection diagnostic for
                    // those (nameref22.sub:50), not the invalid-value error.
                    !current.starts_with(STORAGE_WORD_PREFIX)
                        && !current.starts_with('(')
                        && !current.starts_with(COMPOUND_ASSIGNMENT_MARKER)
                        && !valid_nameref_value(current)
                })
            {
                let current = variables.get(lhs).cloned().unwrap_or_default();
                diag.free(format_args!(
                    "`{current}': invalid variable name for name reference"
                ));
                attr_status = EXECUTION_FAILURE;
                continue;
            }
            // declare.def:841: applying -n to an existing array variable is
            // rejected, but only after the value checks above -- GNU reports
            // the invalid-value error first (nameref22.sub:69 reports
            // `(one two three)': invalid variable name, then :70 reports the
            // array rejection for the same variable).
            if (variables.is_marked(ARRAY_VARS, lhs) || variables.is_marked(ASSOC_VARS, lhs))
                && !(variables.is_marked(NAMEREF_VARS, lhs)
                    && value.is_empty()
                    && !variables.contains_key(lhs))
            {
                diag.free(format_args!("{lhs}: reference variable cannot be an array"));
                attr_status = EXECUTION_FAILURE;
                continue;
            }
            // GNU variables.c bind_variable_value ASS_NAMEREF path: an
            // explicit `declare -n name=` assignment binds "" as the cell,
            // which fails valid_nameref_value and reports
            // `` `': not a valid identifier `` -- the variable is not
            // created, and an existing nameref keeps its cell
            // (nameref24.sub:24 `declare -n name3=`).
            if !append && name.contains('=') && value.is_empty() {
                diag.error(DiagKind::InvalidId(String::new()));
                attr_status = EXECUTION_FAILURE;
                continue;
            }
            if variables.is_marked(READONLY_VARS, lhs) {
                diag.error(DiagKind::Readonly(lhs.to_string()));
                attr_status = EXECUTION_FAILURE;
                continue;
            }
            // GNU drops the -n when the same command requests an array shape:
            // nameref and array/assoc are mutually exclusive and the shape
            // wins (declare.def:962-981 clears the lifted nameref bit when
            // the variable is or becomes an array). At global scope the drop
            // is silent; at function scope the operand still creates its
            // plain local array, but the command reports the conflict once
            // (declare.def:841 fires after the local was materialized).
            if array || assoc {
                if in_function {
                    diag.free(format_args!("{lhs}: reference variable cannot be an array"));
                    attr_status = EXECUTION_FAILURE;
                }
                nameref_marks = false;
            }
        }
        // On a readonly variable only the export, local, nameref, trace and
        // readonly attribute bits may be touched (variables.h:126-128
        // valid/invalid_readonly_attrs): requesting any of the integer,
        // array, assoc or case-mapping attributes in either direction is
        // refused per name (declare.def:893), and an assignment on the same
        // word suppresses every attribute the command would otherwise apply
        // (declare.def:881 skips the operand after the failed assignment).
        // The refusal is all-or-nothing: a mixed request like -ix on a
        // readonly variable applies nothing. -p is display-only and never
        // reaches this gate.
        if !print && variables.is_marked(READONLY_VARS, lhs) {
            let invalid_attrs = integer
                || array
                || assoc
                || uppercase
                || lowercase
                || capcase
                || unset_integer
                || unset_array
                || unset_assoc
                || unset_uppercase
                || unset_lowercase
                || unset_capcase;
            let assignment_with_valid_attrs =
                name.contains('=') && (export || trace || unset_export || unset_trace);
            if invalid_attrs || assignment_with_valid_attrs {
                diag.error(DiagKind::Readonly(lhs.to_string()));
                attr_status = EXECUTION_FAILURE;
                continue;
            }
        }
        if !valid_declare_name(name) {
            // GNU declare.def:389-406: with -p the operand is a NAME to
            // print, `=` included — `declare -p dv=6` reports `dv=6: not
            // found` and assigns nothing, so the `=` carve-out here is
            // dropped (Stage-2 ④ Phase 4, E1).
            if print {
                // GNU declare.def: -p is display-only; the identifier check
                // gates attribute changes, not display, so an invalid name
                // reaches find_variable and reports "not found" — never the
                // full listing fallback (niubash issue #102 probe: bash 5.3.0
                // prints `declare: X(BR): not found` for `declare -p X(BR)`,
                // `not a valid identifier` only without -p).
                diag.free(format_args!("{name}: not found"));
                attr_status = EXECUTION_FAILURE;
                continue;
            }
            diag.error(DiagKind::InvalidId(name.to_string()));
            attr_status = EXECUTION_FAILURE;
            continue;
        }
        assign_names.push(*name);
    }
    let mut valid_assign_names = Vec::new();
    for name in assign_names {
        // GNU declare.def:389: with -p the builtin is display-only — the
        // pflag branch runs before all making-variables machinery, so a
        // type flag riding along on a query operand never triggers a
        // conversion or its validation, and namerefs are never transformed
        // (rubash#2335: `declare -p -A a` on an indexed array prints it).
        if print {
            valid_assign_names.push(name);
            continue;
        }
        let Some(var_name) = declare_base_name(name) else {
            valid_assign_names.push(name);
            continue;
        };
        if (array || assoc) && variables.is_marked(NAMEREF_VARS, var_name) {
            let operand_has_subscript = name.starts_with(var_name)
                && name.len() > var_name.len()
                && name[var_name.len()..].starts_with('[');
            if operand_has_subscript {
                // GNU declare.def:737-752: an array assignment to a nameref
                // WITH a subscript removes the nameref attribute (with an
                // internal warning) and applies the array declaration to the
                // name itself (nameref15.sub: declare -a xref[1]=one).
                bare_diag_line(
                    diag,
                    prolog,
                    variables,
                    &format!("warning: {var_name}: removing nameref attribute"),
                );
                unmark_typed(variables, NAMEREF_VARS, var_name);
                variables.remove(var_name);
            } else if nameref {
                // The command itself toggles -n while declaring the shape:
                // the shape wins and applies to the nameref itself — no
                // chain-follow (declare.def:206 reserves the transform for
                // commands that leave -n/+n alone), the nameref attribute is
                // dropped, and the old cell survives as element 0
                // (declare.def:962 convert_var_to_array keeps the value).
                unmark_typed(variables, NAMEREF_VARS, var_name);
            } else if declare_nameref_chain(variables, var_name).is_some() {
                // GNU declare.def:197-216 declare_transform_name + 764-806:
                // without a subscript the -a/-A flag and the compound
                // assignment follow the chain onto the referenced variable
                // (nameref20/nameref21.sub); the effective_assign_names stage
                // below performs the rewrite, so nothing is rejected here.
            } else {
                // GNU declare.def: a valueless nameref (no target value
                // stored) can be converted to an array by removing the
                // nameref attribute (nameref12.sub: declare -n foo;
                // declare -a foo; declare -p foo shows declare -a foo).
                unmark_typed(variables, NAMEREF_VARS, var_name);
                variables.remove(var_name);
            }
        }
        if assoc
            && variables.is_marked(ARRAY_VARS, var_name)
            && !variables.is_marked(ASSOC_VARS, var_name)
        {
            diag.free(format_args!(
                "{var_name}: cannot convert indexed to associative array"
            ));
            attr_status = EXECUTION_FAILURE;
            continue;
        }
        if array
            && variables.is_marked(ASSOC_VARS, var_name)
            && !variables.is_marked(ARRAY_VARS, var_name)
        {
            // GNU declare.def -> convert_var_to_array refusal: the operand
            // errors and the variable keeps its assoc value untouched —
            // clearing it would print `declare -A m` where GNU still shows
            // the elements (varenv14.sub declare -a assoc case).
            diag.free(format_args!(
                "{var_name}: cannot convert associative to indexed array"
            ));
            attr_status = EXECUTION_FAILURE;
            continue;
        }
        valid_assign_names.push(name);
    }
    let assign_names = valid_assign_names;
    // GNU builtins/declare.def:704-738 (turning off the nameref attribute with
    // an assignment: assign through the chain, then remove the attribute while
    // leaving the nameref's value in place) and declare.def:764-806 (without
    // -n/+n the attributes and assignment apply to the variable the nameref
    // chain references, via find_variable_last_nameref). `declare -p` never
    // follows the chain (nameref18.sub prints the nameref itself).
    let mut effective_assign_names: Vec<String> = Vec::new();
    if !print {
        for name in &assign_names {
            let (raw_lhs, value) = match name.split_once('=') {
                Some((lhs, value)) => (lhs, Some(value)),
                None => (*name, None),
            };
            let append = raw_lhs.strip_suffix('+').is_some();
            let lhs = raw_lhs.strip_suffix('+').unwrap_or(raw_lhs);
            let chain = declare_nameref_chain(variables, lhs);
            if let Some((last_nameref, target)) = chain {
                if unset_nameref {
                    // GNU declare.def:678-682/716-724 (ksh93 compat): the
                    // nameref attribute cannot be removed from a readonly
                    // nameref that carries a cell. At global scope the
                    // last-nameref lookup always lands on the nameref, so
                    // any non-empty cell errors. Inside a function the
                    // operand is first resolved to a local var; when the
                    // cell's target exists the same readonly check fires,
                    // but when the target is missing the rewrite silently
                    // keeps the nameref (nameref17.sub: typeset +n foo4 with
                    // cell -> existing bar4 errors, cell -> missing stays).
                    let cell = variables.get(&last_nameref).cloned().unwrap_or_default();
                    let readonly_with_cell = variables
                        .is_marked(READONLY_VARS, last_nameref.as_str())
                        && !cell.is_empty();
                    if readonly_with_cell {
                        let cell_base = cell.split('[').next().unwrap_or(cell.as_str());
                        let target_exists = !in_function
                            || variables.contains_key(cell_base)
                            || variables.is_marked(ARRAY_VARS, cell_base)
                            || variables.is_marked(ASSOC_VARS, cell_base);
                        if target_exists {
                            diag.error(DiagKind::Readonly(last_nameref.clone()));
                            attr_status = EXECUTION_FAILURE;
                        }
                        continue;
                    }
                    if let Some(value) = value {
                        effective_assign_names.push(if append {
                            format!("{target}+={value}")
                        } else {
                            format!("{target}={value}")
                        });
                    } else {
                        effective_assign_names.push((*name).to_string());
                    }
                    // declare.def:985-986: only the last nameref in the chain
                    // loses the attribute; its cell value is kept as the new
                    // plain variable value.
                    unmark_typed(variables, NAMEREF_VARS, &last_nameref);
                    continue;
                }
                if !nameref {
                    // GNU arrayfunc.c:454 find_or_make_array_variable ->
                    // variables.c:2201 find_variable_nameref_for_create: a
                    // compound `=(...)` value through a nameref requires a
                    // bare-identifier cell; an element cell like `D[2]`
                    // fails sh_invalidid. An attribute-only `declare -a/-A`
                    // instead applies the flag to the cell's array BASE
                    // name (nameref18.sub: `declare -A r` with r -> `A[0]`
                    // yields `declare -A A`).
                    let compound_value =
                        value.is_some_and(|v| v.starts_with(COMPOUND_ASSIGNMENT_MARKER));
                    if compound_value && !valid_identifier(&target) {
                        bare_diag_line(
                            diag,
                            prolog,
                            variables,
                            &format!("`{target}': not a valid identifier"),
                        );
                        attr_status = EXECUTION_FAILURE;
                        continue;
                    }
                    let target = if value.is_none() && (array || assoc) {
                        target
                            .split('[')
                            .next()
                            .unwrap_or(target.as_str())
                            .to_string()
                    } else {
                        target
                    };
                    match value {
                        Some(value) => effective_assign_names.push(if append {
                            format!("{target}+={value}")
                        } else {
                            format!("{target}={value}")
                        }),
                        None => effective_assign_names.push(target),
                    }
                    continue;
                }
            }
            effective_assign_names.push((*name).to_string());
        }
    } else {
        effective_assign_names.extend(assign_names.iter().map(|name| (*name).to_string()));
    }
    let attr_names: Vec<&str> = effective_assign_names.iter().map(String::as_str).collect();
    // GNU declare.def:1031-1034: a failed ASS_NAMEREF assignment to a
    // freshly-created (invisible/empty-cell) variable deletes it outright;
    // the attribute pass must not resurrect the name.
    let mut deleted_names = std::collections::HashSet::new();
    // GNU declare.def:810-823 checks whether the variable is already bound
    // BEFORE the command's own assignment runs: `declare -A foo` on an
    // existing variable converts it through convert_var_to_assoc
    // (arrayfunc.c:111, a 128-bucket table), while an unbound name takes
    // make_new_assoc_variable (variables.c:2851, 1024 buckets). Snapshot
    // operand names now -- after assign_declare_names an operand like
    // `declare -A foo=v` would look pre-existing even though GNU created it.
    let preexisting_vars: std::collections::HashSet<String> = attr_names
        .iter()
        .map(|name| {
            let base = name
                .split_once('=')
                .map(|(base, _)| base)
                .unwrap_or(name)
                .trim_end_matches('+');
            let base = base.split('[').next().unwrap_or(base);
            base.to_string()
        })
        .filter(|base| variables.contains_key(base) || frame_locals.iter().any(|n| n == base))
        .collect();
    // GNU declare.def:389-406: with -p the builtin is display-only —
    // the pflag branch runs before all making-variables machinery, so a
    // valued print operand is never bound (`declare -p dv=6` reports
    // `dv=6: not found`, E1). The assign stage is skipped outright.
    let mut locale_funnel = LocaleFunnelRecord::default();
    if !print
        && assign_declare_names(
            command_name,
            &attr_names,
            variables,
            frame_locals,
            nameref_marks,
            in_function,
            array,
            assoc,
            integer,
            true,
            &mut deleted_names,
            diag,
            prolog,
            oob_word,
            written_hints,
            &mut locale_funnel,
        )? != EXECUTION_SUCCESS
    {
        attr_status = EXECUTION_FAILURE;
    }
    // GNU err_badarraysub + any_failed (arrayfunc.c:804-809, rubash#635):
    // an out-of-range subscript stops the whole declaration; attribute
    // passes below must not run. The word travels up for the fatal
    // diagnostic at the Executor layer, which owns abort + status.
    if oob_word.is_some() {
        return Ok(EXECUTION_FAILURE);
    }
    let names = attr_names;
    // GNU declare.def: with explicit operands, -p prints exactly those
    // names and reports per-name errors ("not a valid identifier", "not
    // found"); it never falls back to a full listing. Operands rejected in
    // the loop above (e.g. `declare -p 'X(BR)'` on an inherited
    // CommonProgramFiles(x86)-style name) leave nothing to print — the
    // empty-names listing below is only for the no-operand invocation.
    if names.is_empty() && had_name_args {
        return Ok(attr_status);
    }
    let options = DeclareOptions {
        export,
        array,
        assoc,
        integer,
        uppercase,
        lowercase,
        capcase,
        nameref: nameref_marks,
        readonly,
        unset_export,
        unset_array,
        unset_assoc,
        unset_integer,
        unset_uppercase,
        unset_lowercase,
        unset_capcase,
        unset_nameref,
        unset_readonly,
        trace,
        unset_trace,
    };
    // GNU declare.def: -p is display-only; `declare -np b` must not create
    // or mark b (nameref23.sub:41 -- GNU prints "b: not found" for a failed
    // `declare -n b="1"` followed by `declare -np b`, while an attr pass
    // would leave a nameref mark behind).
    if !print {
        attr_status = apply_declare_attrs(
            command_name,
            &names,
            variables,
            options,
            attr_status,
            &deleted_names,
            &preexisting_vars,
            in_function,
            diag,
            prolog,
        )?;
        // declare.def:978-981: array/compound operands drop the deferred -n.
        for name in &drop_nameref_attrs {
            unmark_typed(variables, NAMEREF_VARS, name);
        }
        // GNU variables.c:5819/5919 sv_path -> phash_flush (rubash#1551):
        // a declare/typeset/local operand naming PATH reaches a bind (or
        // the value-less make-variable path) and empties the command
        // hash table — verified oracle for `declare PATH=/x`,
        // `declare PATH`, and `local PATH` alike. The print path above
        // is display-only and excluded (declare -p PATH keeps the
        // table), as are attribute-only setattr operations.
        let names_path = names.iter().any(|operand| {
            let base = operand
                .split_once('=')
                .map(|(base, _)| base)
                .unwrap_or(operand)
                .trim_end_matches('+');
            base.split('[').next().unwrap_or(base) == "PATH"
        });
        if names_path {
            crate::builtins::hash::flush_command_hash_table(variables);
        }
        // GNU builtins/declare.def:1105: stupidly_hack_special_variables
        // runs after EVERY variable the declare loop processes, on top
        // of any fire the bind itself produced (rubash#3066). The
        // assign pass's record (see LocaleFunnelRecord) says which
        // operands still owe that hack fire: compound binds (their
        // assign-phase fire was the pre-declaration assignment's,
        // subst.c:8169) and operands that never fired there
        // (attribute-only conversions of an existing scalar —
        // `LC_ALL=bogus; declare -a LC_ALL` warns again with the
        // converted element value — and element binds). A plain
        // scalar bind's assign-phase fire already plays the hack's
        // role and is not doubled. The hack runs here, after the
        // attribute pass, so a conversion's storage and marks are in
        // their final shape when sv_locale reads the projection
        // (variables.c:6272 -> get_string_value, variables.c:2495).
        for (index, operand) in names.iter().enumerate() {
            let base = operand
                .split_once('=')
                .map(|(base, _)| base)
                .unwrap_or(operand)
                .trim_end_matches('+');
            let base = base.split('[').next().unwrap_or(base);
            if deleted_names.contains(base) {
                continue;
            }
            // A compound operand's assign-phase fire was not the hack
            // (it is still owed); a plain bind's was (skip); anything
            // else never fired in the assign pass (owed outright).
            if locale_funnel.fired.contains(&index) && !locale_funnel.compound.contains(&index) {
                continue;
            }
            if let Some(warning) = crate::locale::locale_assignment_warning(base, variables) {
                bare_diag_line(diag, prolog, variables, &format!("warning: {warning}"));
            }
        }
    }

    // GNU declare.def:373-380: with no operands, no -p, and no minus-form
    // attribute options (flags_on == 0; `+x`-style options only populate
    // flags_off), `declare` lists variables exactly like `set` with no
    // arguments — set_builtin(NULL) -> print_var_list (variables.c):
    // assignment form with print_var_value quoting (`NAME=value`,
    // `IFS=$' \t\n'`), NOT the `declare -p` declaration form
    // (`declare -- NAME="value"`). rubash#406.
    let flags_on = export
        || array
        || assoc
        || integer
        || uppercase
        || lowercase
        || capcase
        || nameref
        || readonly
        || trace;
    let plain = names.is_empty() && !had_name_args && !print && !flags_on;
    if plain {
        crate::builtins::set::print_shell_variables(
            variables,
            underlay,
            dynamic_set_views,
            stdout,
        )?;
        return Ok(attr_status);
    }
    // Listing mode = the no-operand enumeration (as opposed to a
    // named lookup); the printer's att_invisible rule keys on it
    // (rubash#2973).
    let listing_mode = names.is_empty() && !had_name_args;
    if listing_mode {
        print = true;
    }

    if !print {
        return Ok(attr_status);
    }

    print::print_declare_names(
        command_name,
        &print_names,
        variables,
        options,
        plain,
        listing_mode,
        attr_status,
        dynamic_views,
        underlay,
        stdout,
        diag,
        prolog,
    )
}

#[cfg(test)]
#[path = "declare_tests.rs"]
mod tests;
