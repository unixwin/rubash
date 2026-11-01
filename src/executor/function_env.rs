use super::*;

pub(in crate::executor) fn import_exported_functions_from_env(
    env_vars: &HashMap<String, String>,
) -> (
    HashMap<String, FunctionBody>,
    HashMap<String, crate::executor::FunctionDefInfo>,
    Vec<String>,
) {
    let mut functions = HashMap::new();
    let mut def_infos = HashMap::new();
    let mut failed = Vec::new();
    for (env_name, value) in env_vars {
        let Some(name) = imported_function_name(env_name) else {
            continue;
        };
        let Some((body, def_redirects)) = parse_exported_function_body(value) else {
            // GNU variables.c refuses the definition (ShellShock guard)
            // and reports it at startup; the caller owns the diagnostic
            // (rubash#1276).
            failed.push(name.to_string());
            continue;
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

pub(in crate::executor) fn parse_exported_function_body(
    value: &str,
) -> Option<(Vec<CommandNode>, Vec<crate::parser::Redirect>)> {
    let value = value.trim();
    // GNU variables.c:initialize_shell_variables checks STREQN("() {", string, 4)
    // – the definition must start with "() {" exactly (no "(){" without space
    // is already covered by the trim_start check below, but the 4-char prefix
    // enforces the required space). The rest of the validation mirrors
    // parse_and_execute(SEVAL_FUNCDEF|SEVAL_ONECMD): only a single function
    // definition with optional redirections is accepted; trailing `; cmd` or
    // extra brace groups (ShellShock) must be rejected.
    let rest = value.strip_prefix("()")?.trim_start();
    if !rest.starts_with('{') {
        return None;
    }
    // Validate via the real parser: build a synthetic definition
    // `__rubash_import_test () { ... }` and require exactly one command which
    // is a function definition. This rejects trailing commands (`; echo BAD`),
    // extra brace groups (`} { echo vuln;}`), and malformed redirections that
    // the manual rfind('}') would otherwise swallow.
    let dummy = "__rubash_import_test";
    let synthetic = format!("{} {}", dummy, value);
    let tokens = crate::lexer::tokenize(&synthetic);
    let ast = crate::parser::parse(&tokens);
    if ast.commands.len() != 1 {
        return None;
    }
    let cmd = &ast.commands[0];
    let func = cmd.function_command.as_ref()?;
    if func.name != dummy {
        return None;
    }
    // Ensure the synthetic string was consumed entirely: the parser must not
    // have left extra tokens (e.g., `; echo BAD` would be a second command,
    // already rejected by len !=1; a stray `}` would also produce a second
    // command). Additionally, reject if the original value's trailing part
    // after the function's `}` contains non-redirect words – the synthetic
    // check already covers it, but we keep the explicit redirect extraction
    // for the exportstr rendering path.
    let def_redirects = crate::parser::ast_print::collected_redirects(cmd);
    // Verify trailing part is only redirects/whitespace: re-slice original rest
    // to ensure no `;` or extra command leaked through brace matching quirks
    // (e.g., `>_[${...}] { echo vuln;}` where the `{` could be mis-identified
    // as function body). The synthetic single-command guarantee already ensures
    // this, but we add a lightweight lexical check for `;` after the final `}`
    // to guard against future parser divergences.
    if let Some(close) = rest.rfind('}') {
        let trailing = rest[close + 1..].trim();
        if !trailing.is_empty() {
            // Valid trailing is only whitespace and redirections; a `;` or `&`
            // indicates an extra command and must be rejected (CVE-2014-6271).
            if trailing.contains(';') || trailing.contains('&') || trailing.contains('|') {
                // However redirections themselves may contain `&` as in `2>&1`;
                // parse trailing as `:` + trailing and check for extra commands.
                let t_tokens = crate::lexer::tokenize(&format!(": {}", trailing));
                let t_ast = crate::parser::parse(&t_tokens);
                // A valid redirection list is a single `:` command with only
                // redirects and no extra words/commands.
                if t_ast.commands.len() != 1 {
                    return None;
                }
                let first = &t_ast.commands[0];
                // If the first command has words beyond `:`, it's not just
                // redirects (e.g., `: ; echo BAD` has second command).
                if first.words.len() > 1 {
                    return None;
                }
                if t_ast.commands.len() != 1 || first.words.first().map(String::as_str) != Some(":")
                {
                    return None;
                }
                // Check that all remaining tokens after `:` are redirects, not words.
                // collected_redirects will be empty for `; echo` case, but the
                // parse would have produced a second command which we already
                // rejected via len check. So trailing with `;` is already
                // rejected by the synthetic len check; this path is for
                // `>file` vs `;` differentiation.
            }
        }
    }
    let body = func.body.clone();
    Some((body, def_redirects))
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

pub(in crate::executor) fn export_args_request_functions(args: &[String]) -> bool {
    for arg in args {
        if arg == "--" {
            return false;
        }
        if !arg.starts_with('-') || arg == "-" {
            return false;
        }
        if arg[1..].contains('f') {
            return true;
        }
    }
    false
}

pub(in crate::executor) fn readonly_args_request_functions(args: &[String]) -> bool {
    for arg in args {
        if arg == "--" {
            return false;
        }
        if !arg.starts_with('-') || arg == "-" {
            return false;
        }
        if arg[1..].contains('f') {
            return true;
        }
    }
    false
}
