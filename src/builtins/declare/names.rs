pub(super) fn nameref_self_reference(arg: &str) -> bool {
    let Some((name, value)) = arg.split_once('=') else {
        return false;
    };
    let name = name.strip_suffix('+').unwrap_or(name);
    name == value
}

pub(super) fn declare_base_name(arg: &str) -> Option<&str> {
    let name = arg.split_once('=').map(|(name, _)| name).unwrap_or(arg);
    let name = name.strip_suffix('+').unwrap_or(name);
    let name = name.split_once('[').map(|(name, _)| name).unwrap_or(name);
    valid_identifier(name).then_some(name)
}

pub(super) fn valid_declare_name(arg: &str) -> bool {
    let name = arg.split_once('=').map(|(name, _)| name).unwrap_or(arg);
    let name = name.strip_suffix('+').unwrap_or(name);
    if let Some((base, subscript)) = name.split_once('[') {
        let Some(subscript) = subscript.strip_suffix(']') else {
            return false;
        };
        return !subscript.is_empty() && valid_identifier(base);
    }
    valid_identifier(name)
}

pub(super) fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}
