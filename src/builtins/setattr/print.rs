use std::collections::HashSet;
use std::io::{self, Write};

use super::marks::marked_vars;
use super::value::{
    format_array_value, is_array_value, quote_export_value, quote_listing_value, valid_identifier,
};
use super::{
    ARRAY_VARS, ASSOC_VARS, EXPORTED_VARS, INTEGER_VARS, LOWERCASE_VARS, READONLY_VARS,
    UPPERCASE_VARS,
};

pub(super) fn print_readonly<W>(
    env_vars: &crate::shell::var_table::VarTable,
    array_filter: bool,
    assoc_filter: bool,
    posix_mode: bool,
    underlay: &crate::shell::var_table::TempenvListingUnderlay,
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
        // rubash#1569: a name bound by the current command's own
        // tempenv frame renders its pre-frame binding (or nothing, when
        // the frame created it / the pre-frame binding was not
        // readonly) — the tempvar itself is never listed.
        if let Some(under) = underlay.get(&name) {
            let Some((under_value, under_attrs)) = under else {
                continue;
            };
            if !under_attrs.readonly {
                continue;
            }
            let is_array = under_attrs.array
                || under_attrs.assoc
                || under_value.as_deref().is_some_and(is_array_value);
            let is_assoc = under_attrs.assoc;
            if array_filter && !is_array {
                continue;
            }
            if assoc_filter && !is_assoc {
                continue;
            }
            if !valid_identifier(&name) {
                continue;
            }
            match under_value {
                Some(value) if is_array => {
                    if value.is_empty() {
                        if posix_mode {
                            writeln!(
                                stdout,
                                "{}",
                                posix_valueless_line(
                                    "readonly",
                                    under_attrs.array,
                                    is_assoc,
                                    &name
                                )
                            )?;
                        } else {
                            let attrs = setattr_array_attrs_for(
                                true,
                                under_attrs.exported,
                                under_attrs.integer,
                                under_attrs.uppercase,
                                under_attrs.lowercase,
                                under_attrs.assoc,
                            );
                            writeln!(stdout, "declare {attrs} {name}")?;
                        }
                        continue;
                    }
                    if posix_mode {
                        writeln!(
                            stdout,
                            "readonly {} {name}={}",
                            if is_assoc { "-A" } else { "-a" },
                            format_array_for_listing(env_vars, &name, is_assoc, value)
                        )?;
                        continue;
                    }
                    let attrs = setattr_array_attrs_for(
                        true,
                        under_attrs.exported,
                        under_attrs.integer,
                        under_attrs.uppercase,
                        under_attrs.lowercase,
                        under_attrs.assoc,
                    );
                    writeln!(
                        stdout,
                        "declare {attrs} {name}={}",
                        format_array_for_listing(env_vars, &name, is_assoc, value)
                    )?;
                }
                Some(value) => {
                    if posix_mode {
                        writeln!(stdout, "readonly {name}=\"{}\"", quote_export_value(value))?;
                        continue;
                    }
                    let attrs = setattr_scalar_attrs_for(
                        true,
                        under_attrs.exported,
                        under_attrs.integer,
                        under_attrs.uppercase,
                        under_attrs.lowercase,
                    );
                    writeln!(
                        stdout,
                        "declare {attrs} {name}={}",
                        quote_listing_value(value)
                    )?;
                }
                None => {
                    if posix_mode {
                        writeln!(
                            stdout,
                            "{}",
                            posix_valueless_line("readonly", is_array, is_assoc, &name)
                        )?;
                    } else {
                        let attrs = setattr_scalar_attrs_for(
                            true,
                            under_attrs.exported,
                            under_attrs.integer,
                            under_attrs.uppercase,
                            under_attrs.lowercase,
                        );
                        writeln!(stdout, "declare {attrs} {name}")?;
                    }
                }
            }
            continue;
        }
        // GNU setattr.def: 'readonly -a'/'readonly -A' without names list
        // only the readonly indexed/associative arrays, while 'readonly -p'
        // lists every readonly variable. The -a/-A attribute flags double as
        // listing filters.
        let is_assoc = assocs.contains(&name);
        let is_array = arrays.contains(&name)
            || is_assoc
            || env_vars
                .get(&name)
                .is_some_and(|value| is_array_value(value));
        if array_filter && !is_array {
            continue;
        }
        if assoc_filter && !is_assoc {
            continue;
        }
        // Same invalid_env rule as print_exported: an inherited entry whose
        // name is not a valid identifier never became a shell variable, so
        // the readonly listing must not resurrect it (niubash issue #102).
        if !valid_identifier(&name) {
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
                        writeln!(
                            stdout,
                            "{}",
                            posix_valueless_line(
                                "readonly",
                                arrays.contains(&name),
                                is_assoc,
                                &name
                            )
                        )?;
                    } else {
                        let attrs = setattr_array_attrs(
                            &name,
                            true,
                            exported.contains(&name),
                            &integers,
                            &uppercase,
                            &lowercase,
                            is_assoc,
                        );
                        writeln!(stdout, "declare {attrs} {name}")?;
                    }
                    continue;
                }
                // POSIX mode (bash posix-mode notes): readonly displays
                // "readonly -a name=value" for arrays instead of the
                // declare-format listing (array.tests readonly -a probe).
                if posix_mode {
                    writeln!(
                        stdout,
                        "readonly {} {name}={}",
                        if is_assoc { "-A" } else { "-a" },
                        format_array_for_listing(env_vars, &name, is_assoc, value)
                    )?;
                    continue;
                }
                let attrs = setattr_array_attrs(
                    &name,
                    true,
                    exported.contains(&name),
                    &integers,
                    &uppercase,
                    &lowercase,
                    is_assoc,
                );
                writeln!(
                    stdout,
                    "declare {attrs} {name}={}",
                    format_array_for_listing(env_vars, &name, is_assoc, value)
                )?;
            } else {
                // POSIX mode (bash posix-mode notes, item 52): readonly
                // displays "readonly name=\"value\"" instead of the
                // declare-format listing.
                if posix_mode {
                    writeln!(stdout, "readonly {name}=\"{}\"", quote_export_value(value))?;
                    continue;
                }
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
                    "declare {attrs} {name}={}",
                    quote_listing_value(value)
                )?;
            }
        } else if posix_mode {
            writeln!(
                stdout,
                "{}",
                posix_valueless_line("readonly", is_array, is_assoc, &name)
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
            writeln!(stdout, "declare {attrs} {name}")?;
        }
    }
    Ok(())
}

