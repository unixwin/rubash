//! Pathname expansion (globbing) for command words.

use std::path::{Path, PathBuf};

use crate::executor::path::{shell_directory_entries, shell_path_to_windows};

pub(crate) enum PathnameExpansion {
    Matches(Vec<String>),
    NoMatch,
    Fail(String),
}

/// Check if a shopt option is enabled.
fn shopt_enabled(env_vars: &std::collections::HashMap<String, String>, name: &str) -> bool {
    crate::builtins::shopt::option_enabled(env_vars, name)
}

/// Check if a word contains glob or extglob pattern characters.
fn contains_glob_or_extglob(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '\\' || ch == '\x11' {
            index += 2;
            continue;
        }
        if matches!(ch, '*' | '?' | '[') {
            return true;
        }
        if matches!(ch, '@' | '+' | '!') && chars.get(index + 1) == Some(&'(') {
            return true;
        }
        index += 1;
    }
    false
}

/// Expand glob patterns (* ? [...]) in a word against the filesystem.
pub(crate) fn pathname_expand_word(
    word: &str,
    env_vars: &std::collections::HashMap<String, String>,
) -> PathnameExpansion {
    if word.is_empty() {
        return PathnameExpansion::NoMatch;
    }
    if word.starts_with('"') || word.starts_with('\'') {
        return PathnameExpansion::NoMatch;
    }
    if word.contains('{') || word.contains('}') {
        return PathnameExpansion::NoMatch;
    }
    if crate::builtins::set::shell_option_enabled(env_vars, "noglob") {
        return PathnameExpansion::NoMatch;
    }

    let nullglob = shopt_enabled(env_vars, "nullglob");
    let failglob = shopt_enabled(env_vars, "failglob");
    let dotglob = shopt_enabled(env_vars, "dotglob");
    let globskipdots = shopt_enabled(env_vars, "globskipdots");
    let nocaseglob = shopt_enabled(env_vars, "nocaseglob");
    let globstar = shopt_enabled(env_vars, "globstar");
    let extglob = shopt_enabled(env_vars, "extglob");

    // Shell-side pattern check (pathexp.c unquoted_glob_pattern_p): a word
    // whose only bracket-like text is invalid (an unquoted `/` inside a
    // bracket, an unclosed bracket) is a literal word, not a pattern, so
    // nullglob must not swallow it (glob7.tests).
    if !unquoted_glob_pattern_p(word, extglob) {
        return PathnameExpansion::NoMatch;
    }

    if word.contains("**") && globstar {
        return globstar_expand(
            word,
            nullglob,
            failglob,
            nocaseglob,
            dotglob,
            globskipdots,
            env_vars,
        );
    }

    if word.contains('/') {
        return pathname_expand_segments(
            word,
            nullglob,
            failglob,
            nocaseglob,
            dotglob,
            globskipdots,
            extglob,
            env_vars,
        );
    }

    // No `/`: the whole word is a component matched against "." — the same
    // glob_vector path glob_filename takes for a slash-free word.
    // Assigning GLOBIGNORE also makes dotfiles matchable (bash variables.c
    // setup_glob_ignore clears noglob_dot_filenames).
    let dot_mode = if dotglob || globignore_assigned(env_vars) {
        DotMode::DotDot
    } else {
        DotMode::Period
    };
    let matches = glob_vector_expand(
        word,
        ".",
        nocaseglob,
        extglob,
        dot_mode,
        globskipdots,
        env_vars,
    );
    let matches = apply_globignore(matches, env_vars);
    if matches.is_empty() {
        return unmatched_expansion(word, nullglob, failglob);
    }
    PathnameExpansion::Matches(matches)
}

fn pathname_expand_segments(
    word: &str,
    nullglob: bool,
    failglob: bool,
    nocaseglob: bool,
    dotglob: bool,
    globskipdots: bool,
    extglob: bool,
    env_vars: &std::collections::HashMap<String, String>,
) -> PathnameExpansion {
    // Assigning GLOBIGNORE also makes dotfiles matchable (bash variables.c
    // setup_glob_ignore clears noglob_dot_filenames).
    let dot_mode = if dotglob || globignore_assigned(env_vars) {
        DotMode::DotDot
    } else {
        DotMode::Period
    };
    let matches = glob_filename_expand(word, nocaseglob, extglob, dot_mode, globskipdots, env_vars);
    let matches = apply_globignore(matches, env_vars);
    if matches.is_empty() {
        return unmatched_expansion(word, nullglob, failglob);
    }
    PathnameExpansion::Matches(matches)
}

/// Port of glob.c glob_filename (glob.c:1120-1535) for non-globstar words:
/// split the pattern at the LAST unquoted `/` (glob_dirscan semantics), glob
/// the directory part (recursing when it holds unquoted wildcards), then match
/// the filename tail inside each directory found.
fn glob_filename_expand(
    pattern: &str,
    nocaseglob: bool,
    extglob: bool,
    dot_mode: DotMode,
    globskipdots: bool,
    env_vars: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    let (directory, filename) = split_last_unquoted_slash(pattern);

    // glob.c:1179: when the directory part carries unquoted wildcards, glob
    // it first and match the tail inside every directory found.
    if !directory.is_empty() && unquoted_glob_pattern_p(directory, extglob) {
        // glob.c:1239 strips the final `/` (quoted or not) before recursing.
        let d = match directory.strip_suffix('/') {
            Some(stripped) => stripped,
            None => directory,
        };
        let dirs = glob_filename_expand(d, nocaseglob, extglob, dot_mode, globskipdots, env_vars);
        let mut out = Vec::new();
        for dname in dirs {
            if filename.is_empty() {
                // glob.c:1287-1289: scan even on a NULL filename, so `b*/`
                // yields only directories, with the `/` re-added by
                // glob_dir_to_array.
                if shell_path_to_windows(&dname, env_vars).is_dir() {
                    out.push(format!("{dname}/"));
                }
            } else {
                for name in glob_vector_expand(
                    filename,
                    &dname,
                    nocaseglob,
                    extglob,
                    dot_mode,
                    globskipdots,
                    env_vars,
                ) {
                    out.push(join_dir_to_name(&dname, &name));
                }
            }
        }
        return out;
    }

    if filename.is_empty() {
        // glob.c:1432-1480 only_filename: the word ends in `/` and the
        // directory part has no unquoted wildcards; the directory name is
        // returned verbatim (the hasglob==2 dequote branch is dead code in
        // bash 5.3: glob_pattern_p never returns 2).
        return vec![directory.to_string()];
    }

    // glob.c:1482-1535: the directory part is glob-free; dequote it for the
    // scan and use the dequoted form as the output prefix.
    let scan_dir = if directory.is_empty() {
        ".".to_string()
    } else {
        dequote_pathname(directory)
    };
    let names = glob_vector_expand(
        filename,
        &scan_dir,
        nocaseglob,
        extglob,
        dot_mode,
        globskipdots,
        env_vars,
    );
    if directory.is_empty() {
        return names;
    }
    names
        .into_iter()
        .map(|name| join_dir_to_name(&scan_dir, &name))
        .collect()
}

