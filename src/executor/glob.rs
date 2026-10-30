//! Pathname expansion (globbing) for command words.

use std::path::{Path, PathBuf};

use crate::executor::path::{shell_directory_entries_in, shell_path_to_windows};

/// G-02: resolve a logical glob directory to its physical backing path,
/// anchoring relative forms at the session cwd. Before G-02 the relative
/// forms fell through to the process cwd, which the session owned.
fn anchored_physical(
    dir: &str,
    env_vars: &std::collections::HashMap<String, String>,
    cwd: &Path,
) -> PathBuf {
    let physical = shell_path_to_windows(dir, env_vars);
    if physical.is_absolute() {
        physical
    } else {
        cwd.join(physical)
    }
}

/// The session cwd for call sites that hold no Executor (completion,
/// declare storage): $PWD is the session cwd's logical display form.
pub(crate) fn env_cwd_path(env_vars: &std::collections::HashMap<String, String>) -> PathBuf {
    if let Some(pwd) = env_vars.get("PWD") {
        let physical = shell_path_to_windows(pwd, env_vars);
        if physical.is_absolute() {
            return physical;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}

pub(crate) enum PathnameExpansion {
    Matches(Vec<String>),
    NoMatch,
    Fail(String),
}

/// Re-encode a compound array assignment element token (quotes still
/// inline) into the glob engine's escape model.
///
/// GNU re-parses the element text of `arr=( ... )` with the real parser
/// (arrayfunc.c:574 `parse_string_to_word_list`) and hands the resulting
/// words to pathname expansion via arrayfunc.c:610
/// `expand_words_no_vars` -> subst.c:12602 `glob_expand_word_list`. By then
/// each word has been through expand_word_internal's quote processing: the
/// quote characters themselves are gone (subst.c:11883 `string_extract_
/// single_quoted` / subst.c:11719 `string_extract_double_quoted`) and every
/// character that came from inside a quoted span is CTLESC-protected
/// (subst.c:4773 `quote_string` via the `add_quoted_string` label). Quoting
/// therefore protects only the characters it covers — a quoted segment
/// followed by an unquoted glob still pathname-expands
/// (`arr=("dir"/*.txt)` stores the matches, not the pattern), and the
/// pattern decision at pathexp.c:66 `unquoted_glob_pattern_p` skips the
/// CTLESC-protected characters instead of bailing on the whole word.
///
/// Rubash's compound-assignment tokens reach the globber with the quote
/// characters still inline, so this performs the same reduction: quote
/// characters are dropped, every character of a quoted span is
/// CTLESC-protected (the exact `quote_string` form), and unquoted text —
/// including unquoted backslash escapes, which the engine already treats
/// as hiding the next character — stays verbatim.
///
/// Only the compound-assignment call sites may use this: quote characters
/// in command-substitution output or field-split products are DATA (GNU
/// never re-parses that text), and re-encoding them as operators would
/// corrupt those patterns.
pub(crate) fn compound_element_glob_pattern(token: &str) -> String {
    // Fast path: no quote operators, nothing to re-encode (also keeps
    // tokens whose quote characters are data untouched by construction).
    if !token.contains(['\'', '"']) {
        return token.to_string();
    }
    let mut out = String::with_capacity(token.len());
    let mut chars = token.chars().peekable();
    // Mirror the single/double quote state of read_token_word: inside `'`
    // everything is literal; inside `"` a backslash only escapes `"`, `\`,
    // `$`, `` ` `` and newline (subst.c skip_double_quoted), otherwise the
    // backslash itself stays as data.
    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                for quoted in chars.by_ref() {
                    if quoted == '\'' {
                        break;
                    }
                    out.push(crate::executor::markers::CTLESC);
                    out.push(quoted);
                }
            }
            '"' => {
                while let Some(quoted) = chars.next() {
                    if quoted == '"' {
                        break;
                    }
                    if quoted == '\\' && matches!(chars.peek(), Some('"' | '\\' | '$' | '`' | '\n'))
                    {
                        // The escape is consumed by the quote processing;
                        // only the escaped character survives as data.
                        if let Some(escaped) = chars.next() {
                            out.push(crate::executor::markers::CTLESC);
                            out.push(escaped);
                        }
                        continue;
                    }
                    out.push(crate::executor::markers::CTLESC);
                    out.push(quoted);
                }
            }
            // An unquoted backslash keeps both characters in the word at
            // glob time (GNU keeps the pair; quote removal is deferred),
            // and unquoted_glob_pattern_p plus dequote_pathname already
            // treat the pair as a protected literal.
            '\\' => {
                out.push('\\');
                if let Some(escaped) = chars.next() {
                    out.push(escaped);
                }
            }
            _ => out.push(ch),
        }
    }
    out
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
        if ch == '\\' || ch == crate::executor::markers::CTLESC {
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
///
/// Admission mirrors GNU exactly (rubash#832 family): the only pattern test
/// is `unquoted_glob_pattern_p` below, the port of pathexp.c:66 — the same
/// predicate glob_expand_word_list consults (subst.c:12626: "If the word
/// isn't an assignment and contains an unquoted pattern matching character,
/// then glob it"). Quote and brace characters are NOT on that list: once a
/// `'` `"` `{` `}` reaches this function it is expansion OUTPUT data
/// (subst.c:6643 comsub_shouldquote CTLESC-quotes only IFS-special bytes for
/// an unquoted enclosing word, so `echo $(echo "'")*` globs in GNU), and a
/// literal brace survives only when brace expansion did not consume it
/// (`echo {a*` is an unmatched-brace literal word that still globs, GNU
/// subst.c brace expansion runs before pathname expansion and never
/// re-scans output). Genuine quoting suppression rides the CTLESC carrier
/// (markers::CTLESC, handled inside unquoted_glob_pattern_p) and the
/// callers' raw-word suppress_glob flags — never a re-sniff of the expanded
/// text. The former `starts_with('"')/contains('{')` NoMatch shortcuts made
/// globbing depend on word TEXT instead of quoting state: with a matching
/// file in CWD, GNU printed `'y` / `{ab` where rubash kept the literal
/// pattern — a CWD-observable divergence on the #832 face.
pub(crate) fn pathname_expand_word(
    word: &str,
    env_vars: &std::collections::HashMap<String, String>,
    cwd: &Path,
) -> PathnameExpansion {
    if word.is_empty() {
        return PathnameExpansion::NoMatch;
    }
    // Whitelist admission (rubash#117 discipline): `unquoted_glob_pattern_p`
    // below only reports a pattern when the word holds an unquoted glob
    // metacharacter, and every one of its true arms needs one of these
    // bytes present (`?*`, `[`, extglob `+@!` before `(` — the `]` arm
    // needs a preceding `[`). Without any of them the word is provably
    // pattern-free under every option combination (noglob/nullglob/
    // failglob only matter for real patterns), so skip the seven option
    // lookups + the quote-aware scan. A present byte runs the full
    // pipeline unchanged.
    if !word
        .bytes()
        .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b']' | b'(' | b'+' | b'@' | b'!'))
    {
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

    // GNU glob (glob.c glob_filename / pathexp.c): globstar recursion is
    // a property of a WHOLE path segment — a `**` that is only part of a
    // segment (`**.txt`, `a**`, `sub**/x`) is exactly equivalent to `*`
    // and must take the ordinary segment engines below, whose matchers
    // already treat consecutive stars as one. Routing every `**`-
    // containing word into the textual globstar splitter passed those
    // words through literally or misread them as recursive
    // (rubash#1466).
    if word.contains("**") && globstar && word.split('/').any(|segment| segment == "**") {
        return globstar_expand(
            word,
            nullglob,
            failglob,
            nocaseglob,
            dotglob,
            globskipdots,
            env_vars,
            cwd,
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
            cwd,
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
        cwd,
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
    cwd: &Path,
) -> PathnameExpansion {
    // Assigning GLOBIGNORE also makes dotfiles matchable (bash variables.c
    // setup_glob_ignore clears noglob_dot_filenames).
    let dot_mode = if dotglob || globignore_assigned(env_vars) {
        DotMode::DotDot
    } else {
        DotMode::Period
    };
    let matches = glob_filename_expand(
        word,
        nocaseglob,
        extglob,
        dot_mode,
        globskipdots,
        env_vars,
        cwd,
    );
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
    cwd: &Path,
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
        let dirs = glob_filename_expand(
            d,
            nocaseglob,
            extglob,
            dot_mode,
            globskipdots,
            env_vars,
            cwd,
        );
        let mut out = Vec::new();
        for dname in dirs {
            if filename.is_empty() {
                // glob.c:1287-1289: scan even on a NULL filename, so `b*/`
                // yields only directories, with the `/` re-added by
                // glob_dir_to_array. rubash#1917 A3: a synthetic /proc
                // directory (a live pid dir) is a directory for this
                // check even though nothing physical backs it.
                if anchored_physical(&dname, env_vars, cwd).is_dir()
                    || crate::special_paths::synthetic_proc_dir_entries(&dname).is_some()
                {
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
                    cwd,
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
        cwd,
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
    cwd: &Path,
) -> Vec<String> {
    // G-02: every physical access anchors at the session cwd; `dir` keeps
    // its logical form for result construction only.
    let dir_physical = anchored_physical(dir, env_vars, cwd);
    // rubash#1917 A3: a synthetic /proc directory has no physical
    // backing; its entries come from the registry's one enumeration
    // source. Asked once here, the answer serves all three branches
    // below (directory probe, stat fast path, scan).
    let proc_entries = crate::special_paths::synthetic_proc_dir_entries(dir);
    // glob.c:669-691: an empty pattern just requires DIR to be a directory
    // and yields one empty name (glob_dir_to_array re-attaches the slash).
    if pattern.is_empty() {
        return if dir_physical.is_dir() || proc_entries.is_some() {
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
        // glob.c:703-712: the stat path joins DIR and the dequoted tail
        // (the physical join is the anchored equivalent).
        if dir_physical.join(&npat).exists() {
            return vec![npat];
        }
        // rubash#1917 A3: inside a synthetic /proc directory the joined
        // name may itself be synthetic (a content file, a link entry,
        // or a live pid directory) with nothing physical to stat.
        if proc_entries.is_some() {
            let joined = join_dir_to_name(dir, &npat);
            if crate::special_paths::synthetic_proc_content(&joined).is_some()
                || crate::special_paths::is_synthetic_link_path(&joined)
                || crate::special_paths::synthetic_proc_dir_entries(&joined).is_some()
            {
                return vec![npat];
            }
        }
        return Vec::new();
    }
    let entry_names: Vec<String> = match &proc_entries {
        Some(names) => names.clone(),
        None => match shell_directory_entries_in(&dir_physical) {
            Ok(entries) => entries.into_iter().map(|entry| entry.name).collect(),
            Err(_) => return Vec::new(),
        },
    };
    // rubash#1533: in a byte locale glob matching is byte-wise (glob.c
    // matches the compiled pattern against raw dirent bytes), so stage
    // both sides as byte units whenever the locale is not UTF-8; the
    // split is the identity on ASCII and on the carrier plane.
    let byte_locale = !crate::locale::is_utf8();
    let pattern_chars: Vec<char> = if byte_locale {
        super::conditional::pattern::split_text_chars_into_byte_units(pattern.chars().collect())
    } else {
        pattern.chars().collect()
    };
    let mut names = Vec::new();
    // With globskipdots off, `.` and `..` come back from readdir and the
    // dot-filter/matcher decide whether they match (glob10.sub).
    if !globskipdots && !skipname(&pattern_chars, ".", dot_mode, globskipdots) {
        names.push(".".to_string());
        names.push("..".to_string());
    }
    names.extend(entry_names);
    // GNU glob.c:641 glob_vector matches every dirent against the same
    // compiled byte-domain pattern; decode each candidate into one reused
    // scratch buffer instead of a fresh Vec<char> per file (no per-file
    // staging allocation, same class as the #281 Vec<char> removal).
    let mut scratch: Vec<char> = Vec::new();
    let mut matches: Vec<String> = names
        .into_iter()
        .filter(|name| {
            scratch.clear();
            if byte_locale {
                scratch.extend(
                    super::conditional::pattern::split_text_chars_into_byte_units(
                        name.chars().collect(),
                    ),
                );
            } else {
                scratch.extend(name.chars());
            }
            !skipname(&pattern_chars, name, dot_mode, globskipdots)
                && component_matches(&pattern_chars, &scratch, nocaseglob, extglob, dot_mode)
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

/// Split at the LAST `/` (glob.c:1136 `strrchr (pathname, '/')`): the split
/// is quoting-blind — a `\/` or CTLESC-quoted slash still separates the
/// directory part from the filename (`a\/b*` globs directory `a` for `b*`).
/// glob_dirscan additionally skips extglob `x(...)` spans when extended_glob
/// is set; that nuance is handled by the extglob paths, not here. The
/// directory part keeps its trailing slash.
fn split_last_unquoted_slash(pattern: &str) -> (&str, &str) {
    match pattern.rfind('/') {
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
    // rubash#1533: in a byte locale GNU's FNM_CASEFOLD folds per byte
    // (ASCII only); a Unicode fold would equate é with É, which are
    // different byte strings there.
    if !crate::locale::is_utf8() {
        let pattern_lower = pattern.to_ascii_lowercase();
        let word_lower = word.to_ascii_lowercase();
        return super::case_pattern_matches(&pattern_lower, &word_lower);
    }
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
    cwd: &Path,
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
            cwd,
        );
    }
    let prefix = parts[0];
    // rubash#1466: the single-** walk below treats the prefix as a
    // LITERAL directory (`sub/**/*.txt`). A prefix carrying a pattern
    // segment (`*/**/*.txt`) must be matched segment-by-segment first —
    // the multi walk's collector is segment-driven and handles a `**`
    // segment at any position, so delegate those words to it. Literal
    // prefixes (``, `/`, `./`, `sub/`) stay on the single path.
    if prefix.split('/').any(|segment| {
        !segment.is_empty() && unquoted_glob_pattern_p(segment, shopt_enabled(env_vars, "extglob"))
    }) {
        return expand_multi_globstar(
            word,
            nullglob,
            failglob,
            nocaseglob,
            dotglob,
            globskipdots,
            env_vars,
            cwd,
        );
    }
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
            cwd,
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
    // G-02: the walk's physical root anchors at the session cwd.
    let physical_base_dir = anchored_physical(&logical_base_dir, env_vars, cwd);

    let mut matches = Vec::new();
    // GNU `lib/**` includes the zero-depth match: the base directory itself
    // with its trailing slash preserved (`echo lib/**` starts with `lib/`).
    // A bare `**` (empty prefix) has no depth-0 operand to emit - the base is
    // the cwd and GNU prints no `./` entry.
    // The pure `prefix/**/` form (dirs_only with an empty suffix) has the
    // same zero-depth match — `d/**/` yields `d/` itself before `d/e/` —
    // but a patterned remainder (`**/foo*/`) does not: the base is only
    // emitted when it matches the pattern, which the walk handles
    // (rubash#1323).
    if (match_all || (dirs_only && suffix.is_empty()))
        && !prefix.is_empty()
        && logical_base_dir != "/"
    {
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
        // glob.c glob_filename: the symlink skip in the walk applies
        // only when the directory part consists solely of `**` segments
        // (empty prefix here). Any literal prefix (`./`, `sub/`, `/`)
        // makes symlinked directories ordinary for the remaining
        // pattern (rubash#1672).
        !prefix.is_empty(),
        env_vars,
    );
    // A `./` prefix is part of every result GNU prints (`./**/` yields
    // `./ ./d/ ./d/e/`): the walk normalizes the base `.` away when
    // joining, so re-attach the literal prefix to the walked entries.
    // The zero-depth entry pushed above already carries it (`./`).
    if prefix == "./" {
        for entry in &mut matches {
            if entry != "./" && !entry.starts_with("./") {
                *entry = format!("./{entry}");
            }
        }
    }

    let matches = apply_globignore(matches, env_vars);
    if matches.is_empty() {
        return unmatched_expansion(word, nullglob, failglob);
    }
    // GNU pathexp.c shell_glob_filename + sh_sortglob: the collected
    // globstar walk is re-sorted globally (byte order under C/POSIX via
    // strcoll_posixcmp, live collation otherwise).
    let mut matches = matches;
    globsort_matches(&mut matches, env_vars);
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
    cwd: &Path,
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
            cwd,
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
    // G-02: a relative multi-globstar walk roots at the session cwd.
    let physical_base = if absolute {
        shell_path_to_windows(base, env_vars)
    } else {
        cwd.to_path_buf()
    };
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
        absolute,
        env_vars,
    );
    crate::locale::sort_collated(&mut matches);
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
    // G-02: the walk carries the physical directory explicitly.
    let entries = match shell_directory_entries_in(physical) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
    crate::locale::sort_collated(&mut names);
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
    absolute: bool,
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
            absolute,
            env_vars,
        );
        let entries = match shell_directory_entries_in(physical) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        let mut names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
        crate::locale::sort_collated(&mut names);
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
            let is_symlink = std::fs::symlink_metadata(&child_physical)
                .map(|meta| meta.file_type().is_symlink())
                .unwrap_or(false);
            if is_symlink {
                // glob.c glob_filename (the all_starstar symlink special
                // case): what a symlinked directory enumerated by `**`
                // contributes depends on the rest of the pattern.
                if index + 1 == segments.len() {
                    // `**` is the final segment: the link itself is a
                    // match (globstar3.sub: `c` for `**`, `c/` for `**/`).
                    if !dirs_only || is_dir {
                        matches.push(if dirs_only {
                            format!("{child_logical}/")
                        } else {
                            child_logical.clone()
                        });
                    }
                } else if absolute || segments[..index].iter().any(|s| *s != "**") {
                    // The directory part is not all-`**` (a literal or
                    // absolute prefix precedes this `**`): the remaining
                    // segments are applied inside the linked directory.
                    // The recursive call consumes the next segment, so
                    // the `**` enumeration itself still never descends
                    // into the link (rubash#1672, multi-segment forms).
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
                        absolute,
                        env_vars,
                    );
                }
                // All-`**` prefix with segments remaining: the link
                // contributes nothing — it is skipped, never emitted as
                // a bare final match (rubash#1671: `**/deep/*` used to
                // append every cwd symlink name, dangling included).
                continue;
            }
            if index + 1 < segments.len() && !is_dir {
                continue;
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
                absolute,
                env_vars,
            );
        }
        return;
    }
    let entries = match shell_directory_entries_in(physical) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
    crate::locale::sort_collated(&mut names);
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
            absolute,
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
                        false,
                        env_vars,
                    );
                }
            }
        } else {
            let mut next: Vec<(String, PathBuf)> = Vec::new();
            for (logical, physical) in &work {
                let entries = match shell_directory_entries_in(physical) {
                    Ok(entries) => entries,
                    Err(_) => continue,
                };
                let mut names: Vec<String> =
                    entries.iter().map(|entry| entry.name.clone()).collect();
                crate::locale::sort_collated(&mut names);
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
        // GNU prints the failglob diagnostic with the word after quote
        // processing (the CTLESC-protected characters decoded, unquoted
        // backslash escapes removed): `arr=("dir"/zzz*)` under failglob
        // reports `no match: dir/zzz*`, not the raw quoting. dequote_
        // pathname performs that same reduction, so the Fail payload that
        // callers print never leaks CTLESC carriers.
        PathnameExpansion::Fail(dequote_pathname(word))
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
    follow_symlink_suffix: bool,
    env_vars: &std::collections::HashMap<String, String>,
) {
    // G-02: list the carried physical directory, not the logical form.
    let entries = match shell_directory_entries_in(physical_dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut names = synthetic_dot_names(suffix, globskipdots);
    names.extend(entries.iter().map(|entry| entry.name.clone()));
    // GNU glob.c sorts each directory's entries during the depth-first
    // walk; the single-** caller re-sorts the full list globally via
    // sh_sortglob, so `builtins.o` orders beside `builtins/...`.
    crate::locale::sort_collated(&mut names);
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
                follow_symlink_suffix,
                env_vars,
            );
        }
        // glob.c glob_filename (the all_starstar symlink special case):
        // the recursion skip above is GNU's rule only when the pattern's
        // directory part consists solely of `**` segments. With a literal
        // prefix (`./**/*.txt`, `sub/**/*.txt`, an absolute prefix —
        // follow_symlink_suffix), a symlinked directory enumerated by
        // the walk is treated as an ordinary directory for the REMAINING
        // pattern: the suffix is applied to its entries, one level, with
        // no recursion into it (so `sub/**/g.txt` still never reaches
        // `sub/linkd/deep/g.txt`). A suffix-free remainder (bare
        // `PREFIX/**`, pure `PREFIX/**/`) is the enumeration itself and
        // applies nothing (rubash#1672).
        if follow_symlink_suffix
            && !suffix.is_empty()
            && is_dir
            && is_symlink
            && name != "."
            && name != ".."
        {
            if let Ok(sub_entries) = shell_directory_entries_in(&physical_path) {
                for sub in &sub_entries {
                    let sub_name = &sub.name;
                    if sub_name == "." || sub_name == ".." {
                        continue;
                    }
                    if sub_name.starts_with('.') && !include_dotfiles {
                        continue;
                    }
                    let sub_is_dir = sub.path.is_dir();
                    let sub_matched = if dirs_only {
                        if nocaseglob {
                            sub_is_dir && case_pattern_matches_nocase(suffix, sub_name)
                        } else {
                            sub_is_dir && super::case_pattern_matches(suffix, sub_name)
                        }
                    } else if nocaseglob {
                        case_pattern_matches_nocase(suffix, sub_name)
                    } else {
                        super::case_pattern_matches(suffix, sub_name)
                    };
                    if sub_matched {
                        let sub_logical = join_path_segment(&logical_path, sub_name);
                        matches.push(if dirs_only {
                            format!("{}/", sub_logical)
                        } else {
                            sub_logical
                        });
                    }
                }
            }
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
    // pathexp.c:507-515 setup_glob_ignore: the GLOBIGNORE side effect and
    // the dotglob shopt share ONE cell (glob_dot_filenames). Assigning a
    // NON-NULL value parses >=1 ignore spec and turns the cell ON
    // (materialized at bind time into the shopt storage, see
    // apply_shell_assignment); a set-but-NULL assignment leaves the cell
    // untouched (neither branch fires) — it does NOT turn dotglob on by
    // itself (rubash#313: bash-completion's `local GLOBIGNORE=""` made `*`
    // match `.gitignore`). Only a NON-NULL value still enables matching
    // here, covering imports that bypassed the bind hook.
    env_vars
        .get("GLOBIGNORE")
        .is_some_and(|value| !value.is_empty())
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
            // pathexp.c glob_name_is_acceptable: `.`/`..` as the
            // TERMINAL component are never acceptable (always removed
            // when GLOBIGNORE is active in GNU; rubash has historically
            // kept them — the terminal-component computation is kept
            // here only for that check, the match itself is against
            // the full collected name, trailing `/` included).
            let trimmed = name.strip_suffix('/').unwrap_or(name);
            let base = trimmed.rsplit('/').next().unwrap_or(trimmed);
            base != "."
                && base != ".."
                && !patterns.iter().any(|pattern| {
                    // GNU pathexp.c:546 glob_name_is_acceptable():
                    // strmatch(pattern, name, FNM_PATHNAME|EXTFLAG|
                    // NOCASEGLOB) on the FULL pathname — there is no
                    // basename fallback, and patterns containing `/`
                    // are not special-cased. Matching slash patterns
                    // against the basename made `sub1/*` unmatchable
                    // (rubash#1321), and matching dir results without
                    // their trailing `/` hid the trailing-wildcard
                    // behavior that hides `sub1/` under `sub*`
                    // (rubash#1322).
                    ignore_pattern_matches(pattern, name, extglob, nocase)
                })
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
    // Plain patterns go through the strmatch FNM_PATHNAME port below —
    // the case-pattern engine lets `*` cross `/`, which would wrongly
    // ignore `d/x/inner` under a GLOBIGNORE of `*/inner`.
    // rubash#1533: byte locale — ASCII-only fold and byte-unit staging
    // (see glob_vector_expand / case_pattern_matches_nocase).
    let byte_locale = !crate::locale::is_utf8();
    let stage = |text: &str| -> Vec<char> {
        if byte_locale {
            super::conditional::pattern::split_text_chars_into_byte_units(text.chars().collect())
        } else {
            text.chars().collect()
        }
    };
    if nocase {
        let (pattern_lower, word_lower) = if byte_locale {
            (pattern.to_ascii_lowercase(), word.to_ascii_lowercase())
        } else {
            (pattern.to_lowercase(), word.to_lowercase())
        };
        return strmatch_pathname(&stage(&pattern_lower), &stage(&word_lower));
    }
    strmatch_pathname(&stage(pattern), &stage(word))
}

/// Port of bash lib/glob GMATCH (sm_loop.c) under FNM_PATHNAME, as
/// used by pathexp.c glob_name_is_acceptable() for GLOBIGNORE. The
/// rules that differ from the case-pattern engine:
/// - `?`, `[...]` and a `*` followed by more pattern never match `/`.
/// - A `*` that is the LAST pattern character matches the entire
///   remainder of the name, slashes included (sm_loop.c case '*':
///   `if (p == pe) return 0` before the collapse loop).
/// - A wildcard run ending the pattern (`x*?`, `x**`) likewise
///   succeeds once its `?`s have consumed their characters
///   (`if (p == pe && (c == '?' || c == '*')) return 0`).
/// - `**` is not special: a second `*` in a run is checked against
///   the current character like any other run member.
/// FNM_PERIOD/FNM_DOTDOT/FNM_LEADING_DIR are not in the
/// glob_name_is_acceptable flag set and are not modeled.
fn strmatch_pathname(pattern: &[char], name: &[char]) -> bool {
    let mut p = pattern;
    let mut s = name;
    while let Some(&c) = p.first() {
        p = &p[1..];
        let sc = s.first().copied();
        match c {
            '?' => match sc {
                None | Some('/') => return false,
                Some(_) => s = &s[1..],
            },
            '\\' => {
                // sm_loop case '\\': a trailing backslash matches a
                // single trailing backslash in the name; otherwise
                // the next pattern character is a literal.
                if p.is_empty() {
                    return s.len() == 1 && s[0] == '\\';
                }
                let lit = p[0];
                p = &p[1..];
                if sc != Some(lit) {
                    return false;
                }
                s = &s[1..];
            }
            '[' => match parse_bracket_match(p) {
                Some(class) => {
                    match sc {
                        None | Some('/') => return false,
                        Some(ch) => {
                            if !bracket_class_matches(&class.body, ch) {
                                return false;
                            }
                        }
                    }
                    p = &p[class.pattern_len..];
                    s = &s[1..];
                }
                None => {
                    // Unterminated: literal '['.
                    if sc != Some('[') {
                        return false;
                    }
                    s = &s[1..];
                }
            },
            '*' => {
                if p.is_empty() {
                    // `*` is the last pattern character.
                    return true;
                }
                // Collapse the following `?`/`*` run (sm_loop.c):
                // each run member first checks the current character
                // is not `/`; `?` additionally consumes one char.
                let mut rest = p;
                let mut cur = s;
                let mut next: char;
                loop {
                    let Some(&cc) = rest.first() else {
                        // Unreachable for plain patterns (the run
                        // exit below fires first); kept for parity
                        // with sm_loop's c == '\0' arm: the wildcards
                        // are the last element, so the match succeeds
                        // only if the remaining name has no `/`.
                        return !cur.contains(&'/');
                    };
                    rest = &rest[1..];
                    next = cc;
                    if cc != '?' && cc != '*' {
                        break;
                    }
                    if cur.first() == Some(&'/') {
                        return false;
                    }
                    if cc == '?' {
                        match cur.first() {
                            None | Some('/') => return false,
                            Some(_) => cur = &cur[1..],
                        }
                    }
                    if rest.is_empty() {
                        // Run reached the end of the pattern:
                        // sm_loop `if (p == pe && (c == '?' || c ==
                        // '*')) return 0`.
                        return true;
                    }
                }
                if next == '/' {
                    // sm_loop: consume the name up to the next `/`
                    // and continue after it.
                    let mut j = 0;
                    while j < cur.len() && cur[j] != '/' {
                        j += 1;
                    }
                    if j >= cur.len() {
                        return false;
                    }
                    return strmatch_pathname(rest, &cur[j + 1..]);
                }
                // General case: the `*` spans a slash-free prefix of
                // the current component; retry matching at `next`
                // from each position in the component.
                let comp_end = cur.iter().position(|&ch| ch == '/').unwrap_or(cur.len());
                let mut retry: Vec<char> = Vec::with_capacity(rest.len() + 1);
                retry.push(next);
                retry.extend_from_slice(rest);
                for k in 0..=comp_end {
                    if strmatch_pathname(&retry, &cur[k..]) {
                        return true;
                    }
                }
                return false;
            }
            literal => {
                if sc != Some(literal) {
                    return false;
                }
                s = &s[1..];
            }
        }
    }
    s.is_empty()
}

struct BracketMatch {
    /// Pattern length consumed by the whole `[...]` (relative to the
    /// pattern slice starting just after the `[`).
    pattern_len: usize,
    /// The class body including any leading `!`/`^` negation.
    body: Vec<char>,
}

/// Parse a `[...]` bracket expression; `pattern` starts just after
/// the `[`. Returns the consumed length (including the closing `]`,
/// relative to `pattern`) and the class body. `None` when
/// unterminated.
fn parse_bracket_match(pattern: &[char]) -> Option<BracketMatch> {
    let mut i = 0;
    if i < pattern.len() && (pattern[i] == '!' || pattern[i] == '^') {
        i += 1;
    }
    // A `]` as the first class character is a literal.
    if i < pattern.len() && pattern[i] == ']' {
        i += 1;
    }
    while i < pattern.len() {
        if pattern[i] == '\\' && i + 1 < pattern.len() {
            i += 2;
            continue;
        }
        if pattern[i] == ']' {
            return Some(BracketMatch {
                pattern_len: i + 1,
                body: pattern[..i].to_vec(),
            });
        }
        i += 1;
    }
    None
}

/// Test one character against a bracket class body (the text between
/// `[` and `]`, including any leading `!`/`^` negation and any POSIX
/// `[:class:]` items).
fn bracket_class_matches(class: &[char], ch: char) -> bool {
    let (negate, body) = match class.first() {
        Some('!') | Some('^') => (true, &class[1..]),
        _ => (false, class),
    };
    let mut i = 0;
    let mut matched = false;
    while i < body.len() {
        // POSIX character class [:alpha:] etc.
        if body[i] == '[' && i + 1 < body.len() && body[i + 1] == ':' {
            if let Some(end) = (i + 2..body.len().saturating_sub(1))
                .find(|&j| body[j] == ':' && body[j + 1] == ']')
            {
                let name: String = body[i + 2..end].iter().collect();
                if posix_class_matches(&name, ch) {
                    matched = true;
                }
                i = end + 2;
                continue;
            }
        }
        let lo = if body[i] == '\\' && i + 1 < body.len() {
            i += 1;
            body[i]
        } else {
            body[i]
        };
        if i + 2 < body.len() && body[i + 1] == '-' && body[i + 2] != ']' {
            let hi = if body[i + 2] == '\\' && i + 3 < body.len() {
                body[i + 3]
            } else {
                body[i + 2]
            };
            if lo <= ch && ch <= hi {
                matched = true;
            }
            i += if body[i + 2] == '\\' && i + 3 < body.len() {
                4
            } else {
                3
            };
            continue;
        }
        if lo == ch {
            matched = true;
        }
        i += 1;
    }
    matched != negate
}

fn posix_class_matches(name: &str, ch: char) -> bool {
    match name {
        "alpha" => ch.is_alphabetic(),
        "digit" => ch.is_ascii_digit(),
        "alnum" => ch.is_alphanumeric(),
        "space" => ch.is_whitespace(),
        "upper" => ch.is_uppercase(),
        "lower" => ch.is_lowercase(),
        "xdigit" => ch.is_ascii_hexdigit(),
        "punct" => ch.is_ascii_punctuation(),
        "print" => !ch.is_control(),
        "graph" => !ch.is_control() && ch != ' ',
        "cntrl" => ch.is_control(),
        "blank" => ch == ' ' || ch == '\t',
        _ => false,
    }
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
const CTLESC: char = crate::executor::markers::CTLESC;

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
/// it (glob.c:271-272,287-288): a raw `.` or an escaped `\.`. GNU sees the
/// escaped form as `\` + `.` because shell_glob_filename re-quotes the word
/// with quote_string_for_globbing (pathexp.c:447-453), translating the
/// lexer's CTLESC back to a backslash — so the CTLESC-quoted dot counts too.
fn pattern_leading_dot(pattern: &[char]) -> bool {
    match pattern.first() {
        Some('.') => true,
        Some(&c) if c == '\\' || c == CTLESC => pattern.get(1) == Some(&'.'),
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
    // Match the sub-slice in place: gmatch only reads name[0..len], so
    // passing &name[s..srest] at offset 0 is identical to matching a copied
    // prefix buffer (sm_loop.c:843/881) with no per-split allocation.
    gmatch(alt, 0, &name[s..srest], 0, extglob, guard, nocase)
}

fn chars_match(pattern: char, candidate: char, nocase: bool) -> bool {
    // Byte-chars (U+E100+byte, a carried raw byte) compare as their byte
    // value (rubash#729 — glob.c matches raw bytes; sm_loop.c GLOBCHAR is
    // compared byte-wise). PUA marker codepoints exceed 0xFF and can never
    // equal a carried byte, so #735 E400-escaped user data stays data.
    let base = crate::executor::markers::BYTE_CHAR_BASE;
    let pattern_byte = (pattern as u32)
        .checked_sub(base)
        .filter(|offset| *offset < 0x100);
    let candidate_byte = (candidate as u32)
        .checked_sub(base)
        .filter(|offset| *offset < 0x100);
    if let (Some(p), Some(c)) = (pattern_byte, candidate_byte) {
        return p == c;
    }
    if let Some(p) = pattern_byte {
        return p as u32 == candidate as u32;
    }
    if let Some(c) = candidate_byte {
        // Only an ASCII literal can name the same byte (GNU compares
        // bytes; a multibyte literal like U+00E9 is bytes C3 A9 in GNU and
        // never equals a lone carried 0xE9).
        return pattern.is_ascii() && pattern as u32 == c as u32;
    }
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
            crate::locale::sort_collated_reverse(matches);
        } else {
            crate::locale::sort_collated(matches);
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
    // One locale resolution for the whole tie-break sort (rubash#681
    // slice b): byte order under the C collation, host collation
    // otherwise — identical results either way (see collation_is_c).
    let c_locale = crate::locale::collation_is_c();
    let name_cmp = |a: &str, b: &str| {
        if c_locale {
            if reverse {
                b.cmp(a)
            } else {
                a.cmp(b)
            }
        } else if reverse {
            crate::locale::strcoll_posixcmp(b, a)
        } else {
            crate::locale::strcoll_posixcmp(a, b)
        }
    };
    let indices: Vec<usize> = (0..matches.len()).collect();
    let mut sorted = indices;
    sorted.sort_by(|&ia, &ib| {
        let (na, nb) = (&matches[ia], &matches[ib]);
        let x = match base {
            SORT_SIZE | SORT_MTIME | SORT_ATIME | SORT_CTIME => {
                let key = |s: &(i64, i64, i64, i64)| match base {
                    SORT_SIZE => s.0,
                    SORT_MTIME => s.1,
                    SORT_ATIME => s.2,
                    _ => s.3,
                };
                let o = key(&stats[ia]).cmp(&key(&stats[ib]));
                if reverse {
                    o.reverse()
                } else {
                    o
                }
            }
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
    use super::*;

    /// Contract for the #681 per-file staging removal: scratch-buffer reuse
    /// plus the zero-copy extglob alt subslice must preserve dotfile,
    /// casefold and extglob matching (whitelist rubash#117).
    #[test]
    fn glob_vector_expand_scratch_reuse_matches() {
        let root = std::env::temp_dir().join("rubash-glob-scratch-test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for name in ["a.txt", "b.txt", "c.log", ".hidden.txt", "AB.TXT"] {
            std::fs::write(root.join(name), "x").unwrap();
        }
        let dir = root.to_string_lossy().to_string();
        let env = std::collections::HashMap::new();
        // `*` hides dotfiles under Period mode (dotglob off).
        let mut got = glob_vector_expand(
            "*.txt",
            &dir,
            false,
            false,
            DotMode::Period,
            true,
            &env,
            Path::new("/"),
        );
        got.sort();
        assert_eq!(got, vec!["a.txt".to_string(), "b.txt".to_string()]);
        // nocaseglob folds AB.TXT in.
        let mut got = glob_vector_expand(
            "*.txt",
            &dir,
            true,
            false,
            DotMode::Period,
            true,
            &env,
            Path::new("/"),
        );
        got.sort();
        assert_eq!(
            got,
            vec![
                "AB.TXT".to_string(),
                "a.txt".to_string(),
                "b.txt".to_string()
            ]
        );
        // Extglob exercises the zero-copy alt subslice path.
        let mut got = glob_vector_expand(
            "+(a|b).txt",
            &dir,
            false,
            true,
            DotMode::Period,
            true,
            &env,
            Path::new("/"),
        );
        got.sort();
        assert_eq!(got, vec!["a.txt".to_string(), "b.txt".to_string()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The rubash#148 re-encoding: quoted spans collapse to CTLESC-protected
    /// characters (GNU subst.c:4773 quote_string), unquoted text (including
    /// backslash escapes) stays verbatim, quote characters are dropped.
    #[test]
    fn compound_element_glob_pattern_reencodes_quoting() {
        let c = crate::executor::markers::CTLESC;
        // Fast path: no quote operators.
        assert_eq!(compound_element_glob_pattern("dir/*.txt"), "dir/*.txt");
        // Quoted prefix: every quoted char protected, glob suffix active.
        assert_eq!(
            compound_element_glob_pattern("\"dir\"/*.txt"),
            format!("{c}d{c}i{c}r/*.txt")
        );
        // Single quotes protect their span the same way.
        assert_eq!(
            compound_element_glob_pattern("'dir'/*.txt"),
            format!("{c}d{c}i{c}r/*.txt")
        );
        // A quoted metacharacter is inert; an unquoted one stays active.
        assert_eq!(
            compound_element_glob_pattern("\"p\"/\"*\""),
            format!("{c}p/{c}*")
        );
        // Inside double quotes only `"\$` and backquote/newline consume the
        // backslash (subst.c skip_double_quoted); `\*` keeps both chars.
        assert_eq!(
            compound_element_glob_pattern("\"a\\*b\"/x?y"),
            format!("{c}a{c}\\{c}*{c}b/x?y")
        );
        // An unquoted backslash keeps both characters for the engine's own
        // escape handling.
        assert_eq!(compound_element_glob_pattern("dir/\\*.txt"), "dir/\\*.txt");
    }

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

        let PathnameExpansion::Matches(matches) =
            pathname_expand_word("/etc/*", &env_vars, Path::new("/"))
        else {
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
