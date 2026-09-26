//! `printf` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/printf.def (`printf_builtin`)

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};

use crate::shell::VariableStore;

mod escape;
mod float;
mod identifier;
mod number;
mod spec;
pub(crate) mod time;
mod value;

use crate::executor::markers::STORAGE_WORD_PREFIX_STR;
use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};
use escape::expand_format_escape;
use identifier::valid_identifier;
use spec::{parse_format_spec, resolve_dynamic_format_args, valid_format_specifier};
use time::format_time_value;
use value::format_value;

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

#[derive(Debug, Clone, Default)]
struct FormatSpec {
    raw: String,
    left_adjust: bool,
    zero_pad: bool,
    alternate_form: bool,
    explicit_sign: bool,
    leading_space_sign: bool,
    width: Option<usize>,
    width_from_arg: bool,
    precision: Option<usize>,
    precision_from_arg: bool,
    time_format: Option<String>,
    specifier: char,
    /// GNU printf.def:897-918 decodeint() reports ERANGE when an inline
    /// (non-`*`) width or precision overflows `int`. The flag is set in
    /// parse_format_spec and surfaced as a diagnostic in render_one_pass.
    inline_width_overflow: bool,
    inline_precision_overflow: bool,
}

#[derive(Debug, Clone)]
struct RenderedPrintf {
    output: String,
    status: i32,
    errors: Vec<String>,
    stop_output: bool,
}

enum ParsedFormat {
    Spec(FormatSpec),
    Missing(String),
}

struct ParsedNumber<T> {
    value: T,
    invalid: Option<String>,
}

/// Execute `printf` with arguments after the command name.
pub fn execute(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(
        args.iter().map(String::as_str),
        env_vars,
        &mut stdout,
        &mut stderr,
    )
}

pub(crate) fn execute_with_io<'a, I, W, E>(
    args: I,
    env_vars: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    execute_with_io_and_store(args, env_vars, None, stdout, stderr)
}

pub(crate) fn execute_with_io_and_store<'a, I, W, E>(
    args: I,
    env_vars: &mut HashMap<String, String>,
    mut variables: Option<&mut VariableStore>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    // W_ARRAYREF (in-band ARRAYREF_FLAG) is consumed by the executor's
    // SET_VFLAGS pre-pass (printf.def:305); strip any surviving prefix off
    // the operand text so it never leaks into format/arguments.
    let stripped_args: Vec<String> = args
        .into_iter()
        .map(|arg| {
            crate::builtins::arrayref::take_arrayref_flag(arg)
                .1
                .to_string()
        })
        .collect();
    let args: Vec<&str> = stripped_args.iter().map(String::as_str).collect();
    let mut output_var = None;
    let mut index = 0;

    let mut end_options = false;
    if args.get(index) == Some(&"--") {
        index += 1;
        end_options = true;
    }

    if !end_options
        && matches!(args.get(index), Some(option) if option.starts_with('-') && !option.starts_with("-v"))
    {
        writeln!(
            stderr,
            "{}printf: {}: invalid option",
            diagnostic_prefix(env_vars),
            args[index]
        )?;
        writeln!(stderr, "printf: usage: printf [-v var] format [arguments]")?;
        return Ok(EX_USAGE);
    }

    if !end_options {
        let name = match args.get(index) {
            Some(&"-v") => {
                let Some(name) = args.get(index + 1) else {
                    writeln!(
                        stderr,
                        "{}printf: -v: option requires an argument",
                        diagnostic_prefix(env_vars)
                    )?;
                    return Ok(EX_USAGE);
                };
                index += 2;
                Some(*name)
            }
            Some(option) => option
                .strip_prefix("-v")
                .filter(|name| !name.is_empty())
                .map(|name| {
                    index += 1;
                    name
                }),
            None => None,
        };

        if let Some(name) = name {
            if !valid_identifier(name) && !valid_printf_array_target(name, env_vars) {
                // GNU prints the expanded operand text; decode the
                // marker-encoded assoc key the executor delivered so the
                // diagnostic names `a[80's]`, not its carrier bytes.
                let display = parse_printf_array_target(name)
                    .map(|(base, subscript)| {
                        let key =
                            crate::executor::arithmetic::decode_arithmetic_assoc_key(subscript)
                                .unwrap_or_else(|| subscript.to_string());
                        format!("{base}[{key}]")
                    })
                    .unwrap_or_else(|| name.to_string());
                writeln!(
                    stderr,
                    "{}printf: `{}': not a valid identifier",
                    diagnostic_prefix(env_vars),
                    display
                )?;
                return Ok(EX_USAGE);
            }

            output_var = Some(name);
            if args.get(index) == Some(&"--") {
                index += 1;
                end_options = true;
            }
        }
    }

    if !end_options && matches!(args.get(index), Some(option) if option.starts_with('-')) {
        writeln!(
            stderr,
            "{}printf: {}: invalid option",
            diagnostic_prefix(env_vars),
            args[index]
        )?;
        writeln!(stderr, "printf: usage: printf [-v var] format [arguments]")?;
        return Ok(EX_USAGE);
    }

    let Some(format) = args.get(index) else {
        writeln!(stderr, "printf: usage: printf [-v var] format [arguments]")?;
        return Ok(EX_USAGE);
    };

    let rendered = render(format, &args[index + 1..], env_vars);
    let mut assign_status = None;
    if let Some(name) = output_var {
        assign_status = assign_printf_output(
            env_vars,
            name,
            rendered.output,
            variables.as_deref_mut(),
            stderr,
        )?;
    } else {
        stdout.write_all(&escape::raw_bytes(&rendered.output))?;
    }

    for error in rendered.errors {
        writeln!(stderr, "{error}")?;
    }

    Ok(assign_status.unwrap_or(rendered.status))
}