/// Render an array/assoc value for the export/readonly listings.
/// Associative values must go through the assoc formatter (declare's
/// storage renderer, ordered by bucket like GNU's print_assoc_assignment);
/// the indexed formatter would misread the assoc mirror as element text
/// (audit B4-F12: `declare -ax A1=([0]="[k]=v")` for a valued assoc).
fn format_array_for_listing(
    env_vars: &crate::shell::var_table::VarTable,
    name: &str,
    is_assoc: bool,
    value: &str,
) -> String {
    if is_assoc {
        let nbuckets = crate::executor::assoc_nbuckets(env_vars, name);
        crate::builtins::declare::format_assoc_for_output(value, nbuckets)
    } else {
        format_array_value(value)
    }
}

/// GNU variables.c (print_var_list in POSIX mode, via setattr.def's
/// posix print path): a variable with NO value prints in the utility
/// form — `export name`, `export -a name`, `export -A name`,
/// `readonly name`, ... — never the `declare -x name` form the default
/// listing uses (audit B4-F12; verified against the 5.3.20 oracle for
/// scalar, indexed-array, and associative valueless names).
fn posix_valueless_line(utility: &str, is_array: bool, is_assoc: bool, name: &str) -> String {
    if is_assoc {
        format!("{utility} -A {name}")
    } else if is_array {
        format!("{utility} -a {name}")
    } else {
        format!("{utility} {name}")
    }
}

