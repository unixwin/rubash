//! tilde module.
//!
//! GNU Bash source ownership:
// - lib/tilde/tilde.c
// - lib/tilde/tilde.h

use std::collections::HashMap;

pub const QUOTED_ASSIGNMENT_VALUE: char = '\x1c';

pub fn home_value(env_vars: &HashMap<String, String>) -> String {
    // Bash's tilde expansion follows HOME when it is set. USERPROFILE is
    // only a Windows fallback for shells that have no HOME value.
    let names = ["HOME", "USERPROFILE"];

    names
        .into_iter()
        .find_map(|name| {
            env_vars
                .get(name)
                .filter(|value| !value.is_empty())
                .cloned()
                .or_else(|| std::env::var(name).ok().filter(|value| !value.is_empty()))
        })
        .unwrap_or_default()
}

pub fn expand_word_prefix(word: &str, env_vars: &HashMap<String, String>) -> Option<String> {
    if let Some(rest) = word.strip_prefix("~/") {
        return Some(format!("{}/{}", home_value(env_vars), rest));
    }

    match word {
        "~" => Some(home_value(env_vars)),
        "~+" => env_vars.get("PWD").cloned(),
        "~-" => env_vars.get("OLDPWD").cloned(),
        _ => expand_user_tilde_word(word, env_vars),
    }
}

/// GNU lib/tilde/tilde.c:361-410: `~user` and `~user/rest` resolve the user
/// through getpwnam and glue the passwd home directory with the rest of the
/// word. The user name ends at the first unquoted `/` or `:`, and the
/// remainder glues verbatim (GNU: `echo ~root:sub` prints `/root:sub`). A
/// user that cannot be found returns None, and the caller keeps the word
/// verbatim (GNU savestring(filename)).
fn expand_user_tilde_word(word: &str, env_vars: &HashMap<String, String>) -> Option<String> {
    let rest = word.strip_prefix('~')?;
    let (user, tail) = match rest.find(['/', ':']) {
        Some(index) => (&rest[..index], Some(&rest[index..])),
        None => (rest, None),
    };
    if user.is_empty() {
        return None;
    }

    let home = passwd_home_for_user(user, env_vars)?;
    Some(match tail {
        Some(tail) => format!("{home}{tail}"),
        None => home,
    })
}

/// Windows has no system passwd database, so `~user` reads the winuxcmd
/// `/etc/passwd` convention: the file resolves through the same
/// shell_path_to_windows mapping the cd builtin uses, which means
/// `__RUBASH_SHELL_ROOT` / `WINUXSH_ROOT` / `RUBASH_ROOT` decide where
/// `/etc` lives. No passwd file ships with the repository — tests create
/// their own fixture under a temporary root and delete it afterwards.
pub fn passwd_home_for_user(user: &str, env_vars: &HashMap<String, String>) -> Option<String> {
    let path = crate::executor::path::shell_path_to_windows("/etc/passwd", env_vars);
    let content = std::fs::read_to_string(path).ok()?;
    passwd_home_from_content(&content, user)
}

/// passwd(5) line shape: name:password:uid:gid:gecos:home:shell. The home
/// field is index 5. Comment and empty lines are skipped. A matching entry
/// wins even with an empty home field, mirroring getpwnam handing back
/// whatever pw_dir holds.
fn passwd_home_from_content(content: &str, user: &str) -> Option<String> {
    content
        .lines()
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .find_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let _password = fields.next()?;
            let _uid = fields.next()?;
            let _gid = fields.next()?;
            let _gecos = fields.next()?;
            let home = fields.next()?;
            (name == user).then(|| home.to_string())
        })
}

pub fn expand_assignment_value(value: &str, env_vars: &HashMap<String, String>) -> String {
    let Some(value) = value.strip_prefix(QUOTED_ASSIGNMENT_VALUE) else {
        if !assignment_value_needs_tilde_expansion(value, true) {
            return value.to_string();
        }
        return expand_assignment_tilde_value(value, env_vars, true);
    };

    value.to_string()
}

pub fn strip_assignment_quote_marker(value: &str) -> &str {
    value.strip_prefix(QUOTED_ASSIGNMENT_VALUE).unwrap_or(value)
}

pub fn assignment_value_needs_tilde_expansion(value: &str, expand_after_colon: bool) -> bool {
    let value = strip_assignment_quote_marker(value);
    let bytes = value.as_bytes();
    if assignment_tilde_segment_starts_at(bytes, 0) {
        return true;
    }
    if !expand_after_colon {
        return false;
    }

    bytes
        .iter()
        .enumerate()
        .any(|(index, byte)| *byte == b':' && assignment_tilde_segment_starts_at(bytes, index + 1))
}

fn assignment_tilde_segment_starts_at(bytes: &[u8], start: usize) -> bool {
    // GNU expands every unquoted `~` that starts an assignment value or
    // follows a `:` — `~user` included. The expansion pass itself keeps
    // anything the passwd lookup cannot resolve verbatim (the GNU
    // savestring(filename) fallback), so a generous predicate never turns a
    // `~user` assignment into a literal by accident.
    bytes.get(start) == Some(&b'~')
}