/// GNU printf.c diagnostics go through builtin_error -> error_prolog, which
/// prefixes `./script: line N:` when running a script file and falls back to
/// the shell name without script context (mirrors builtins/trap.rs).
fn diagnostic_prefix(env_vars: &HashMap<String, String>) -> String {
    if let (Some(script), Some(line)) = (
        env_vars.get("__RUBASH_SCRIPT_NAME"),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }
    "rubash: ".to_string()
}

fn valid_printf_array_target(name: &str, env_vars: &HashMap<String, String>) -> bool {
    // GNU printf.def:305: valid_array_reference(vname, arrayflags) with the
    // VA_NOEXPAND flags SET_VFLAGS derives from array_expand_once
    // (builtins/common.h:279) — a malformed quoted subscript (`a[80's]`)
    // is not a valid identifier under the flag-0 matched-pair scan. The
    // executor pre-pass already applied the SET_VFLAGS W_ARRAYREF half and
    // rewrote the operand to its normalized `name[index]`/`name[\x1e..]`
    // form, which the flag-0 scan validates (VA_ONEWORD never applies to
    // the rewritten carrier text).
    let noexpand = crate::builtins::shopt::option_enabled(env_vars, "array_expand_once");
    if !crate::executor::subscript_expansion::valid_array_reference_env(
        name, noexpand, false, env_vars,
    ) {
        return false;
    }

    // GNU valid_array_reference (arrayfunc.c) is purely syntactic: the
    // subscript only has to be non-empty and quote-balanced. `a[@]` is a
    // VALID reference — the `@`: bad array subscript diagnostic fires at
    // bind time inside bind_variable -> assign_array_element, not here.
    parse_printf_array_target(name).is_some()
}

/// GNU builtins/common.c:949 builtin_bind_variable -> bind_variable:
/// a `-v` operand that is a nameref assigns through the resolved target
/// (variables.c find_variable_nameref_for_assignment). An empty nameref
/// cell assigns the cell itself (the nameref value), matching
/// `declare -n er; printf -v er z` -> `declare -n er="z"`.
fn resolve_printf_bind_name(env_vars: &HashMap<String, String>, name: &str) -> String {
    if !valid_identifier(name) || !is_marked(env_vars, "__RUBASH_NAMEREF_VARS", name) {
        return name.to_string();
    }
    let mut current = name.to_string();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..8 {
        if !seen.insert(current.clone()) {
            return name.to_string();
        }
        let cell = env_vars.get(&current).cloned().unwrap_or_default();
        if cell.is_empty() {
            return current;
        }
        if !valid_identifier(cell.as_str())
            || !is_marked(env_vars, "__RUBASH_NAMEREF_VARS", cell.as_str())
        {
            return cell;
        }
        current = cell;
    }
    name.to_string()
}