/// Port of glob.c glob_vector (glob.c:640-930) for one directory: either the
/// stat fast path for a glob-free tail, or the skipname-filtered scan with the
/// component matcher and the per-directory GLOBSORT application.
fn glob_vector_expand(
    pattern: &str,
    dir: &str,
    nocaseglob: bool,
    extglob: bool,
    dot_mode: DotMode,
    globskipdots: bool,
    env_vars: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    let dir_physical = shell_path_to_windows(dir, env_vars);
    // glob.c:669-691: an empty pattern just requires DIR to be a directory
    // and yields one empty name (glob_dir_to_array re-attaches the slash).
    if pattern.is_empty() {
        return if dir_physical.is_dir() {
            vec![String::new()]
        } else {
            Vec::new()
        };
    }
    // glob.c:695-752: a tail without unquoted wildcards (glob_loop.c
    // glob_pattern_p == 0) skips the directory scan and stats DIR/PAT,
    // returning the dequoted name on success.
    if !glob_pattern_p_component(pattern) {
        let npat = dequote_pathname(pattern);
        // glob.c:703-712: the stat path joins DIR and the dequoted tail with
        // a `/` unless DIR is empty or already ends with one.
        let full = if dir.is_empty() || dir.ends_with('/') {
            format!("{dir}{npat}")
        } else {
            format!("{dir}/{npat}")
        };
        return if shell_path_to_windows(&full, env_vars).exists() {
            vec![npat]
        } else {
            Vec::new()
        };
    }
    let entries = match shell_directory_entries(dir, env_vars) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let mut names = Vec::new();
    // With globskipdots off, `.` and `..` come back from readdir and the
    // dot-filter/matcher decide whether they match (glob10.sub).
    if !globskipdots && !skipname(&pattern_chars, ".", dot_mode, globskipdots) {
        names.push(".".to_string());
        names.push("..".to_string());
    }
    names.extend(entries.into_iter().map(|entry| entry.name));
    let mut matches: Vec<String> = names
        .into_iter()
        .filter(|name| {
            let name_chars: Vec<char> = name.chars().collect();
            !skipname(&pattern_chars, name, dot_mode, globskipdots)
                && component_matches(&pattern_chars, &name_chars, nocaseglob, extglob, dot_mode)
        })
        .collect();
    globsort_matches(&mut matches, env_vars);
    matches
}

/// glob_loop.c INTERNAL_GLOB_PATTERN_P (glob.c's internal pattern test for a
/// single component): wildcards, a completed bracket expression, or an
/// extglob operator followed by `(` make a pattern. Note there is no slash
/// handling and no extglob flag check here, matching bash 5.3.
fn glob_pattern_p_component(pattern: &str) -> bool {
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0usize;
    let mut bopen = 0usize;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        match c {
            '?' | '*' => return true,
            '[' => bopen += 1,
            ']' => {
                if bopen > 0 {
                    return true;
                }
            }
            '+' | '@' | '!' => {
                if chars.get(i) == Some(&'(') {
                    return true;
                }
            }
            '\\' | CTLESC => {
                if i >= chars.len() {
                    return false;
                }
                i += 1;
            }
            _ => {}
        }
    }
    false
}

/// Split at the LAST unquoted `/` (glob.c:1137-1147 strrchr/glob_dirscan): a
/// backslash- or CTLESC-quoted slash is data, not a separator. The directory
/// part keeps its trailing slash.
fn split_last_unquoted_slash(pattern: &str) -> (&str, &str) {
    let mut last: Option<usize> = None;
    let mut escaped = false;
    for (offset, ch) in pattern.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' | CTLESC => escaped = true,
            '/' => last = Some(offset),
            _ => {}
        }
    }
    match last {
        Some(offset) => {
            let slash_end = offset + '/'.len_utf8();
            (&pattern[..slash_end], &pattern[slash_end..])
        }
        None => ("", pattern),
    }
}

/// glob.c udequote_pathname (glob.c:429-448) extended to rubash's CTLESC
/// marker: a backslash or CTLESC quoting any character reduces to that
/// character.
fn dequote_pathname(pathname: &str) -> String {
    let mut output = String::new();
    let mut chars = pathname.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some(next) => output.push(next),
                None => output.push('\\'),
            },
            CTLESC => {
                if let Some(next) = chars.next() {
                    output.push(next);
                }
            }
            _ => output.push(ch),
        }
    }
    output
}

/// glob.c glob_dir_to_array (glob.c:1030-1101) name joining: an empty
/// directory passes names through; otherwise a `/` is added unless DIR
/// already ends with one.
fn join_dir_to_name(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        return name.to_string();
    }
    if dir.ends_with('/') {
        format!("{}{}", dir, name)
    } else {
        format!("{}/{}", dir, name)
    }
}

fn join_path_segment(prefix: &str, segment: &str) -> String {
    if prefix.is_empty() {
        segment.to_string()
    } else if prefix == "/" {
        format!("/{segment}")
    } else {
        format!("{prefix}/{segment}")
    }
}

/// Match one pathname component with the GNU strmatch engine, including the
/// leading-dot rules (FNM_PERIOD/FNM_DOTDOT) and the directory-scan dot
/// filter already applied by the caller via skipname().
fn component_matches(
    pattern: &[char],
    name: &[char],
    nocaseglob: bool,
    extglob: bool,
    dot_mode: DotMode,
) -> bool {
    // Bash's CTLESC marker (0x81) in a filename is an indivisible literal for
    // pathname expansion; wildcards never consume it (see
    // pathname_pattern_matches).
    if cfg!(windows)
        && name.contains(&'\u{81}')
        && contains_glob_or_extglob(&pattern.iter().collect::<String>())
    {
        return false;
    }
    gmatch(pattern, 0, name, 0, extglob, dot_mode, nocaseglob)
}

fn case_pattern_matches_nocase(pattern: &str, word: &str) -> bool {
    let pattern_lower = pattern.to_lowercase();
    let word_lower = word.to_lowercase();
    super::case_pattern_matches(&pattern_lower, &word_lower)
}

fn globstar_expand(
    word: &str,
    nullglob: bool,
    failglob: bool,
    nocaseglob: bool,
    dotglob: bool,
    globskipdots: bool,
    env_vars: &std::collections::HashMap<String, String>,
) -> PathnameExpansion {
    let parts: Vec<&str> = word.split("**").collect();
    // GNU collapses adjacent ** segments (**/** == **, a/**/** == a/**):
    // probe-verified on WSL GNU 5.2.21 (**/** lists every path exactly once,
    // identical to **). Rebuild the collapsed word and re-enter so those
    // forms reuse the tested single-** path. Only words keeping two or more
    // NON-adjacent ** segments (e.g. **/a/**) reach the multi walk below,
    // plus multi-segment remainders (**/foo*/*) and trailing-slash pattern
    // remainders (**/foo*/).
    if parts.len() > 2 {
        return expand_multi_globstar(
            word,
            nullglob,
            failglob,
            nocaseglob,
            dotglob,
            globskipdots,
            env_vars,
        );
    }
    let prefix = parts[0];
    // GNU globstar forms (parse.y/glob.c GLOBSTAR): a bare `**` matches every
    // file and directory at any depth (empty remainder), `**/` matches
    // directories only with a trailing slash in the output, and `**/pattern`
    // applies the pattern at every depth. Symlinked directories are listed as
    // entries but never recursed into (loop avoidance).
    let raw_suffix = parts[1];
    let single_remainder = !raw_suffix.starts_with('/') || !raw_suffix[1..].contains('/');
    if !single_remainder {
        return expand_multi_globstar(
            word,
            nullglob,
            failglob,
            nocaseglob,
            dotglob,
            globskipdots,
            env_vars,
        );
    }
    // GNU `**/foo*/` matches directories named foo* with the trailing slash
    // preserved: a trailing slash on a non-empty remainder means
    // directories-only with the pattern still applied.
    let trailing_slash = raw_suffix.len() > 1 && raw_suffix.ends_with('/');
    let dirs_only = raw_suffix == "/" || trailing_slash;
    let match_all = raw_suffix.is_empty();
    let suffix = raw_suffix.trim_matches('/');

    let logical_base_dir = if prefix.is_empty() {
        ".".to_string()
    } else if prefix == "/" {
        "/".to_string()
    } else {
        prefix.trim_end_matches('/').to_string()
    };
    let physical_base_dir = shell_path_to_windows(&logical_base_dir, env_vars);

    let mut matches = Vec::new();
    // GNU `lib/**` includes the zero-depth match: the base directory itself
    // with its trailing slash preserved (`echo lib/**` starts with `lib/`).
    // A bare `**` (empty prefix) has no depth-0 operand to emit - the base is
    // the cwd and GNU prints no `./` entry.
    if match_all && !prefix.is_empty() && logical_base_dir != "/" {
        let output = if dirs_only || !prefix.is_empty() {
            format!("{}/", logical_base_dir)
        } else {
            logical_base_dir.clone()
        };
        matches.push(output);
    }
    collect_globstar_matches(
        &logical_base_dir,
        &physical_base_dir,
        suffix,
        match_all,
        dirs_only,
        &mut matches,
        nocaseglob,
        dotglob,
        globskipdots,
        env_vars,
    );

    let matches = apply_globignore(matches, env_vars);
    if matches.is_empty() {
        return unmatched_expansion(word, nullglob, failglob);
    }
    // Depth-first per-directory order is the GNU globstar emission order;
    // do not re-sort globally here (each directory level is sorted in
    // collect_globstar_matches instead).
    PathnameExpansion::Matches(matches)
}

