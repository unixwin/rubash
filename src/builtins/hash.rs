//! hash module.
//!
//! GNU Bash source ownership:
// - builtins/hash.def

use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};
use crate::shell::var_table::VarTable;
use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;
const HASH_TABLE: &str = "__RUBASH_HASH_TABLE";

pub fn execute(args: &[String], env_vars: &mut VarTable) -> io::Result<i32> {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    execute_with_io(args, env_vars, &mut stdout, &mut stderr)
}

pub(crate) fn execute_with_io<W, E>(
    args: &[String],
    env_vars: &mut VarTable,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    execute_with_io_and_functions(
        args,
        env_vars,
        &std::collections::HashSet::new(),
        stdout,
        stderr,
    )
}

/// Same as [`execute_with_io`], with the names GNU hash.def never
/// hashes — shell builtins and shell functions — supplied by the
/// executor: the bare builtin cannot see either set from the variable
/// table alone (rubash#1215).
pub(crate) fn execute_with_io_and_functions<W, E>(
    args: &[String],
    env_vars: &mut VarTable,
    unhashable_names: &std::collections::HashSet<String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    // GNU builtins/hash.def:86-90: when hashing is disabled (set +h), the
    // entire `hash` builtin refuses with "hash: hashing disabled" and
    // returns failure, before any option parsing.
    if !crate::builtins::set::shell_option_enabled(env_vars, "hashall") {
        writeln!(stderr, "{}hash: hashing disabled", script_prefix(env_vars))?;
        return Ok(EXECUTION_FAILURE);
    }

    let mut delete = false;
    let mut reset = false;
    let mut pathname: Option<&str> = None;
    let mut translate = false;
    let mut reusable = false;
    let mut names: Vec<&str> = Vec::new();
    let mut index = 0;
    // GNU builtins/bashgetopt.c:76 (NOTOPT) and :84-89: option parsing ends
    // at the first non-option word or at a bare `--` (consumed, not an
    // operand); everything from there on is a name. Mirrors
    // `internal_getopt (list, "dlp:rt")` in builtins/hash.def:95.
    // (rubash#1375 `hash -- ls`, #1389 trailing `--`, #1382 `-p` name tail.)
    let mut parsing_options = true;

    while let Some(arg) = args.get(index) {
        if parsing_options && arg == "--" {
            parsing_options = false;
            index += 1;
            continue;
        }
        // GNU hash.def's getopt table maps --help to CASE_HELPOPT: long
        // help to stdout, EX_USAGE status (rubash#1229). Only while still
        // parsing options — after `--` it is a name to look up.
        if parsing_options && arg == "--help" {
            crate::builtins::help::print_builtin_help("hash", stdout)?;
            return Ok(EX_USAGE);
        }
        if parsing_options && arg.starts_with('-') && *arg != "-" {
            let mut chars = arg[1..].char_indices();
            while let Some((offset, option)) = chars.next() {
                match option {
                    'r' => {
                        // GNU builtins/hash.def:107-109: `-r` only SETS
                        // expunge_hash_table — option parsing continues and
                        // any operands are processed AFTER the flush
                        // (hash.def:150-151 phash_flush, then the name loop
                        // rehashes each name). Returning here made
                        // `hash -r name` silently delete just that entry
                        // with rc 0; GNU clears the table, then rehashes
                        // the name and reports `hash: name: not found`
                        // (rc 1) when it does not resolve (rubash#1489).
                        // The flush itself is applied below, after the
                        // needarg/usage checks, so `hash -r ls` still
                        // hashes `ls` and `hash -r -d` still reports the
                        // -d needarg error (rubash#1382).
                        reset = true;
                    }
                    'd' => delete = true,
                    't' => translate = true,
                    'l' => {
                        reusable = true;
                    }
                    'p' => {
                        // GNU: `-p` takes the rest of the word or the next
                        // word as its pathname argument (bashgetopt.c).
                        let value_start = 1 + offset + option.len_utf8();
                        if value_start < arg.len() {
                            pathname = Some(&arg[value_start..]);
                        } else {
                            let Some(value) = args.get(index + 1).map(String::as_str) else {
                                writeln!(
                                    stderr,
                                    "{}hash: -p: option requires an argument",
                                    script_prefix(env_vars)
                                )?;
                                writeln!(
                                    stderr,
                                    "hash: usage: hash [-lr] [-p pathname] [-dt] [name ...]"
                                )?;
                                return Ok(EX_USAGE);
                            };
                            index += 1;
                            pathname = Some(value);
                        }
                        break;
                    }
                    other => {
                        writeln!(
                            stderr,
                            "{}hash: -{other}: invalid option",
                            script_prefix(env_vars)
                        )?;
                        writeln!(
                            stderr,
                            "hash: usage: hash [-lr] [-p pathname] [-dt] [name ...]"
                        )?;
                        return Ok(EX_USAGE);
                    }
                }
            }
        } else {
            parsing_options = false;
            names.push(arg.as_str());
        }
        index += 1;
    }

    // GNU builtins/hash.def:124-128: hash -d/-t with no arguments reports
    // "hash: -d: option requires an argument" via sh_needarg.
    if names.is_empty() && (delete || translate) {
        let opt = if delete { "-d" } else { "-t" };
        writeln!(
            stderr,
            "{}hash: {opt}: option requires an argument",
            script_prefix(env_vars)
        )?;
        return Ok(EXECUTION_FAILURE);
    }

    // GNU builtins/hash.def:131-135: `hash -p pathname` with no name is a
    // usage error (EX_USAGE), not a table print (rubash#1360) — and it
    // is reported BEFORE the -r expunge below (hash.def's checks precede
    // phash_flush), so the table survives a malformed `hash -r -p path`;
    // it also precedes the restricted-shell pathname checks, matching
    // hash_builtin's order (rubash#1382).
    if pathname.is_some() && names.is_empty() {
        writeln!(
            stderr,
            "hash: usage: hash [-lr] [-p pathname] [-dt] [name ...]"
        )?;
        return Ok(EX_USAGE);
    }

    // GNU builtins/hash.def:150-151: with expunge set, the whole table is
    // flushed before the name loop (and before -t target listing); with
    // no names at all the builtin then ends silently at rc 0 — the table
    // print above-all is skipped whenever expunge was requested
    // (rubash#1489; ordering cross-checked for rubash#1382).
    if reset {
        env_vars.remove(HASH_TABLE);
        // The internal lookup cache holds the same remembered locations,
        // so it must be dropped too or misses/hits stay stale.
        crate::executor::path::clear_command_lookup_cache(env_vars);
        if names.is_empty() {
            return Ok(EXECUTION_SUCCESS);
        }
    }

    let mut table = hash_table(env_vars);
    // GNU builtins/hash.def hash_builtin: list_targets is handled
    // before the pathname/delete/add name loop (and before the
    // restricted-shell pathname checks), so `hash -t -p /bin/ls n`
    // looks `n` up instead of inserting it, and `hash -d -t ls`
    // prints the target instead of deleting it (rubash#1382).
    if translate {
        let mut status = EXECUTION_SUCCESS;
        // GNU builtins/hash.def list_hashed_filename_targets: with several
        // names each line is prefixed with `name\t`.
        let multiple = names.len() > 1;
        for name in names {
            // GNU phash_search -> hash_search bumps times_found on a hit
            // (hashlib.c:254).
            if let Some((path, hits, seq)) = table.get_mut(name) {
                *hits += 1;
                let path = path.clone();
                let _ = (hits, seq);
                // GNU hashcmd.c phash_search: a stored relative path prints
                // `./`-prefixed when that form is executable, and reports
                // not-found when it resolves to `.` itself unexecutably
                // (rubash#1361).
                match phash_search_target(&path) {
                    Some(target) => {
                        if reusable {
                            writeln!(stdout, "builtin hash -p {target} {name}")?;
                        } else {
                            if multiple {
                                write!(stdout, "{name}\t")?;
                            }
                            writeln!(stdout, "{target}")?;
                        }
                    }
                    None => {
                        writeln!(stderr, "{}hash: {name}: not found", script_prefix(env_vars))?;
                        status = EXECUTION_FAILURE;
                    }
                }
            } else {
                writeln!(stderr, "{}hash: {name}: not found", script_prefix(env_vars))?;
                status = EXECUTION_FAILURE;
            }
        }
        store_hash_table(env_vars, &table);
        return Ok(status);
    }

    if let Some(pathname) = pathname {
        // GNU builtins/hash.def:156-176: in a restricted shell `hash -p`
        // refuses an absolute pathname (sh_restricted -> "hash: <path>:
        // restricted") and requires a relative one to resolve through $PATH
        // (sh_notfound -> "hash: <name>: not found"); both fail the builtin.
        if crate::builtins::set::shell_option_enabled(env_vars, "restricted") {
            if pathname.contains('/') || pathname.contains('\\') {
                writeln!(
                    stderr,
                    "{}hash: {pathname}: restricted",
                    script_prefix(env_vars)
                )?;
                return Ok(EXECUTION_FAILURE);
            }
            if crate::executor::path::find_user_command(pathname, env_vars).is_none() {
                writeln!(
                    stderr,
                    "{}hash: {pathname}: not found",
                    script_prefix(env_vars)
                )?;
                return Ok(EXECUTION_FAILURE);
            }
        }
        // (The no-name usage check for `-p` now lives above, before the
        // expunge flush, in hash_builtin's order — rubash#1382. The
        // directory check itself lives in the insert loop below:
        // GNU tests file_isdir(pathname) per name, after the
        // absolute_program skip — rubash#1360.)
        // GNU builtins/hash.def:190-196: with `-p`, every name is inserted
        // via phash_insert; a name containing '/' is silently skipped
        // (absolute_program → continue), never stored (rubash#1360 #1362).
        // GNU hash.def `hash -p PATH NAME`: phash_insert(name, pathname)
        // makes subsequent lookups of `name` return `pathname` without a
        // PATH scan. Mirror that in the in-memory lookup cache so
        // find_user_command(name) returns the same path external_inner
        // would execute. Convert through shell_path_to_windows so the
        // cached PathBuf matches the Windows-native form produced by
        // find_user_command_uncached.
        // GNU builtins/hash.def hash_builtin name loop: with a pathname,
        // file_isdir(pathname) is tested per name (after the slash skip):
        // a directory pathname reports "<pathname>: Is a directory",
        // sets failure and stores nothing for that name — for ANY
        // directory, not just "/" (rubash#1360).
        let mut status = EXECUTION_SUCCESS;
        for name in &names {
            if name.contains('/') {
                continue;
            }
            if std::path::Path::new(pathname).is_dir() {
                writeln!(
                    stderr,
                    "{}hash: {pathname}: Is a directory",
                    script_prefix(env_vars)
                )?;
                status = EXECUTION_FAILURE;
                continue;
            }
            table.insert(
                (*name).to_string(),
                (pathname.to_string(), 0, next_hash_seq(&table)),
            );
            let cached_path = crate::executor::path::shell_path_to_windows(pathname, env_vars);
            crate::executor::path::set_command_lookup_cache(name, Some(cached_path), env_vars);
        }
        store_hash_table(env_vars, &table);
        return Ok(status);
    }

    if delete {
        let mut status = EXECUTION_SUCCESS;
        for name in names {
            // GNU builtins/hash.def hash_builtin name loop: the
            // absolute_program(w) → continue check is the loop's first
            // statement, ahead of the delete branch — a slash name is
            // silently skipped by `hash -d` too, never reported and never
            // removed (rubash#1362 delete facet).
            if name.contains('/') {
                continue;
            }
            if table.remove(name).is_none() {
                writeln!(stderr, "{}hash: {name}: not found", script_prefix(env_vars))?;
                status = EXECUTION_FAILURE;
            } else {
                // GNU hash.def `hash -d NAME`: phash_remove(w) drops the entry
                // from the hash table. The internal lookup cache holds the
                // same information and must be invalidated in lockstep or the
                // next `find_user_command(name)` returns the stale path.
                crate::executor::path::remove_command_lookup_cache(name, env_vars);
            }
        }
        store_hash_table(env_vars, &table);
        return Ok(status);
    }

    // GNU builtins/hash.def:138-146: the table print happens only when no
    // names were given (`list == 0`) and the table was not just expunged;
    // `-l` with names goes through the name loop above (rubash#1368).
    if names.is_empty() {
        if !table.is_empty() {
            if !reusable {
                writeln!(stdout, "hits\tcommand")?;
                // GNU hash.def print_hashed_commands -> hash_walk iterates
                // bucket_array in index order (hashlib.c:397-415), each
                // bucket's chain newest-first (hash_search inserts at the
                // head, hashlib.c:267-269). The bucket is
                // hash_string(name) & (FILENAME_HASH_BUCKETS-1) = &255
                // (hashcmd.h:24), and `hits` is times_found — a real
                // counter that stays 0 for -p/BASH_CMDS inserts.
                let mut entries: Vec<_> = table.into_iter().collect();
                entries.sort_by(|left, right| {
                    gnu_hash_bucket(&left.0)
                        .cmp(&gnu_hash_bucket(&right.0))
                        .then(right.1 .2.cmp(&left.1 .2))
                });
                for (name, (path, hits, _)) in entries {
                    writeln!(stdout, "{hits:4}\t{path}")?;
                }
                return Ok(EXECUTION_SUCCESS);
            }
            // GNU print_hashed_commands walks the same bucket order for
            // the portable (-l) form (hash_walk over bucket_array); the
            // raw HashMap order is process-random and scrambles the
            // listing (rubash#1382, whose repro diffs `hash -l` output
            // for the two entries `n` and `-l`).
            let mut entries: Vec<_> = table.into_iter().collect();
            entries.sort_by(|left, right| {
                gnu_hash_bucket(&left.0)
                    .cmp(&gnu_hash_bucket(&right.0))
                    .then(right.1 .2.cmp(&left.1 .2))
            });
            for (name, (path, _, _)) in entries {
                writeln!(stdout, "builtin hash -p {path} {name}")?;
            }
            return Ok(EXECUTION_SUCCESS);
        }
        // GNU builtins/hash.def:141-144: the empty-table message prints
        // only for the plain form — with -l (list_portably) it is
        // suppressed once shell_compatibility_level > 50 (bash 5.3 runs
        // at 51), so `hash -l` on an empty table is silent (rubash#1052);
        // it is likewise suppressed in posix mode, where an empty table
        // prints nothing (bash posix-mode notes; rubash#1054).
        if !reusable && !crate::builtins::set::shell_option_enabled(env_vars, "posix") {
            writeln!(stdout, "hash: hash table empty")?;
        }
        return Ok(EXECUTION_SUCCESS);
    }

    // GNU hash.def bare-name form: `hash NAME...` re-resolves each NAME
    // against PATH and re-inserts the result. The flow is phash_remove(name)
    // + find_user_command(name) + phash_insert(name, path); a name that does
    // not resolve reports "hash: NAME: not found" and sets failure.
    if !names.is_empty() {
        let mut status = EXECUTION_SUCCESS;
        for name in names {
            // GNU builtins/hash.def:192-193: absolute_program(w) → continue.
            // A name containing '/' is never hashed and never reported
            // (rubash#1362); general.c:864 defines it as containing a slash.
            if name.contains('/') {
                continue;
            }
            // GNU builtins/hash.def hash_builtin: a name that resolves to
            // a shell builtin or a shell function is skipped silently —
            // builtins and functions are never hashed, so `hash echo`
            // succeeds with no entry created and `hash -t echo` afterwards
            // reports not found (rubash#1215). Reserved words are NOT
            // skipped: GNU reports `hash: if: not found` for them, which
            // the PATH search below produces naturally.
            if unhashable_names.contains(name) {
                continue;
            }
            // Drop any stale remembered location so the PATH scan is
            // authoritative.
            crate::executor::path::remove_command_lookup_cache(name, env_vars);
            match crate::executor::path::find_user_command(name, env_vars) {
                Some(path) => {
                    let path_string = path.to_string_lossy().to_string();
                    table.insert(
                        name.to_string(),
                        (path_string.clone(), 0, next_hash_seq(&table)),
                    );
                    // find_user_command already inserted the result into the
                    // internal cache, so no extra set_command_lookup_cache is
                    // needed here.
                    let _ = path_string;
                }
                None => {
                    table.remove(name);
                    writeln!(stderr, "{}hash: {name}: not found", script_prefix(env_vars))?;
                    status = EXECUTION_FAILURE;
                }
            }
        }
        store_hash_table(env_vars, &table);
        return Ok(status);
    }

    Ok(EXECUTION_SUCCESS)
}

