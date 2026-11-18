use crate::shell::var_table::VarTable;
use std::collections::HashMap;
use std::env;
use std::path::{Component, Path, PathBuf};

use crate::executor::path::shell_path_to_windows;

pub(super) fn starts_with_dot_component(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::CurDir | Component::ParentDir)
    )
}

pub(super) fn current_logical_pwd(env_vars: &HashMap<String, String>, cwd: &Path) -> PathBuf {
    if let Some(pwd) = shell_var(env_vars, "PWD") {
        if cfg!(windows) && pwd.starts_with('/') {
            return PathBuf::from(pwd);
        }

        let path = PathBuf::from(pwd);
        if path.is_absolute() {
            return path;
        }
    }

    // G-02: the no-$PWD fallback is the session's physical cwd, not the
    // process cwd.
    cwd.to_path_buf()
}

/// The shell's VALIDATED logical cwd — the base `cd` builds a logical
/// destination from. GNU builtins/cd.def change_to_directory makes the
/// new path absolute against the_current_working_directory, never
/// against the raw $PWD variable: a user assignment to $PWD
/// desynchronizes the variable, and bash keeps building from the
/// validated value until the next successful cd rebinds the variable
/// from it (bindpwd, cd.def:136 — which still records the RAW old
/// variable in OLDPWD, so current_logical_pwd keeps that job). rubash
/// keeps no separate validated store, so the value is recomputed here
/// with the same same-directory judgment pwd's logical face uses
/// (logical_pwd_if_current) — itself the runtime form of the check
/// variables.c set_pwd applies to an imported $PWD at startup
/// (ABSPATH + same_file(temp_string, ".")): $PWD when it names the
/// session's current directory, the physical session cwd otherwise
/// (rubash#2143).
pub(crate) fn validated_logical_pwd(env_vars: &HashMap<String, String>, cwd: &Path) -> PathBuf {
    if let Some(logical) = crate::builtins::pwd::logical_pwd_if_current(cwd, env_vars) {
        return PathBuf::from(logical);
    }

    if cfg!(windows) && env_vars.contains_key("__RUBASH_PHYSICAL_PWD") {
        // Bridge state (see logical_posix_test_dir in cd.rs): the
        // session's logical names have no physical directory to
        // validate against, so a '/'-spelled $PWD stays the logical
        // truth exactly as current_logical_pwd has always treated it.
        if let Some(pwd) = shell_var(env_vars, "PWD") {
            if pwd.starts_with('/') {
                return PathBuf::from(pwd);
            }
        }
    }

    cwd.to_path_buf()
}

pub(super) fn logical_destination(old_pwd: &Path, target: &Path) -> PathBuf {
    let combined = if target.is_absolute() {
        target.to_path_buf()
    } else {
        old_pwd.join(target)
    };

    normalize_logical_path(&combined)
}

pub(super) fn logical_destination_display(old_pwd: &Path, target: &Path) -> String {
    let target_display = path_display_text(target);
    let old_display = path_display_text(old_pwd);
    // The raw combined text is the only place the exactly-two-slashes
    // prefix survives; both Path::components and split('/') erase it.
    let absolute_target = target.is_absolute() || target_display.starts_with('/');
    let combined_raw = if absolute_target {
        target_display.clone()
    } else {
        format!("{old_display}/{target_display}")
    };
    // rubash#2211: the two-slashes EVIDENCE is the spelling the new path
    // was built FROM — the target's own spelling for an absolute
    // target, the old directory's spelling for a relative one — never
    // the joined text. Joining manufactures the prefix at the seam:
    // old "/" + "home" joins to "//home" although neither side spelled
    // a double slash (GNU binds /home), and old "//" + "etc" joins to
    // "///etc", which reads as a collapsible triple although the move
    // stays under the // root (GNU binds //etc).
    let slash_evidence = if absolute_target {
        target_display.as_str()
    } else {
        old_display.as_str()
    };
    if !cfg!(windows) {
        return preserve_leading_double_slash(
            slash_evidence,
            &shell_display_path(&logical_destination(old_pwd, target)),
        );
    }

    let normalized = if absolute_target {
        normalize_logical_display(&target_display)
    } else {
        normalize_logical_display(&combined_raw)
    };
    let normalized = windows_slash_drive_display_to_native(&normalized).unwrap_or(normalized);
    preserve_leading_double_slash(slash_evidence, &normalized)
}

/// The existence half of GNU's logical canonicalization, which the
/// textual destination above cannot express: lib/sh/pathcanon.c
/// sh_canonpath runs cd's logical mode with PATH_CHECKEXISTS |
/// PATH_CHECKDOTDOT — every real component is stat-checked as a
/// directory the moment it is appended, and the accumulated path is
/// stat-checked again before a `..` chops its last component, so a
/// component the textual walk ERASES must still name an existing
/// directory or canonpath returns NULL. change_to_directory
/// (builtins/cd.def) then adjudicates by the verbatim chdir, never by
/// the canonical form. The surviving components need no check here —
/// the destination probe's canonicalize walks them through the
/// kernel — so this walk replays only the erasures: it rebuilds the
/// combined spelling's component stack exactly as
/// logical_destination_display does, and each time a `..` would pop a
/// component, the accumulated prefix (that component included) must
/// resolve to an existing directory. A `..` at the stack floor pops
/// nothing and checks nothing (canonpath's dotdot watermark: the root
/// of a rooted spelling, or the leading `..` run of an unrooted one,
/// which canon keeps verbatim).
pub(super) fn logical_canon_erased_components_exist(
    old_pwd: &Path,
    target: &Path,
    env_vars: &HashMap<String, String>,
) -> bool {
    let target_display = path_display_text(target);
    let old_display = path_display_text(old_pwd);
    let absolute_target = target.is_absolute() || target_display.starts_with('/');
    let combined_raw = if absolute_target {
        target_display.clone()
    } else {
        format!("{old_display}/{target_display}")
    };
    let rooted = combined_raw.starts_with('/');
    let mut stack: Vec<&str> = Vec::new();
    let mut floor = 0usize;
    for part in combined_raw.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if stack.len() > floor {
                    let prefix = if rooted {
                        format!("/{}", stack.join("/"))
                    } else {
                        stack.join("/")
                    };
                    let fs_path = filesystem_path_for_display(&prefix, env_vars);
                    if !std::fs::metadata(&fs_path)
                        .map(|meta| meta.is_dir())
                        .unwrap_or(false)
                    {
                        return false;
                    }
                    stack.pop();
                } else if !rooted {
                    stack.push("..");
                    floor = stack.len();
                }
            }
            other => stack.push(other),
        }
    }
    true
}

