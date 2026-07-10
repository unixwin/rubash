use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

use super::marks::marked_vars;
use super::value::{format_array_value, is_array_value, quote_export_value};
use super::{
    ARRAY_VARS, EXPORTED_VARS, INTEGER_VARS, LOWERCASE_VARS, READONLY_VARS, UPPERCASE_VARS,
};

pub(super) fn print_readonly<W>(
    env_vars: &HashMap<String, String>,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    let readonly = marked_vars(env_vars, READONLY_VARS);
    let exported = marked_vars(env_vars, EXPORTED_VARS);
    let arrays = marked_vars(env_vars, ARRAY_VARS);
    let integers = marked_vars(env_vars, INTEGER_VARS);
    let uppercase = marked_vars(env_vars, UPPERCASE_VARS);
    let lowercase = marked_vars(env_vars, LOWERCASE_VARS);
    let mut names: Vec<_> = readonly.into_iter().collect();
    names.sort();
    for name in names {
        if let Some(value) = env_vars.get(&name) {
            if arrays.contains(&name) || is_array_value(value) {
                let attrs = setattr_array_attrs(
                    &name,
                    true,
                    exported.contains(&name),
                    &integers,
                    &uppercase,
                    &lowercase,
                );
                writeln!(
                    stdout,
                    "declare {attrs} {name}={}",
                    format_array_value(value)
                )?;
            } else {
                let attrs = setattr_scalar_attrs(
                    &name,
                    true,
                    exported.contains(&name),
                    &integers,
                    &uppercase,
                    &lowercase,
                );
                writeln!(
                    stdout,
                    "declare {attrs} {name}=\"{}\"",
                    quote_export_value(value)
                )?;
            }
        } else {
            let attrs = setattr_scalar_attrs(
                &name,
                true,
                exported.contains(&name),
                &integers,
                &uppercase,
                &lowercase,
            );
            writeln!(stdout, "declare {attrs} {name}")?;
        }
    }
    Ok(())
}

pub(super) fn print_exported<W>(
    env_vars: &HashMap<String, String>,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    let readonly = marked_vars(env_vars, READONLY_VARS);
    let arrays = marked_vars(env_vars, ARRAY_VARS);
    let integers = marked_vars(env_vars, INTEGER_VARS);
    let uppercase = marked_vars(env_vars, UPPERCASE_VARS);
    let lowercase = marked_vars(env_vars, LOWERCASE_VARS);
    let mut names: Vec<_> = marked_vars(env_vars, EXPORTED_VARS).into_iter().collect();
    names.sort();

    for name in names {
        if name.starts_with("__RUBASH_") {
            continue;
        }
        if let Some(value) = env_vars.get(&name) {
            if arrays.contains(&name) || is_array_value(value) {
                let attrs = setattr_array_attrs(
                    &name,
                    readonly.contains(&name),
                    true,
                    &integers,
                    &uppercase,
                    &lowercase,
                );
                writeln!(
                    stdout,
                    "declare {attrs} {name}={}",
                    format_array_value(value)
                )?;
            } else {
                let attrs = setattr_scalar_attrs(
                    &name,
                    readonly.contains(&name),
                    true,
                    &integers,
                    &uppercase,
                    &lowercase,
                );
                writeln!(
                    stdout,
                    "declare {attrs} {}=\"{}\"",
                    name,
                    quote_export_value(value)
                )?;
            }
        } else {
            let attrs = setattr_scalar_attrs(
                &name,
                readonly.contains(&name),
                true,
                &integers,
                &uppercase,
                &lowercase,
            );
            writeln!(stdout, "declare {attrs} {name}")?;
        }
    }

    Ok(())
}

fn setattr_scalar_attrs(
    name: &str,
    readonly: bool,
    exported: bool,
    integers: &HashSet<String>,
    uppercase: &HashSet<String>,
    lowercase: &HashSet<String>,
) -> String {
    let mut attrs = String::from("-");
    if integers.contains(name) {
        attrs.push('i');
    }
    if readonly {
        attrs.push('r');
    }
    if exported {
        attrs.push('x');
    }
    if lowercase.contains(name) {
        attrs.push('l');
    }
    if uppercase.contains(name) {
        attrs.push('u');
    }
    attrs
}

fn setattr_array_attrs(
    name: &str,
    readonly: bool,
    exported: bool,
    integers: &HashSet<String>,
    uppercase: &HashSet<String>,
    lowercase: &HashSet<String>,
) -> String {
    let mut attrs = String::from("-a");
    attrs.push_str(
        setattr_scalar_attrs(name, readonly, exported, integers, uppercase, lowercase)
            .trim_start_matches('-'),
    );
    attrs
}
