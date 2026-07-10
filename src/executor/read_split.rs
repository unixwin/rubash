use super::*;

pub(in crate::executor) fn read_array_storage(values: &[String]) -> String {
    if values
        .iter()
        .any(|value| value.is_empty() || value.contains(['\n', '\r']))
    {
        let rendered = values
            .iter()
            .enumerate()
            .map(|(index, value)| format!("[{index}]={}", render_read_array_element(value)))
            .collect::<Vec<_>>()
            .join(" ");
        return format!("\x1d({rendered})");
    }

    format!("({})", values.join(" "))
}

pub(in crate::executor) fn render_read_array_element(value: &str) -> String {
    if value.contains(['\n', '\r']) {
        let mut rendered = String::from("$'");
        for ch in value.chars() {
            match ch {
                '\n' => rendered.push_str("\\n"),
                '\r' => rendered.push_str("\\r"),
                '\\' => rendered.push_str("\\\\"),
                '\'' => rendered.push_str("\\'"),
                other => rendered.push(other),
            }
        }
        rendered.push('\'');
        return rendered;
    }

    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(in crate::executor) fn read_scalar_fields(
    line: &str,
    names_len: usize,
    ifs: &str,
) -> Vec<String> {
    if names_len == 0 {
        return Vec::new();
    }
    if names_len == 1 {
        return vec![line.to_string()];
    }
    if ifs.is_empty() {
        let mut fields = vec![line.to_string()];
        while fields.len() < names_len {
            fields.push(String::new());
        }
        return fields;
    }
    if ifs.trim().is_empty() {
        let mut fields = line
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        while fields.len() < names_len {
            fields.push(String::new());
        }
        if fields.len() > names_len {
            let rest = fields.split_off(names_len - 1).join(" ");
            fields.push(rest);
        }
        return fields;
    }

    let mut fields = line
        .splitn(names_len, |ch| ifs.contains(ch))
        .map(str::to_string)
        .collect::<Vec<_>>();
    while fields.len() < names_len {
        fields.push(String::new());
    }
    fields
}

pub(in crate::executor) fn read_scalar_fields_with_backslashes(
    line: &str,
    names_len: usize,
    ifs: &str,
) -> Vec<String> {
    if names_len == 0 {
        return Vec::new();
    }
    if names_len == 1 || ifs.is_empty() {
        let mut fields = vec![unescape_read_backslashes(line)];
        while fields.len() < names_len {
            fields.push(String::new());
        }
        return fields;
    }

    let split_on_ifs_whitespace = ifs.trim().is_empty();
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('\n') => {}
                Some('\r') if chars.peek() == Some(&'\n') => {
                    chars.next();
                }
                Some(next) => current.push(next),
                None => {}
            }
            continue;
        }

        if ifs.contains(ch) {
            if split_on_ifs_whitespace {
                if !current.is_empty() {
                    fields.push(std::mem::take(&mut current));
                }
            } else if fields.len() + 1 < names_len {
                fields.push(std::mem::take(&mut current));
            } else {
                current.push(ch);
            }
            continue;
        }

        current.push(ch);
    }

    if !split_on_ifs_whitespace || !current.is_empty() {
        fields.push(current);
    }

    if split_on_ifs_whitespace && fields.len() > names_len {
        let rest = fields.split_off(names_len - 1).join(" ");
        fields.push(rest);
    }
    while fields.len() < names_len {
        fields.push(String::new());
    }
    fields
}

pub(super) fn mark_env_name(env_vars: &mut HashMap<String, String>, key: &str, name: &str) {
    let mut names: Vec<String> = env_vars
        .get(key)
        .map(|value| {
            value
                .split('\x1f')
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if !names.iter().any(|current| current == name) {
        names.push(name.to_string());
    }
    env_vars.insert(key.to_string(), names.join("\x1f"));
}