fn assign_printf_output(
    env_vars: &mut HashMap<String, String>,
    name: &str,
    output: String,
    mut variables: Option<&mut VariableStore>,
    stderr: &mut dyn Write,
) -> io::Result<Option<i32>> {
    // GNU variables.c:2201-2204 find_variable_nameref_for_assignment ->
    // valid_nameref_value: binding a value into a valueless nameref stores
    // it as the new cell, so the output text is validated as a nameref
    // target — `printf -v r /` reports `` printf: `/': not a valid
    // identifier `` and fails (nameref11.sub lines 34/39).
    if valid_identifier(name)
        && is_marked(env_vars, "__RUBASH_NAMEREF_VARS", name)
        && env_vars.get(name).map_or(true, |cell| cell.is_empty())
    {
        if !valid_identifier(&output) && parse_printf_array_target(&output).is_none() {
            writeln!(
                stderr,
                "{}printf: `{}': not a valid identifier",
                diagnostic_prefix(env_vars),
                output
            )?;
            return Ok(Some(1));
        }
        env_vars.insert(name.to_string(), output);
        return Ok(None);
    }
    let resolved = resolve_printf_bind_name(env_vars, name);
    let name = resolved.as_str();
    // GNU printf.def:114 `v == 0 || ASSIGN_DISALLOWED(v, 0)` ->
    // EXECUTION_FAILURE; bind_variable prints the readonly diagnostic
    // naming the resolved variable (variables.c assign_readonly).
    let readonly_name = parse_printf_array_target(name)
        .map(|(base, _)| base)
        .unwrap_or(name);
    if is_marked(env_vars, "__RUBASH_READONLY_VARS", readonly_name) {
        writeln!(
            stderr,
            "{}{}: readonly variable",
            diagnostic_prefix(env_vars),
            name
        )?;
        return Ok(Some(1));
    }
    if let Some((base, subscript)) = parse_printf_array_target(name) {
        if is_marked(env_vars, "__RUBASH_ASSOC_VARS", base) {
            // The executor pre-resolved the operand under ExpandedOnce rules
            // and marker-encoded the resulting key; plain argv text stays
            // literal.
            let key = crate::executor::arithmetic::decode_arithmetic_assoc_key(subscript)
                .unwrap_or_else(|| subscript.to_string());
            if let Some(store) = variables.as_deref_mut() {
                let _ = store.set_associative_element(base, &key, output.clone());
            }
            assign_printf_assoc_element(env_vars, base, &key, output);
        } else if let Some(index) = resolve_printf_indexed_subscript(env_vars, base, subscript) {
            if let Some(store) = variables.as_deref_mut() {
                let _ = store.set_indexed_element(base, index as i64, output.clone());
            }
            assign_printf_indexed_element(env_vars, base, index, output);
        } else {
            // GNU bind_variable -> assign_array_element -> array_expand_index:
            // `@`/`*` (and any subscript the index expansion rejects) reports
            // `name[sub]: bad array subscript` (builtin_error) and fails the
            // assignment with status 1.
            writeln!(
                stderr,
                "{}{name}: bad array subscript",
                diagnostic_prefix(env_vars),
            )?;
            return Ok(Some(1));
        }
        return Ok(None);
    }

    if let Some(store) = variables.as_deref_mut() {
        let _ = store.set_scalar(name, output.clone());
    }
    env_vars.insert(name.to_string(), output);
    Ok(None)
}

fn parse_printf_array_target(name: &str) -> Option<(&str, &str)> {
    let (base, subscript) = name.split_once('[')?;
    let subscript = subscript.strip_suffix(']')?;
    valid_identifier(base).then_some((base, subscript))
}

fn resolve_printf_indexed_subscript(
    env_vars: &HashMap<String, String>,
    name: &str,
    subscript: &str,
) -> Option<usize> {
    if subscript.is_empty() {
        return None;
    }

    let raw_index = crate::executor::arithmetic::eval_conditional_arith_value(subscript, env_vars)?;
    if raw_index >= 0 {
        return usize::try_from(raw_index).ok();
    }

    let current = env_vars.get(name)?;
    let max_index = indexed_entries(current).keys().next_back().copied()?;
    let resolved = i128::try_from(max_index)
        .ok()?
        .checked_add(1)?
        .checked_add(raw_index)?;
    usize::try_from(resolved).ok()
}

fn assign_printf_indexed_element(
    env_vars: &mut HashMap<String, String>,
    name: &str,
    index: usize,
    output: String,
) {
    let mut entries = env_vars
        .get(name)
        .map(|value| indexed_entries(value))
        .unwrap_or_default();
    entries.insert(index, output);
    env_vars.insert(name.to_string(), format_indexed_storage(entries));
    mark_printf_var(env_vars, "__RUBASH_ARRAY_VARS", name);
}

fn assign_printf_assoc_element(
    env_vars: &mut HashMap<String, String>,
    name: &str,
    key: &str,
    output: String,
) {
    let mut entries = env_vars
        .get(name)
        .map(|value| assoc_entries(value))
        .unwrap_or_default();
    if let Some((_, value)) = entries
        .iter_mut()
        .rev()
        .find(|(entry_key, _)| entry_key == key)
    {
        *value = output;
    } else {
        entries.push((key.to_string(), output));
    }
    env_vars.insert(name.to_string(), format_assoc_storage(entries));
}

