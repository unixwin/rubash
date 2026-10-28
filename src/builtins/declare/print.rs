use std::collections::HashSet;
use std::io::{self, Write};

use super::attrs::DeclareOptions;
use super::diagnostic::diagnostic_prefix;
use super::marks::{exported_vars, marked_vars};
use super::names::valid_identifier;
use super::output::{
    print_declaration, print_plain_declaration, print_unset_declaration, DeclarationAttrs,
};
use super::{
    ARRAY_VARS, ASSOC_VARS, DECLARED_UNSET_VARS, EXECUTION_FAILURE, EXPORTED_VARS, INTEGER_VARS,
    LOWERCASE_VARS, NAMEREF_VARS, READONLY_VARS, UPPERCASE_VARS,
};

pub(super) fn print_declare_names<W, E>(
    command_name: &str,
    names: &[&str],
    variables: &crate::shell::var_table::VarTable,
    options: DeclareOptions,
    plain: bool,
    mut status: i32,
    dynamic_views: &[(String, String)],
    underlay: &crate::shell::var_table::TempenvListingUnderlay,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    // rubash#1569: the underlay applies to LISTINGS only. An explicit
    // `declare -p name` lookup goes through find_variable, which sees
    // the tempvar (variables.c:1980) — only the all-variables listing
    // (print_var_list over the real contexts) skips it.
    let empty_underlay = crate::shell::var_table::TempenvListingUnderlay::new();
    let underlay = if names.is_empty() {
        underlay
    } else {
        &empty_underlay
    };
    let filter_exported = options.export;
    let filter_readonly = options.readonly;
    let filter_array = options.array;
    let filter_assoc = options.assoc;
    let filter_integer = options.integer;
    let filter_uppercase = options.uppercase;
    let filter_lowercase = options.lowercase;
    let filter_nameref = options.nameref;

    let names_to_print = if names.is_empty() {
        // Listing mode only: declaration_names_to_print consumes these
        // three sets as candidate sources as well as filters, so they are
        // materialized here — a full listing walks the table anyway. The
        // named-print path below never builds them; its per-name
        // attribute reads are point queries (VarTable::is_marked).
        let uppercase = marked_vars(variables, UPPERCASE_VARS);
        let lowercase = marked_vars(variables, LOWERCASE_VARS);
        let namerefs = marked_vars(variables, NAMEREF_VARS);
        let mut listed = declaration_names_to_print(
            variables,
            filter_exported,
            filter_readonly,
            filter_array,
            filter_assoc,
            filter_integer,
            filter_uppercase,
            filter_lowercase,
            filter_nameref,
            &uppercase,
            &lowercase,
            &namerefs,
        );
        // Dynamic variables whose values live outside the table (a live
        // PIPESTATUS array) have no cell to enumerate, but GNU's full
        // `declare -p` listing includes them once materialized
        // (rubash#1363). The executor passes their views in listing
        // mode only; merge the names that pass the attribute filters —
        // a dynamic view is an array cell, and the remaining filters
        // consult the mark sets exactly like the table-driven listing.
        for (view_name, _) in dynamic_views {
            if listed.iter().any(|name| name == view_name) {
                continue;
            }
            let passes = (!filter_exported || variables.is_marked(EXPORTED_VARS, view_name))
                && (!filter_readonly || variables.is_marked(READONLY_VARS, view_name))
                && (!filter_assoc)
                && (!filter_integer || variables.is_marked(INTEGER_VARS, view_name))
                && (!filter_uppercase || variables.is_marked(UPPERCASE_VARS, view_name))
                && (!filter_lowercase || variables.is_marked(LOWERCASE_VARS, view_name))
                && (!filter_nameref || variables.is_marked(NAMEREF_VARS, view_name));
            if passes {
                listed.push(view_name.clone());
            }
        }
        // rubash#1569: adjudicate the listing against the pre-frame
        // bindings for names the current command's own tempenv frame
        // bound — frame-created names vanish; shadowed names pass or
        // fail the attribute filters by their UNDERLYING attributes
        // and render from the snapshot in the loop below. Names whose
        // underlying binding has no table cell (a declared-but-unset
        // variable) are added here when they pass.
        if !underlay.is_empty() {
            let passes_under = |attrs: &crate::shell::var_table::VarAttrs| {
                (!filter_exported || attrs.exported)
                    && (!filter_readonly || attrs.readonly)
                    && (!filter_array || attrs.array)
                    && (!filter_assoc || attrs.assoc)
                    && (!filter_integer || attrs.integer)
                    && (!filter_uppercase || attrs.uppercase)
                    && (!filter_lowercase || attrs.lowercase)
                    && (!filter_nameref || attrs.nameref)
            };
            listed.retain(|name| match underlay.get(name) {
                None => true,
                Some(None) => false,
                Some(Some((_, attrs))) => passes_under(attrs),
            });
            for (name, under) in underlay.iter() {
                let Some((_, attrs)) = under else {
                    continue;
                };
                if passes_under(attrs) && !listed.iter().any(|listed| listed == name) {
                    listed.push(name.clone());
                }
            }
        }
        listed.sort();
        listed
    } else {
        names
            .iter()
            .copied()
            .map(|name| name.split_once('=').map(|(name, _)| name).unwrap_or(name))
            .map(|name| name.strip_suffix('+').unwrap_or(name))
            .map(str::to_string)
            .collect()
    };
    // rubash#1569: for names the current command's own tempenv frame
    // bound, the per-name attributes come from the pre-frame snapshot,
    // not the tempvar's live marks (capcase has no VarAttrs slot and
    // stays live — a case-converted underlying keeps its mark, which
    // is exactly the attribute the pre-frame binding carries). The
    // reads are point queries: the snapshot answers for underlay
    // names, the live marks for every other name — the same answers
    // the snapshot-adjusted mark sets gave, without materializing
    // (and re-sorting) a name set per attribute for every print.
    let underlay_attr =
        |name: &str, key: &str, pick: fn(&crate::shell::var_table::VarAttrs) -> bool| -> bool {
            match underlay.get(name) {
                Some(under) => under.as_ref().is_some_and(|(_, a)| pick(a)),
                None => variables.is_marked(key, name),
            }
        };
    for name in names_to_print {
        let attrs = DeclarationAttrs {
            exported: underlay_attr(&name, EXPORTED_VARS, |a| a.exported),
            readonly: underlay_attr(&name, READONLY_VARS, |a| a.readonly),
            array: underlay_attr(&name, ARRAY_VARS, |a| a.array),
            assoc: underlay_attr(&name, ASSOC_VARS, |a| a.assoc),
            integer: underlay_attr(&name, INTEGER_VARS, |a| a.integer),
            uppercase: underlay_attr(&name, UPPERCASE_VARS, |a| a.uppercase),
            lowercase: underlay_attr(&name, LOWERCASE_VARS, |a| a.lowercase),
            capcase: variables.is_marked(super::CAPCASE_VARS, &name),
            nameref: underlay_attr(&name, NAMEREF_VARS, |a| a.nameref),
            trace: underlay_attr(&name, super::TRACE_VARS, |a| a.trace),
        };
        // A dynamic variable's cell is whatever the last bind stored; GNU's
        // declare -p reads the dynamic_value getter, so the CURRENT dynamic
        // value replaces any stored shadow bytes (GNU: `export RANDOM=42;
        // declare -p RANDOM` prints the next draw with -ix, rubash#541).
        if let Some(rendered) = dynamic_views
            .iter()
            .find(|(view_name, _)| view_name.as_str() == name.as_str())
            .map(|(_, text)| text)
        {
            writeln!(stdout, "{rendered}")?;
        } else if let Some(Some((under_value, _))) = underlay.get(&name) {
            // rubash#1569: render the pre-frame binding's stored text;
            // a binding that existed without env text (declared but
            // unset) renders as an unset declaration with the snapshot
            // attributes computed above.
            match under_value {
                Some(value) => {
                    let nbuckets = crate::executor::assoc_nbuckets(variables, &name);
                    if plain {
                        print_plain_declaration(&name, value, attrs, nbuckets, stdout)?;
                    } else {
                        print_declaration(&name, value, attrs, nbuckets, stdout)?;
                    }
                }
                None => {
                    print_unset_declaration(&name, attrs, stdout)?;
                }
            }
        } else if let Some(value) = variables.get(&name) {
            let nbuckets = crate::executor::assoc_nbuckets(variables, &name);
            if plain {
                print_plain_declaration(&name, value, attrs, nbuckets, stdout)?;
            } else {
                print_declaration(&name, value, attrs, nbuckets, stdout)?;
            }
        } else if attrs.has_scalar_attribute()
            || underlay_attr(&name, DECLARED_UNSET_VARS, |a| a.declared_unset)
        {
            print_unset_declaration(&name, attrs, stdout)?;
        } else {
            writeln!(
                stderr,
                "{}{command_name}: {}: not found",
                diagnostic_prefix(variables),
                name
            )?;
            status = EXECUTION_FAILURE;
        }
    }

    Ok(status)
}

