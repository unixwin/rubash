//! shopt module.
//!
//! GNU Bash source ownership:
// - builtins/shopt.def

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;

pub fn execute(args: &[String]) -> io::Result<i32> {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    execute_with_io(args, &mut stdout, &mut stderr)
}

fn execute_with_io<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let mut print = args.is_empty();
    let mut status = EXECUTION_SUCCESS;

    for arg in args {
        match arg.as_str() {
            "-s" | "-u" | "-q" => {}
            "-p" => print = true,
            option if option.starts_with('-') => {
                writeln!(stderr, "rubash: shopt: {option}: invalid option")?;
                status = EXECUTION_FAILURE;
            }
            _ => {}
        }
    }

    if print {
        writeln!(stdout, "expand_aliases\toff")?;
    }

    Ok(status)
}