/// POSIX leaves the meaning of exactly two leading slashes
/// implementation-defined, and GNU bash preserves them through logical
/// path normalization — `cd //tmp` binds PWD=`//tmp`, and the prefix
/// behaves like a root of its own (`cd ..` from there yields `//`, not
/// `/`); three or more slashes collapse to one. On Windows this prefix
/// is the UNC root (rubash#1415). The component-wise normalizers cannot
/// represent it, so re-apply it from the raw combined text.
pub(crate) fn preserve_leading_double_slash(raw: &str, normalized: &str) -> String {
    let exactly_two = raw.starts_with("//") && !raw.starts_with("///");
    if exactly_two && normalized.starts_with('/') && !normalized.starts_with("//") {
        format!("/{normalized}")
    } else {
        normalized.to_string()
    }
}

pub(super) fn shell_var(env_vars: &HashMap<String, String>, name: &str) -> Option<String> {
    // HOME and OLDPWD are shell variables managed by the shell itself
    // (unset removes them from env_vars). On Windows, env::var still
    // returns the original process value after unset because
    // apply_required_windows_child_environment re-adds HOME from
    // USERPROFILE. So for HOME and OLDPWD, only check env_vars.
    if name == "HOME" || name == "OLDPWD" {
        return env_vars
            .get(name)
            .cloned()
            .filter(|value| !value.is_empty());
    }
    env_vars
        .get(name)
        .cloned()
        .or_else(|| env::var(name).ok())
        .filter(|value| !value.is_empty())
}

pub(super) fn filesystem_path_for_display(
    dir: &str,
    env_vars: &HashMap<String, String>,
) -> PathBuf {
    shell_path_to_windows(dir, env_vars)
}

pub(super) fn set_shell_env(env_vars: &mut VarTable, name: &str, value: String) {
    // Gated like every other mirror (#916): PWD/OLDPWD are always special,
    // so behavior is unchanged, but future callers inherit the gate.
    env_vars.insert(name.to_string(), value.clone());
    crate::executor::local_helpers::set_process_env(name, &value);
}

pub(super) fn shell_display_path(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if value.is_empty() {
        "/".to_string()
    } else {
        windows_slash_drive_display_to_native(&value).unwrap_or(value)
    }
}

/// OLDPWD carries the previous *logical* PWD value verbatim (builtins/cd.def:
/// `set_working_directory` stores the old `$PWD` string). It must not be
/// routed through `shell_display_path`, whose slash-drive→native bridge
/// would rewrite `/c/Users/...` into `C:/Users/...` and desynchronize
/// `OLDPWD` from the value `$PWD` held.
pub(super) fn logical_pwd_var_display(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if value.is_empty() {
        "/".to_string()
    } else {
        value
    }
}

pub(super) use crate::executor::path::shell_pwd_display_path;

fn windows_slash_drive_display_to_native(path: &str) -> Option<String> {
    if !cfg!(windows) {
        return None;
    }

    let bytes = path.as_bytes();
    if bytes.len() == 2 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() {
        let drive = (bytes[1] as char).to_ascii_uppercase();
        return Some(format!("{drive}:/"));
    }
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[2] == b'/' && bytes[1].is_ascii_alphabetic() {
        let drive = (bytes[1] as char).to_ascii_uppercase();
        return Some(format!("{drive}:{}", &path[2..]));
    }

    None
}

fn normalize_logical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        normalized
    }
}

fn normalize_logical_display(path: &str) -> String {
    let mut parts = Vec::new();
    let absolute = path.starts_with('/');

    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }

    let normalized = parts.join("/");
    if absolute {
        format!("/{normalized}")
    } else if normalized.is_empty() {
        ".".to_string()
    } else {
        normalized
    }
}

fn path_display_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::{logical_destination_display, shell_display_path};
    #[cfg(windows)]
    use std::path::Path;

    #[cfg(windows)]
    #[test]
    fn logical_destination_display_converts_slash_drive_to_native() {
        assert_eq!(
            logical_destination_display(Path::new("/c/Users/example"), Path::new("repo")),
            "C:/Users/example/repo"
        );
        assert_eq!(
            logical_destination_display(Path::new("/c/Users/example/repo"), Path::new("..")),
            "C:/Users/example"
        );
        assert_eq!(
            logical_destination_display(Path::new("C:/Users/example"), Path::new("repo")),
            "C:/Users/example/repo"
        );
    }

    #[cfg(windows)]
    #[test]
    fn shell_display_path_preserves_posix_bridge_but_not_slash_drive() {
        assert_eq!(
            shell_display_path(Path::new("/c/Users/example")),
            "C:/Users/example"
        );
        assert_eq!(shell_display_path(Path::new("/usr")), "/usr");
    }
}
