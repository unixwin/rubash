use std::io::{self, Write};

use super::storage::{
    format_array_value, format_assoc_value, parse_single_element_array, quote_double,
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
    pub(super) nameref: bool,
}

impl DeclarationAttrs {
    pub(super) fn has_scalar_attribute(self) -> bool {
        self.exported
            || self.readonly
            || self.integer
            || self.uppercase
            || self.lowercase
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
    } else if let Some(array_value) = parse_single_element_array(value) {
        let attrs = declaration_array_attrs(attrs);
        writeln!(
            stdout,
            "declare {} {}=([0]=\"{}\")",
            attrs,
            name,
            quote_double(array_value)
        )
    } else if let Some(attrs) = declaration_scalar_attrs(attrs) {
        writeln!(
            stdout,
            "declare {attrs} {}=\"{}\"",
            name,
            quote_double(value)
        )
    } else {
        writeln!(stdout, "declare -- {}=\"{}\"", name, quote_double(value))
    }
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
    if attrs.nameref {
        flags.push('n');
    }
    if attrs.integer {
        flags.push('i');
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
    (flags.len() > 1).then_some(flags)
}

fn declaration_array_attrs(attrs: DeclarationAttrs) -> String {
    let mut flags = String::from("-a");
    if attrs.nameref {
        flags.push('n');
    }
    if attrs.integer {
        flags.push('i');
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
    flags
}

fn declaration_assoc_attrs(attrs: DeclarationAttrs) -> String {
    let mut flags = String::from("-A");
    if attrs.nameref {
        flags.push('n');
    }
    if attrs.integer {
        flags.push('i');
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
    flags
}
