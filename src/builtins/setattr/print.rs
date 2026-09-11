use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

use super::marks::marked_vars;
use super::value::{format_array_value, is_array_value, quote_export_value};
use super::{
    ARRAY_VARS, ASSOC_VARS, EXPORTED_VARS, INTEGER_VARS, LOWERCASE_VARS, READONLY_VARS,
    UPPERCASE_VARS,
};

pub(super) fn print_readonly<W>(
    env_vars: &HashMap<String, String>,
    array_filter: bool,
    assoc_filter: bool,
    posix_mode: bool,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    let readonly = marked_vars(env_vars, READONLY_VARS);
    let exported = marked_vars(env_vars, EXPORTED_VARS);
    let arrays = marked_vars(env_vars, ARRAY_VARS);
    let assocs = marked_vars(env_vars, ASSOC_VARS);
    let integers = marked_vars(env_vars, INTEGER_VARS);
    let uppercase = marked_vars(env_vars, UPPERCASE_VARS);
    let lowercase = marked_vars(env_vars, LOWERCASE_VARS);
    let mut names: Vec<_> = readonly.into_iter().collect();
    names.sort();
    for name in names {
        // GNU setattr.def: 'readonly -a'/'readonly -A' without names list
        // only the readonly indexed/associative arrays, while 'readonly -p'
        // lists every readonly variable. The -a/-A attribute flags double as
        // listing filters.
        let is_array = arrays.contains(&name)
            || env_vars
                .get(&name)
                .is_some_and(|value| is_array_value(value));
        let is_assoc = assocs.contains(&name);
        if array_filter && !is_array {
            continue;
        }
        if assoc_filter && !is_assoc {
            continue;
        }
        if let Some(value) = env_vars.get(&name) {
            if is_array {
                // GNU setattr.def: an empty readonly array (declared with a
                // size hint, never assigned elements) lists without the value
                // part -- "declare -ar c" in the default listing and
                // "readonly -a c" in posix mode (array.tests:62,101,103).
                if value.is_empty() {
                    if posix_mode {
                        writeln!(stdout, "readonly -a {name}")?;
                    } else {
                        let attrs = setattr_array_attrs(
                            &name,
                            true,
                            exported.contains(&name),
                            &integers,
                            &uppercase,
                            &lowercase,
                        );
                        writeln!(stdout, "declare {attrs} {name}")?;
                    }
                    continue;
                }
                // POSIX mode (bash posix-mode notes): readonly displays
                // "readonly -a name=value" for arrays instead of the
                // declare-format listing (array.tests readonly -a probe).
                if posix_mode {
                    writeln!(stdout, "readonly -a {name}={}", format_array_value(value))?;
                    continue;
                }
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
        } else if posix_mode && is_array {
            writeln!(stdout, "readonly -a {name}")?;
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
