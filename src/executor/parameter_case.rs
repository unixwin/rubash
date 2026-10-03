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
    let pattern = if pattern.is_empty() { "?" } else { pattern };
    let mut changed_first = false;

    value
        .chars()
        .enumerate()
        .map(|(char_index, ch)| {
            let char_value = ch.to_string();
            let matches = case_pattern_matches(pattern, &char_value);
            let should_change = matches
                && match operation {
                    CaseMod::UpperAll | CaseMod::LowerAll | CaseMod::ToggleAll => true,
                    // subst.c case_transform: the First operators test only
                    // the word's first character against the pattern — when
                    // it does not match, the word is left alone (no scan for
                    // a later matching character; casemod.tests ${@^[rstlne]}).
                    CaseMod::UpperFirst | CaseMod::LowerFirst | CaseMod::ToggleFirst => {
                        char_index == 0 && !changed_first
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