pub(crate) fn set_hashed_path(env_vars: &mut VarTable, name: &str, path: &str) {
    let mut table = hash_table(env_vars);
    table.insert(
        name.to_string(),
        (path.to_string(), 0, next_hash_seq(&table)),
    );
    store_hash_table(env_vars, &table);
    // BASH_CMDS[name]=value mirrors `hash -p value name`; keep the internal
    // lookup cache in sync so find_user_command(name) returns value.
    let cached_path = crate::executor::path::shell_path_to_windows(path, env_vars);
    crate::executor::path::set_command_lookup_cache(name, Some(cached_path), env_vars);
}

pub(crate) fn remove_hashed_path(env_vars: &mut VarTable, name: &str) {
    let mut table = hash_table(env_vars);
    table.remove(name);
    store_hash_table(env_vars, &table);
    // unset 'BASH_CMDS[name]' mirrors `hash -d name`; drop the internal cache
    // entry so the next lookup re-scans PATH.
    crate::executor::path::remove_command_lookup_cache(name, env_vars);
}

pub(crate) fn hashed_path(env_vars: &HashMap<String, String>, name: &str) -> Option<String> {
    hash_table(env_vars).remove(name).map(|(path, _, _)| path)
}

pub(crate) fn hashed_entries(env_vars: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut entries: Vec<_> = hash_table(env_vars)
        .into_iter()
        .map(|(name, (path, _, _))| (name, path))
        .collect();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

/// GNU hashcmd.c:123-191 phash_search, as used by `hash -t`
/// (list_hashed_filename_targets): resolve the stored path to its display
/// target. A stored relative path (RELPATH, general.h:267) prints
/// `./`-prefixed when that dotted form is executable; when it is not
/// executable and its directory is the current directory, the entry counts
/// as not found (None). Absolute paths print as stored. (rubash#1361)
fn phash_search_target(path: &str) -> Option<String> {
    if !path.starts_with('/') {
        let dotted: std::borrow::Cow<str> = if path.starts_with("./") {
            std::borrow::Cow::Borrowed(path)
        } else {
            std::borrow::Cow::Owned(format!("./{path}"))
        };
        if crate::executor::path::is_executable_file(std::path::Path::new(dotted.as_ref())) {
            return Some(dotted.into_owned());
        }
        // hashcmd.c:183-196: only when the dotted form is not executable
        // and the stored path starts with '.'.
        if path.starts_with('.') {
            if let Some((dir, _)) = path.rsplit_once('/') {
                let dir = if dir.is_empty() { "." } else { dir };
                if same_file(".", dir) {
                    return None;
                }
            }
        }
        return Some(path.to_string());
    }
    Some(path.to_string())
}

/// device+inode identity (hashcmd.c same_file); canonicalized-path
/// comparison is close enough for the directory-identity test here.
fn same_file(first: &str, second: &str) -> bool {
    match (std::fs::canonicalize(first), std::fs::canonicalize(second)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// GNU hashlib.c:208-222 hash_string — the FNV-1 variant
/// (`i += (i<<1)+(i<<4)+(i<<7)+(i<<8)+(i<<24); i ^= c`) — reduced to the
/// hashed_filenames bucket index by `& (FILENAME_HASH_BUCKETS-1)` =
/// `& 255` (hashcmd.h:24 + hashlib.c:51 HASH_BUCKET).
fn gnu_hash_bucket(name: &str) -> u32 {
    let mut hash: u32 = 2166136261;
    for byte in name.bytes() {
        hash = hash.wrapping_add(
            (hash << 1)
                .wrapping_add(hash << 4)
                .wrapping_add(hash << 7)
                .wrapping_add(hash << 8)
                .wrapping_add(hash << 24),
        );
        hash ^= u32::from(byte);
    }
    hash & 255
}

/// The next bucket-chain position: hash_search inserts at the bucket head
/// (hashlib.c:267-269), so a later insert prints before earlier entries in
/// the same bucket — seq tracks insertion order for that LIFO sort.
fn next_hash_seq(table: &HashMap<String, (String, u32, u64)>) -> u64 {
    table.values().map(|(_, _, seq)| *seq).max().unwrap_or(0) + 1
}

fn hash_table(env_vars: &HashMap<String, String>) -> HashMap<String, (String, u32, u64)> {
    env_vars
        .get(HASH_TABLE)
        .map(|value| {
            value
                .split(DATA_DOLLAR)
                .filter_map(|entry| {
                    let (name, rest) = entry.split_once('=')?;
                    let (path, tail) = rest
                        .split_once(crate::executor::markers::HASH_ENV_FIELD_SEP)
                        .unwrap_or((rest, "0"));
                    let (hits, seq) = tail
                        .split_once(crate::executor::markers::HASH_ENV_FIELD_SEP)
                        .unwrap_or((tail, "0"));
                    Some((
                        name.to_string(),
                        (
                            path.to_string(),
                            hits.parse().unwrap_or(0),
                            seq.parse().unwrap_or(0),
                        ),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn store_hash_table(env_vars: &mut VarTable, table: &HashMap<String, (String, u32, u64)>) {
    env_vars.insert(
        HASH_TABLE.to_string(),
        table
            .iter()
            .map(|(name, (path, hits, seq))| {
                format!(
                    "{name}={path}{s}{hits}{s}{seq}",
                    s = crate::executor::markers::HASH_ENV_FIELD_SEP
                )
            })
            .collect::<Vec<_>>()
            .join(DATA_DOLLAR_STR),
    );
}

/// Empty the command hash table in this shell state (rubash#1551).
///
/// GNU variables.c:5819 registers an assignment function for PATH —
/// `{ "PATH", sv_path }` — and sv_path (variables.c:5919) is exactly
/// `phash_flush()` (hashcmd.c:57): ANY operation that binds a value to
/// PATH or unbinds it empties the whole remembered-location table,
/// even a same-value bind. Attribute-only operations (`export PATH`,
/// `readonly PATH`, `declare -p PATH`) never bind and do not flush.
/// Every bind funnel calls this next to its store, mirroring the
/// #1531 sv_locale funnel discipline: the executor assignment funnel
/// (plain, `+=`, temp-prefix, read/for/select/getopts binds), the
/// declare/typeset/local operand path, export/readonly value binds,
/// `unset`, and the temp-assignment / function-local restore paths
/// (a restore is a re-bind of the saved value in GNU's model).
pub(crate) fn flush_command_hash_table(env_vars: &mut VarTable) {
    env_vars.remove(HASH_TABLE);
    // The process-global lookup cache mirrors this table in its
    // `hashed` map; both stores must die together or the next lookup
    // would serve the stale location the flush just forgot.
    crate::executor::path::flush_command_lookup_cache_hashed(env_vars);
}

/// GNU hashlib.c:254: a hash_search hit increments times_found — every
/// phash_search (`type`, `command -v`, exec resolution, `hash -t`) counts.
pub(crate) fn bump_hashed_path_hit(env_vars: &mut VarTable, name: &str) {
    let mut table = hash_table(env_vars);
    if let Some((_, hits, _)) = table.get_mut(name) {
        *hits += 1;
        store_hash_table(env_vars, &table);
    }
}

/// GNU execute_cmd.c:5808 + findcmd.c:416-426: a `command -p` lookup
/// that finds a pathname inserts it into the hash table (CMDSRCH_HASH
/// is passed alongside CMDSRCH_STDPATH), overwriting any poisoned or
/// stale entry with times_found=1 — the -p walk never consulted the
/// table, so the insert is unconditional and the next plain lookup
/// serves the standard-path result (rubash#1648). The internal lookup
/// cache mirror is synced alongside, or the next plain lookup would
/// still serve the stale hashed entry it consults first.
pub(crate) fn record_standard_path_resolution(
    env_vars: &mut VarTable,
    name: &str,
    shell_path: &str,
) {
    let mut table = hash_table(env_vars);
    table.insert(
        name.to_string(),
        (shell_path.to_string(), 1, next_hash_seq(&table)),
    );
    store_hash_table(env_vars, &table);
    let cached_path = crate::executor::path::shell_path_to_windows(shell_path, env_vars);
    crate::executor::path::set_command_lookup_cache(name, Some(cached_path), env_vars);
}

/// GNU findcmd.c:365-426: a command resolution that hits hashed_filenames
/// bumps times_found (phash_search -> hashlib.c:254); one resolved through
/// PATH enters the table with times_found=1 (phash_insert found=1).
/// Called from the external-command dispatch once `find_user_command`
/// resolved `name` to `shell_path`.
pub(crate) fn record_command_resolution(env_vars: &mut VarTable, name: &str, shell_path: &str) {
    let mut table = hash_table(env_vars);
    if let Some((_, hits, _)) = table.get_mut(name) {
        *hits += 1;
    } else {
        table.insert(
            name.to_string(),
            (shell_path.to_string(), 1, next_hash_seq(&table)),
        );
    }
    store_hash_table(env_vars, &table);
}

fn script_prefix(env_vars: &HashMap<String, String>) -> String {
    if let (Some(script), Some(line)) = (
        env_vars.get("__RUBASH_SCRIPT_NAME"),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }
    "rubash: ".to_string()
}
