use std::io::{self, Write};

use super::storage::{
    format_array_value, format_assoc_value, parse_single_element_array, quote_declare_value,
};

#[derive(Clone, Copy)]
pub(super) struct DeclarationAttrs {
    pub(super) exported: bool,
    pub(super) readonly: bool,
    pub(super) array: bool,
    pub(super) assoc: bool,
    pub(super) integer: bool,
    pub(super) uppercase: bool,
    pub(super) lowercase: bool,
    pub(super) capcase: bool,
    pub(super) nameref: bool,
}

impl DeclarationAttrs {
    pub(super) fn has_scalar_attribute(self) -> bool {
        self.exported
            || self.readonly
            || self.integer
            || self.uppercase
            || self.lowercase
            || self.capcase
            || self.nameref
    }
}

pub(super) fn print_declaration<W>(
    name: &str,
    value: &str,
    attrs: DeclarationAttrs,
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
                format_assoc_value(value)
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
    } else if value.starts_with("\x1d(") {
        let attrs = declaration_array_attrs(attrs);
        writeln!(
            stdout,
            "declare {attrs} {name}={}",
            format_array_value(value)
        )
    } else if let Some(array_value) = parse_single_element_array(value) {
        let attrs = declaration_array_attrs(attrs);
        writeln!(
            stdout,
            "declare {} {}=([0]={})",
            attrs,
            name,
            quote_declare_value(array_value)
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

pub(super) fn print_plain_declaration<W>(
    name: &str,
    value: &str,
    attrs: DeclarationAttrs,
    stdout: &mut W,
) -> io::Result<()>
where
    W: Write,
{
    let rendered = if attrs.assoc {
        format_assoc_value(value)
    } else if attrs.array {
        format_array_value(value)
    } else {
        quote_declare_value(value)
    };
    writeln!(stdout, "{name}={rendered}")
}

pub(super) fn print_unset_declaration<W>(
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
