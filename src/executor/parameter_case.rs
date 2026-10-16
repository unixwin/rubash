use super::*;

pub(in crate::executor) fn prompt_username(_env_vars: &HashMap<String, String>) -> String {
    // GNU parse.y:6542-6551 (decode_prompt_string `case 'u'`): the prompt
    // renders current_user.user_name, which get_current_user_info
    // (shell.c:1877-1915) fills ONCE per process from
    // getpwuid(current_user.uid) -- the passwd database, never the USER
    // environment variable. A session running with USER="" (or USER=spoof)
    // therefore still shows the real account name in \u (rubash#421); a
    // userless lookup renders GNU's literal "I have no name!". Port: the
    // OS account API (GetUserNameW on Windows -- the token's account name,
    // the geteuid analog; getpwuid on Unix), cached once like current_user.
    static USER_NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    USER_NAME.get_or_init(os_account_user_name).clone()
}

fn os_account_user_name() -> String {
    #[cfg(windows)]
    {
        // advapi32 GetUserNameW: the account name of the running access
        // token. The name buffer starts generous (UNLEN + 1 = 257); a
        // ERROR_INSUFFICIENT_BUFFER retry is the documented contract but
        // cannot fire with UNLEN-sized input.
        const UNLEN: u32 = 256;
        let mut buffer = [0u16; UNLEN as usize + 1];
        let mut size = buffer.len() as u32;
        let ok = unsafe {
            windows_sys::Win32::System::WindowsProgramming::GetUserNameW(
                buffer.as_mut_ptr(),
                &mut size,
            )
        };
        if ok != 0 && size > 0 {
            let end = buffer[..size as usize]
                .iter()
                .position(|&ch| ch == 0)
                .unwrap_or(size as usize);
            return String::from_utf16_lossy(&buffer[..end]);
        }
        "I have no name!".to_string()
    }
    #[cfg(unix)]
    {
        // shell.c:1890 getpwuid(geteuid()); a missing passwd entry takes
        // the literal fallback at shell.c:1905.
        let uid = unsafe { libc::geteuid() };
        let entry = unsafe { libc::getpwuid(uid) };
        if entry.is_null() {
            return "I have no name!".to_string();
        }
        let name = unsafe { (*entry).pw_name };
        if name.is_null() {
            return "I have no name!".to_string();
        }
        unsafe { std::ffi::CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned()
    }
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
    if pattern.is_empty() {
        match operation {
            CaseMod::UpperAll => return value.chars().flat_map(char::to_uppercase).collect(),
            CaseMod::LowerAll => return value.chars().flat_map(char::to_lowercase).collect(),
            // An empty pattern behaves as `?`, matching every character, so
            // toggle-all needs no matcher either (rubash#608).
            CaseMod::ToggleAll => {
                return value
                    .chars()
                    .flat_map(|ch| {
                        if ch.is_lowercase() {
                            ch.to_uppercase().collect::<Vec<_>>()
                        } else if ch.is_uppercase() {
                            ch.to_lowercase().collect::<Vec<_>>()
                        } else {
                            vec![ch]
                        }
                    })
                    .collect()
            }
            _ => {}
        }
    }
    // First-only operators test just the word's first character against the
    // pattern (subst.c case_transform; casemod.tests ${@^[rstlne]}): handle
    // it directly and append the rest verbatim instead of running the
    // matcher once per character (rubash#608). An empty pattern behaves as
    // `?` and matches unconditionally, so no matcher call is needed there
    // either; otherwise a stack buffer carries the single character with no
    // heap allocation.
    if matches!(
        operation,
        CaseMod::UpperFirst | CaseMod::LowerFirst | CaseMod::ToggleFirst
    ) {
        let mut chars = value.chars();
        let Some(first) = chars.next() else {
            return String::new();
        };
        let matched = if pattern.is_empty() {
            true
        } else {
            let mut buf = [0u8; 4];
            case_pattern_matches(pattern, first.encode_utf8(&mut buf))
        };
        if !matched {
            return value.to_string();
        }
        let mut out = String::with_capacity(value.len());
        match operation {
            CaseMod::UpperFirst => out.extend(first.to_uppercase()),
            CaseMod::LowerFirst => out.extend(first.to_lowercase()),
            _ => {
                if first.is_lowercase() {
                    out.extend(first.to_uppercase())
                } else if first.is_uppercase() {
                    out.extend(first.to_lowercase())
                } else {
                    out.push(first)
                }
            }
        }
        out.push_str(chars.as_str());
        return out;
    }
    // First operators returned above; only All operators reach here.
    let pattern = if pattern.is_empty() { "?" } else { pattern };
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        // Stack buffer instead of the per-character heap `to_string()`.
        let mut buf = [0u8; 4];
        if !case_pattern_matches(pattern, ch.encode_utf8(&mut buf)) {
            out.push(ch);
            continue;
        }
        match operation {
            CaseMod::UpperAll | CaseMod::UpperFirst => out.extend(ch.to_uppercase()),
            CaseMod::LowerAll | CaseMod::LowerFirst => out.extend(ch.to_lowercase()),
            CaseMod::ToggleAll | CaseMod::ToggleFirst if ch.is_lowercase() => {
                out.extend(ch.to_uppercase())
            }
            CaseMod::ToggleAll | CaseMod::ToggleFirst if ch.is_uppercase() => {
                out.extend(ch.to_lowercase())
            }
            CaseMod::ToggleAll | CaseMod::ToggleFirst => out.push(ch),
        }
    }
    out
}
