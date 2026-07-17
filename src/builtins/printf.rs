//! `printf` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/printf.def (`printf_builtin`)

use std::collections::HashMap;
use std::io::{self, Write};

mod escape;
mod float;
mod identifier;
mod number;
mod spec;
pub(crate) mod time;
mod value;

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
    let args: Vec<&str> = args.into_iter().collect();
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
        writeln!(stderr, "rubash: printf: {}: invalid option", args[index])?;
        writeln!(stderr, "printf: usage: printf [-v var] format [arguments]")?;
        return Ok(EX_USAGE);
    }

    if !end_options {
        let name = match args.get(index) {
            Some(&"-v") => {
                let Some(name) = args.get(index + 1) else {
                    writeln!(stderr, "rubash: printf: -v: option requires an argument")?;
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
            if !valid_identifier(name) {
                writeln!(stderr, "rubash: printf: `{}`: not a valid identifier", name)?;
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
        writeln!(stderr, "rubash: printf: {}: invalid option", args[index])?;
        writeln!(stderr, "printf: usage: printf [-v var] format [arguments]")?;
        return Ok(EX_USAGE);
    }

    let Some(format) = args.get(index) else {
        writeln!(stderr, "printf: usage: printf [-v var] format [arguments]")?;
        return Ok(EX_USAGE);
    };

    let rendered = render(format, &args[index + 1..], env_vars);
    if let Some(name) = output_var {
        env_vars.insert(name.to_string(), rendered.output);
    } else {
        stdout.write_all(rendered.output.as_bytes())?;
    }

    for error in rendered.errors {
        writeln!(stderr, "{error}")?;
    }

    Ok(rendered.status)
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
                                "rubash: printf: `{format}': missing format character"
                            )],
                            stop_output: true,
                        };
                    }
                };

                if spec.time_format.is_some() && spec.specifier != 'T' {
                    errors.push(format!(
                        "rubash: printf: warning: `{}': invalid time format specification",
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
                            "rubash: printf: `{}': invalid format character",
                            spec.specifier
                        )],
                        stop_output: true,
                    };
                };
                errors.extend(resolve_dynamic_format_args(&mut spec, args, arg_index));

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
    if errors.is_empty() {
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