// GNU multi-globstar expansion. Adjacent ** segments collapse (**/** == **,
// a/**/** == a/** - probe-verified on WSL GNU 5.2.21: **/** lists every path
// exactly once, identical to **); a collapsed word re-enters the tested
// single-** path. Words keeping two or more non-adjacent ** segments
// (e.g. **/a/**) expand as a concatenation over every directory matching
// the prefix: each match contributes its own subtree copy, so results
// repeat once per matching ancestor and nothing is deduplicated.
fn expand_multi_globstar(
    word: &str,
    nullglob: bool,
    failglob: bool,
    nocaseglob: bool,
    dotglob: bool,
    globskipdots: bool,
    env_vars: &std::collections::HashMap<String, String>,
) -> PathnameExpansion {
    let absolute = word.starts_with('/');
    let dirs_only = word.len() > 1 && word.ends_with('/');
    let core = word.trim_matches('/');
    let mut segments: Vec<&str> = Vec::new();
    let mut collapsed = false;
    for segment in core.split('/') {
        if segment.is_empty() {
            continue;
        }
        if segment == "**" && segments.last() == Some(&"**") {
            collapsed = true;
            continue;
        }
        segments.push(segment);
    }
    let star_count = segments.iter().filter(|segment| **segment == "**").count();
    if collapsed && star_count <= 1 {
        let rebuilt = format!(
            "{}{}{}",
            if absolute { "/" } else { "" },
            segments.join("/"),
            if dirs_only { "/" } else { "" }
        );
        let mut expanded = globstar_expand(
            &rebuilt,
            nullglob,
            failglob,
            nocaseglob,
            dotglob,
            globskipdots,
            env_vars,
        );
        if !dirs_only {
            if let PathnameExpansion::Matches(values) = expanded {
                let base = segments
                    .iter()
                    .take_while(|segment| **segment != "**")
                    .copied()
                    .collect::<Vec<_>>()
                    .join("/");
                let slash_base = format!("{base}/");
                expanded = PathnameExpansion::Matches(
                    values
                        .into_iter()
                        .map(|value| {
                            if value == slash_base {
                                base.clone()
                            } else {
                                value
                            }
                        })
                        .collect(),
                );
            }
        }
        return expanded;
    }
    let base = if absolute { "/" } else { "." };
    let physical_base = shell_path_to_windows(base, env_vars);
    let mut matches = Vec::new();
    collect_multi_globstar_paths(
        &segments,
        0,
        base,
        &physical_base,
        dirs_only,
        &mut matches,
        nocaseglob,
        dotglob,
        globskipdots,
        env_vars,
    );
    matches.sort();
    let matches = apply_globignore(matches, env_vars);
    if matches.is_empty() {
        return unmatched_expansion(word, nullglob, failglob);
    }
    PathnameExpansion::Matches(matches)
}

// Collects the directories reachable from (logical, physical) without ever
// following symlinks, depth-first with each directory level sorted, self
// first. Hidden directories are skipped unless dotglob.
#[allow(dead_code)]
fn star_descend(
    logical: &str,
    physical: &Path,
    out: &mut Vec<(String, PathBuf)>,
    dotglob: bool,
    env_vars: &std::collections::HashMap<String, String>,
) {
    let entries = match shell_directory_entries(logical, env_vars) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
    names.sort();
    for name in names {
        if name.starts_with('.') && !dotglob {
            continue;
        }
        if name == "." || name == ".." {
            continue;
        }
        let child_physical = entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.path.clone())
            .unwrap_or_else(|| physical.join(&name));
        if !child_physical.is_dir() {
            continue;
        }
        let is_symlink = std::fs::symlink_metadata(&child_physical)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false);
        if is_symlink {
            continue;
        }
        let child_logical = join_path_segment(if logical == "." { "" } else { logical }, &name);
        out.push((child_logical.clone(), child_physical.clone()));
        star_descend(&child_logical, &child_physical, out, dotglob, env_vars);
    }
}