pub fn expand_assignment_tilde_value(
    value: &str,
    env_vars: &HashMap<String, String>,
    expand_after_colon: bool,
) -> String {
    if !expand_after_colon {
        return expand_tilde_segment(value, env_vars);
    }

    let mut output = String::new();
    let mut start = 0;
    for (index, ch) in value.char_indices() {
        if index == 0 || ch != ':' {
            continue;
        }
        output.push_str(&expand_tilde_segment(&value[start..index], env_vars));
        output.push(':');
        start = index + ch.len_utf8();
    }
    output.push_str(&expand_tilde_segment(&value[start..], env_vars));
    output
}

fn expand_tilde_segment(segment: &str, env_vars: &HashMap<String, String>) -> String {
    let Some(rest) = segment.strip_prefix('~') else {
        return segment.to_string();
    };

    if rest.is_empty() || rest.starts_with('/') {
        let home = home_value(env_vars);
        if home.is_empty() {
            return segment.to_string();
        }
        return format!("{home}{rest}");
    }

    // GNU lib/tilde/tilde.c:361-410: `~user[/rest]` resolves through
    // getpwnam. Within an assignment segment the user name ends at `/`
    // (the PATH-style `:` split already happened in
    // expand_assignment_tilde_value); an unresolvable user keeps the
    // segment verbatim.
    let (user, tail) = match rest.find('/') {
        Some(index) => (&rest[..index], Some(&rest[index..])),
        None => (rest, None),
    };
    match passwd_home_for_user(user, env_vars) {
        Some(home) => match tail {
            Some(tail) => format!("{home}{tail}"),
            None => home,
        },
        None => segment.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_home_value_prefers_home_before_userprofile() {
        let env_vars = HashMap::from([
            ("HOME".to_string(), "C:/from-home".to_string()),
            ("USERPROFILE".to_string(), "C:/from-userprofile".to_string()),
        ]);

        assert_eq!(home_value(&env_vars), "C:/from-home");
    }

    #[test]
    fn assignment_tilde_candidate_detects_only_expandable_segments() {
        assert!(!assignment_value_needs_tilde_expansion("plain", true));
        assert!(!assignment_value_needs_tilde_expansion("user~name", true));
        assert!(assignment_value_needs_tilde_expansion("~user/bin", true));
        assert!(assignment_value_needs_tilde_expansion("~", true));
        assert!(assignment_value_needs_tilde_expansion("~/bin", true));
        assert!(assignment_value_needs_tilde_expansion("bin:~/tools", true));
        assert!(assignment_value_needs_tilde_expansion("bin:~user/tools", true));
        assert!(!assignment_value_needs_tilde_expansion(
            "bin:~/tools",
            false
        ));
    }

    /// Creates a temporary RUBASH_ROOT with an etc/passwd fixture. No passwd
    /// file is ever committed to the repository — the fixture lives in the
    /// process temp directory and each test removes its own root.
    fn fixture_env(user: &str, home: &str) -> (std::path::PathBuf, HashMap<String, String>) {
        let root = std::env::temp_dir().join(format!(
            "rubash-tilde-user-{}-{}",
            user,
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("etc")).unwrap();
        std::fs::write(
            root.join("etc").join("passwd"),
            format!(
                "# comment line\nroot:x:0:0:root:/root:/bin/bash\n{user}:x:1000:1000::{}:/bin/rubash\n",
                home.replace('\\', "/")
            ),
        )
        .unwrap();
        let env_vars = HashMap::from([(
            "RUBASH_ROOT".to_string(),
            root.to_string_lossy().to_string(),
        )]);
        (root, env_vars)
    }

    #[test]
    fn tilde_user_resolves_passwd_home() {
        let (root, env) = fixture_env("niu", "/c/Users/niu-home");

        assert_eq!(
            expand_word_prefix("~niu", &env),
            Some("/c/Users/niu-home".to_string())
        );
        assert_eq!(
            expand_word_prefix("~niu/docs", &env),
            Some("/c/Users/niu-home/docs".to_string())
        );
        assert_eq!(expand_word_prefix("~missinguser", &env), None);
        assert_eq!(expand_word_prefix("~missinguser/x", &env), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn tilde_user_word_colon_tail_glues_verbatim() {
        let (root, env) = fixture_env("niu", "/c/Users/niu-home");

        assert_eq!(
            expand_word_prefix("~niu:sub", &env),
            Some("/c/Users/niu-home:sub".to_string())
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn tilde_user_assignment_segments_expand_and_fall_back() {
        let (root, env) = fixture_env("niu", "/c/Users/niu-home");

        assert_eq!(
            expand_assignment_value("~niu/bin", &env),
            "/c/Users/niu-home/bin"
        );
        assert_eq!(
            expand_assignment_value("bin:~niu/tools", &env),
            "bin:/c/Users/niu-home/tools"
        );
        assert_eq!(expand_assignment_value("~missing/x", &env), "~missing/x");
        assert_eq!(
            expand_assignment_value("a:~missing/x:b", &env),
            "a:~missing/x:b"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