pub(super) fn print_exported<W>(
    env_vars: &crate::shell::var_table::VarTable,
    posix_mode: bool,
    filter_array: bool,
    filter_assoc: bool,
    underlay: &crate::shell::var_table::TempenvListingUnderlay,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    let readonly = marked_vars(env_vars, READONLY_VARS);
    let arrays = marked_vars(env_vars, ARRAY_VARS);
    let assocs = marked_vars(env_vars, ASSOC_VARS);
    let integers = marked_vars(env_vars, INTEGER_VARS);
    let uppercase = marked_vars(env_vars, UPPERCASE_VARS);
    let lowercase = marked_vars(env_vars, LOWERCASE_VARS);
    let mut names: Vec<_> = marked_vars(env_vars, EXPORTED_VARS).into_iter().collect();
    names.sort();
    // rubash#1395: bare `export -a`/`-A` must filter by type (GNU lists
    // only exported indexed/assoc arrays, not all exported vars).
    let filter_by_type = filter_array || filter_assoc;
    names.retain(|name| {
        // rubash#1569: a name bound by the current command's own
        // tempenv frame is adjudicated by its pre-frame binding, not
        // by the tempvar's live marks.
        if let Some(under) = underlay.get(name) {
            let Some((_, attrs)) = under else {
                return false;
            };
            if !attrs.exported {
                return false;
            }
            if !filter_by_type {
                return true;
            }
            return (!filter_array || (attrs.array && !attrs.assoc))
                && (!filter_assoc || attrs.assoc);
        }
        if !filter_by_type {
            return true;
        }
        let is_array = arrays.contains(name);
        let is_assoc = assocs.contains(name);
        (!filter_array || (is_array && !is_assoc)) && (!filter_assoc || is_assoc)
    });

    for name in names {
        if name.starts_with("__RUBASH_") {
            continue;
        }
        // GNU variables.c:511-526 (initialize_shell_variables): environment
        // entries whose names are not valid identifiers (Windows inherits
        // names like `CommonProgramFiles(x86)`) are bound into the invisible
        // invalid_env table instead of shell_variables, so the export
        // listing never prints them. They still reach child processes
        // through the export environment (maybe_make_export_env,
        // variables.c:5064). Source-ing a snapshot line like
        // `declare -x CommonProgramFiles(x86)=...` is a bash syntax error,
        // which broke Hermes env snapshots (niubash issue #102).
        if !valid_identifier(&name) {
            continue;
        }
        // rubash#1569: render the pre-frame binding for a name the
        // current command's own tempenv frame bound (the retain above
        // already dropped the names with no exported pre-frame
        // binding, and the names the frame created).
        if let Some(Some((under_value, under_attrs))) = underlay.get(&name) {
            match under_value {
                Some(value) if under_attrs.array || under_attrs.assoc || is_array_value(value) => {
                    if value.is_empty() {
                        // Cell-less pre-frame binding (declared, never
                        // assigned): valueless in both modes, like the
                        // print_readonly underlay arm (audit B4-F12).
                        if posix_mode {
                            writeln!(
                                stdout,
                                "{}",
                                posix_valueless_line(
                                    "export",
                                    under_attrs.array,
                                    under_attrs.assoc,
                                    &name
                                )
                            )?;
                        } else {
                            let attrs = setattr_array_attrs_for(
                                under_attrs.readonly,
                                true,
                                under_attrs.integer,
                                under_attrs.uppercase,
                                under_attrs.lowercase,
                                under_attrs.assoc,
                            );
                            writeln!(stdout, "declare {attrs} {name}")?;
                        }
                        continue;
                    }
                    if posix_mode {
                        // GNU posix utility form carries the variable's
                        // own type flag (-A for associative, -a for
                        // indexed; audit B4-F12).
                        let flag = if under_attrs.assoc { "-A" } else { "-a" };
                        writeln!(
                            stdout,
                            "export {flag} {name}={}",
                            format_array_for_listing(env_vars, &name, under_attrs.assoc, value)
                        )?;
                    } else {
                        let attrs = setattr_array_attrs_for(
                            under_attrs.readonly,
                            true,
                            under_attrs.integer,
                            under_attrs.uppercase,
                            under_attrs.lowercase,
                            under_attrs.assoc,
                        );
                        writeln!(
                            stdout,
                            "declare {attrs} {name}={}",
                            format_array_for_listing(env_vars, &name, under_attrs.assoc, value)
                        )?;
                    }
                }
                Some(value) => {
                    if posix_mode {
                        writeln!(stdout, "export {name}=\"{}\"", quote_export_value(value))?;
                    } else {
                        let attrs = setattr_scalar_attrs_for(
                            under_attrs.readonly,
                            true,
                            under_attrs.integer,
                            under_attrs.uppercase,
                            under_attrs.lowercase,
                        );
                        writeln!(
                            stdout,
                            "declare {attrs} {}={}",
                            name,
                            quote_listing_value(value)
                        )?;
                    }
                }
                None => {
                    if posix_mode {
                        writeln!(
                            stdout,
                            "{}",
                            posix_valueless_line(
                                "export",
                                under_attrs.array,
                                under_attrs.assoc,
                                &name
                            )
                        )?;
                    } else {
                        let attrs = setattr_scalar_attrs_for(
                            under_attrs.readonly,
                            true,
                            under_attrs.integer,
                            under_attrs.uppercase,
                            under_attrs.lowercase,
                        );
                        writeln!(stdout, "declare {attrs} {name}")?;
                    }
                }
            }
            continue;
        }
        if let Some(value) = env_vars.get(&name) {
            let is_assoc = assocs.contains(&name);
            if arrays.contains(&name) || is_assoc || is_array_value(value) {
                // GNU variables.c print_var_list: a declared array/assoc
                // with NO cell (never assigned) is var_isset-false and
                // prints without a value in both modes (the mirror holds
                // "" for it, "()" for an explicitly emptied one) — the
                // same gate print_readonly applies (audit B4-F12).
                if value.is_empty() {
                    if posix_mode {
                        writeln!(
                            stdout,
                            "{}",
                            posix_valueless_line("export", arrays.contains(&name), is_assoc, &name)
                        )?;
                    } else {
                        let attrs = setattr_array_attrs(
                            &name,
                            readonly.contains(&name),
                            true,
                            &integers,
                            &uppercase,
                            &lowercase,
                            is_assoc,
                        );
                        writeln!(stdout, "declare {attrs} {name}")?;
                    }
                    continue;
                }
                // POSIX mode (bash posix-mode notes, item 52): the export
                // listing uses the posix utility format, arrays included
                // ("export -a name=([0]="x" ...)" against GNU 5.3), with
                // the variable's own type flag (-A for associative).
                if posix_mode {
                    let flag = if is_assoc { "-A" } else { "-a" };
                    writeln!(
                        stdout,
                        "export {flag} {name}={}",
                        format_array_for_listing(env_vars, &name, is_assoc, value)
                    )?;
                } else {
                    let attrs = setattr_array_attrs(
                        &name,
                        readonly.contains(&name),
                        true,
                        &integers,
                        &uppercase,
                        &lowercase,
                        is_assoc,
                    );
                    writeln!(
                        stdout,
                        "declare {attrs} {name}={}",
                        format_array_for_listing(env_vars, &name, is_assoc, value)
                    )?;
                }
            } else if posix_mode {
                writeln!(stdout, "export {name}=\"{}\"", quote_export_value(value))?;
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
                    "declare {attrs} {}={}",
                    name,
                    quote_listing_value(value)
                )?;
            }
        } else if posix_mode {
            writeln!(
                stdout,
                "{}",
                posix_valueless_line(
                    "export",
                    arrays.contains(&name),
                    assocs.contains(&name),
                    &name
                )
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
            writeln!(stdout, "declare {attrs} {name}")?;
        }
    }

    Ok(())
}