// Enumerate each globstar transition independently. Keeping the zero-depth and
// consuming transitions as separate recursive branches preserves GNU's real
// multiplicity for non-adjacent globstars; the final sort restores pathname
// expansion ordering after those branches have been concatenated.
#[allow(clippy::too_many_arguments)]
fn collect_multi_globstar_paths(
    segments: &[&str],
    index: usize,
    logical: &str,
    physical: &Path,
    dirs_only: bool,
    matches: &mut Vec<String>,
    nocaseglob: bool,
    dotglob: bool,
    globskipdots: bool,
    env_vars: &std::collections::HashMap<String, String>,
) {
    if index == segments.len() {
        if logical != "." && (!dirs_only || physical.is_dir()) {
            matches.push(if dirs_only {
                format!("{}/", logical)
            } else {
                logical.to_string()
            });
        }
        return;
    }
    let segment = segments[index];
    if segment == "**" {
        collect_multi_globstar_paths(
            segments,
            index + 1,
            logical,
            physical,
            dirs_only,
            matches,
            nocaseglob,
            dotglob,
            globskipdots,
            env_vars,
        );
        let entries = match shell_directory_entries(logical, env_vars) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        let mut names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
        names.sort();
        for name in names {
            if (name.starts_with('.') && !dotglob) || name == "." || name == ".." {
                continue;
            }
            let child_physical = entries
                .iter()
                .find(|entry| entry.name == name)
                .map(|entry| entry.path.clone())
                .unwrap_or_else(|| physical.join(&name));
            let child_logical = join_path_segment(if logical == "." { "" } else { logical }, &name);
            let is_dir = child_physical.is_dir();
            if index + 1 < segments.len() {
                let is_symlink = std::fs::symlink_metadata(&child_physical)
                    .map(|meta| meta.file_type().is_symlink())
                    .unwrap_or(false);
                if !is_dir || is_symlink {
                    continue;
                }
            }
            collect_multi_globstar_paths(
                segments,
                index,
                &child_logical,
                &child_physical,
                dirs_only,
                matches,
                nocaseglob,
                dotglob,
                globskipdots,
                env_vars,
            );
        }
        return;
    }
    let entries = match shell_directory_entries(logical, env_vars) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
    names.sort();
    for name in names {
        if (name.starts_with('.') && !dotglob) || name == "." || name == ".." {
            continue;
        }
        let matched = if nocaseglob {
            case_pattern_matches_nocase(segment, &name)
        } else {
            super::case_pattern_matches(segment, &name)
        };
        if !matched {
            continue;
        }
        let child_physical = entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.path.clone())
            .unwrap_or_else(|| physical.join(&name));
        let child_logical = join_path_segment(if logical == "." { "" } else { logical }, &name);
        if index + 1 < segments.len() {
            let is_symlink = std::fs::symlink_metadata(&child_physical)
                .map(|meta| meta.file_type().is_symlink())
                .unwrap_or(false);
            if !child_physical.is_dir() || is_symlink {
                continue;
            }
        }
        collect_multi_globstar_paths(
            segments,
            index + 1,
            &child_logical,
            &child_physical,
            dirs_only,
            matches,
            nocaseglob,
            dotglob,
            globskipdots,
            env_vars,
        );
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
fn collect_multi_globstar(
    segments: &[&str],
    logical_base: &str,
    physical_base: &Path,
    dirs_only: bool,
    matches: &mut Vec<String>,
    nocaseglob: bool,
    dotglob: bool,
    globskipdots: bool,
    env_vars: &std::collections::HashMap<String, String>,
) {
    let last = segments.len() - 1;
    let mut work: Vec<(String, PathBuf)> =
        vec![(logical_base.to_string(), physical_base.to_path_buf())];
    for (index, segment) in segments.iter().enumerate() {
        let is_last = index == last;
        if *segment == "**" {
            let mut expanded: Vec<(String, PathBuf)> = Vec::new();
            for (logical, physical) in &work {
                expanded.push((logical.clone(), physical.clone()));
                star_descend(logical, physical, &mut expanded, dotglob, env_vars);
            }
            work = expanded;
            if is_last {
                for (logical, physical) in &work {
                    collect_globstar_matches(
                        logical,
                        physical,
                        "",
                        true,
                        dirs_only,
                        matches,
                        nocaseglob,
                        dotglob,
                        globskipdots,
                        env_vars,
                    );
                }
            }
        } else {
            let mut next: Vec<(String, PathBuf)> = Vec::new();
            for (logical, physical) in &work {
                let entries = match shell_directory_entries(logical, env_vars) {
                    Ok(entries) => entries,
                    Err(_) => continue,
                };
                let mut names: Vec<String> =
                    entries.iter().map(|entry| entry.name.clone()).collect();
                names.sort();
                for name in names {
                    if name.starts_with('.') && !dotglob {
                        continue;
                    }
                    if name == "." || name == ".." {
                        continue;
                    }
                    let matched = if nocaseglob {
                        case_pattern_matches_nocase(segment, &name)
                    } else {
                        super::case_pattern_matches(segment, &name)
                    };
                    if !matched {
                        continue;
                    }
                    let child_physical = entries
                        .iter()
                        .find(|entry| entry.name == name)
                        .map(|entry| entry.path.clone())
                        .unwrap_or_else(|| physical.join(&name));
                    let child_logical =
                        join_path_segment(if logical == "." { "" } else { logical }, &name);
                    let is_dir = child_physical.is_dir();
                    if is_last {
                        if !dirs_only || is_dir {
                            let output = if dirs_only {
                                format!("{}/", child_logical)
                            } else {
                                child_logical.clone()
                            };
                            matches.push(output);
                        }
                    } else if is_dir {
                        let is_symlink = std::fs::symlink_metadata(&child_physical)
                            .map(|meta| meta.file_type().is_symlink())
                            .unwrap_or(false);
                        if !is_symlink {
                            next.push((child_logical, child_physical));
                        }
                    }
                }
            }
            work = next;
        }
    }
}

fn unmatched_expansion(word: &str, nullglob: bool, failglob: bool) -> PathnameExpansion {
    if failglob {
        PathnameExpansion::Fail(word.to_string())
    } else if nullglob {
        PathnameExpansion::Matches(Vec::new())
    } else {
        PathnameExpansion::NoMatch
    }
}

fn collect_globstar_matches(
    logical_dir: &str,
    physical_dir: &Path,
    suffix: &str,
    match_all: bool,
    dirs_only: bool,
    matches: &mut Vec<String>,
    nocaseglob: bool,
    dotglob: bool,
    globskipdots: bool,
    env_vars: &std::collections::HashMap<String, String>,
) {
    let entries = match shell_directory_entries(logical_dir, env_vars) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut names = synthetic_dot_names(suffix, globskipdots);
    names.extend(entries.iter().map(|entry| entry.name.clone()));
    // GNU glob.c sorts each directory's entries and emits depth-first:
    // results group by directory in traversal order rather than a global
    // byte sort over full paths (which would interleave `builtins.o` with
    // `builtins/...`).
    names.sort();
    let include_dotfiles =
        dotglob || suffix.starts_with('.') || globignore_patterns(env_vars).is_some();
    for name in names {
        if name.starts_with('.') && !include_dotfiles {
            continue;
        }
        let physical_path = entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.path.clone())
            .unwrap_or_else(|| physical_dir.join(&name));
        // GNU globstar output uses plain relative paths: the cwd-relative
        // base `.` contributes no `./` prefix to matched entries (glob.c
        // globstar expansion - `echo **` yields `a a/aa b` not `./a ./a/aa
        // ./b`). Keep `.` for the directory read but normalize it away in
        // the joined logical path.
        let join_prefix = if logical_dir == "." { "" } else { logical_dir };
        let logical_path = join_path_segment(join_prefix, &name);
        let is_dir = physical_path.is_dir();
        // dirs_only is checked before match_all so the multi-globstar final
        // ** can reuse this routine for directories-only emission while
        // still applying a pattern (**/foo*/ -> suffix "foo*").
        let matched = if dirs_only {
            if suffix.is_empty() {
                is_dir
            } else if nocaseglob {
                is_dir && case_pattern_matches_nocase(suffix, &name)
            } else {
                is_dir && super::case_pattern_matches(suffix, &name)
            }
        } else if match_all {
            true
        } else if nocaseglob {
            case_pattern_matches_nocase(suffix, &name)
        } else {
            super::case_pattern_matches(suffix, &name)
        };
        if matched {
            let output = if dirs_only {
                format!("{}/", logical_path)
            } else {
                logical_path.clone()
            };
            matches.push(output);
        }
        // Recurse into real directories only: GNU never follows symlinked
        // directories during ** recursion (loop avoidance; the symlink
        // itself still appears as a matched entry above).
        let is_symlink = std::fs::symlink_metadata(&physical_path)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false);
        if is_dir && !is_symlink && name != "." && name != ".." {
            collect_globstar_matches(
                &logical_path,
                &physical_path,
                suffix,
                match_all,
                dirs_only,
                matches,
                nocaseglob,
                dotglob,
                globskipdots,
                env_vars,
            );
        }
    }
}

