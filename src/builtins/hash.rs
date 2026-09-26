//! hash module.
//!
//! GNU Bash source ownership:
// - builtins/hash.def

use crate::executor::markers::{DATA_DOLLAR, DATA_DOLLAR_STR};
use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;
const HASH_TABLE: &str = "__RUBASH_HASH_TABLE";

pub fn execute(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    execute_with_io(args, env_vars, &mut stdout, &mut stderr)
}

pub(crate) fn execute_with_io<W, E>(
    args: &[String],
    env_vars: &mut HashMap<String, String>,
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

    let mut print = args.is_empty();
    let mut delete = false;
    let mut pathname = None;
    let mut translate = false;
    let mut reusable = false;
    let mut names = Vec::new();
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        if arg == "-p" || arg.starts_with("-p") {
            let value = if arg == "-p" {
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
                value
            } else {
                &arg[2..]
            };
            pathname = Some(value);
            if let Some(name) = args.get(index + 1) {
                names.push(name.as_str());
            }
            break;
        } else if let Some(options) = arg.strip_prefix('-') {
            for option in options.chars() {
                match option {
                    'r' => {
                        env_vars.remove(HASH_TABLE);
                        // GNU `hash -r` forgets all remembered locations; the
                        // internal lookup cache holds the same information, so
                        // it must be dropped too or misses/hits stay stale.
                        crate::executor::path::clear_command_lookup_cache();
                        return Ok(EXECUTION_SUCCESS);
                    }
                    'd' => delete = true,
                    't' => translate = true,
                    'l' => {
                        reusable = true;
                        print = true;
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

    let mut table = hash_table(env_vars);
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
        if let Some(name) = names.first().copied() {
            if pathname == "/" {
                writeln!(
                    stderr,
                    "{}hash: {pathname}: Is a directory",
                    script_prefix(env_vars)
                )?;
                return Ok(EXECUTION_FAILURE);
            }
            table.insert(
                name.to_string(),
                (pathname.to_string(), 0, next_hash_seq(&table)),
            );
            store_hash_table(env_vars, &table);
            // GNU hash.def `hash -p PATH NAME`: phash_insert(name, pathname)
            // makes subsequent lookups of `name` return `pathname` without a
            // PATH scan. Mirror that in the in-memory lookup cache so
            // find_user_command(name) returns the same path external_inner
            // would execute. Convert through shell_path_to_windows so the
            // cached PathBuf matches the Windows-native form produced by
            // find_user_command_uncached.
            let cached_path = crate::executor::path::shell_path_to_windows(pathname, env_vars);
            crate::executor::path::set_command_lookup_cache(name, Some(cached_path));
            return Ok(EXECUTION_SUCCESS);
        }
        print = true;
    }

    if delete {
        let mut status = EXECUTION_SUCCESS;
        for name in names {
            if table.remove(name).is_none() {
                writeln!(stderr, "{}hash: {name}: not found", script_prefix(env_vars))?;
                status = EXECUTION_FAILURE;
            } else {
                // GNU hash.def `hash -d NAME`: phash_remove(w) drops the entry
                // from the hash table. The internal lookup cache holds the
                // same information and must be invalidated in lockstep or the
                // next `find_user_command(name)` returns the stale path.
                crate::executor::path::remove_command_lookup_cache(name);
            }
        }
        store_hash_table(env_vars, &table);
        return Ok(status);
    }

    if translate {
        let mut status = EXECUTION_SUCCESS;
        for name in names {
            // GNU phash_search -> hash_search bumps times_found on a hit
            // (hashlib.c:254).
            if let Some((path, hits, seq)) = table.get_mut(name) {
                *hits += 1;
                let path = path.clone();
                let _ = (hits, seq);
                if reusable {
                    writeln!(stdout, "builtin hash -p {path} {name}")?;
                } else {
                    writeln!(stdout, "{path}")?;
                }
            } else {
                writeln!(stderr, "{}hash: {name}: not found", script_prefix(env_vars))?;
                status = EXECUTION_FAILURE;
            }
        }
        store_hash_table(env_vars, &table);
        return Ok(status);
    }

    if print {
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
            for (name, (path, _, _)) in table {
                writeln!(stdout, "builtin hash -p {path} {name}")?;
            }
            return Ok(EXECUTION_SUCCESS);
        }
        // GNU hash.def prints the empty-table message on stdout.
        writeln!(stdout, "hash: hash table empty")?;
        return Ok(EXECUTION_SUCCESS);
    }

    // GNU hash.def bare-name form: `hash NAME...` re-resolves each NAME
    // against PATH and re-inserts the result. The flow is phash_remove(name)
    // + find_user_command(name) + phash_insert(name, path); a name that does
    // not resolve reports "hash: NAME: not found" and sets failure.
    if !names.is_empty() {
        let mut status = EXECUTION_SUCCESS;
        for name in names {
            // Drop any stale remembered location so the PATH scan is
            // authoritative.
            crate::executor::path::remove_command_lookup_cache(name);
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

pub(crate) fn set_hashed_path(env_vars: &mut HashMap<String, String>, name: &str, path: &str) {
    let mut table = hash_table(env_vars);
    table.insert(
        name.to_string(),
        (path.to_string(), 0, next_hash_seq(&table)),
    );
    store_hash_table(env_vars, &table);
    // BASH_CMDS[name]=value mirrors `hash -p value name`; keep the internal
    // lookup cache in sync so find_user_command(name) returns value.
    let cached_path = crate::executor::path::shell_path_to_windows(path, env_vars);
    crate::executor::path::set_command_lookup_cache(name, Some(cached_path));
}

pub(crate) fn remove_hashed_path(env_vars: &mut HashMap<String, String>, name: &str) {
    let mut table = hash_table(env_vars);
    table.remove(name);
    store_hash_table(env_vars, &table);
    // unset 'BASH_CMDS[name]' mirrors `hash -d name`; drop the internal cache
    // entry so the next lookup re-scans PATH.
    crate::executor::path::remove_command_lookup_cache(name);
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

fn store_hash_table(
    env_vars: &mut HashMap<String, String>,
    table: &HashMap<String, (String, u32, u64)>,
) {
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

/// GNU hashlib.c:254: a hash_search hit increments times_found — every
/// phash_search (`type`, `command -v`, exec resolution, `hash -t`) counts.
pub(crate) fn bump_hashed_path_hit(env_vars: &mut HashMap<String, String>, name: &str) {
    let mut table = hash_table(env_vars);
    if let Some((_, hits, _)) = table.get_mut(name) {
        *hits += 1;
        store_hash_table(env_vars, &table);
    }
}

/// GNU findcmd.c:365-426: a command resolution that hits hashed_filenames
/// bumps times_found (phash_search -> hashlib.c:254); one resolved through
/// PATH enters the table with times_found=1 (phash_insert found=1).
/// Called from the external-command dispatch once `find_user_command`
/// resolved `name` to `shell_path`.
pub(crate) fn record_command_resolution(
    env_vars: &mut HashMap<String, String>,
    name: &str,
    shell_path: &str,
) {
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
