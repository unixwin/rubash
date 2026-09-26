# Command Lookup Cache and Arithmetic Mode Split

Date: 2026-09-14
Baseline: WSL GNU Bash 5.3.0(1)-release at `/usr/local/bin/bash`

## Command Lookup Cache Synchronization

### Background

GNU bash keeps one conceptual command hash table (`findcmd.c` +
`builtins/hash.def`). Rubash previously split it into:

1. A user-visible serialized table (`__RUBASH_HASH_TABLE`) used by `hash`,
   `type`, `BASH_CMDS`, and related compatibility paths.
2. An in-memory `CommandLookupCache` (`src/executor/path.rs`) used by
   `find_user_command()` for actual external command resolution.

Mutations of the visible table did not invalidate or update the internal
cache, so `find_user_command` kept returning stale paths.

### Gaps Closed (commits fccf98de, 8d326bb2)

| Gap | GNU source | Rubash fix |
|---|---|---|
| `hash -d NAME` clears internal cache | `hash.def` phash_remove | `remove_command_lookup_cache(name)` |
| `hash -p PATH NAME` updates internal cache | `hash.def` phash_insert | `set_command_lookup_cache(name, path)` |
| bare `hash NAME` rehashes | `hash.def` remove+search+insert | new rehash branch |
| `check_hashed_filenames` stat validation | `findcmd.c:367-380` | `cached_path_still_valid(path)` on hit |
| `hashing_enabled` bypass (`set +h`) | `findcmd.c:356-365` | hashall check in `find_user_command` |
| `hash` builtin reports "hashing disabled" | `hash.def:86-90` | hashall check at top of `execute_with_io` |
| temp env PATH bypass (`PATH=foo cmd`) | `findcmd.c:356-359` | `__RUBASH_TEMP_PATH` tag in `apply_temporary_assignments` |

### Verification

WSL GNU Bash 5.3.0(1) script-file probes confirmed:
- `hash -p PATH NAME` then run -> uses the explicit path
- `hash NAME` after PATH change -> re-resolves to the new path
- `hash NAME` not found -> "hash: X: not found"
- `hash -r` -> clears the table
- `set +h` -> `hash` prints "hash: hashing disabled"
- `shopt -s checkhash` + delete cached executable -> re-searches PATH
- `shopt -u checkhash` + delete cached executable -> uses stale path, fails
- `PATH=foo cmd` -> uses temp PATH, does not pollute the normal cache

### Known architectural limitation

Normal command execution does not backfill the user-visible
`__RUBASH_HASH_TABLE` because `find_user_command` cannot mutate
`env_vars`. This means `hash` (no args) and `hash -t NAME` may show
"hash table empty" / "not found" even after a command has been run.
This is a pre-existing architectural gap, not introduced by the cache
synchronization work.

## Arithmetic Diagnostic Mode Split

### Discovery

GNU bash 5.3.0(1) has an invocation-mode split for `$(( ))` expansion
diagnostics that is observable **only in the expansion context**, not in
command contexts:

| Invocation | `$(( 4+ ))` | `(( 4+ ))` | `let '4+'` |
|---|---|---|---|
| `bash script.sh` | `arithmetic syntax error` | `arithmetic syntax error` | `arithmetic syntax error` |
| `bash -c '...'` | `syntax error` | `arithmetic syntax error` | `arithmetic syntax error` |

### Root cause

This is a real GNU bash 5.3.0(1) behavior, not a measurement error. Two
agents independently measured different results because they used
different invocation modes. The split was confirmed with `env -i` clean
environment runs.

### Rubash fix (commit 7666a550)

Rubash tags `-c` invocations with `__RUBASH_IS_C=1` (main.rs).
`arithmetic_error_message` now takes `env_vars` and uses
`command_context = !is_c_mode` so:
- `-c` mode `$(( ))` -> `syntax error` (matches GNU `-c`)
- script mode `$(( ))` -> `arithmetic syntax error` (matches GNU script)
- all command contexts (`(( ))`, `let`, `[[ ]]`) -> `arithmetic syntax error`
  regardless of mode

### Impact on test harness

Test cases using `-c` mode (e.g. `compat_issue_regressions.rs`) should
expect `syntax error` for expansion-context diagnostics. Test cases
using script-file mode should expect `arithmetic syntax error`. The
previous agent conflict (commits 45a2beb8 vs 3e348d53) was caused by
both agents being correct for their respective invocation modes but
not knowing the other mode existed.
