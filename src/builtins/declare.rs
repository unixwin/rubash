//! declare module.
//!
//! GNU Bash source ownership:
// - builtins/declare.def

use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;

pub fn execute(args: &[String], variables: &HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    execute_with_io(args, variables, &mut stdout, &mut stderr)
}

fn execute_with_io<W, E>(
    args: &[String],
    variables: &HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let mut print = false;
    let mut names = Vec::new();

    for arg in args {
        if arg == "-p" {
            print = true;
        } else if arg.starts_with('-') {
            writeln!(stderr, "rubash: declare: {}: unsupported option", arg)?;
            return Ok(EXECUTION_FAILURE);
        } else {
            names.push(arg.as_str());
        }
    }

    if !print {
        return Ok(EXECUTION_SUCCESS);
    }

    let mut status = EXECUTION_SUCCESS;
    for name in names {
        if let Some(value) = variables.get(name) {
            print_declaration(name, value, stdout)?;
        } else {
            writeln!(stderr, "rubash: declare: {}: not found", name)?;
            status = EXECUTION_FAILURE;
        }
    }

    Ok(status)
}

fn print_declaration<W>(name: &str, value: &str, stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    if let Some(array_value) = parse_single_element_array(value) {
        writeln!(stdout, "declare -a {}=([0]=\"{}\")", name, quote_double(array_value))
    } else {
        writeln!(stdout, "declare -- {}=\"{}\"", name, quote_double(value))
    }
}

fn parse_single_element_array(value: &str) -> Option<&str> {
    value.strip_prefix('(')?.strip_suffix(')')
}

fn quote_double(value: &str) -> String {
    let mut quoted = String::new();
    for ch in value.chars() {
        match ch {
            '\\' | '"' | '$' | '`' => {
                quoted.push('\\');
                quoted.push(ch);
            }
            _ => quoted.push(ch),
        }
    }
    quoted
}

