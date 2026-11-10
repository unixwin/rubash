use super::*;

/// How one `BASH_FUNC_*%%` entry's import failed, split the way GNU
/// splits it (rubash#2728). The caller owns printing: both classes end
/// with the single `error importing function definition` line
/// (variables.c:441), but what precedes it differs by class.
pub(in crate::executor) enum ImportedFunctionFailure {
    /// The value engaged the import (starts with `() {`) but the
    /// parser rejected the rebuilt definition: GNU's parse_and_execute
    /// prints the parser's own diagnostics (builtins/evalstring.c,
    /// parse_command failure arm) — `(line, message)` pairs in GNU's
    /// line numbering (the definition's first line is line 0) — and
    /// NO warning line.
    Parse {
        name: String,
        diagnostics: Vec<(usize, String)>,
    },
    /// The value parsed but is not one straight function definition
    /// (builtins/evalstring.c:465-486 SEVAL_FUNCDEF check): GNU prints
    /// `warning: <name>: ignoring function definition attempt` and
    /// nothing else before the caller's error line.
    IgnoredAttempt { name: String },
}

pub(in crate::executor) fn import_exported_functions_from_env(
    env_vars: &HashMap<String, String>,
) -> (
    HashMap<String, FunctionBody>,
    HashMap<String, crate::executor::FunctionDefInfo>,
    Vec<ImportedFunctionFailure>,
) {
    let mut functions = HashMap::new();
    let mut def_infos = HashMap::new();
    let mut failed = Vec::new();
    for (env_name, value) in env_vars {
        let Some(name) = imported_function_name(env_name) else {
            continue;
        };
        let (body, def_redirects) = match classify_exported_function_value(name, value) {
            ExportedFunctionParse::Defined(body, def_redirects) => (body, def_redirects),
            // GNU variables.c: the `() {` prefix check failed, so the
            // entry never engaged the function import — it is bound as
            // an ordinary variable and the function silently stays
            // undefined. Not a failure, no diagnostic (rubash#2728).
            ExportedFunctionParse::NotAFunctionValue => continue,
            ExportedFunctionParse::ParseFailure(diagnostics) => {
                failed.push(ImportedFunctionFailure::Parse {
                    name: name.to_string(),
                    diagnostics,
                });
                continue;
            }
            ExportedFunctionParse::IgnoredAttempt => {
                failed.push(ImportedFunctionFailure::IgnoredAttempt {
                    name: name.to_string(),
                });
                continue;
            }
        };
        functions.insert(name.to_string(), Rc::new(Ast { commands: body }));
        // The exportstr carries the function-definition redirections after
        // the closing brace (`() { ... } 1>&2`), the way GNU's
        // named_function_string(FUNC_EXTERNAL) renders them, so a re-imported
        // function still prints them.
        def_infos.insert(
            name.to_string(),
            crate::executor::FunctionDefInfo {
                body_kind: Some(crate::parser::FunctionBodyKind::BraceGroup),
                def_redirects,
            },
        );
    }
    (functions, def_infos, failed)
}

pub(in crate::executor) fn imported_function_name(env_name: &str) -> Option<&str> {
    let name = env_name.strip_prefix("BASH_FUNC_")?.strip_suffix("%%")?;
    if is_imported_function_name(name) {
        Some(name)
    } else {
        None
    }
}

pub(in crate::executor) fn is_imported_function_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('=')
        && !name
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '(' | ')' | '{' | '}' | ';' | '&' | '|'))
}

pub(in crate::executor) fn is_exportable_function_name(name: &str) -> bool {
    is_imported_function_name(name) && !name.contains('/') && !name.contains('\\')
}

/// The four outcomes of one exported-function value, mirroring GNU's
/// import decision tree (rubash#2728):
/// variables.c:395-446 (initialize_shell_variables, FUNCTION_IMPORT)
/// gates on STREQN("() {", string, 4) over the RAW value, rebuilds
/// `name + " " + value`, and runs parse_and_execute with
/// SEVAL_FUNCDEF|SEVAL_ONECMD (builtins/evalstring.c).
pub(in crate::executor) enum ExportedFunctionParse {
    /// One straight function definition: defined, att_exported kept.
    Defined(Vec<CommandNode>, Vec<crate::parser::Redirect>),
    /// The `() {` prefix check failed: the entry is an ordinary
    /// variable; the function silently stays undefined.
    NotAFunctionValue,
    /// parse_command failed: the parser's diagnostics, as
    /// `(line, message)` pairs in GNU's line numbering.
    ParseFailure(Vec<(usize, String)>),
    /// Parsed, but not one straight function definition
    /// (evalstring.c:465-486): rejected with the warning line.
    IgnoredAttempt,
}

