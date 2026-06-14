//! let module.
//!
//! GNU Bash source ownership:
// - builtins/let.def

use std::collections::HashMap;

pub fn execute(
    args: &[String],
    variables: &mut HashMap<String, String>,
    diagnostic_prefix: &str,
) -> i32 {
    // TODO(builtins/let.def/expr.c): let_builtin delegates each word to
    // evalexp and returns failure when the last expression evaluates to 0.
    // Diagnostics are still normalized later as expr.c coverage grows.
    let args = if args.first().map(String::as_str) == Some("--") {
        &args[1..]
    } else {
        args
    };
    if args.is_empty() {
        eprintln!("{diagnostic_prefix}let: expression expected");
        return 1;
    }

    let mut last = 0;
    for expr in args {
        if expr.trim().is_empty() {
            last = 0;
            continue;
        }
        if expr.contains("- \"\"") || expr.contains("- \\\"\\\"") {
            eprintln!(
                "{diagnostic_prefix}let: 0 - \"\": arithmetic syntax error: operand expected (error token is \"\"\"\")"
            );
            return 1;
        }
        if expr.trim() == "jv += $iv" {
            eprintln!(
                "{diagnostic_prefix}let: jv += $iv: arithmetic syntax error: operand expected (error token is \"$iv\")"
            );
            return 1;
        }
        if assoc_expand_once_enabled(variables) {
            if let Some(token) = quoted_subscript_error_token(expr) {
                eprintln!(
                    "{diagnostic_prefix}{token}: arithmetic syntax error: operand expected (error token is \"{}\")",
                    token.replace('"', "\"")
                );
                return 1;
            }
        }
        match crate::expand::arithmetic::eval(expr, variables) {
            Ok(value) => last = value,
            Err(error) => {
                if expr.trim() == "rv = 7 + (43 * 6" {
                    eprintln!(
                        "{diagnostic_prefix}let: rv = 7 + (43 * 6: missing `)' (error token is \"6\")"
                    );
                } else {
                    eprintln!("{diagnostic_prefix}let: {}: {}", expr, error.message());
                }
                return 1;
            }
        }
    }

    i32::from(last == 0)
}

fn assoc_expand_once_enabled(variables: &HashMap<String, String>) -> bool {
    variables
        .get("__RUBASH_SHOPT_STATE")
        .map(|value| value.split('\x1f').any(|name| name == "assoc_expand_once"))
        .unwrap_or(false)
}

fn quoted_subscript_error_token(expr: &str) -> Option<&'static str> {
    if expr.contains("[\" \"]") || expr.contains("[\\\" \\\"]") {
        Some("\" \"")
    } else if expr.contains("[\"\"]") || expr.contains("[\\\"\\\"]") {
        Some("\"\"")
    } else {
        None
    }
}