/// The declaration line for one name, rendered by the same printer
/// the builtin's per-name loop uses (`print_declaration` /
/// `print_unset_declaration` over the mirror value and the mark
/// sets). This is the single-name entry to that one rendering path:
/// the 1.5.0 state views (PLAN row 6) render variable snapshots
/// through here so an inspection snapshot and `declare -p NAME` can
/// never drift into two formatters. The tempenv underlay and dynamic
/// views are listing-time concerns of the builtin and deliberately
/// not modeled here — callers that need them consult those paths
/// first, exactly as the loop above does. Returns None when the name
/// has no binding the printer would render (the builtin reports it
/// as not found).
pub(crate) fn declaration_line(
    name: &str,
    variables: &crate::shell::var_table::VarTable,
    plain: bool,
) -> Option<String> {
    let attrs = DeclarationAttrs {
        exported: variables.is_marked(EXPORTED_VARS, name),
        readonly: variables.is_marked(READONLY_VARS, name),
        array: variables.is_marked(ARRAY_VARS, name),
        assoc: variables.is_marked(ASSOC_VARS, name),
        integer: variables.is_marked(INTEGER_VARS, name),
        uppercase: variables.is_marked(UPPERCASE_VARS, name),
        lowercase: variables.is_marked(LOWERCASE_VARS, name),
        capcase: variables.is_marked(super::CAPCASE_VARS, name),
        nameref: variables.is_marked(NAMEREF_VARS, name),
        trace: variables.is_marked(super::TRACE_VARS, name),
    };
    let mut out: Vec<u8> = Vec::new();
    if let Some(value) = variables.get(name) {
        let nbuckets = crate::executor::assoc_nbuckets(variables, name);
        if plain {
            print_plain_declaration(name, value, attrs, nbuckets, &mut out).ok()?;
        } else {
            print_declaration(name, value, attrs, nbuckets, &mut out).ok()?;
        }
    } else if attrs.has_scalar_attribute() || variables.is_marked(DECLARED_UNSET_VARS, name) {
        print_unset_declaration(name, attrs, &mut out).ok()?;
    } else {
        return None;
    }
    let text = String::from_utf8(out).ok()?;
    Some(text.trim_end_matches('\n').to_string())
}

