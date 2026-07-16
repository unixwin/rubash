use super::*;

pub(in crate::executor) fn prompt_username(env_vars: &HashMap<String, String>) -> String {
    env_vars
        .get("USER")
        .or_else(|| env_vars.get("USERNAME"))
        .cloned()
        .or_else(|| env::var("USER").ok())
        .or_else(|| env::var("USERNAME").ok())
        .unwrap_or_default()
}

pub(in crate::executor) fn prompt_hostname(
    env_vars: &HashMap<String, String>,
    full: bool,
) -> String {
    let hostname = env_vars
        .get("HOSTNAME")
        .or_else(|| env_vars.get("COMPUTERNAME"))
        .cloned()
        .or_else(|| env::var("HOSTNAME").ok())
        .or_else(|| env::var("COMPUTERNAME").ok())
        .unwrap_or_default();
    if full {
        hostname
    } else {
        hostname.split('.').next().unwrap_or(&hostname).to_string()
    }
}

#[derive(Clone, Copy)]
pub(in crate::executor) enum CaseMod {
    UpperFirst,
    UpperAll,
    LowerFirst,
    LowerAll,
    ToggleFirst,
    ToggleAll,
}

pub(in crate::executor) fn parse_parameter_case_mod(name: &str) -> Option<(&str, CaseMod, &str)> {
    if name.contains("//") {
        return None;
    }
    if let Some((var_name, pattern)) = name.split_once("^^") {
        return Some((var_name, CaseMod::UpperAll, pattern));
    }
    if let Some((var_name, pattern)) = name.split_once(",,") {
        return Some((var_name, CaseMod::LowerAll, pattern));
    }
    if let Some((var_name, pattern)) = name.split_once("~~") {
        return Some((var_name, CaseMod::ToggleAll, pattern));
    }
    if let Some((var_name, pattern)) = name.split_once('^') {
        return Some((var_name, CaseMod::UpperFirst, pattern));
    }
    if let Some((var_name, pattern)) = name.split_once(',') {
        return Some((var_name, CaseMod::LowerFirst, pattern));
    }
    if let Some((var_name, pattern)) = name.split_once('~') {
        return Some((var_name, CaseMod::ToggleFirst, pattern));
    }
    None
}

pub(in crate::executor) fn apply_parameter_case_mod(
    value: &str,
    operation: CaseMod,
    pattern: &str,
) -> String {
    let pattern = if pattern.is_empty() { "?" } else { pattern };
    let mut changed_first = false;

    value
        .chars()
        .map(|ch| {
            let char_value = ch.to_string();
            let matches = case_pattern_matches(pattern, &char_value);
            let should_change = matches
                && match operation {
                    CaseMod::UpperAll | CaseMod::LowerAll | CaseMod::ToggleAll => true,
                    CaseMod::UpperFirst | CaseMod::LowerFirst | CaseMod::ToggleFirst => {
                        !changed_first
                    }
                };

            if should_change {
                changed_first = true;
                match operation {
                    CaseMod::UpperFirst | CaseMod::UpperAll => ch.to_uppercase().collect(),
                    CaseMod::LowerFirst | CaseMod::LowerAll => ch.to_lowercase().collect(),
                    CaseMod::ToggleFirst | CaseMod::ToggleAll if ch.is_lowercase() => {
                        ch.to_uppercase().collect()
                    }
                    CaseMod::ToggleFirst | CaseMod::ToggleAll if ch.is_uppercase() => {
                        ch.to_lowercase().collect()
                    }
                    CaseMod::ToggleFirst | CaseMod::ToggleAll => char_value,
                }
            } else {
                char_value
            }
        })
        .collect()
}