/// Bool-flag form of `setattr_scalar_attrs` for callers that hold a
/// VarAttrs instead of the marked-name sets (the tempenv underlay
/// renders in print_exported/print_readonly, rubash#1569). The flag
/// order matches the set-based form exactly (i, r, x, l, u).
fn setattr_scalar_attrs_for(
    readonly: bool,
    exported: bool,
    integer: bool,
    uppercase: bool,
    lowercase: bool,
) -> String {
    let mut attrs = String::from("-");
    if integer {
        attrs.push('i');
    }
    if readonly {
        attrs.push('r');
    }
    if exported {
        attrs.push('x');
    }
    if lowercase {
        attrs.push('l');
    }
    if uppercase {
        attrs.push('u');
    }
    attrs
}

fn setattr_array_attrs_for(
    readonly: bool,
    exported: bool,
    integer: bool,
    uppercase: bool,
    lowercase: bool,
    assoc: bool,
) -> String {
    let mut attrs = String::from(if assoc { "-A" } else { "-a" });
    attrs.push_str(
        setattr_scalar_attrs_for(readonly, exported, integer, uppercase, lowercase)
            .trim_start_matches('-'),
    );
    attrs
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
    assoc: bool,
) -> String {
    let mut attrs = String::from(if assoc { "-A" } else { "-a" });
    attrs.push_str(
        setattr_scalar_attrs(name, readonly, exported, integers, uppercase, lowercase)
            .trim_start_matches('-'),
    );
    attrs
}