/// Returns the GLOBIGNORE pattern list when the variable is set to a
/// non-null value. A null GLOBIGNORE still enables the dotfile side effect
/// (see globignore_assigned) but contributes no filter patterns.
fn globignore_patterns(
    env_vars: &std::collections::HashMap<String, String>,
) -> Option<Vec<String>> {
    let value = env_vars.get("GLOBIGNORE")?;
    if value.is_empty() {
        return None;
    }
    let extglob = crate::builtins::shopt::option_enabled(env_vars, "extglob");
    Some(split_ignore_specs(value, extglob))
}

/// pathexp.c split_ignorespec (pathexp.c:604-624): split on `:` with
/// skip_to_delim's SD_EXTGLOB|SD_GLOB semantics — a colon inside a bracket
/// expression or an extglob group does not delimit
/// (`@([-.,:; _]):[![:alnum:]]`).
fn split_ignore_specs(value: &str, extglob: bool) -> Vec<String> {
    let chars: Vec<char> = value.chars().collect();
    let mut specs: Vec<String> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut paren_depth = 0usize;
    while i < chars.len() {
        match chars[i] {
            '\\' | CTLESC => {
                i += 2;
                continue;
            }
            '[' => {
                i += 1;
                // `]` as the first member (after `!`/`^`) is a literal.
                if i < chars.len() && matches!(chars[i], '!' | '^') {
                    i += 1;
                }
                if i < chars.len() && chars[i] == ']' {
                    i += 1;
                }
                while i < chars.len() && chars[i] != ']' {
                    // `[:class:]` inside the bracket: skip to `:]`.
                    if chars[i] == '[' && chars.get(i + 1) == Some(&':') {
                        let mut j = i + 2;
                        while j + 1 < chars.len() && !(chars[j] == ':' && chars[j + 1] == ']') {
                            j += 1;
                        }
                        i = if j + 1 < chars.len() {
                            j + 2
                        } else {
                            chars.len()
                        };
                        continue;
                    }
                    i += 1;
                }
                if i < chars.len() {
                    i += 1; // closing `]`
                }
                continue;
            }
            '(' if paren_depth > 0
                || (extglob && i > 0 && matches!(chars[i - 1], '+' | '*' | '?' | '@' | '!')) =>
            {
                paren_depth += 1;
            }
            ')' if paren_depth > 0 => {
                paren_depth -= 1;
            }
            ':' if paren_depth == 0 => {
                specs.push(chars[start..i].iter().collect());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < chars.len() {
        specs.push(chars[start..].iter().collect());
    }
    specs.into_iter().filter(|s| !s.is_empty()).collect()
}

/// True when GLOBIGNORE has been assigned at all (set, even to the null
/// string). WSL GNU 5.2.21 probe: after `GLOBIGNORE=` dotfiles still match
/// (the assignment enables the dotglob side effect) while only unsetting
/// restores the default; pattern filtering itself requires a non-null
/// value.
fn globignore_assigned(env_vars: &std::collections::HashMap<String, String>) -> bool {
    env_vars.contains_key("GLOBIGNORE")
}

/// Filters a collected pathname-expansion match list by GLOBIGNORE (bash
/// glob.c ignorable()): when GLOBIGNORE is set non-null, a match whose
/// basename matches any ignore pattern is removed, and `.` / `..` are
/// always removed. Assigning GLOBIGNORE also enables dotfile matching (the
/// dotglob side effect), which the callers handle at collection time.
fn apply_globignore(
    matches: Vec<String>,
    env_vars: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    let Some(patterns) = globignore_patterns(env_vars) else {
        return matches;
    };
    // pathexp.c:545: GLOBIGNORE patterns match with FNM_EXTFLAG (extglob)
    // and FNM_NOCASEGLOB (nocaseglob).
    let extglob = crate::builtins::shopt::option_enabled(env_vars, "extglob");
    let nocase = crate::builtins::shopt::option_enabled(env_vars, "nocaseglob");
    matches
        .into_iter()
        .filter(|name| {
            let base = name.rsplit('/').next().unwrap_or(name);
            base != "."
                && base != ".."
                && !patterns
                    .iter()
                    .any(|pattern| ignore_pattern_matches(pattern, base, extglob, nocase))
        })
        .collect()
}

fn ignore_pattern_matches(pattern: &str, word: &str, extglob: bool, nocase: bool) -> bool {
    if extglob && super::parameter_decode::pattern_uses_extglob_syntax(pattern) {
        if nocase {
            return super::conditional::extglob_case_pattern_matches_nocase(pattern, word);
        }
        return super::conditional::extglob_case_pattern_matches(pattern, word);
    }
    if nocase {
        return super::conditional::case_pattern_matches_nocase(pattern, word);
    }
    super::case_pattern_matches(pattern, word)
}

fn synthetic_dot_names(pattern: &str, globskipdots: bool) -> Vec<String> {
    if globskipdots || !pattern.starts_with('.') {
        Vec::new()
    } else {
        vec![".".to_string(), "..".to_string()]
    }
}

// ===== GNU pathname-expansion match engine (lib/glob) ======================

/// Rubash's in-word quote marker (bash CTLESC): the next character is quoted
/// data, not a pattern character.
const CTLESC: char = '\x11';

/// Port of pathexp.c unquoted_glob_pattern_p (pathexp.c:66-134): the shell-side
/// decision of whether a word is a pathname pattern. Implements POSIX 2.13.3:
/// an unquoted `/` cannot appear in a bracket expression, so `[qwe/qwe]` and
/// `[qwe/` are literal words, not patterns (glob7.tests). Quoted characters
/// (CTLESC-marked) are never special.
fn unquoted_glob_pattern_p(string: &str, extended_glob: bool) -> bool {
    let chars: Vec<char> = string.chars().collect();
    let mut i = 0usize;
    let mut open = 0usize;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        match c {
            '?' | '*' => return true,
            '[' => open += 1,
            ']' => {
                if open > 0 {
                    return true;
                }
            }
            '/' => {
                if open > 0 {
                    open = 0;
                }
            }
            '+' | '@' | '!' => {
                if extended_glob && chars.get(i) == Some(&'(') {
                    return true;
                }
            }
            '\\' => {
                if chars.get(i) == Some(&CTLESC) {
                    i += 1;
                    if chars.get(i) == Some(&CTLESC) {
                        i += 1;
                    }
                } else {
                    // An unquoted backslash quotes (hides) the next character.
                    if i >= chars.len() {
                        return false;
                    }
                    i += 1;
                }
            }
            CTLESC => {
                if i >= chars.len() {
                    return false;
                }
                i += 1;
            }
            _ => {}
        }
    }
    false
}

/// Which leading-dot rule is in force while matching a pathname component
/// (glob.c:774: FNM_PERIOD when dotglob is off, FNM_DOTDOT when on).
#[derive(Clone, Copy, PartialEq, Eq)]
enum DotMode {
    /// The leading-dot flags have been dropped (sm_loop.c xflags): no guard.
    Off,
    /// dotglob off: a leading `.` in the name must be matched by a literal
    /// pattern dot; `*` and `?` can never match it (sm_loop.c:104,150).
    Period,
    /// dotglob on: `*`/`?` match dotfiles but never exactly `.` or `..`
    /// (sm_loop.c:114,160).
    DotDot,
}

fn is_dot_or_dotdot(name: &str) -> bool {
    name == "." || name == ".."
}

/// smatch.c SDOT_OR_DOTDOT: the name starting at `n` is exactly `.` or `..`.
fn sdot_or_dotdot_at(name: &[char], n: usize) -> bool {
    name.get(n) == Some(&'.')
        && (name.len() == n + 1 || (name.get(n + 1) == Some(&'.') && name.len() == n + 2))
}

/// True when the pattern begins with a literal dot as glob.c skipname counts
/// it: a raw `.` or a backslash-escaped `\.` (glob.c:271-272,287-288). A
/// CTLESC-quoted dot is deliberately NOT counted, mirroring the C check.
fn pattern_leading_dot(pattern: &[char]) -> bool {
    match pattern.first() {
        Some('.') => true,
        Some('\\') => pattern.get(1) == Some(&'.'),
        _ => false,
    }
}

/// Find the `)` closing the extglob group whose `(` is at `open_idx`, with
/// PATSCAN semantics (sm_loop.c): backslash/CTLESC escapes, bracket nesting
/// (including a leading `!`/`^` and POSIX `[:...:]`/`[. .]`/`[= =]` spans) and
/// paren nesting are respected. Returns the index of the closing `)`.
fn patscan_group_end(pattern: &[char], open_idx: usize) -> Option<usize> {
    let mut pnest = 0i32;
    let mut bnest = 0i32;
    let mut bfirst: Option<usize> = None;
    let mut i = open_idx + 1;
    while i < pattern.len() {
        let c = pattern[i];
        if matches!(c, '\\' | CTLESC) {
            i += 2;
            continue;
        }
        match c {
            '[' => {
                if bnest == 0 {
                    let mut first = i + 1;
                    if matches!(pattern.get(first), Some('!') | Some('^')) {
                        first += 1;
                    }
                    bfirst = Some(first);
                    bnest += 1;
                } else if matches!(pattern.get(i + 1), Some(':') | Some('.') | Some('=')) {
                    // POSIX sub-bracket span: skip to its terminator.
                    let open = pattern[i + 1];
                    let mut j = i + 2;
                    while j + 1 < pattern.len() {
                        if pattern[j] == open && pattern[j + 1] == ']' {
                            i = j + 1;
                            break;
                        }
                        j += 1;
                    }
                }
            }
            ']' => {
                if bnest > 0 && bfirst != Some(i) {
                    bnest -= 1;
                    bfirst = None;
                }
            }
            '(' => {
                if bnest == 0 {
                    pnest += 1;
                }
            }
            ')' => {
                if bnest == 0 {
                    if pnest <= 0 {
                        return Some(i);
                    }
                    pnest -= 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split an extglob group body on top-level `|` (PATSCAN with delim '|'):
/// escapes, brackets and nested parens are respected. Empty alternatives are
/// kept (`@(|foo)` has a first alternative that matches the empty string).
fn split_top_alternatives(body: &[char]) -> Vec<Vec<char>> {
    let mut result = Vec::new();
    let mut current = Vec::new();
    let mut pnest = 0i32;
    let mut bnest = 0i32;
    let mut bfirst: Option<usize> = None;
    let mut i = 0usize;
    while i < body.len() {
        let c = body[i];
        if matches!(c, '\\' | CTLESC) {
            current.push(c);
            if i + 1 < body.len() {
                current.push(body[i + 1]);
            }
            i += 2;
            continue;
        }
        match c {
            '[' => {
                if bnest == 0 {
                    let mut first = i + 1;
                    if matches!(body.get(first), Some('!') | Some('^')) {
                        first += 1;
                    }
                    bfirst = Some(first);
                    bnest += 1;
                }
                current.push(c);
            }
            ']' => {
                if bnest > 0 && bfirst != Some(i) {
                    bnest -= 1;
                    bfirst = None;
                }
                current.push(c);
            }
            '(' => {
                if bnest == 0 {
                    pnest += 1;
                }
                current.push(c);
            }
            ')' => {
                if bnest == 0 && pnest > 0 {
                    pnest -= 1;
                }
                current.push(c);
            }
            '|' if bnest == 0 && pnest == 0 => {
                result.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
        i += 1;
    }
    result.push(current);
    result
}

/// True when the pattern starts with an extglob operator followed by `(`.
fn extglob_at(pattern: &[char], p: usize) -> bool {
    matches!(pattern.get(p), Some('+' | '*' | '?' | '@' | '!')) && pattern.get(p + 1) == Some(&'(')
}

/// Port of glob.c skipname (glob.c:256-292) plus extglob_skipname
/// (glob.c:190-253): the directory-scan pre-filter deciding whether a dirent
/// is even considered. Returns true when the name must be skipped.
fn skipname(pattern: &[char], name: &str, dot_mode: DotMode, globskipdots: bool) -> bool {
    if extglob_at(pattern, 0) {
        return extglob_skipname(pattern, name, dot_mode, globskipdots);
    }
    if globskipdots && is_dot_or_dotdot(name) {
        return true;
    }
    if pattern_leading_dot(pattern) {
        return false;
    }
    let first = match name.chars().next() {
        Some(c) => c,
        None => return false,
    };
    if first != '.' {
        return false;
    }
    match dot_mode {
        // glob.c:271-274: with dotglob on only `.`/`..` require a literal dot.
        DotMode::DotDot => is_dot_or_dotdot(name),
        // glob.c:287-289: with dotglob off every dotfile requires one.
        DotMode::Period | DotMode::Off => true,
    }
}

/// Port of glob.c extglob_skipname (glob.c:190-253).
fn extglob_skipname(pattern: &[char], name: &str, dot_mode: DotMode, globskipdots: bool) -> bool {
    let op = pattern[0];
    let wild = op == '*' || op == '?';
    let Some(close) = patscan_group_end(pattern, 1) else {
        // Invalid extglob pattern: glob.c returns 0 (do not skip).
        return false;
    };
    let inner = &pattern[2..close];
    let after = &pattern[close + 1..];
    let alternatives = split_top_alternatives(inner);

    // glob.c:209-218: a whole-pattern group with a single alternative
    // recurses on that alternative (the GX_NEGATE flag is ignored by
    // skipname, so negation does not change dot visibility here).
    if after.is_empty() && alternatives.len() == 1 {
        return skipname(&alternatives[0], name, dot_mode, globskipdots);
    }

    // glob.c:225-242: if any alternative says "do not skip", we do not skip.
    for alt in &alternatives {
        if !skipname(alt, name, dot_mode, globskipdots) {
            return false;
        }
    }

    // glob.c:249-250: `*()`/`?()` can match zero occurrences, so a pattern
    // with trailing characters checks the trailing part with the original
    // flags. `!(bar).foo` (wild == false) falls through and skips dotfiles,
    // while `*(bar).foo` / `?(bar).foo` make them visible again.
    if wild && !after.is_empty() {
        return skipname(after, name, dot_mode, globskipdots);
    }
    true
}

/// Port of sm_loop.c GMATCH (strmatch) for one pathname component: pattern and
/// name are char slices; `guard` carries the leading-dot rule, which only
/// bites at the very start of the name (sm_loop.c conditions on `n == string`)
/// except for the `!` negation guard, which the C code applies at the current
/// match start whenever the flags are still in effect.
fn gmatch(
    pattern: &[char],
    p: usize,
    name: &[char],
    n: usize,
    extglob: bool,
    guard: DotMode,
    nocase: bool,
) -> bool {
    let mut p = p;
    let mut n = n;
    while p < pattern.len() {
        let c = pattern[p];
        // EXTMATCH dispatch (sm_loop.c:85-93): the leading-dot flags survive
        // only while matching at the very start of the name.
        if extglob && extglob_at(pattern, p) {
            let local_guard = if n == 0 { guard } else { DotMode::Off };
            return extmatch(c, pattern, p + 1, name, n, local_guard, extglob, nocase);
        }
        match c {
            '?' => {
                let Some(sc) = name.get(n) else { return false };
                if guard == DotMode::Period && n == 0 && *sc == '.' {
                    return false;
                }
                if guard == DotMode::DotDot && n == 0 && sdot_or_dotdot_at(name, 0) {
                    return false;
                }
                p += 1;
                n += 1;
            }
            '*' => {
                if guard == DotMode::Period && n == 0 && name.first() == Some(&'.') {
                    return false;
                }
                if guard == DotMode::DotDot && n == 0 && sdot_or_dotdot_at(name, 0) {
                    return false;
                }
                let star_start = p;
                while p < pattern.len() && pattern[p] == '*' {
                    p += 1;
                }
                if p == pattern.len() {
                    return true;
                }
                // sm_loop.c:206-224: a run of stars directly followed by `(`
                // forms the `*(...)` extglob operator (zero or more
                // occurrences of the group). With several stars the earlier
                // ones stay wildcards: sm_loop.c:277-312 backtracks the first
                // star while the last star acts as the operator
                // (`ab**(e|f)` matches abcdef: `*` eats `cd`, `*(e|f)` `ef`).
                if pattern[p] == '(' {
                    if p - 1 == star_start {
                        let local_guard = if n == 0 { guard } else { DotMode::Off };
                        return extmatch(
                            '*',
                            pattern,
                            p + 1,
                            name,
                            n,
                            local_guard,
                            extglob,
                            nocase,
                        );
                    }
                    for split in n..=name.len() {
                        if extmatch(
                            '*',
                            pattern,
                            p + 1,
                            name,
                            split,
                            DotMode::Off,
                            extglob,
                            nocase,
                        ) {
                            return true;
                        }
                    }
                    return false;
                }
                // sm_loop.c:306: the backtracking recursion drops the leading
                // dot flags; the guards were already applied above.
                for split in n..=name.len() {
                    if gmatch(pattern, p, name, split, extglob, DotMode::Off, nocase) {
                        return true;
                    }
                }
                return false;
            }
            '\\' | CTLESC => {
                // Backslash escape / quoted character: the next pattern
                // character is a literal (sm_loop.c:121-138).
                let Some(lit) = pattern.get(p + 1) else {
                    return false;
                };
                let Some(sc) = name.get(n) else {
                    return false;
                };
                if !chars_match(*lit, *sc, nocase) {
                    return false;
                }
                p += 2;
                n += 1;
            }
            '[' => {
                let Some(sc) = name.get(n).copied() else {
                    return false;
                };
                match super::conditional::case_bracket_expression_matches_with_case(
                    pattern,
                    p,
                    Some(sc),
                    nocase,
                ) {
                    Some((matched, next)) => {
                        if !matched {
                            return false;
                        }
                        p = next;
                        n += 1;
                    }
                    None => {
                        // Unterminated bracket: `[` is a literal character
                        // (glob.tests ok 32).
                        if !chars_match('[', sc, nocase) {
                            return false;
                        }
                        p += 1;
                        n += 1;
                    }
                }
            }
            lit => {
                let Some(sc) = name.get(n) else {
                    return false;
                };
                if !chars_match(lit, *sc, nocase) {
                    return false;
                }
                p += 1;
                n += 1;
            }
        }
    }
    n == name.len()
}

/// Port of sm_loop.c EXTMATCH for one extglob group. `paren` is the index of
/// the group's `(`; the operator is at `paren - 1`.
fn extmatch(
    op: char,
    pattern: &[char],
    paren: usize,
    name: &[char],
    s: usize,
    guard: DotMode,
    extglob: bool,
    nocase: bool,
) -> bool {
    let Some(close) = patscan_group_end(pattern, paren) else {
        return false;
    };
    let body = &pattern[paren + 1..close];
    let rest = close + 1;
    let alternatives = split_top_alternatives(body);

    // sm_loop.c:876: when nothing follows the group, the group must match the
    // whole remainder, so the split loop starts at the end.
    let rest_empty = rest >= pattern.len();

    let rest_matches =
        |srest: usize, xg: DotMode| gmatch(pattern, rest, name, srest, extglob, xg, nocase);
    let group_repeat_matches = |srest: usize, xg: DotMode| {
        // sm_loop.c:852 / 881: re-apply the whole group (operator included).
        gmatch(pattern, paren - 1, name, srest, extglob, xg, nocase)
    };
    let alt_matches =
        |alt: &[char], srest: usize| alt_matches_slice(alt, name, s, srest, extglob, guard, nocase);
    // sm_loop.c xflags: the leading-dot flags are dropped once the match
    // position has moved past the string start.
    let xflags = |srest: usize| if srest > s { DotMode::Off } else { guard };

    match op {
        '+' | '*' => {
            // sm_loop.c:830: `*()` first tries zero occurrences.
            if op == '*' && rest_matches(s, guard) {
                return true;
            }
            for srest in s..=name.len() {
                if alternatives.iter().any(|alt| alt_matches(alt, srest)) {
                    if rest_matches(srest, xflags(srest))
                        || (srest != s && group_repeat_matches(srest, xflags(srest)))
                    {
                        return true;
                    }
                }
            }
            false
        }
        '?' | '@' => {
            // sm_loop.c:867: `?()` first tries zero occurrences.
            if op == '?' && rest_matches(s, guard) {
                return true;
            }
            let start = if rest_empty { name.len() } else { s };
            for srest in start..=name.len() {
                for alt in &alternatives {
                    if alt_matches(alt, srest) && rest_matches(srest, xflags(srest)) {
                        return true;
                    }
                }
            }
            false
        }
        '!' => {
            for srest in s..=name.len() {
                let m1 = alternatives.iter().any(|alt| alt_matches(alt, srest));
                if !m1 {
                    // sm_loop.c:904-911: when nothing matched and the string
                    // starts with a dot (or is `.`/`..` under FNM_DOTDOT), the
                    // negation must not produce a match.
                    if guard == DotMode::Period && name.get(s) == Some(&'.') {
                        return false;
                    }
                    if guard == DotMode::DotDot && sdot_or_dotdot_at(name, s) {
                        return false;
                    }
                    if rest_matches(srest, xflags(srest)) {
                        return true;
                    }
                }
            }
            false
        }
        _ => false,
    }
}

/// Match the alternative `alt` against name[s..srest] (sm_loop.c:843/881: the
/// sub-pattern is matched against a prefix of the remaining string, with the
/// current flags — the leading-dot guard included). The guard's position
/// conditions refer to the sub-match start, which is position 0 of the slice.
fn alt_matches_slice(
    alt: &[char],
    name: &[char],
    s: usize,
    srest: usize,
    extglob: bool,
    guard: DotMode,
    nocase: bool,
) -> bool {
    let slice: Vec<char> = name[s..srest].to_vec();
    gmatch(alt, 0, &slice, 0, extglob, guard, nocase)
}

fn chars_match(pattern: char, candidate: char, nocase: bool) -> bool {
    if nocase {
        pattern.eq_ignore_ascii_case(&candidate)
    } else {
        pattern == candidate
    }
}

// ===== GLOBSORT (pathexp.c setup_globsort / sh_sortglob) ====================

const SORT_NONE: u8 = 0;
const SORT_NAME: u8 = 1;
const SORT_SIZE: u8 = 2;
const SORT_MTIME: u8 = 3;
const SORT_ATIME: u8 = 4;
const SORT_CTIME: u8 = 5;
const SORT_BLOCKS: u8 = 6;
const SORT_NUMERIC: u8 = 7;
const SORT_NOSORT: u8 = 8;
const SORT_REVERSE: u8 = 128;

struct GlobSortSpec {
    kind: u8,
}

/// Port of pathexp.c setup_globsort: leading whitespace skipped, a leading
/// `+` is ignored, a leading `-` reverses, an unknown name collapses to
/// SORT_NONE (the reverse bit is dropped with it).
fn globsort_spec(env_vars: &std::collections::HashMap<String, String>) -> GlobSortSpec {
    let default = GlobSortSpec { kind: SORT_NONE };
    let Some(value) = env_vars.get("GLOBSORT") else {
        return default;
    };
    let mut rest = value.as_str();
    while let Some(first) = rest.chars().next() {
        if first.is_whitespace() {
            rest = &rest[1..];
        } else {
            break;
        }
    }
    let mut reverse = false;
    if let Some(stripped) = rest.strip_prefix('+') {
        rest = stripped;
    } else if let Some(stripped) = rest.strip_prefix('-') {
        reverse = true;
        rest = stripped;
    }
    if rest.is_empty() {
        // A bare `+` is the default ascending name sort; a bare `-` is the
        // descending name sort.
        return GlobSortSpec {
            kind: if reverse {
                SORT_NAME | SORT_REVERSE
            } else {
                SORT_NAME
            },
        };
    }
    let kind = match rest {
        "name" => SORT_NAME,
        "size" => SORT_SIZE,
        "mtime" => SORT_MTIME,
        "atime" => SORT_ATIME,
        "ctime" => SORT_CTIME,
        "blocks" => SORT_BLOCKS,
        "numeric" => SORT_NUMERIC,
        "nosort" => SORT_NOSORT,
        _ => SORT_NONE,
    };
    GlobSortSpec {
        kind: if kind == SORT_NONE {
            SORT_NONE
        } else if reverse {
            kind | SORT_REVERSE
        } else {
            kind
        },
    }
}

/// Port of pathexp.c sh_sortglob. Sorts the collected match list in place.
/// `physical` maps each match to its backing path for stat-based sorts.
fn globsort_matches(
    matches: &mut Vec<String>,
    env_vars: &std::collections::HashMap<String, String>,
) {
    let spec = globsort_spec(env_vars);
    if spec.kind == SORT_NOSORT || spec.kind == (SORT_NOSORT | SORT_REVERSE) {
        return;
    }
    if spec.kind == SORT_NONE || spec.kind == SORT_NAME || spec.kind == (SORT_NAME | SORT_REVERSE) {
        let reverse = spec.kind & SORT_REVERSE != 0;
        if reverse {
            matches.sort_by(|a, b| b.cmp(a));
        } else {
            matches.sort();
        }
        return;
    }
    let reverse = spec.kind & SORT_REVERSE != 0;
    let base = spec.kind & !SORT_REVERSE;
    // globsort_buildarray: stat each name once; failures get the null stat
    // (size -1, times -1), sorting failed entries first.
    let stats: Vec<(i64, i64, i64, i64)> = matches
        .iter()
        .map(|name| {
            let physical = shell_path_to_windows(name, env_vars);
            match std::fs::metadata(&physical) {
                Ok(meta) => {
                    let size = meta.len() as i64;
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos() as i64)
                        .unwrap_or(-1);
                    let atime = meta
                        .accessed()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos() as i64)
                        .unwrap_or(-1);
                    let ctime = meta
                        .created()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos() as i64)
                        .unwrap_or(-1);
                    (size, mtime, atime, ctime)
                }
                Err(_) => (-1, -1, -1, -1),
            }
        })
        .collect();
    let name_cmp = |a: &str, b: &str| {
        if reverse {
            b.cmp(a)
        } else {
            a.cmp(b)
        }
    };
    let indices: Vec<usize> = (0..matches.len()).collect();
    let mut sorted = indices;
    sorted.sort_by(|&ia, &ib| {
        let (na, nb) = (&matches[ia], &matches[ib]);
        let x = match base {
            SORT_SIZE => stats[ia].0.cmp(&stats[ib].0),
            SORT_MTIME => stats[ia].1.cmp(&stats[ib].1),
            SORT_ATIME => stats[ia].2.cmp(&stats[ib].2),
            SORT_CTIME => stats[ia].3.cmp(&stats[ib].3),
            SORT_NUMERIC => {
                let va = all_digits(na);
                let vb = all_digits(nb);
                match (va, vb) {
                    (true, true) => {
                        let o = numeric_value(na).cmp(&numeric_value(nb));
                        if reverse {
                            o.reverse()
                        } else {
                            o
                        }
                    }
                    (false, false) => name_cmp(na, nb),
                    (true, false) => {
                        if reverse {
                            std::cmp::Ordering::Greater
                        } else {
                            std::cmp::Ordering::Less
                        }
                    }
                    (false, true) => {
                        if reverse {
                            std::cmp::Ordering::Less
                        } else {
                            std::cmp::Ordering::Greater
                        }
                    }
                }
            }
            // atime/blocks are not ported: no suite exercises them; fall back
            // to the name comparison (documented deviation).
            _ => name_cmp(na, nb),
        };
        // Ties fall back to the (possibly reversed) name comparison
        // (globsort_sizecmp / globsort_timecmp).
        if x == std::cmp::Ordering::Equal {
            name_cmp(na, nb)
        } else {
            x
        }
    });
    let ordered: Vec<String> = sorted.into_iter().map(|i| matches[i].clone()).collect();
    *matches = ordered;
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

fn numeric_value(s: &str) -> u128 {
    s.chars().fold(0u128, |acc, c| {
        acc.wrapping_mul(10)
            .wrapping_add(c.to_digit(10).unwrap_or(0) as u128)
    })
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::{component_matches, pathname_expand_word, DotMode, PathnameExpansion};
    #[cfg(windows)]
    use std::collections::HashMap;

    #[cfg(windows)]
    #[test]
    fn logical_root_glob_reads_backing_directory_and_returns_logical_names() {
        let root = std::env::temp_dir().join("rubash-logical-root-glob");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("etc")).unwrap();
        std::fs::write(root.join("etc").join("config"), "value").unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert(
            "RUBASH_ROOT".to_string(),
            root.to_string_lossy().to_string(),
        );

        let PathnameExpansion::Matches(matches) = pathname_expand_word("/etc/*", &env_vars) else {
            panic!("logical root glob did not produce matches");
        };
        assert_eq!(matches, vec!["/etc/config".to_string()]);

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn ctlesc_filename_is_not_consumed_by_pathname_wildcards() {
        let marker_name = "uni\u{81}code";

        assert!(component_matches(
            &marker_name.chars().collect::<Vec<char>>(),
            &marker_name.chars().collect::<Vec<char>>(),
            false,
            false,
            DotMode::Period
        ));
        assert!(!component_matches(
            &"uni?code".chars().collect::<Vec<char>>(),
            &marker_name.chars().collect::<Vec<char>>(),
            false,
            false,
            DotMode::Period
        ));
        assert!(!component_matches(
            &"uni*code".chars().collect::<Vec<char>>(),
            &marker_name.chars().collect::<Vec<char>>(),
            false,
            false,
            DotMode::Period
        ));
    }
}