pub(crate) fn declaration_names_to_print(
    variables: &crate::shell::var_table::VarTable,
    export: bool,
    readonly: bool,
    array: bool,
    assoc: bool,
    integer: bool,
    uppercase: bool,
    lowercase: bool,
    nameref: bool,
    uppercase_vars: &HashSet<String>,
    lowercase_vars: &HashSet<String>,
    nameref_vars: &HashSet<String>,
) -> Vec<String> {
    let exported = exported_vars(variables);
    let readonly_vars = marked_vars(variables, READONLY_VARS);
    let arrays = marked_vars(variables, ARRAY_VARS);
    let assocs = marked_vars(variables, ASSOC_VARS);
    let integers = marked_vars(variables, INTEGER_VARS);
    let filter_by_attr =
        export || readonly || array || assoc || integer || uppercase || lowercase || nameref;
    let mut names: Vec<String> = variables
        .keys()
        .filter(|name| !name.starts_with("__RUBASH_"))
        // GNU variables.c:511-526 (initialize_shell_variables): environment
        // entries with invalid identifier names (Windows passes names like
        // `CommonProgramFiles(x86)`) are bound into the invisible
        // invalid_env table, not shell_variables — so export -p (which
        // iterates shell variables) never prints them. They are still
        // exported to children via maybe_make_export_env
        // (variables.c:5064-5117).
        .filter(|name| valid_identifier(name))
        .filter(|name| {
            if !filter_by_attr {
                return true;
            }
            (!export || exported.contains(*name))
                && (!readonly || readonly_vars.contains(*name))
                && (!array || arrays.contains(*name))
                && (!assoc || assocs.contains(*name))
                && (!integer || integers.contains(*name))
                && (!uppercase || uppercase_vars.contains(*name))
                && (!lowercase || lowercase_vars.contains(*name))
                && (!nameref || nameref_vars.contains(*name))
        })
        .cloned()
        .collect();
    for name in exported
        .iter()
        .chain(readonly_vars.iter())
        .chain(nameref_vars.iter())
    {
        if name.starts_with("__RUBASH_") {
            continue;
        }
        // Same invalid_env rule as the primary listing above: an invalid
        // name never became a shell variable, so attribute-marker loops
        // must not resurrect it into the listing (niubash issue #102).
        if !valid_identifier(name) {
            continue;
        }
        if !filter_by_attr && !variables.contains_key(name) {
            continue;
        }
        if filter_by_attr
            && !((!export || exported.contains(name))
                && (!readonly || readonly_vars.contains(name))
                && (!array || arrays.contains(name))
                && (!assoc || assocs.contains(name))
                && (!integer || integers.contains(name))
                && (!uppercase || uppercase_vars.contains(name))
                && (!lowercase || lowercase_vars.contains(name))
                && (!nameref || nameref_vars.contains(name)))
        {
            continue;
        }
        if !names.iter().any(|current| current == name) {
            names.push(name.clone());
        }
    }
    names.sort();
    names
}