fn indexed_entries(value: &str) -> BTreeMap<usize, String> {
    let Some(rendered) = value.strip_prefix(STORAGE_WORD_PREFIX) else {
        return value
            .strip_prefix('(')
            .and_then(|value| value.strip_suffix(')'))
            .map(split_storage_words)
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                let value = value
                    .split_once('=')
                    .map(|(_, value)| value)
                    .unwrap_or(&value);
                (index, unquote_storage_value(value))
            })
            .collect();
    };

    let Some(inner) = rendered
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return BTreeMap::new();
    };

    split_storage_words(inner)
        .into_iter()
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            let index = key
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<usize>()
                .ok()?;
            Some((index, unquote_storage_value(value)))
        })
        .collect()
}

fn assoc_entries(value: &str) -> Vec<(String, String)> {
    let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Vec::new();
    };

    split_storage_words(inner)
        .into_iter()
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            Some((
                unquote_storage_value(key.trim_start_matches('[').trim_end_matches(']')),
                unquote_storage_value(value),
            ))
        })
        .collect()
}

fn format_indexed_storage(entries: BTreeMap<usize, String>) -> String {
    let rendered = entries
        .into_iter()
        .map(|(index, value)| format!("[{index}]={}", quote_storage_value(&value)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("{STORAGE_WORD_PREFIX_STR}({rendered})")
}

fn format_assoc_storage(entries: Vec<(String, String)>) -> String {
    format!(
        "({})",
        entries
            .into_iter()
            .map(|(key, value)| format!(
                "[{}]={}",
                quote_assoc_key(&key),
                quote_storage_value(&value)
            ))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn quote_assoc_key(key: &str) -> String {
    // The storage form is re-parsed by split_storage_words on every
    // read: a bare `'` opens a single-quote span that swallows the
    // rest of the pair list (assoc9.sub printf -v a[$b] with
    // b="80's" stored key `80s`), so it forces quoting like
    // whitespace, `"`, `\`, `]`, backtick and `$` — matching the
    // executor/declare quoters.
    if !key.is_empty()
        && !key
            .chars()
            .any(|ch| ch.is_ascii_whitespace() || matches!(ch, '\'' | '"' | '\\' | ']' | '`' | '$'))
    {
        return key.to_string();
    }
    quote_storage_value(key)
}

fn quote_storage_value(value: &str) -> String {
    if value.contains(['\n', '\r', '\'']) {
        return format!(
            "$'{}'",
            value
                .replace('\\', "\\\\")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\'', "\\'")
        );
    }

    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
            .replace('`', "\\`")
    )
}

fn unquote_storage_value(value: &str) -> String {
    if let Some(inner) = value
        .strip_prefix("$'")
        .and_then(|value| value.strip_suffix('\''))
    {
        return inner
            .replace("\\n", "\n")
            .replace("\\r", "\r")
            .replace("\\'", "'")
            .replace("\\\\", "\\");
    }

    if let Some(inner) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        return unescape_double_quoted_storage(inner);
    }

    value.to_string()
}

fn unescape_double_quoted_storage(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(escaped) = chars.next() {
                output.push(escaped);
            }
        } else {
            output.push(ch);
        }
    }
    output
}

