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
        match crate::expand::arithmetic::eval(expr, variables) {
            Ok(value) => last = value,
            Err(error) => {
                eprintln!("{diagnostic_prefix}let: {}: {}", expr, error.message());
                return 1;
            }
        }
    }

    i32::from(last == 0)
}