pub(in crate::executor) fn classify_exported_function_value(
    name: &str,
    value: &str,
) -> ExportedFunctionParse {
    // variables.c:404: STREQN("() {", string, 4) on the raw value —
    // no trim: a leading space or a missing space after `()` means
    // the entry never engages the import at all.
    if !value.starts_with("() {") {
        return ExportedFunctionParse::NotAFunctionValue;
    }
    // variables.c:415-424: the definition text is rebuilt as
    // `name + " " + value` and parsed under the function's own name.
    let synthetic = format!("{name} {value}");
    let tokens = crate::lexer::tokenize(&synthetic);
    let ast = crate::parser::parse(&tokens);
    // A syntax error anywhere in the definition bubbles to the top
    // command's marker assignments (parser propagate_subtree_parse_
    // error), which is exactly the unit GNU's parse_command aborts.
    if let Some(marked) = ast
        .commands
        .iter()
        .find(|command| command.has_parse_error_marker())
    {
        return ExportedFunctionParse::ParseFailure(import_parse_diagnostics(marked, &synthetic));
    }
    let straight_definition = |cmd: &CommandNode| {
        cmd.function_command
            .as_ref()
            .filter(|func| func.name == name)
            .map(|func| {
                (
                    func.body.clone(),
                    crate::parser::ast_print::collected_redirects(cmd),
                )
            })
    };
    match ast.commands.as_slice() {
        [only] => match straight_definition(only) {
            Some((body, def_redirects)) => ExportedFunctionParse::Defined(body, def_redirects),
            None => ExportedFunctionParse::IgnoredAttempt,
        },
        [first, second, ..] => {
            // SEVAL_ONECMD (evalstring.c:577): parse_and_execute stops
            // after the first command, so input on LATER lines is never
            // read and the definition stands; a second command sharing
            // the definition's line means the parsed unit was not one
            // straight definition (the ShellShock shape).
            let def_end = first.end_line.or(first.line).unwrap_or(1);
            let next_line = second.line.unwrap_or(usize::MAX);
            if next_line > def_end {
                if let Some((body, def_redirects)) = straight_definition(first) {
                    return ExportedFunctionParse::Defined(body, def_redirects);
                }
            }
            ExportedFunctionParse::IgnoredAttempt
        }
        [] => ExportedFunctionParse::IgnoredAttempt,
    }
}

/// The parser's own diagnostics for a rejected import, in the shape
/// GNU's yyerror path prints them (parse.y report_syntax_error via
/// error.c parser_error): a `syntax error ...` line, plus — for the
/// near-token and plain classes — the offending line echoed verbatim.
/// Lines are converted to GNU's string-input numbering: the
/// definition's first line is line 0 where the synthetic text counts
/// it as line 1.
fn import_parse_diagnostics(cmd: &CommandNode, synthetic: &str) -> Vec<(usize, String)> {
    let gnu_line = |line: usize| line.saturating_sub(1);
    // The offending-line echo shares the diagnostic's GNU line number
    // and reproduces that line of the rebuilt definition verbatim.
    let echo = |line: usize| -> (usize, String) {
        let text = synthetic
            .lines()
            .nth(line.saturating_sub(1))
            .unwrap_or_default();
        (gnu_line(line), format!("`{text}'"))
    };
    if let Some(spec) = cmd.get_assignment("__RUBASH_PARSE_ERROR_NEAR__") {
        let mut fields = spec.split(crate::executor::markers::PARSE_ERROR_FIELD_SEP);
        let token = fields.next().unwrap_or_default();
        let line = fields
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_else(|| cmd.line.unwrap_or(1));
        return vec![
            (
                gnu_line(line),
                format!("syntax error near unexpected token `{token}'"),
            ),
            echo(line),
        ];
    }
    if let Some(spec) = cmd.get_assignment("__RUBASH_PARSE_ERROR_EOF_COMPOUND__") {
        let mut fields = spec.split(crate::executor::markers::PARSE_ERROR_FIELD_SEP);
        let compound_name = fields.next().unwrap_or("(");
        let open_line = fields
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1);
        let eof_line = fields
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1);
        return vec![(
            gnu_line(eof_line),
            format!(
                "syntax error: unexpected end of file from `{compound_name}' command on line {}",
                gnu_line(open_line)
            ),
        )];
    }
    if cmd.has_assignment("__RUBASH_PARSE_ERROR_EOF__") {
        let line = cmd.line.unwrap_or(1);
        return vec![(line, "syntax error: unexpected end of file".to_string())];
    }
    if let Some(spec) = cmd.get_assignment("__RUBASH_PARSE_ERROR_COND__") {
        // Conditional diagnostics carry their own message fields after
        // (shape, token, line): (line, message) pairs, each rendered
        // verbatim — the same reading command_execute's COND arm uses.
        let mut fields = spec.split(crate::executor::markers::PARSE_ERROR_FIELD_SEP);
        let _shape = fields.next();
        let _token = fields.next();
        let _line = fields.next();
        let mut out = Vec::new();
        while let (Some(line), Some(message)) = (fields.next(), fields.next()) {
            let line = line.parse::<usize>().unwrap_or(1);
            out.push((gnu_line(line), message.to_string()));
        }
        if !out.is_empty() {
            return out;
        }
    }
    let message = cmd
        .get_assignment("__RUBASH_PARSE_ERROR__")
        .map(String::as_str)
        .unwrap_or("unexpected token");
    let line = cmd.line.unwrap_or(1);
    if message.starts_with("syntax error:") || message.starts_with("unexpected EOF") {
        vec![(gnu_line(line), message.to_string())]
    } else {
        vec![
            (gnu_line(line), format!("syntax error near {message}")),
            echo(line),
        ]
    }
}