fn split_storage_words(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        match quote {
            Some(quote_ch) => {
                current.push(ch);
                if ch == '\\' {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                } else if ch == quote_ch {
                    quote = None;
                }
            }
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                current.push(ch);
            }
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

fn is_marked(env_vars: &HashMap<String, String>, marker: &str, name: &str) -> bool {
    env_vars
        .get(marker)
        .map(|value| value.split(DATA_DOLLAR).any(|marked| marked == name))
        .unwrap_or(false)
}

fn mark_printf_var(env_vars: &mut HashMap<String, String>, marker: &str, name: &str) {
    if is_marked(env_vars, marker, name) {
        return;
    }
    env_vars
        .entry(marker.to_string())
        .and_modify(|value| {
            if !value.is_empty() {
                value.push(DATA_DOLLAR);
            }
            value.push_str(name);
        })
        .or_insert_with(|| name.to_string());
}

fn render(format: &str, args: &[&str], env_vars: &mut HashMap<String, String>) -> RenderedPrintf {
    let mut output = String::new();
    let mut arg_index = 0;
    let mut errors = Vec::new();

    if args.is_empty() {
        return render_one_pass(format, args, &mut arg_index, output, env_vars);
    }

    while arg_index < args.len() {
        let before_arg = arg_index;
        let rendered = render_one_pass(format, args, &mut arg_index, output, env_vars);
        output = rendered.output;
        errors.extend(rendered.errors);
        if rendered.stop_output {
            return RenderedPrintf {
                output,
                status: status_from_errors(&errors),
                errors,
                stop_output: true,
            };
        }

        if arg_index == before_arg {
            break;
        }
    }

    RenderedPrintf {
        output,
        status: status_from_errors(&errors),
        errors,
        stop_output: false,
    }
}

fn render_one_pass(
    format: &str,
    args: &[&str],
    arg_index: &mut usize,
    mut output: String,
    env_vars: &mut HashMap<String, String>,
) -> RenderedPrintf {
    let mut chars = format.chars().peekable();
    let mut errors = Vec::new();

    while let Some(ch) = chars.next() {
        match ch {
            '\\' => output.push_str(&expand_format_escape(&mut chars)),
            '%' => {
                if chars.peek() == Some(&'%') {
                    chars.next();
                    output.push('%');
                    continue;
                }

                let mut spec = match parse_format_spec(&mut chars) {
                    ParsedFormat::Spec(spec) => spec,
                    ParsedFormat::Missing(format) => {
                        return RenderedPrintf {
                            output,
                            status: EXECUTION_FAILURE,
                            errors: vec![format!(
                                "{}printf: `{format}': missing format character",
                                diagnostic_prefix(env_vars)
                            )],
                            stop_output: true,
                        };
                    }
                };

                if spec.time_format.is_some() && spec.specifier != 'T' {
                    errors.push(format!(
                        "{}printf: warning: `{}': invalid time format specification",
                        diagnostic_prefix(env_vars),
                        spec.specifier
                    ));
                    output.push_str(&spec.raw);
                    continue;
                }

                if !valid_format_specifier(spec.specifier) {
                    return RenderedPrintf {
                        output,
                        status: EXECUTION_FAILURE,
                        errors: vec![format!(
                            "{}printf: `{}': invalid format character",
                            diagnostic_prefix(env_vars),
                            spec.specifier
                        )],
                        stop_output: true,
                    };
                };
                errors.extend(resolve_dynamic_format_args(&mut spec, args, arg_index));

                // GNU printf.def:897-918 decodeint() reports ERANGE when an
                // inline (non-`*`) width or precision overflows int. The
                // flags are set in parse_format_spec; surface them here
                // where the diagnostic prefix is available.
                if spec.inline_width_overflow {
                    errors.push(format!(
                        "{}printf: warning: {}: Numerical result out of range",
                        diagnostic_prefix(env_vars),
                        spec.raw
                    ));
                }
                if spec.inline_precision_overflow {
                    errors.push(format!(
                        "{}printf: warning: {}: Numerical result out of range",
                        diagnostic_prefix(env_vars),
                        spec.raw
                    ));
                }

                if spec.specifier == 'n' {
                    let name = next_arg(args, arg_index);
                    if valid_identifier(name) {
                        env_vars.insert(name.to_string(), output.chars().count().to_string());
                    }
                } else if spec.specifier == 'T' {
                    let value = if *arg_index < args.len() {
                        next_arg(args, arg_index)
                    } else {
                        "-1"
                    };
                    let (rendered, error) = format_time_value(value, &spec, env_vars);
                    if let Some(error) = error {
                        errors.push(error);
                    }
                    output.push_str(&rendered);
                } else {
                    let value = next_arg(args, arg_index);
                    let (rendered, stop_output, error) = format_value(value, &spec);
                    if let Some(error) = error {
                        errors.push(error);
                    }
                    output.push_str(&rendered);
                    if stop_output {
                        return RenderedPrintf {
                            output,
                            status: status_from_errors(&errors),
                            errors,
                            stop_output: true,
                        };
                    }
                }
            }
            other => output.push(other),
        }
    }
    RenderedPrintf {
        output,
        status: status_from_errors(&errors),
        errors,
        stop_output: false,
    }
}

fn status_from_errors(errors: &[String]) -> i32 {
    if errors
        .iter()
        .all(|error| error.contains(": printf: warning:"))
    {
        EXECUTION_SUCCESS
    } else if errors.is_empty() {
        EXECUTION_SUCCESS
    } else {
        EXECUTION_FAILURE
    }
}

fn next_arg<'a>(args: &'a [&str], arg_index: &mut usize) -> &'a str {
    let value = args.get(*arg_index).copied().unwrap_or("");
    *arg_index += 1;
    value
}

#[cfg(test)]
#[path = "printf_tests.rs"]
mod tests;
