use std::io::{self, Write};

use super::storage::{format_array_value, format_assoc_value, quote_declare_value};

#[derive(Clone, Copy)]
pub(crate) struct DeclarationAttrs {
    pub(crate) exported: bool,
    pub(crate) readonly: bool,
    pub(crate) array: bool,
    pub(crate) assoc: bool,
    pub(crate) integer: bool,
    pub(crate) uppercase: bool,
    pub(crate) lowercase: bool,
    pub(crate) capcase: bool,
    pub(crate) nameref: bool,
    pub(crate) trace: bool,
}

impl DeclarationAttrs {
    pub(crate) fn has_scalar_attribute(self) -> bool {
        self.exported
            || self.readonly
            || self.integer
            || self.uppercase
            || self.lowercase
            || self.capcase
            || self.nameref
            || self.trace
    }
}

pub(crate) fn print_declaration<W>(
    name: &str,
    value: &str,
    attrs: DeclarationAttrs,
    nbuckets: usize,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    if attrs.assoc {
        let attrs = declaration_assoc_attrs(attrs);
        if value.is_empty() {
            writeln!(stdout, "declare {attrs} {name}")
        } else {
            writeln!(
                stdout,
                "declare {attrs} {name}={}",
                format_assoc_value(value, nbuckets)
            )
        }
    } else if attrs.array {
        let attrs = declaration_array_attrs(attrs);
        if value.is_empty() {
            writeln!(stdout, "declare {attrs} {name}")
        } else {
            writeln!(
                stdout,
                "declare {attrs} {name}={}",
                format_array_value(value)
            )
        }
    } else if value.starts_with(concat!("\u{1d}", "(")) {
        let attrs = declaration_array_attrs(attrs);
        writeln!(
            stdout,
            "declare {attrs} {name}={}",
            format_array_value(value)
        )
    } else if let Some(attrs) = declaration_scalar_attrs(attrs) {
        writeln!(
            stdout,
            "declare {attrs} {}={}",
            name,
            quote_declare_value(value)
        )
    } else {
        writeln!(stdout, "declare -- {}={}", name, quote_declare_value(value))
    }
}

pub(crate) fn print_plain_declaration<W>(
    name: &str,
    value: &str,
    attrs: DeclarationAttrs,
    nbuckets: usize,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    let rendered = if attrs.assoc {
        format_assoc_value(value, nbuckets)
    } else if attrs.array {
        format_array_value(value)
    } else {
        quote_declare_value(value)
    };
    writeln!(stdout, "{name}={rendered}")
}

pub(crate) fn print_unset_declaration<W>(
    name: &str,
    attrs: DeclarationAttrs,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    if let Some(attrs) = declaration_scalar_attrs(attrs) {
        writeln!(stdout, "declare {attrs} {name}")
    } else {
        writeln!(stdout, "declare -- {name}")
    }
}

fn declaration_scalar_attrs(attrs: DeclarationAttrs) -> Option<String> {
    let mut flags = String::from("-");
    // GNU prints a declared-but-unassigned array as "declare -a name" and
    // an associative one as "declare -A name" (declare -p on a size-hint
    // declaration like "declare -a b[256]").
    if attrs.array {
        flags.push('a');
    }
    if attrs.assoc {
        flags.push('A');
    }
    if attrs.integer {
        flags.push('i');
    }
    if attrs.nameref {
        flags.push('n');
    }
    if attrs.readonly {
        flags.push('r');
    }
    if attrs.trace {
        flags.push('t');
    }
    if attrs.exported {
        flags.push('x');
    }
    if attrs.lowercase {
        flags.push('l');
    }
    if attrs.uppercase {
        flags.push('u');
    }
    if attrs.capcase {
        flags.push('c');
    }
    (flags.len() > 1).then_some(flags)
}

fn declaration_array_attrs(attrs: DeclarationAttrs) -> String {
    let mut flags = String::from("-a");
    if attrs.integer {
        flags.push('i');
    }
    if attrs.nameref {
        flags.push('n');
    }
    if attrs.readonly {
        flags.push('r');
    }
    if attrs.trace {
        flags.push('t');
    }
    if attrs.exported {
        flags.push('x');
    }
    if attrs.lowercase {
        flags.push('l');
    }
    if attrs.uppercase {
        flags.push('u');
    }
    if attrs.capcase {
        flags.push('c');
    }
    flags
}

fn declaration_assoc_attrs(attrs: DeclarationAttrs) -> String {
    let mut flags = String::from("-A");
    if attrs.integer {
        flags.push('i');
    }
    if attrs.nameref {
        flags.push('n');
    }
    if attrs.readonly {
        flags.push('r');
    }
    if attrs.trace {
        flags.push('t');
    }
    if attrs.exported {
        flags.push('x');
    }
    if attrs.lowercase {
        flags.push('l');
    }
    if attrs.uppercase {
        flags.push('u');
    }
    if attrs.capcase {
        flags.push('c');
    }
    flags
}