pub(in crate::executor) fn parse_exported_function_body(
    value: &str,
) -> Option<(Vec<CommandNode>, Vec<crate::parser::Redirect>)> {
    // Carrier-comparison callers (compound_exec) only need the
    // defined-or-not reading under a name that is already a live
    // function; the name is irrelevant to the body shape, so reuse
    // the classifier under a placeholder.
    match classify_exported_function_value("__rubash_carrier_probe", value) {
        ExportedFunctionParse::Defined(body, def_redirects) => Some((body, def_redirects)),
        _ => None,
    }
}

pub(in crate::executor) fn exported_function_env_name(name: &str) -> String {
    format!("BASH_FUNC_{name}%%")
}

pub(in crate::executor) fn exported_function_env_value(
    body: &[CommandNode],
    def_redirects: &[crate::parser::Redirect],
) -> String {
    if body.is_empty() {
        if def_redirects.is_empty() {
            "() { :; }".to_string()
        } else {
            format!("() {{ :; }}{}", redirect_suffix_text(def_redirects))
        }
    } else {
        // GNU variables.c:4875 make_env_array_from_var_list renders every
        // exported function with named_function_string(NULL, cmd, 0) — a
        // full AST reprint (comments dropped at parse, `$'..'` words
        // carried decoded and single-quoted) in the non-multiline export
        // form: indentation 1 / indentation_amount 0 (print_cmd.c:1428-
        // 1431), so `() {  a;\n b\n}` with nested keywords at one space.
        // The connector rules (`;` separator, `;`-after-newline
        // suppression, `&`-connector space) live in the printer's
        // print_command_list, the same port declare -f uses (rubash#1057,
        // #294, #2500).
        crate::parser::ast_print::exported_function_string(body, def_redirects)
    }
}

/// Function-definition redirections rendered after the closing brace of an
/// exportstr value (print_cmd.c prints them via print_redirection_list).
fn redirect_suffix_text(def_redirects: &[crate::parser::Redirect]) -> String {
    if def_redirects.is_empty() {
        return String::new();
    }
    format!(
        " {}",
        crate::parser::ast_print::redirect_list_text(def_redirects)
    )
}

/// Whether an export/readonly invocation diverts to its `-f`
/// function face: a MINUS-signed `f` in the option region, scanned
/// with the row's own machine (Stage-2 ④ Phase 4 — the old
/// `contains('f')` walk also fired on an `f` after an invalid
/// letter, e.g. `-Zf`, which the machine stops at). Diagnostics go
/// to a discard sink: the face reports them itself.
fn setattr_args_request_functions(row_name: &str, args: &[String]) -> bool {
    let row = crate::executor::builtin_table::builtin_table_row(row_name).expect("setattr row");
    let mut sink = Vec::new();
    let mut diag = crate::executor::builtin_diag::Diag::new(row, &mut sink);
    let mut opt = crate::executor::builtin_opt::BuiltinOpt::new(row, args, &mut diag, false);
    loop {
        match opt.next() {
            crate::executor::builtin_opt::OptEvent::Opt(c) => {
                if c == b'f' && opt.opttype() == b'-' {
                    return true;
                }
            }
            crate::executor::builtin_opt::OptEvent::OptArg(_, _) => {}
            crate::executor::builtin_opt::OptEvent::Help
            | crate::executor::builtin_opt::OptEvent::End => return false,
        }
    }
}

pub(in crate::executor) fn export_args_request_functions(args: &[String]) -> bool {
    setattr_args_request_functions("export", args)
}

pub(in crate::executor) fn readonly_args_request_functions(args: &[String]) -> bool {
    setattr_args_request_functions("readonly", args)
}
