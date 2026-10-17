# Fast-path differential audit (rubash ↔ WinuxCmd ↔ GNU)

Status: audit complete, 2026-10-17.
Installed shell under test: `niu` 1.4.0, rubash `e785c3c8f610`, WinuxCmd 1.1.12.
GNU reference: Git-for-Windows coreutils 8.32 under `D:/Git/usr/bin`, real bash
`D:/Git/bin/bash.exe`.
Harness: [`scripts/fast-path-differential.py`](../../scripts/fast-path-differential.py).

This document is the differential map for rubash's **in-process fast paths**.
It records the inventory derived from the code, the reachability of each arm,
every confirmed divergence with byte-level evidence and classification, and the
classes that look systemic. Known divergences are encoded as `known=` markers in
the harness so the script stays green and trips only on a real change.

## 1. Method

rubash dispatches a command word through three layers (see
[`docs/builtins.md`](../builtins.md)): real builtin → hidden in-process fast path
→ external WinuxCmd. A divergence is only real if it is shown, so every arm is
driven three ways on the same input bytes:

* **FAST** — `niu -c '<producer> | <cmd> …'` (the in-process arm);
* **EXT** — the same body with the command replaced by the **absolute path** to
  the WinuxCmd binary (`…/winuxcmd/usr/bin/<cmd>.exe`). The absolute path
  defeats both the named inline arm and the `command` builtin, reproducing the
  exact fallback the fast path is supposed to replace;
* **GNU** — the same body under `D:/Git/bin/bash.exe` with GNU coreutils first
  on `PATH`.

**stdout and exit status are compared as raw bytes.** stderr is compared in
addition only for the four cases that are expected to print a diagnostic
(`sleep_missing`, `sleep_invalid`, `sleep_badunit`, `basename_extra`, marked
`stderr=True` in the harness; Appendix A tags those rows in its `stderr`
column), because fallback diagnostics legitimately differ in wording (e.g.
WinuxCmd `head: invalid option -- 'x'` vs GNU `head: unknown option -- x`) and
comparing them everywhere would drown the signal. The primary signal is
**FAST vs EXT** (an optimisation must reproduce its fallback); GNU classifies
whether the fallback or the fast path is the one that is wrong.

Input classes exercised per command: plain text; control bytes in the carrier
set (ESC, form feed) and outside it (VT); CRLF; empty input; trailing newline and
none; 200 KB input (truncation/re-encode); multi-byte UTF-8; NUL; and every flag
shape the arm claims to support plus malformed shapes it must reject.

> Note on `command`: `niu -c '… | command sed s/a/A/'` is **not** a faithful
> fallback reference — on `e785c3c8` it leaks the transport marker pair into the
> final stdout (`a\xee\x80\x80\xee\x80\x9cb…` for a carried ESC) where both the
> absolute WinuxCmd path and the actual declined fast-path fallback are clean.
> That is a `command`-builtin plumbing defect, is not a fast path, and is out of
> scope here; the harness uses the absolute binary for this reason.

## 2. Inventory derived from the code

### 2.1 Pipeline-stage arms — `src/executor/pipeline_exec.rs`

`execute_pipeline_stage_inner` (line 2567) matches the stage head
(`match name`, line 2807) and runs these in-process:

| arm | line | shape it accepts | fallback when it declines |
|---|---|---|---|
| `true` / `:` | 2808 | always | none |
| `false` | 2809 | always | none |
| `echo` | 2810 | all args via `echo::write_echo_decoded` | none |
| `printf` | 2823 | all args via `printf::execute_with_io` | none |
| `trap` | 2840 | all args via `trap::execute_with_io` | none |
| `head` | 2865 | line mode only: `-n N`, `-nN`, `--lines=N`, `-N`, `-q`-style flags skipped; declines on `-c`/`--bytes` and on any file operand → external | external |
| `cat` | 2898 | full `parse_cat_argv` + `cat_format`; `--help`/`--version` → external; file operands read in-process | external |
| `sed` | 3089 | `apply_simple_sed_args` (`s///`, `s///g`, `Nd`, `-e`, `-e…`) → otherwise external | external |
| `grep` | 3100 | `[-cinvq] [--] PATTERN` with stdin only, literal/`^`-anchored pattern without regex metacharacters | external |
| `wc` | 3154 | exactly `-c`, `-l`, `-w`, or no operand (lines/words/bytes) | external |
| `tr` | 3192 | two plain set operands (or `\n` special); any `-`-shaped word, non-ASCII, unknown class, `[x*n]`, escapes, reversed range → external | external |

`sleep`, `dirname`, `basename` are **not** in this match; they are dispatched
earlier (`builtins/sleep.rs`, `executor/printf_path_builtins.rs`).

### 2.2 Command-substitution filter arms — `src/executor/command_substitution_values.rs`

`command_substitution_pipeline_filter` (line 5) matches the stage after a `|`:

`sort` (bare / `-u`), `sed`, `tr`, `head`, `grep` (bare two-word only), `wc`
(`-c` / `-l` / default `-l`), `tail`, `uniq` (bare only). That is **8 arms**, not
the 11 the brief estimated; the remaining match arm is the `_` generic-external
fallback (line 161).

### 2.3 Documented in-process behaviours

`sleep`, `dirname`, `basename` (`docs/builtins.md` fast-path table), `uname` and
`arch` (`builtins/uname.rs`, no fallback), and the in-process `env`
(`executor/external_inner.rs`, whitelisted builtin, coreutils semantics, no
fallback).

### 2.4 Reconciliation with `docs/builtins.md`

`docs/builtins.md` (and the copy in `niubash/docs/src/builtins.md`) list only
five fast paths: `sleep`, `dirname`, `basename` (`niubash` page drops `uname`,
`arch`). They say nothing about the **twelve** pipeline-stage command names
(`true`, `:`, `false`, `echo`, `printf`, `trap`, `head`, `cat`, `sed`, `grep`,
`wc`, `tr`) or the **eight** substitution filters (`sort`, `sed`, `tr`, `head`,
`grep`, `wc`, `tail`, `uniq`). The doc-sync test in `src/executor/builtin_names.rs`
parses only the `builtins-list` and `fastpath-list` machine blocks, so none of
these arms is covered by the guard.

**Mismatches**

1. Code ≫ docs: 12 pipeline arms + 8 substitution arms are undocumented as
   fast paths.
2. `uname`/`arch` are in `rubash/docs/builtins.md` `fastpath-list` but missing
   from the `niubash` page's fast-path table.
3. `env` is correctly described as a hybrid in both.

### 2.5 Reachability of the substitution arms (dead-code finding)

The 8 `command_substitution_values.rs` arms are **unreachable for ordinary
`$( … | … )` substitutions** on this commit, and were already unreachable at the
installed `e785c3c8`:

* `command_substitution_pipeline_filter` has exactly one caller,
  `command_substitution_pipeline_output` (`command_substitution_pipelines.rs:494`);
* that function requires a `|` (line 468) and is called only from
  `timed_command_substitution_inner` (line 585), which is reached only through
  `timed_command_substitution_output` (line 802 of `command_substitution.rs`);
* line 802 is reached only when `command_substitution_body_is_trivial` is true
  (line 792). That predicate (`command_substitution.rs:2217`) admits only
  `[A-Za-z0-9 _-+=.,/:@%^]` — it rejects `|`, so a body that can reach the
  filter can never contain a pipeline stage.

Decisive black-box confirmation: the arms' `wc` default is `-l` (one column),
while the live pipeline arm / external `wc` default (reached through the real
executor) is the three-column `lines words bytes`. `x=$(printf 'hello\n' | wc)`
yields `      1       1       6` — the live path, not the dormant arm. The
observable `$( … | head/-c/wc/…)` behaviour in this document therefore comes
from **§2.1 pipeline_exec**, run by the real executor inside the substitution,
and `$( … | sort/tail/uniq)` comes from the external WinuxCmd binary (there is
no pipeline arm for those). The dormant arms carry the same carrier defects as
`wc`/`tr` (e.g. `-c` byte count) and must be fixed in tandem if ever revived.

## 3. Confirmed divergences

`FAST`/`EXT`/`GNU` are the raw stdout bytes (`\xNN` = one byte) and exit status.
Classification key: **payload decoding** (carrier pair), **counting unit**
(bytes vs chars vs words), **flag parsing**, **path translation**, **string
edge case**, **identity (by design)**.

### 3.1 Pipeline fast path — `wc` byte/word counting (payload decoding + counting unit)

Raw bytes in the carrier set `{0x0c, 0x11, 0x13, 0x14..0x1f}` travel between
pipeline stages as the pair `U+E000 U+E0xx` — 6 UTF-8 bytes per raw byte
(`bytes_to_shell_text`, `substitution_metadata.rs:112`). The inline `wc`
(`pipeline_exec.rs:3154-3191`) measures `input.as_bytes()` / `input.split_whitespace()`
on that transport text instead of the decoded bytes.

| repro | FAST | EXT | GNU | classification | location |
|---|---|---|---|---|---|
| `printf '\033[31mX\033[0m' \| wc -c` | `20` | `10` | `10` | payload decoding / counting unit | `pipeline_exec.rs:3166` |
| `printf 'a\033b\nc\n' \| wc -c` | `11` | `6` | `6` | payload decoding / counting unit | `pipeline_exec.rs:3166` |
| `printf 'a\014b\n' \| wc -c` | `9` | `4` | `4` | payload decoding / counting unit | `pipeline_exec.rs:3166` |
| `printf 'a\014b\n' \| wc -w` | `1` | `2` | `2` | counting unit (carried FF is no longer whitespace) | `pipeline_exec.rs:3168` |
| `printf 'a\033b\nc\n' \| wc` | `      2       2      11` | `      2       2       6` | `      2       2       6` | payload decoding (bytes column) | `pipeline_exec.rs:3181-3185` |
| `printf 'a\014b\n' \| wc` | `      1       1       9` | `      1       2       4` | `      1       2       4` | counting unit (words) + bytes | `pipeline_exec.rs:3181-3185` |
| `echo -e '\033[31m' \| wc -c` | `11` | `6` | `6` | same, any builtin producer | `pipeline_exec.rs:3166` |
| `printf '\033[31m' \| wc -c` | `10` | `5` | `5` | same, any builtin producer | `pipeline_exec.rs:3166` |

`wc -l` is correct (`\n` is not a carrier byte). Already tracked and **fully
covered**: niubash#383 (open) / rubash PR #695 (open) decodes at arm entry for
`-c`, `-w` and the default words/bytes columns in both `pipeline_exec.rs` and
`command_substitution_values.rs`. The carrier mechanism still means every future
byte/word-counting consumer must decode first.

### 3.2 Pipeline fast path — `tr` set membership (payload decoding)

`translate_tr` (`pipeline_exec.rs:3413`) walks `input.chars()`, so a carried
control byte (two PUA chars) is never a member of a set built from real control
characters (`inline_expand_tr_set`, line 3435). `[:cntrl:]`/`[:space:]` therefore
skip exactly the carrier-set bytes (0x0c, 0x11, 0x13, 0x14..0x1f) while mapping
the rest. Note 0x0b is *not* carried and maps correctly — the defect is precisely
the carrier set.

| repro | FAST | EXT | GNU | classification | location |
|---|---|---|---|---|---|
| `printf 'a\033b\nc\n' \| tr '[:cntrl:]' X` | `a\x1bbXcX` | `aXbXcX` | `aXbXcX` | payload decoding | `pipeline_exec.rs:3206-3214` |
| `printf 'a\014b\n' \| tr '[:space:]' _` | `a\x0cb_` | `a_b_` | `a_b_` | payload decoding | `pipeline_exec.rs:3206-3214` |
| `printf 'a\013\014b\n' \| tr '[:cntrl:]' X` | `aX\x0cbX` | `aXXbX` | `aXXbX` | payload decoding (0x0b maps, 0x0c escapes) | `pipeline_exec.rs:3206-3214` |

### 3.3 Pipeline fast path — `head` flag parsing (flag parsing)

The `head` arm (`pipeline_exec.rs:2865`) skips any unknown `-…` word and then
falls back to `head_line_count(&args).unwrap_or(10)` (`pipeline_exec.rs:3383`),
so malformed counts silently become "first 10 lines" with rc 0 instead of an
error, and a valid GNU negative count is lost.

| repro | FAST | EXT | GNU | classification | location |
|---|---|---|---|---|---|
| `printf 'a\nb\nc\n' \| head -n -1` | `a\nb\nc\n` rc 0 | `a\nb\n` rc 0 | `a\nb\n` rc 0 | flag parsing (`-1` parse fails → default 10) | `head_line_count:3392-3396` |
| `printf 'a\nb\nc\n' \| head -n abc` | `a\nb\nc\n` rc 0 | `` rc 1 | `` rc 1 | flag parsing: invalid number accepted | `pipeline_exec.rs:2884-2895` |
| `printf 'a\nb\nc\n' \| head -x` | `a\nb\nc\n` rc 0 | `` rc 1 | `` rc 1 | flag parsing: unknown option ignored | `pipeline_exec.rs:2888-2890` |
| `printf 'a\nb\nc\n' \| head -n` | `a\nb\nc\n` rc 0 | `` rc 1 | `` rc 1 | flag parsing: missing operand accepted | `pipeline_exec.rs:2884-2887` |
| `printf 'a\nb\nc\n' \| head --lines` | `a\nb\nc\n` rc 0 | `` rc 1 | `` rc 1 | flag parsing: missing operand accepted | `pipeline_exec.rs:2884-2887` |
| `x=$(printf 'a\nb\nc\n' \| head -n -1); printf '%s' "$x"` | `a\nb\nc` | `a\nb` | `a\nb` | same arm via real executor inside `$()` | `pipeline_exec.rs:2865` |
| `x=$(printf 'a\nb\nc\n' \| head -n abc); printf '%s' "$x"` | `a\nb\nc` rc 0 | `` rc 1 | `` rc 1 | same arm via `$()` | `pipeline_exec.rs:2865` |

### 3.4 Fast path — `dirname` / `basename` pure-string edge cases (string edge case)

`dirname_value`/`basename_value`/`trim_trailing_slashes`
(`printf_path_builtins.rs:532-569`) do not preserve a leading `//` and collapse an
empty operand to `/`. These are independent of path translation and remain in
the current `dirname_value` on `origin/master`.

| repro | FAST | EXT | GNU | classification | location |
|---|---|---|---|---|---|
| `dirname //` | `/` | `//` | `//` | string edge case | `dirname_value:545-560` |
| `dirname //a` | `/` | `//` | `//` | string edge case | `dirname_value:545-560` |
| `dirname ''` | `/` | `.` | `.` | string edge case | `dirname_value:548-550` |
| `basename ''` | `/` | `` (empty) | `` (empty) | string edge case | `basename_value:535-537` |

### 3.5 Fast path — `dirname` single-letter path translation (path translation) — already tracked, fixed on master

On the installed `e785c3c8` the `dirname` fast path sees an operand already
translated `/a/…` → `A:/…`, then strips the last component.

| repro | FAST (installed 1.4.0) | EXT | GNU |
|---|---|---|---|
| `dirname /a/b/c` | `A:/b` | `/a/b` | `/a/b` |
| `dirname /a/b/` | `A:` | `/a` | `/a` |
| `dirname /a//b` | `A:` | `/a` | `/a` |

This is **niubash#346 item (c)**, fixed by rubash#649 (merged) + niubash#371
(pending release). The worktree code is unchanged, but `#649` stops rubash
pre-translating shell-tree spellings, so the operand reaches the arm as
`/a/b/c`. Reproduces only on the installed build; **not re-filed**.

### 3.6 Identity tools (by design)

`uname -s`/`uname -a` report the rubash persona (`MSYS_NT-…`) rather than the
WinuxCmd/GNU value. Documented as intentional (`rubash#154`); recorded only so
the harness can flag the day the persona changes.

## 4. Commands and shapes verified clean

All compared FAST vs EXT (absolute WinuxCmd) with GNU corroboration; full
per-case results are in Appendix A. No divergence was found for:

`true`, `:`, `false`; `echo` (`-n`, `-e`); `printf` (`%s`, escapes); `trap`
(pipeline arm, no external counterpart); `head` default/`-n2`/`-n 2`/`--lines=2`/
`-2`/`-n 0`/`-c 3`/`-q`/no-trailing-newline/empty/control/FF/UTF-8; `cat` plain,
control, NUL, `-v`, `-A`, `-n`, `-b`, `-s`, `-E`, `-T`, `-e`, no-trailing-newline,
UTF-8, empty, CRLF, file operands (plain/control/`-n`); `sed` (pipe) `s///`,
`s///g`, `1d`, `2d`, `-e`, no-trailing-newline, CRLF, control, UTF-8, BRE `.`
and `[...]`, empty, FF; `grep` (pipe) literal, `-c`, `-n`, `-i`, `-v`, `-q`,
`-cn`, `^`-anchor, no-trailing-newline, empty, control, UTF-8, `-i` UTF-8 with
`LC_ALL=C` pinned (see below), empty pattern, regex (falls back), all-match
`-v`; `wc` `-c`/`-l`/`-w`/default
on ASCII, UTF-8, NUL, empty, no-trailing-newline, and 200 KB input; `tr` plain,
ranges, `\n` special, `[:lower:]`/`[:upper:]`, `-d` fallback, non-ASCII
fallback, empty, control; the `sort`/`sed`/`tr`/`head`/`grep`/`wc`/`tail`/`uniq`
`$()` shapes; `sleep` (`0`, `0s`, `0h`, `-- 0`, missing/invalid); `dirname`
plain/dot/root/trailing/dotdot/multi/`-a`/`-- -a`; `basename` simple/suffix/
partial/root/trailing/`-a`/extra-operand; `uname -s`/`-a` and `arch` personas;
in-process `env` (informational).

Notable *non*-divergences worth recording:

* `grep` on an unterminated final line already adds the newline, matching GNU.
* `head -n +2` matches WinuxCmd **and** GNU (both = 2 lines), so no divergence.
* `basename //` returns `/` in both FAST and WinuxCmd (GNU `//`) — shared, not a
  fast-path defect.
* `sort`/`sed` on CRLF strip the CR in FAST, WinuxCmd **and** the Windows GNU
  build alike — a platform text-mode artifact, not a fast-path defect.

**Locale pin for the non-ASCII fold case.** `grep_utf8_i`
(`printf 'café…' | grep -i É`) is the only case whose verdict depends on the
console locale, so the harness pins `LC_ALL=C` (and `LANG=C`) for all three legs.
Measured on the installed toolchain: under `C`/unset none of the legs fold
(FAST/EXT/GNU all rc 1 → PASS); under `C.UTF-8`/`en_US.UTF-8` GNU folds (rc 0)
while WinuxCmd `grep` does not (still rc 1). The inline arm uses Rust
`to_lowercase()` — locale-independent — so a build whose inline arm folds
non-ASCII under `C` will (correctly) surface as a real FAST-vs-EXT divergence
instead of console noise. This replaces the earlier "unicode case fold" label,
which implied a fold that the installed legs do not perform.

## 5. External (WinuxCmd) defects observed via the fallback

These are cases where the fast path agrees with GNU and the **external**
fallback is wrong — the fix belongs to `unixwin/winuxcmd`, not rubash. Filed
separately.

| repro | FAST | EXT (WinuxCmd) | GNU |
|---|---|---|---|
| `true -x` | rc 0 | rc 1, `true: invalid option -- 'x'` | rc 0 |
| `dirname ///` | `/` | `//` | `/` |
| `printf 'a\r\nb\r\n' \| sort` | `a\nb\n` | `a\nb\n` | `a\r\nb\r\n` |

## 6. Systemic classes

1. **Carrier payload decoding — systemic and the biggest class.** Any in-process
   arm that inspects *bytes, characters, set membership or whitespace* on the
   transport text instead of decoding first is wrong for bytes in
   `{0x0c, 0x11, 0x13, 0x14..0x1f}`. Confirmed in `wc -c`, `wc -w`, `wc`
   default, and `tr` `[:cntrl:]`/`[:space:]`. Commands that measure or classify
   input bytes should be tested next: `wc -m` (currently falls back),
   `grep`/`sed` patterns that match control bytes, `cut`/`fold`/`expand`/`od` if
   ever given inline arms, and the dormant `command_substitution_values.rs`
   `wc -c`/`tr`/`sort` arms if revived. The fix pattern is a single decode at
   arm entry (`shell_text_to_raw_bytes`).
2. **Ad-hoc flag parsing that silently defaults — systemic to the `head`/`tail`
   arms.** `head_line_count` returns `Option` and callers `unwrap_or(10)`, so
   invalid/missing/negative operands are accepted silently. `tail`'s substitution
   arm reuses the same helper (dormant, but would inherit the defect). Any arm
   that hand-rolls option parsing instead of using the shared parser should be
   audited.
3. **Path translation drift between the fast path and the fallback.** The
   `dirname` case is the instance (`niubash#346`); the general rule is that any
   in-process arm that resolves paths must see the same operand spelling as the
   spawned binary. Now that `#649` stops rubash pre-translating coreutils
   spellings this class is smaller, but arms that call `expand_word` and then do
   their own path math remain candidates.
4. **String edge cases in inline reimplementations.** `dirname_value`/
   `basename_value` are hand-rolled; leading `//` and empty operands diverge.
   `basename`/`dirname` option handling (`-a`/`-s`) is delegated to the fallback
   and is fine.
5. **Dormant substitution arms.** The `command_substitution_values.rs` filter
   arms are unreachable post-`rubash#117`; they are an inventory and latent-bug
   hazard, not an active divergence.

## 7. Harness

`scripts/fast-path-differential.py` (Python 3, stdlib only). Why a script and
not a Rust `#[test]`: the matrix must spawn the installed `niu.exe`, the
WinuxCmd binaries and Git-for-Windows GNU coreutils, and diff raw child stdout
bytes. That is inherently out-of-process and machine-specific — a Rust test
would re-implement the plumbing for no gain and could not carry the external GNU
reference. The script is idempotent, uses a per-process scratch directory (so
two concurrent runs cannot collide on fixtures), and takes `--only`, `--list`,
`--json`, `--show-ok` and `--no-ext`.

```
python scripts/fast-path-differential.py
```

Toolchains are resolved through environment overrides with machine defaults:
`NIU` (the shell under test; default `C:\Users\cmx\AppData\Local\Programs\Niubash\niu.exe`),
`WX_BIN` (WinuxCmd `usr/bin`), `GNU_BASH` (`D:\Git\bin\bash.exe`), `GNU_PATH`
(GNU coreutils search path) and `AUDIT_TMP` (scratch root). A missing `NIU` or
`WX_BIN` is a fatal exit-2 diagnostic rather than a silent downgrade; passing
`--no-ext` deliberately drops the WinuxCmd reference and compares every case
against GNU instead (the run header records `EXT=DISABLED (--no-ext)`).

Exit status is `0` when every non-`known` case reproduces its fallback and every
`known` case still diverges; it is `1` when a case regresses, a `known` case
starts matching (so expectations cannot silently rot), or `--only` matches no
cases; `2` on a missing toolchain. Add a case by appending one dict to `CASES`;
mark a confirmed, tracked divergence with `known="<issue/why>"`.

Recorded run on the installed toolchain (2026-10-17):

```
cases=179 pass=146 known=27 extbugs=2 info=4 regressions=0 fixed=0
```

`EXTBUG` marks a case where the fast path agrees with GNU and only the WinuxCmd
fallback is wrong (§5); it is not a regression. `info=4` are the
environment-dependent cases (`trap`×2, `env`×2) that are reported but not
compared.

## 8. Issues filed

| repo | issue | title | covers |
|---|---|---|---|
| rubash | [#707](https://github.com/unixwin/rubash/issues/707) | inline `tr` fast path: carried control bytes escape `[:cntrl:]`/`[:space:]` | §3.2 |
| rubash | [#708](https://github.com/unixwin/rubash/issues/708) | inline `head` fast path silently defaults invalid/missing/negative `-n` | §3.3 |
| rubash | [#709](https://github.com/unixwin/rubash/issues/709) | `dirname`/`basename` fast path: `//`, `//a`, empty operand | §3.4 |
| rubash | [#710](https://github.com/unixwin/rubash/issues/710) | `command_substitution_values.rs` pipeline-filter arms are unreachable (dead) and carry the carrier defects | §2.5 |
| winuxcmd | [#1196](https://github.com/unixwin/WinuxCmd/issues/1196) | `true` rejects a non-empty operand (rc 1); `dirname ///` returns `//` | §5 |

Already tracked, **not** re-filed: the whole `wc` carrier class (`-c`, `-w`,
default columns) — niubash#383 (open) / rubash PR #695 (open), which also
patches the dormant substitution arm; the `dirname /a/b → A:` translation —
niubash#346 (closed, fixed by rubash#649 + niubash#371).

## 9. Unconfirmed / deliberately out of scope

* **`command` builtin carrier leak** — **re-checked and cleared**: five shapes
  (`command sed 's/a/A/'`, `command cat`, `command wc -c`,
  `command tr a-z A-Z`, `a=$(command wc -c)`) on a `0x1B` payload are byte-exact
  (`41 1b 62 0a` / `4`) on **both** current master and the installed 1.4.0
  build, so the original `U+E000 U+E01C` observation does not reproduce. Recorded
  here so nobody chases it again; the flag is withdrawn rather than a fix added.
* **`env`, `trap`, `uname`, `arch` outputs** are environment-/persona-dependent
  and compared informationally; only identity divergence is recorded.
* **`cat` reading real files** is covered for plain/control/`-n`, but not for
  `/dev/fd/N`, process substitution, or binary files — the file-operand read
  path and the pipe path are shared, so the pipe results bound the risk.
* **WinuxCmd `sort` CR-stripping** is recorded but not filed: it is a
  text-mode/platform artifact that also affects the Windows GNU build in some
  pipelines.
* **`wc -m`/`-L`/combined flags** are not audited because the arm declines them
  to the external binary (verified by construction, not an exhaustive flag `*`).
* The `command_substitution_values.rs` arms are dormant, so their per-arm
  divergences could not be observed end-to-end; the carrier defects they would
  exhibit are inferred from their source and the live `pipeline_exec` arms.

## Appendix A — full matrix

Columns: `case`, `result`, `stderr` (`yes` = stderr compared in addition to
stdout/exit; `empty` = stderr asserted empty as a fixture guard; `-` =
stdout/exit only), `FAST`/`EXT`/`GNU` stdout (raw bytes, every non-printable
escaped as `\xNN`) and exit status. `INFO` = environment-dependent, not
compared.

| case | result | stderr | FAST stdout | EXT stdout | GNU stdout | note |
|---|---|---|---|---|---|---|
| `true_plain` | PASS | - | `` rc=0 | `` rc=0 | `` rc=0 | null command, rc 0 |
| `colon_plain` | PASS | - | `` rc=0 | - | `` rc=0 | null command, rc 0 |
| `false_plain` | PASS | - | `` rc=1 | `` rc=1 | `` rc=1 | rc 1, no output |
| `true_args` | EXTBUG | - | `` rc=0 | `` rc=1 | `` rc=0 | true accepts/ignores args |
| `echo_basic` | PASS | - | `hello world\n` rc=0 | `hello world\n` rc=0 | `hello world\n` rc=0 |  |
| `echo_n` | PASS | - | `hi` rc=0 | `hi` rc=0 | `hi` rc=0 | builtin vs winuxcmd echo |
| `echo_e` | PASS | - | `a\tb\n\n` rc=0 | `a\tb\n\n` rc=0 | `a\tb\n\n` rc=0 | escape decoding |
| `printf_s` | PASS | - | `a-b\n` rc=0 | `a-b\n` rc=0 | `a-b\n` rc=0 |  |
| `printf_esc` | PASS | - | `x\ty\n` rc=0 | `x\ty\n` rc=0 | `x\ty\n` rc=0 |  |
| `trap_list` | INFO | - | `` rc=0 | - | `` rc=0 | shell state, informational |
| `trap_l` | INFO | - | ` 1) SIGHUP\t 2) SIGINT\t 3) SIGQUIT\t 4) SIGILL\t 5) SIGTRAP\n 6) SIGABRT\t 7) SIGBUS\t 8) SIGFPE\t 9) SIGKILL\t10) SIGUSR1\n11) SIGSEGV\t12) SIGUSR2\t13) SIGPIPE\t14) SIGALRM\t15) SIGTERM\n16) SIGSTKFLT\t17) SIGCHLD\t18) SIGCONT\t19) SIGSTOP\t20) SIGTSTP\n21) SIGTTIN\t22) SIGTTOU\t23) SIGURG\t24) SIGXCPU\t25) SIGXFSZ\n26) SIGVTALRM\t27) SIGPROF\t28) SIGWINCH\t29) SIGIO\t30) SIGPWR\n31) SIGSYS\t34) SIGRTMIN\t35) SIGRTMIN+1\t36) SIGRTMIN+2\t37) SIGRTMIN+3\n38) SIGRTMIN+4\t39) SIGRTMIN+5\t40) SIGRTMIN+6\t41) SIGRTMIN+7\t42) SIGRTMIN+8\n43) SIGRTMIN+9\t44) SIGRTMIN+10\t45) SIGRTMIN+11\t46) SIGRTMIN+12\t47) SIGRTMIN+13\n48) SIGRTMIN+14\t49) SIGRTMIN+15\t50) SIGRTMAX-14\t51) SIGRTMAX-13\t52) SIGRTMAX-12\n53) SIGRTMAX-11\t54) SIGRTMAX-10\t55) SIGRTMAX-9\t56) SIGRTMAX-8\t57) SIGRTMAX-7\n58) SIGRTMAX-6\t59) SIGRTMAX-5\t60) SIGRTMAX-4\t61) SIGRTMAX-3\t62) SIGRTMAX-2\n63) SIGRTMAX-1\t64) SIGRTMAX\t\n` rc=0 | - | ` 1) SIGHUP\t 2) SIGINT\t 3) SIGQUIT\t 4) SIGILL\t 5) SIGTRAP\n 6) SIGABRT\t 7) SIGEMT\t 8) SIGFPE\t 9) SIGKILL\t10) SIGBUS\n11) SIGSEGV\t12) SIGSYS\t13) SIGPIPE\t14) SIGALRM\t15) SIGTERM\n16) SIGURG\t17) SIGSTOP\t18) SIGTSTP\t19) SIGCONT\t20) SIGCHLD\n21) SIGTTIN\t22) SIGTTOU\t23) SIGIO\t24) SIGXCPU\t25) SIGXFSZ\n26) SIGVTALRM\t27) SIGPROF\t28) SIGWINCH\t29) SIGPWR\t30) SIGUSR1\n31) SIGUSR2\t32) SIGRTMIN\t33) SIGRTMIN+1\t34) SIGRTMIN+2\t35) SIGRTMIN+3\n36) SIGRTMIN+4\t37) SIGRTMIN+5\t38) SIGRTMIN+6\t39) SIGRTMIN+7\t40) SIGRTMIN+8\n41) SIGRTMIN+9\t42) SIGRTMIN+10\t43) SIGRTMIN+11\t44) SIGRTMIN+12\t45) SIGRTMIN+13\n46) SIGRTMIN+14\t47) SIGRTMIN+15\t48) SIGRTMIN+16\t49) SIGRTMAX-15\t50) SIGRTMAX-14\n51) SIGRTMAX-13\t52) SIGRTMAX-12\t53) SIGRTMAX-11\t54) SIGRTMAX-10\t55) SIGRTMAX-9\n56) SIGRTMAX-8\t57) SIGRTMAX-7\t58) SIGRTMAX-6\t59) SIGRTMAX-5\t60) SIGRTMAX-4\n61) SIGRTMAX-3\t62) SIGRTMAX-2\t63) SIGRTMAX-1\t64) SIGRTMAX\t\n` rc=0 | signal table is platform-specific |
| `head_default` | PASS | - | `alpha\nbeta\ngamma\n` rc=0 | `alpha\nbeta\ngamma\n` rc=0 | `alpha\nbeta\ngamma\n` rc=0 |  |
| `head_n2` | PASS | - | `alpha\nbeta\n` rc=0 | `alpha\nbeta\n` rc=0 | `alpha\nbeta\n` rc=0 |  |
| `head_n2join` | PASS | - | `alpha\nbeta\n` rc=0 | `alpha\nbeta\n` rc=0 | `alpha\nbeta\n` rc=0 |  |
| `head_long` | PASS | - | `alpha\nbeta\n` rc=0 | `alpha\nbeta\n` rc=0 | `alpha\nbeta\n` rc=0 |  |
| `head_dash2` | PASS | - | `alpha\nbeta\n` rc=0 | `alpha\nbeta\n` rc=0 | `alpha\nbeta\n` rc=0 |  |
| `head_n0` | PASS | - | `` rc=0 | `` rc=0 | `` rc=0 |  |
| `head_negn1` | KNOWN | - | `alpha\nbeta\ngamma\n` rc=0 | `alpha\nbeta\n` rc=0 | `alpha\nbeta\n` rc=0 | GNU -n -1 = all but last line |
| `head_c` | PASS | - | `alp` rc=0 | `alp` rc=0 | `alp` rc=0 | byte mode falls back |
| `head_q` | PASS | - | `alpha\nbeta\ngamma\n` rc=0 | `alpha\nbeta\ngamma\n` rc=0 | `alpha\nbeta\ngamma\n` rc=0 |  |
| `head_badnum` | KNOWN | - | `alpha\nbeta\ngamma\n` rc=0 | `` rc=1 | `` rc=1 | flag parsing: accepted, rc 0 vs error rc 1 |
| `head_unknown` | KNOWN | - | `alpha\nbeta\ngamma\n` rc=0 | `` rc=1 | `` rc=1 | flag parsing |
| `head_missing_arg` | KNOWN | - | `alpha\nbeta\ngamma\n` rc=0 | `` rc=1 | `` rc=1 | flag parsing |
| `head_long_missing` | KNOWN | - | `alpha\nbeta\ngamma\n` rc=0 | `` rc=1 | `` rc=1 | flag parsing |
| `head_noeol` | PASS | - | `alpha\nbeta\n` rc=0 | `alpha\nbeta\n` rc=0 | `alpha\nbeta\n` rc=0 |  |
| `head_empty` | PASS | - | `` rc=0 | `` rc=0 | `` rc=0 |  |
| `head_ctrl` | PASS | - | `a\x1bb\nc\n` rc=0 | `a\x1bb\nc\n` rc=0 | `a\x1bb\nc\n` rc=0 |  |
| `head_ff` | PASS | - | `a\x0cb\n` rc=0 | `a\x0cb\n` rc=0 | `a\x0cb\n` rc=0 |  |
| `head_utf8` | PASS | - | `caf\xc3\xa9\n\xce\xb1\xce\xb2\xce\xb3\n` rc=0 | `caf\xc3\xa9\n\xce\xb1\xce\xb2\xce\xb3\n` rc=0 | `caf\xc3\xa9\n\xce\xb1\xce\xb2\xce\xb3\n` rc=0 |  |
| `cat_plain` | PASS | - | `alpha\nbeta\ngamma\n` rc=0 | `alpha\nbeta\ngamma\n` rc=0 | `alpha\nbeta\ngamma\n` rc=0 |  |
| `cat_ctrl` | PASS | - | `a\x1bb\nc\n` rc=0 | `a\x1bb\nc\n` rc=0 | `a\x1bb\nc\n` rc=0 | identity passthrough |
| `cat_nul` | PASS | - | `a\x00b\n` rc=0 | `a\x00b\n` rc=0 | `a\x00b\n` rc=0 |  |
| `cat_v` | PASS | - | `a^[b\nc\n` rc=0 | `a^[b\nc\n` rc=0 | `a^[b\nc\n` rc=0 |  |
| `cat_A` | PASS | - | `a^Ib  c$\n` rc=0 | `a^Ib  c$\n` rc=0 | `a^Ib  c$\n` rc=0 |  |
| `cat_n` | PASS | - | `     1\talpha\n     2\tbeta\n     3\tgamma\n` rc=0 | `     1\talpha\n     2\tbeta\n     3\tgamma\n` rc=0 | `     1\talpha\n     2\tbeta\n     3\tgamma\n` rc=0 |  |
| `cat_b` | PASS | - | `     1\ta\n\n\n     2\tb\n` rc=0 | `     1\ta\n\n\n     2\tb\n` rc=0 | `     1\ta\n\n\n     2\tb\n` rc=0 |  |
| `cat_s` | PASS | - | `a\n\nb\n` rc=0 | `a\n\nb\n` rc=0 | `a\n\nb\n` rc=0 |  |
| `cat_E` | PASS | - | `alpha$\nbeta$\ngamma` rc=0 | `alpha$\nbeta$\ngamma` rc=0 | `alpha$\nbeta$\ngamma` rc=0 |  |
| `cat_T` | PASS | - | `a^Ib  c\n` rc=0 | `a^Ib  c\n` rc=0 | `a^Ib  c\n` rc=0 |  |
| `cat_e` | PASS | - | `a^[b$\nc$\n` rc=0 | `a^[b$\nc$\n` rc=0 | `a^[b$\nc$\n` rc=0 |  |
| `cat_noeol` | PASS | - | `alpha\nbeta\ngamma` rc=0 | `alpha\nbeta\ngamma` rc=0 | `alpha\nbeta\ngamma` rc=0 |  |
| `cat_utf8` | PASS | - | `caf\xc3\xa9\n\xce\xb1\xce\xb2\xce\xb3\n\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\n` rc=0 | `caf\xc3\xa9\n\xce\xb1\xce\xb2\xce\xb3\n\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\n` rc=0 | `caf\xc3\xa9\n\xce\xb1\xce\xb2\xce\xb3\n\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\n` rc=0 |  |
| `cat_empty` | PASS | - | `` rc=0 | `` rc=0 | `` rc=0 |  |
| `cat_crlf` | PASS | - | `a\r\nb\r\n` rc=0 | `a\r\nb\r\n` rc=0 | `a\r\nb\r\n` rc=0 |  |
| `cat_file` | PASS | empty | `alpha\nbeta\ngamma\n` rc=0 | `alpha\nbeta\ngamma\n` rc=0 | `alpha\nbeta\ngamma\n` rc=0 | file operand |
| `cat_file_ctrl` | PASS | empty | `a\x1bb\nc\n` rc=0 | `a\x1bb\nc\n` rc=0 | `a\x1bb\nc\n` rc=0 | file operand, carried bytes |
| `cat_file_n` | PASS | empty | `     1\talpha\n     2\tbeta\n     3\tgamma\n` rc=0 | `     1\talpha\n     2\tbeta\n     3\tgamma\n` rc=0 | `     1\talpha\n     2\tbeta\n     3\tgamma\n` rc=0 | file operand + numbering |
| `sed_s` | PASS | - | `Alpha\nbetA\ngAmma\n` rc=0 | `Alpha\nbetA\ngAmma\n` rc=0 | `Alpha\nbetA\ngAmma\n` rc=0 |  |
| `sed_sg` | PASS | - | `AlphA\nbetA\ngAmmA\n` rc=0 | `AlphA\nbetA\ngAmmA\n` rc=0 | `AlphA\nbetA\ngAmmA\n` rc=0 |  |
| `sed_1d` | PASS | - | `beta\ngamma\n` rc=0 | `beta\ngamma\n` rc=0 | `beta\ngamma\n` rc=0 |  |
| `sed_2d` | PASS | - | `alpha\ngamma\n` rc=0 | `alpha\ngamma\n` rc=0 | `alpha\ngamma\n` rc=0 |  |
| `sed_e` | PASS | - | `Alpha\nbetA\ngAmma\n` rc=0 | `Alpha\nbetA\ngAmma\n` rc=0 | `Alpha\nbetA\ngAmma\n` rc=0 |  |
| `sed_noeol` | PASS | - | `Alpha\nbetA\ngAmma` rc=0 | `Alpha\nbetA\ngAmma` rc=0 | `Alpha\nbetA\ngAmma` rc=0 |  |
| `sed_crlf` | PASS | - | `A\nb\n` rc=0 | `A\nb\n` rc=0 | `A\nb\n` rc=0 |  |
| `sed_ctrl` | PASS | - | `A\x1bb\nc\n` rc=0 | `A\x1bb\nc\n` rc=0 | `A\x1bb\nc\n` rc=0 |  |
| `sed_utf8` | PASS | - | `cAf\xc3\xa9\n\xce\xb1\xce\xb2\xce\xb3\n\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\n` rc=0 | `cAf\xc3\xa9\n\xce\xb1\xce\xb2\xce\xb3\n\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\n` rc=0 | `cAf\xc3\xa9\n\xce\xb1\xce\xb2\xce\xb3\n\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\n` rc=0 |  |
| `sed_dot` | PASS | - | `aY\n` rc=0 | `aY\n` rc=0 | `aY\n` rc=0 | BRE dot |
| `sed_class` | PASS | - | `ZlphZ\nZetZ\ngZmmZ\n` rc=0 | `ZlphZ\nZetZ\ngZmmZ\n` rc=0 | `ZlphZ\nZetZ\ngZmmZ\n` rc=0 | BRE bracket |
| `sed_empty` | PASS | - | `` rc=0 | `` rc=0 | `` rc=0 |  |
| `sed_ff` | PASS | - | `A\x0cb\n` rc=0 | `A\x0cb\n` rc=0 | `A\x0cb\n` rc=0 |  |
| `grep_basic` | PASS | - | `beta\n` rc=0 | `beta\n` rc=0 | `beta\n` rc=0 |  |
| `grep_c` | PASS | - | `3\n` rc=0 | `3\n` rc=0 | `3\n` rc=0 |  |
| `grep_n` | PASS | - | `2:beta\n` rc=0 | `2:beta\n` rc=0 | `2:beta\n` rc=0 |  |
| `grep_i` | PASS | - | `alpha\n` rc=0 | `alpha\n` rc=0 | `alpha\n` rc=0 |  |
| `grep_v` | PASS | - | `beta\ngamma\n` rc=0 | `beta\ngamma\n` rc=0 | `beta\ngamma\n` rc=0 |  |
| `grep_q` | PASS | - | `` rc=0 | `` rc=0 | `` rc=0 | quiet, rc 0 |
| `grep_qmiss` | PASS | - | `` rc=1 | `` rc=1 | `` rc=1 | quiet miss, rc 1 |
| `grep_cn` | PASS | - | `3\n` rc=0 | `3\n` rc=0 | `3\n` rc=0 | combined flags |
| `grep_anchor` | PASS | - | `alpha\n` rc=0 | `alpha\n` rc=0 | `alpha\n` rc=0 |  |
| `grep_noeol` | PASS | - | `gamma\n` rc=0 | `gamma\n` rc=0 | `gamma\n` rc=0 |  |
| `grep_noeol_nomatch` | PASS | - | `` rc=1 | `` rc=1 | `` rc=1 | rc 1 |
| `grep_empty` | PASS | - | `` rc=1 | `` rc=1 | `` rc=1 |  |
| `grep_ctrl` | PASS | - | `a\x1bb\n` rc=0 | `a\x1bb\n` rc=0 | `a\x1bb\n` rc=0 |  |
| `grep_utf8` | PASS | - | `caf\xc3\xa9\n` rc=0 | `caf\xc3\xa9\n` rc=0 | `caf\xc3\xa9\n` rc=0 |  |
| `grep_utf8_i` | PASS | - | `` rc=1 | `` rc=1 | `` rc=1 | non-ASCII fold, LC_ALL=C pinned; external greps do not fold, inline must not either |
| `grep_empty_pattern` | PASS | - | `alpha\nbeta\ngamma\n` rc=0 | `alpha\nbeta\ngamma\n` rc=0 | `alpha\nbeta\ngamma\n` rc=0 |  |
| `grep_dot` | PASS | - | `axb\n` rc=0 | `axb\n` rc=0 | `axb\n` rc=0 | regex -> external fallback |
| `grep_v_all` | PASS | - | `` rc=1 | `` rc=1 | `` rc=1 | all match -> rc 1 |
| `wc_c` | PASS | - | `17\n` rc=0 | `17\n` rc=0 | `17\n` rc=0 |  |
| `wc_c_ctrl` | KNOWN | - | `11\n` rc=0 | `6\n` rc=0 | `6\n` rc=0 | known byte-count class |
| `wc_l` | PASS | - | `3\n` rc=0 | `3\n` rc=0 | `3\n` rc=0 |  |
| `wc_l_ctrl` | PASS | - | `2\n` rc=0 | `2\n` rc=0 | `2\n` rc=0 |  |
| `wc_w` | PASS | - | `3\n` rc=0 | `3\n` rc=0 | `3\n` rc=0 |  |
| `wc_w_ff` | KNOWN | - | `1\n` rc=0 | `2\n` rc=0 | `2\n` rc=0 | payload decoding / counting unit |
| `wc_default` | PASS | - | `      3       3      17\n` rc=0 | `      3       3      17\n` rc=0 | `      3       3      17\n` rc=0 |  |
| `wc_default_ctrl` | KNOWN | - | `      2       2      11\n` rc=0 | `      2       2       6\n` rc=0 | `      2       2       6\n` rc=0 | known byte-count class |
| `wc_default_ff` | KNOWN | - | `      1       1       9\n` rc=0 | `      1       2       4\n` rc=0 | `      1       2       4\n` rc=0 | payload decoding / counting unit |
| `wc_noeol` | PASS | - | `16\n` rc=0 | `16\n` rc=0 | `16\n` rc=0 |  |
| `wc_c_nul` | PASS | - | `4\n` rc=0 | `4\n` rc=0 | `4\n` rc=0 |  |
| `wc_c_ff` | KNOWN | - | `9\n` rc=0 | `4\n` rc=0 | `4\n` rc=0 | payload decoding / byte count |
| `wc_c_producer_echo` | KNOWN | - | `11\n` rc=0 | `6\n` rc=0 | `6\n` rc=0 | known byte-count class |
| `wc_c_producer_printf` | KNOWN | - | `10\n` rc=0 | `5\n` rc=0 | `5\n` rc=0 | known byte-count class |
| `wc_utf8_c` | PASS | - | `23\n` rc=0 | `23\n` rc=0 | `23\n` rc=0 |  |
| `wc_utf8_w` | PASS | - | `3\n` rc=0 | `3\n` rc=0 | `3\n` rc=0 |  |
| `wc_empty` | PASS | - | `      0       0       0\n` rc=0 | `      0       0       0\n` rc=0 | `      0       0       0\n` rc=0 |  |
| `wc_m` | PASS | - | `13\n` rc=0 | `13\n` rc=0 | `23\n` rc=0 | chars -> external fallback |
| `wc_lc` | PASS | - | `      3      17\n` rc=0 | `      3      17\n` rc=0 | `      3      17\n` rc=0 | combined -> external fallback |
| `wc_large` | PASS | - | `200001\n` rc=0 | `200001\n` rc=0 | `200001\n` rc=0 | truncation/re-encode |
| `tr_upper` | PASS | - | `ALPHA\nBETA\nGAMMA\n` rc=0 | `ALPHA\nBETA\nGAMMA\n` rc=0 | `ALPHA\nBETA\nGAMMA\n` rc=0 |  |
| `tr_simple` | PASS | - | `xlphx\nyetx\ngxmmx\n` rc=0 | `xlphx\nyetx\ngxmmx\n` rc=0 | `xlphx\nyetx\ngxmmx\n` rc=0 |  |
| `tr_nl` | PASS | - | `alpha_beta_gamma_` rc=0 | `alpha_beta_gamma_` rc=0 | `alpha_beta_gamma_` rc=0 | special \n operand |
| `tr_class` | PASS | - | `ALPHA\nBETA\nGAMMA\n` rc=0 | `ALPHA\nBETA\nGAMMA\n` rc=0 | `ALPHA\nBETA\nGAMMA\n` rc=0 |  |
| `tr_class_cntrl` | KNOWN | - | `a\x1bbXcX` rc=0 | `aXbXcX` rc=0 | `aXbXcX` rc=0 | payload decoding |
| `tr_class_space` | KNOWN | - | `a\x0cb_` rc=0 | `a_b_` rc=0 | `a_b_` rc=0 | payload decoding |
| `tr_cntrl_mixed` | KNOWN | - | `aX\x0cbX` rc=0 | `aXXbX` rc=0 | `aXXbX` rc=0 | payload decoding: only carrier-set bytes escape |
| `tr_d` | PASS | - | `lph\nbet\ngmm\n` rc=0 | `lph\nbet\ngmm\n` rc=0 | `lph\nbet\ngmm\n` rc=0 | option -> external fallback |
| `tr_utf8` | PASS | - | `CAF\xc3\xa9\n\xce\xb1\xce\xb2\xce\xb3\n\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\n` rc=0 | `CAF\xc3\xa9\n\xce\xb1\xce\xb2\xce\xb3\n\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\n` rc=0 | `CAF\xc3\xa9\n\xce\xb1\xce\xb2\xce\xb3\n\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\n` rc=0 | non-ascii set -> fallback |
| `tr_noeol` | PASS | - | `ALPHA\nBETA\nGAMMA` rc=0 | `ALPHA\nBETA\nGAMMA` rc=0 | `ALPHA\nBETA\nGAMMA` rc=0 |  |
| `tr_empty` | PASS | - | `` rc=0 | `` rc=0 | `` rc=0 |  |
| `tr_ctrl` | PASS | - | `A\x1bB\nC\n` rc=0 | `A\x1bB\nC\n` rc=0 | `A\x1bB\nC\n` rc=0 |  |
| `sub_sort` | PASS | - | `alpha\nbeta\ngamma` rc=0 | `alpha\nbeta\ngamma` rc=0 | `alpha\nbeta\ngamma` rc=0 |  |
| `sub_sort_u` | PASS | - | `alpha\nbeta\ngamma` rc=0 | `alpha\nbeta\ngamma` rc=0 | `alpha\nbeta\ngamma` rc=0 |  |
| `sub_sort_noeol` | PASS | - | `alpha\nbeta\ngamma` rc=0 | `alpha\nbeta\ngamma` rc=0 | `alpha\nbeta\ngamma` rc=0 |  |
| `sub_sort_empty` | PASS | - | `` rc=0 | `` rc=0 | `` rc=0 |  |
| `sub_sort_crlf` | PASS | - | `a\nb` rc=0 | `a\nb` rc=0 | `a\r\nb` rc=0 | FAST==EXT drop CR, GNU keeps it: shared text-mode behaviour, not a fast-path divergence |
| `sub_sort_utf8` | PASS | - | `caf\xc3\xa9\n\xce\xb1\xce\xb2\xce\xb3\n\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e` rc=0 | `caf\xc3\xa9\n\xce\xb1\xce\xb2\xce\xb3\n\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e` rc=0 | `caf\xc3\xa9\n\xce\xb1\xce\xb2\xce\xb3\n\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e` rc=0 | locale/sort order |
| `sub_sort_case` | PASS | - | `A\nB\na\nz` rc=0 | `A\nB\na\nz` rc=0 | `A\nB\na\nz` rc=0 | locale collation vs byte order |
| `sub_sort_blank_last` | PASS | - | `\na\nb` rc=0 | `\na\nb` rc=0 | `\na\nb` rc=0 | trailing empty line ordering |
| `sub_sed` | PASS | - | `Alpha\nbetA\ngAmma` rc=0 | `Alpha\nbetA\ngAmma` rc=0 | `Alpha\nbetA\ngAmma` rc=0 |  |
| `sub_sed_1d` | PASS | - | `beta\ngamma` rc=0 | `beta\ngamma` rc=0 | `beta\ngamma` rc=0 |  |
| `sub_sed_ctrl` | PASS | - | `A\x1bb\nc` rc=0 | `A\x1bb\nc` rc=0 | `A\x1bb\nc` rc=0 |  |
| `sub_tr` | PASS | - | `ALPHA\nBETA\nGAMMA` rc=0 | `ALPHA\nBETA\nGAMMA` rc=0 | `ALPHA\nBETA\nGAMMA` rc=0 |  |
| `sub_tr_nl` | PASS | - | `alpha_beta_gamma_` rc=0 | `alpha_beta_gamma_` rc=0 | `alpha_beta_gamma_` rc=0 |  |
| `sub_head` | PASS | - | `alpha\nbeta` rc=0 | `alpha\nbeta` rc=0 | `alpha\nbeta` rc=0 |  |
| `sub_head_default` | PASS | - | `alpha\nbeta\ngamma` rc=0 | `alpha\nbeta\ngamma` rc=0 | `alpha\nbeta\ngamma` rc=0 |  |
| `sub_head_negn1` | KNOWN | - | `alpha\nbeta\ngamma` rc=0 | `alpha\nbeta` rc=0 | `alpha\nbeta` rc=0 | GNU -n -1 = all but last line |
| `sub_head_badnum` | KNOWN | - | `alpha\nbeta\ngamma` rc=0 | `` rc=1 | `` rc=1 | flag parsing |
| `sub_head_noeol` | PASS | - | `alpha\nbeta` rc=0 | `alpha\nbeta` rc=0 | `alpha\nbeta` rc=0 |  |
| `sub_grep` | PASS | - | `beta` rc=0 | `beta` rc=0 | `beta` rc=0 |  |
| `sub_grep_miss` | PASS | - | `` rc=1 | `` rc=1 | `` rc=1 | rc 1 |
| `sub_grep_ctrl` | PASS | - | `a\x1bb` rc=0 | `a\x1bb` rc=0 | `a\x1bb` rc=0 |  |
| `sub_wc_c` | PASS | - | `17` rc=0 | `17` rc=0 | `17` rc=0 |  |
| `sub_wc_c_ctrl` | KNOWN | - | `11` rc=0 | `6` rc=0 | `6` rc=0 | known byte-count class |
| `sub_wc_l` | PASS | - | `3` rc=0 | `3` rc=0 | `3` rc=0 |  |
| `sub_wc_default` | PASS | - | `      3       3      17` rc=0 | `      3       3      17` rc=0 | `      3       3      17` rc=0 | default is -l only |
| `sub_tail` | PASS | - | `beta\ngamma` rc=0 | `beta\ngamma` rc=0 | `beta\ngamma` rc=0 |  |
| `sub_tail_default` | PASS | - | `alpha\nbeta\ngamma` rc=0 | `alpha\nbeta\ngamma` rc=0 | `alpha\nbeta\ngamma` rc=0 |  |
| `sub_tail_badnum` | PASS | - | `` rc=1 | `` rc=1 | `` rc=1 | falls back to external (values.rs tail arm dormant) |
| `sub_tail_noeol` | PASS | - | `beta\ngamma` rc=0 | `beta\ngamma` rc=0 | `beta\ngamma` rc=0 |  |
| `sub_tail_empty` | PASS | - | `` rc=0 | `` rc=0 | `` rc=0 |  |
| `sub_uniq` | PASS | - | `a\nb\na` rc=0 | `a\nb\na` rc=0 | `a\nb\na` rc=0 |  |
| `sub_uniq_plain` | PASS | - | `alpha\nbeta\ngamma` rc=0 | `alpha\nbeta\ngamma` rc=0 | `alpha\nbeta\ngamma` rc=0 |  |
| `sub_uniq_noeol` | PASS | - | `a` rc=0 | `a` rc=0 | `a` rc=0 |  |
| `sleep_zero` | PASS | - | `` rc=0 | `` rc=0 | `` rc=0 |  |
| `sleep_missing` | PASS | yes | `` rc=1 | `` rc=1 | `` rc=1 |  |
| `sleep_invalid` | PASS | yes | `` rc=1 | `` rc=1 | `` rc=1 |  |
| `sleep_badunit` | PASS | yes | `` rc=1 | `` rc=1 | `` rc=1 |  |
| `sleep_unit_s` | PASS | - | `` rc=0 | `` rc=0 | `` rc=0 |  |
| `sleep_unit_h` | PASS | - | `` rc=0 | `` rc=0 | `` rc=0 |  |
| `sleep_dashdash` | PASS | - | `` rc=0 | `` rc=0 | `` rc=0 |  |
| `dirname_simple` | KNOWN | - | `A:/b\n` rc=0 | `/a/b\n` rc=0 | `/a/b\n` rc=0 | path translation: FAST A:/b vs /a/b |
| `dirname_noslash` | PASS | - | `.\n` rc=0 | `.\n` rc=0 | `.\n` rc=0 |  |
| `dirname_dot` | PASS | - | `.\n` rc=0 | `.\n` rc=0 | `.\n` rc=0 |  |
| `dirname_root` | PASS | - | `/\n` rc=0 | `/\n` rc=0 | `/\n` rc=0 |  |
| `dirname_trailing` | KNOWN | - | `A:\n` rc=0 | `/a\n` rc=0 | `/a\n` rc=0 | path translation |
| `dirname_doubleslash` | KNOWN | - | `/\n` rc=0 | `//\n` rc=0 | `//\n` rc=0 | FAST / vs // |
| `dirname_tripleslash` | EXTBUG | - | `/\n` rc=0 | `//\n` rc=0 | `/\n` rc=0 | FAST and GNU ('/') agree; only winuxcmd EXT is wrong (//) |
| `dirname_lead_double_letter` | KNOWN | - | `/\n` rc=0 | `//\n` rc=0 | `//\n` rc=0 | FAST / vs // |
| `dirname_letter_double` | KNOWN | - | `A:\n` rc=0 | `/a\n` rc=0 | `/a\n` rc=0 | path translation |
| `dirname_dotdot` | PASS | - | `..\n` rc=0 | `..\n` rc=0 | `..\n` rc=0 |  |
| `dirname_multi` | PASS | - | `a\nc\n` rc=0 | `a\nc\n` rc=0 | `a\nc\n` rc=0 |  |
| `dirname_dash` | PASS | - | `` rc=1 | `` rc=1 | `` rc=1 |  |
| `dirname_dashdash` | PASS | - | `.\n` rc=0 | `.\n` rc=0 | `.\n` rc=0 |  |
| `dirname_empty` | KNOWN | - | `/\n` rc=0 | `.\n` rc=0 | `.\n` rc=0 | FAST / vs . |
| `basename_empty` | KNOWN | - | `/\n` rc=0 | `\n` rc=0 | `\n` rc=0 | FAST / vs empty |
| `basename_simple` | PASS | - | `c.txt\n` rc=0 | `c.txt\n` rc=0 | `c.txt\n` rc=0 |  |
| `basename_suffix` | PASS | - | `b\n` rc=0 | `b\n` rc=0 | `b\n` rc=0 |  |
| `basename_suffix_partial` | PASS | - | `b.\n` rc=0 | `b.\n` rc=0 | `b.\n` rc=0 |  |
| `basename_root` | PASS | - | `/\n` rc=0 | `/\n` rc=0 | `/\n` rc=0 |  |
| `basename_trailing` | PASS | - | `b\n` rc=0 | `b\n` rc=0 | `b\n` rc=0 |  |
| `basename_noslash` | PASS | - | `foo\n` rc=0 | `foo\n` rc=0 | `foo\n` rc=0 |  |
| `basename_doubleslash` | PASS | - | `/\n` rc=0 | `/\n` rc=0 | `//\n` rc=0 | FAST==EXT ('/'), GNU ('//'): shared POSIX divergence |
| `basename_dash` | PASS | - | `` rc=1 | `` rc=1 | `` rc=1 |  |
| `basename_extra` | PASS | yes | `` rc=1 | `` rc=1 | `` rc=1 |  |
| `uname_s` | KNOWN | - | `MSYS_NT-10.0-19045\n` rc=0 | `Windows_NT\n` rc=0 | `MINGW64_NT-10.0-19045\n` rc=0 | identity tool, intentional divergence |
| `uname_a` | KNOWN | - | `MSYS_NT-10.0-19045 DESKTOP-FDUJ7TN 10.0-19045 2026-10-14 03:32 UTC x86_64 Msys\n` rc=0 | `Windows_NT DESKTOP-FDUJ7TN 10.0 19045 x86_64 unknown PC Windows_NT\n` rc=0 | `MINGW64_NT-10.0-19045 DESKTOP-FDUJ7TN 3.6.3-7674c51e.x86_64 2025-07-01 09:13 UTC x86_64 Msys\n` rc=0 | identity tool, intentional divergence |
| `arch_plain` | PASS | - | `x86_64\n` rc=0 | `x86_64\n` rc=0 | `x86_64\n` rc=0 | rubash persona x86_64 matches winuxcmd/GNU on this host |
| `env_print` | INFO | - | `AGENT=1\nAI_AGENT=opencode\nALLUSERSPROFILE=C:\\ProgramData\nAPPDATA=C:\\Users\\cmx\\AppData\\Roaming\nBASH=C:/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nCAPON_LLVM_BIN=D:\\CaponAVIS_toolchain\\llvm-21.1.8\\bin\nCAPON_TOOLCHAIN_BIN=D:\\CaponAVIS_toolchain\\bin\nCOLUMNS=80\nCOMMONPROGRAMFILES=C:\\Program Files\\Common Files\nCOMMONPROGRAMFILES(X86)=C:\\Program Files (x86)\\Common Files\nCOMMONPROGRAMW6432=C:\\Program Files\\Common Files\nCOMPUTERNAME=DESKTOP-FDUJ7TN\nCOMSPEC=C:\\Windows\\system32\\cmd.exe\nCONAN_HOME=K:\\conan2\nComSpec=C:\\Windows\\system32\\cmd.exe\nDRIVERDATA=C:\\Windows\\System32\\Drivers\\DriverData\nHOME=C:/Users/cmx\nHOMEDRIVE=C:\nHOMEPATH=\\Users\\cmx\nINTERNINKSTORE_API_KEY=sk-dc57eb0cdf3e5757438ecdc745e7bdc6fe9b9a5db52a6d3a5f6187d20a13ec8d\nLOCALAPPDATA=C:\\Users\\cmx\\AppData\\Local\nLOGONSERVER=\\\\DESKTOP-FDUJ7TN\nNIU_SHELL=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\niu.exe\nNUMBER_OF_PROCESSORS=32\nOLDPWD=/c/Users/cmx\nONEDRIVE=C:\\Users\\cmx\\OneDrive\nOPENCODE=1\nOPENCODE_SESSION_ID=ses_eb8672eebffeowu113VAnlGxZO\nOPENCODE_TERMINAL=1\nOS=Windows_NT\nPATH=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\\usr/local/bin;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\\usr/bin;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\\bin;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd;C:\\Program Files\\SqlCmd\\;C:\\Windows\\system32;C:\\Windows;C:\\Windows\\System32\\Wbem;C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\;C:\\Windows\\System32\\OpenSSH\\;C:\\Program Files (x86)\\Windows Kits\\10\\Windows Performance Toolkit\\;C:\\Program Files\\Microsoft SQL Server\\Client SDK\\ODBC\\170\\Tools\\Binn\\;C:\\Program Files (x86)\\Microsoft SQL Server\\150\\Tools\\Binn\\;C:\\Program Files\\Microsoft SQL Server\\150\\Tools\\Binn\\;C:\\Program Files\\Microsoft SQL Server\\150\\DTS\\Binn\\;C:\\Program Files\\nodejs\\;C:\\Users\\cmx\\.cargo\\bin;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WindowsApps;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Packages\\Fastfetch-cli.Fastfetch_Microsoft.Winget.Source_8wekyb3d8bbwe;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Packages\\Ninja-build.Ninja_Microsoft.Winget.Source_8wekyb3d8bbwe;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Links;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Packages\\AlDanial.Cloc_Microsoft.Winget.Source_8wekyb3d8bbwe;K:\\Zed\\bin;C:\\Users\\cmx\\AppData\\Roaming\\npm;C:\\Users\\cmx\\AppData\\Local\\Programs\\codex\\bin\nPATHEXT=.COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC\nPROCESSOR_ARCHITECTURE=AMD64\nPROCESSOR_IDENTIFIER=Intel64 Family 6 Model 143 Stepping 6, GenuineIntel\nPROCESSOR_LEVEL=6\nPROCESSOR_REVISION=8f06\nPROGRAMDATA=C:\\ProgramData\nPROGRAMFILES=C:\\Program Files\nPROGRAMFILES(X86)=C:\\Program Files (x86)\nPROGRAMW6432=C:\\Program Files\nPROMPT=$P$G\nPSMODULEPATH=C:\\Program Files\\WindowsPowerShell\\Modules;C:\\Windows\\system32\\WindowsPowerShell\\v1.0\\Modules;C:\\Program Files (x86)\\Microsoft SQL Server\\150\\Tools\\PowerShell\\Modules\\\nPUBLIC=C:\\Users\\Public\nPWD=/d/repo/rubash-wt-audit-fastpaths\nSESSIONNAME=Console\nSHELL=C:/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nSHLVL=4\nSYSTEMDRIVE=C:\nSYSTEMROOT=C:\\Windows\nSystemRoot=C:\\Windows\nTEMP=C:\\Users\\cmx\\AppData\\Local\\Temp\nTERM=xterm-256color\nTHIS_SH=C:/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nTMP=C:\\Users\\cmx\\AppData\\Local\\Temp\nUSERDOMAIN=DESKTOP-FDUJ7TN\nUSERDOMAIN_ROAMINGPROFILE=DESKTOP-FDUJ7TN\nUSERNAME=cmx\nUSERPROFILE=C:\\Users\\cmx\nWINDIR=C:\\Windows\nWINUXCMD_HOME=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\nWINUXCMD_PATH=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd/usr/bin/winuxcmd.exe\nWINUXSH_ROOT=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\nWINUXSH_SHELL=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\niu.exe\nWSLENV=WT_SESSION:WT_PROFILE_ID:\nWT_PROFILE_ID={d434181a-8d38-4f78-89bd-bdef5b5d48b6}\nWT_SESSION=185f336f-9ff6-40af-adc1-a33947be23eb\n` rc=0 | `AGENT=1\nAI_AGENT=opencode\nALLUSERSPROFILE=C:\\ProgramData\nAPPDATA=C:\\Users\\cmx\\AppData\\Roaming\nBASH=C:/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nCAPON_LLVM_BIN=D:\\CaponAVIS_toolchain\\llvm-21.1.8\\bin\nCAPON_TOOLCHAIN_BIN=D:\\CaponAVIS_toolchain\\bin\nCOLUMNS=80\nCOMMONPROGRAMFILES=C:\\Program Files\\Common Files\nCOMMONPROGRAMFILES(X86)=C:\\Program Files (x86)\\Common Files\nCOMMONPROGRAMW6432=C:\\Program Files\\Common Files\nCOMPUTERNAME=DESKTOP-FDUJ7TN\nCOMSPEC=C:\\Windows\\system32\\cmd.exe\nCONAN_HOME=K:\\conan2\nDRIVERDATA=C:\\Windows\\System32\\Drivers\\DriverData\nHOME=C:/Users/cmx\nHOMEDRIVE=C:\nHOMEPATH=\\Users\\cmx\nINTERNINKSTORE_API_KEY=sk-dc57eb0cdf3e5757438ecdc745e7bdc6fe9b9a5db52a6d3a5f6187d20a13ec8d\nLOCALAPPDATA=C:\\Users\\cmx\\AppData\\Local\nLOGONSERVER=\\\\DESKTOP-FDUJ7TN\nNIU_SHELL=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\niu.exe\nNUMBER_OF_PROCESSORS=32\nOLDPWD=/c/Users/cmx\nONEDRIVE=C:\\Users\\cmx\\OneDrive\nOPENCODE=1\nOPENCODE_SESSION_ID=ses_eb8672eebffeowu113VAnlGxZO\nOPENCODE_TERMINAL=1\nOS=Windows_NT\nPATH=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\\usr\\local\\bin;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\\usr\\bin;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\\bin;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd;C:\\Program Files\\SqlCmd\\;C:\\Windows\\system32;C:\\Windows;C:\\Windows\\System32\\Wbem;C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\;C:\\Windows\\System32\\OpenSSH\\;C:\\Program Files (x86)\\Windows Kits\\10\\Windows Performance Toolkit\\;C:\\Program Files\\Microsoft SQL Server\\Client SDK\\ODBC\\170\\Tools\\Binn\\;C:\\Program Files (x86)\\Microsoft SQL Server\\150\\Tools\\Binn\\;C:\\Program Files\\Microsoft SQL Server\\150\\Tools\\Binn\\;C:\\Program Files\\Microsoft SQL Server\\150\\DTS\\Binn\\;C:\\Program Files\\nodejs\\;C:\\Users\\cmx\\.cargo\\bin;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WindowsApps;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Packages\\Fastfetch-cli.Fastfetch_Microsoft.Winget.Source_8wekyb3d8bbwe;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Packages\\Ninja-build.Ninja_Microsoft.Winget.Source_8wekyb3d8bbwe;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Links;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Packages\\AlDanial.Cloc_Microsoft.Winget.Source_8wekyb3d8bbwe;K:\\Zed\\bin;C:\\Users\\cmx\\AppData\\Roaming\\npm;C:\\Users\\cmx\\AppData\\Local\\Programs\\codex\\bin\nPATHEXT=.COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC\nPROCESSOR_ARCHITECTURE=AMD64\nPROCESSOR_IDENTIFIER=Intel64 Family 6 Model 143 Stepping 6, GenuineIntel\nPROCESSOR_LEVEL=6\nPROCESSOR_REVISION=8f06\nPROGRAMDATA=C:\\ProgramData\nPROGRAMFILES=C:\\Program Files\nPROGRAMFILES(X86)=C:\\Program Files (x86)\nPROGRAMW6432=C:\\Program Files\nPROMPT=$P$G\nPSMODULEPATH=C:\\Program Files\\WindowsPowerShell\\Modules;C:\\Windows\\system32\\WindowsPowerShell\\v1.0\\Modules;C:\\Program Files (x86)\\Microsoft SQL Server\\150\\Tools\\PowerShell\\Modules\\\nPUBLIC=C:\\Users\\Public\nPWD=/d/repo/rubash-wt-audit-fastpaths\nSESSIONNAME=Console\nSHELL=C:/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nSHLVL=4\nSYSTEMDRIVE=C:\nSYSTEMROOT=C:\\Windows\nTEMP=C:\\Users\\cmx\\AppData\\Local\\Temp\nTERM=xterm-256color\nTHIS_SH=C:/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nTMP=C:\\Users\\cmx\\AppData\\Local\\Temp\nUSERDOMAIN=DESKTOP-FDUJ7TN\nUSERDOMAIN_ROAMINGPROFILE=DESKTOP-FDUJ7TN\nUSERNAME=cmx\nUSERPROFILE=C:\\Users\\cmx\nWINDIR=C:\\Windows\nWINUXCMD_HOME=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\nWINUXCMD_PATH=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd/usr/bin/winuxcmd.exe\nWINUXSH_ROOT=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\nWINUXSH_SHELL=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\niu.exe\nWSLENV=WT_SESSION:WT_PROFILE_ID:\nWT_PROFILE_ID={d434181a-8d38-4f78-89bd-bdef5b5d48b6}\nWT_SESSION=185f336f-9ff6-40af-adc1-a33947be23eb\n` rc=0 | `COMMONPROGRAMFILES(X86)=C:\\Program Files (x86)\\Common Files\nPROGRAMFILES(X86)=C:\\Program Files (x86)\nSHELL=/c/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nNUMBER_OF_PROCESSORS=32\nPSMODULEPATH=C:\\Program Files\\WindowsPowerShell\\Modules;C:\\Windows\\system32\\WindowsPowerShell\\v1.0\\Modules;C:\\Program Files (x86)\\Microsoft SQL Server\\150\\Tools\\PowerShell\\Modules\\\nPROCESSOR_LEVEL=6\nAI_AGENT=opencode\nUSERDOMAIN_ROAMINGPROFILE=DESKTOP-FDUJ7TN\nWT_SESSION=185f336f-9ff6-40af-adc1-a33947be23eb\nPROGRAMFILES=C:\\Program Files\nMSYSTEM=MINGW64\nPATHEXT=.COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC\nWINUXSH_ROOT=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\nPROGRAMDATA=C:\\ProgramData\nOS=Windows_NT\nAGENT=1\nHOMEDRIVE=C:\nCOMMONPROGRAMW6432=C:\\Program Files\\Common Files\nUSERDOMAIN=DESKTOP-FDUJ7TN\nPWD=/d/repo/rubash-wt-audit-fastpaths\nUSERPROFILE=C:\\Users\\cmx\nINTERNINKSTORE_API_KEY=sk-dc57eb0cdf3e5757438ecdc745e7bdc6fe9b9a5db52a6d3a5f6187d20a13ec8d\nOPENCODE_SESSION_ID=ses_eb8672eebffeowu113VAnlGxZO\nBASH=/usr/bin/bash\nALLUSERSPROFILE=C:\\ProgramData\nHOME=/c/Users/cmx\nUSERNAME=cmx\nOPENCODE=1\nPLINK_PROTOCOL=ssh\nCOLUMNS=80\nTHIS_SH=C:/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nCOMSPEC=C:\\Windows\\system32\\cmd.exe\nAPPDATA=C:\\Users\\cmx\\AppData\\Roaming\nSYSTEMROOT=C:\\Windows\nLOCALAPPDATA=C:\\Users\\cmx\\AppData\\Local\nPROMPT=$P$G\nCOMPUTERNAME=DESKTOP-FDUJ7TN\nDRIVERDATA=C:\\Windows\\System32\\Drivers\\DriverData\nTERM=xterm-256color\nLOGONSERVER=\\\\DESKTOP-FDUJ7TN\nPROGRAMW6432=C:\\Program Files\nONEDRIVE=C:\\Users\\cmx\\OneDrive\nTEMP=/tmp\nWINUXCMD_HOME=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\nCAPON_TOOLCHAIN_BIN=D:\\CaponAVIS_toolchain\\bin\nSHLVL=3\nPROCESSOR_REVISION=8f06\nCONAN_HOME=K:\\conan2\nCOMMONPROGRAMFILES=C:\\Program Files\\Common Files\nEXEPATH=D:\\Git\\bin\nPROCESSOR_IDENTIFIER=Intel64 Family 6 Model 143 Stepping 6, GenuineIntel\nNIU_SHELL=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\niu.exe\nSESSIONNAME=Console\nWINUXSH_SHELL=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\niu.exe\nWSLENV=WT_SESSION:WT_PROFILE_ID:\nHOMEPATH=\\Users\\cmx\nTMP=/tmp\nPATH=/mingw64/bin:/usr/bin:/c/Users/cmx/bin:/usr/bin:/bin:/c/Users/cmx/AppData/Local/Programs/Niubash/winuxcmd/usr/local/bin:/c/Users/cmx/AppData/Local/Programs/Niubash/winuxcmd/usr/bin:/c/Users/cmx/AppData/Local/Programs/Niubash/winuxcmd/bin:/c/Users/cmx/AppData/Local/Programs/Niubash/winuxcmd:/c/Program Files/SqlCmd:/c/Windows/system32:/c/Windows:/c/Windows/System32/Wbem:/c/Windows/System32/WindowsPowerShell/v1.0:/c/Windows/System32/OpenSSH:/c/Program Files (x86)/Windows Kits/10/Windows Performance Toolkit:/c/Program Files/Microsoft SQL Server/Client SDK/ODBC/170/Tools/Binn:/c/Program Files (x86)/Microsoft SQL Server/150/Tools/Binn:/c/Program Files/Microsoft SQL Server/150/Tools/Binn:/c/Program Files/Microsoft SQL Server/150/DTS/Binn:/c/Program Files/nodejs:/c/Users/cmx/.cargo/bin:/c/Users/cmx/AppData/Local/Microsoft/WindowsApps:/c/Users/cmx/AppData/Local/Programs/Niubash:/c/Users/cmx/AppData/Local/Microsoft/WinGet/Packages/Fastfetch-cli.Fastfetch_Microsoft.Winget.Source_8wekyb3d8bbwe:/c/Users/cmx/AppData/Local/Microsoft/WinGet/Packages/Ninja-build.Ninja_Microsoft.Winget.Source_8wekyb3d8bbwe:/c/Users/cmx/AppData/Local/Microsoft/WinGet/Links:/c/Users/cmx/AppData/Local/Microsoft/WinGet/Packages/AlDanial.Cloc_Microsoft.Winget.Source_8wekyb3d8bbwe:/k/Zed/bin:/c/Users/cmx/AppData/Roaming/npm:/c/Users/cmx/AppData/Local/Programs/codex/bin\nOPENCODE_TERMINAL=1\nWINUXCMD_PATH=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd/usr/bin/winuxcmd.exe\nCAPON_LLVM_BIN=D:\\CaponAVIS_toolchain\\llvm-21.1.8\\bin\nWINDIR=C:\\Windows\nPROCESSOR_ARCHITECTURE=AMD64\nWT_PROFILE_ID={d434181a-8d38-4f78-89bd-bdef5b5d48b6}\nPUBLIC=C:\\Users\\Public\nSYSTEMDRIVE=C:\nOLDPWD=/c/Users/cmx\n_=/usr/bin/env\n` rc=0 | sorted env dump, informational |
| `env_set` | INFO | - | `AGENT=1\nAI_AGENT=opencode\nALLUSERSPROFILE=C:\\ProgramData\nAPPDATA=C:\\Users\\cmx\\AppData\\Roaming\nBASH=C:/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nCAPON_LLVM_BIN=D:\\CaponAVIS_toolchain\\llvm-21.1.8\\bin\nCAPON_TOOLCHAIN_BIN=D:\\CaponAVIS_toolchain\\bin\nCOLUMNS=80\nCOMMONPROGRAMFILES=C:\\Program Files\\Common Files\nCOMMONPROGRAMFILES(X86)=C:\\Program Files (x86)\\Common Files\nCOMMONPROGRAMW6432=C:\\Program Files\\Common Files\nCOMPUTERNAME=DESKTOP-FDUJ7TN\nCOMSPEC=C:\\Windows\\system32\\cmd.exe\nCONAN_HOME=K:\\conan2\nComSpec=C:\\Windows\\system32\\cmd.exe\nDRIVERDATA=C:\\Windows\\System32\\Drivers\\DriverData\nFOO=bar\nHOME=C:/Users/cmx\nHOMEDRIVE=C:\nHOMEPATH=\\Users\\cmx\nINTERNINKSTORE_API_KEY=sk-dc57eb0cdf3e5757438ecdc745e7bdc6fe9b9a5db52a6d3a5f6187d20a13ec8d\nLOCALAPPDATA=C:\\Users\\cmx\\AppData\\Local\nLOGONSERVER=\\\\DESKTOP-FDUJ7TN\nNIU_SHELL=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\niu.exe\nNUMBER_OF_PROCESSORS=32\nOLDPWD=/c/Users/cmx\nONEDRIVE=C:\\Users\\cmx\\OneDrive\nOPENCODE=1\nOPENCODE_SESSION_ID=ses_eb8672eebffeowu113VAnlGxZO\nOPENCODE_TERMINAL=1\nOS=Windows_NT\nPATH=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\\usr/local/bin;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\\usr/bin;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\\bin;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd;C:\\Program Files\\SqlCmd\\;C:\\Windows\\system32;C:\\Windows;C:\\Windows\\System32\\Wbem;C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\;C:\\Windows\\System32\\OpenSSH\\;C:\\Program Files (x86)\\Windows Kits\\10\\Windows Performance Toolkit\\;C:\\Program Files\\Microsoft SQL Server\\Client SDK\\ODBC\\170\\Tools\\Binn\\;C:\\Program Files (x86)\\Microsoft SQL Server\\150\\Tools\\Binn\\;C:\\Program Files\\Microsoft SQL Server\\150\\Tools\\Binn\\;C:\\Program Files\\Microsoft SQL Server\\150\\DTS\\Binn\\;C:\\Program Files\\nodejs\\;C:\\Users\\cmx\\.cargo\\bin;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WindowsApps;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Packages\\Fastfetch-cli.Fastfetch_Microsoft.Winget.Source_8wekyb3d8bbwe;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Packages\\Ninja-build.Ninja_Microsoft.Winget.Source_8wekyb3d8bbwe;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Links;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Packages\\AlDanial.Cloc_Microsoft.Winget.Source_8wekyb3d8bbwe;K:\\Zed\\bin;C:\\Users\\cmx\\AppData\\Roaming\\npm;C:\\Users\\cmx\\AppData\\Local\\Programs\\codex\\bin\nPATHEXT=.COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC\nPROCESSOR_ARCHITECTURE=AMD64\nPROCESSOR_IDENTIFIER=Intel64 Family 6 Model 143 Stepping 6, GenuineIntel\nPROCESSOR_LEVEL=6\nPROCESSOR_REVISION=8f06\nPROGRAMDATA=C:\\ProgramData\nPROGRAMFILES=C:\\Program Files\nPROGRAMFILES(X86)=C:\\Program Files (x86)\nPROGRAMW6432=C:\\Program Files\nPROMPT=$P$G\nPSMODULEPATH=C:\\Program Files\\WindowsPowerShell\\Modules;C:\\Windows\\system32\\WindowsPowerShell\\v1.0\\Modules;C:\\Program Files (x86)\\Microsoft SQL Server\\150\\Tools\\PowerShell\\Modules\\\nPUBLIC=C:\\Users\\Public\nPWD=/d/repo/rubash-wt-audit-fastpaths\nSESSIONNAME=Console\nSHELL=C:/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nSHLVL=4\nSYSTEMDRIVE=C:\nSYSTEMROOT=C:\\Windows\nSystemRoot=C:\\Windows\nTEMP=C:\\Users\\cmx\\AppData\\Local\\Temp\nTERM=xterm-256color\nTHIS_SH=C:/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nTMP=C:\\Users\\cmx\\AppData\\Local\\Temp\nUSERDOMAIN=DESKTOP-FDUJ7TN\nUSERDOMAIN_ROAMINGPROFILE=DESKTOP-FDUJ7TN\nUSERNAME=cmx\nUSERPROFILE=C:\\Users\\cmx\nWINDIR=C:\\Windows\nWINUXCMD_HOME=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\nWINUXCMD_PATH=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd/usr/bin/winuxcmd.exe\nWINUXSH_ROOT=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\nWINUXSH_SHELL=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\niu.exe\nWSLENV=WT_SESSION:WT_PROFILE_ID:\nWT_PROFILE_ID={d434181a-8d38-4f78-89bd-bdef5b5d48b6}\nWT_SESSION=185f336f-9ff6-40af-adc1-a33947be23eb\n` rc=0 | `AGENT=1\nAI_AGENT=opencode\nALLUSERSPROFILE=C:\\ProgramData\nAPPDATA=C:\\Users\\cmx\\AppData\\Roaming\nBASH=C:/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nCAPON_LLVM_BIN=D:\\CaponAVIS_toolchain\\llvm-21.1.8\\bin\nCAPON_TOOLCHAIN_BIN=D:\\CaponAVIS_toolchain\\bin\nCOLUMNS=80\nCOMMONPROGRAMFILES=C:\\Program Files\\Common Files\nCOMMONPROGRAMFILES(X86)=C:\\Program Files (x86)\\Common Files\nCOMMONPROGRAMW6432=C:\\Program Files\\Common Files\nCOMPUTERNAME=DESKTOP-FDUJ7TN\nCOMSPEC=C:\\Windows\\system32\\cmd.exe\nCONAN_HOME=K:\\conan2\nDRIVERDATA=C:\\Windows\\System32\\Drivers\\DriverData\nFOO=bar\nHOME=C:/Users/cmx\nHOMEDRIVE=C:\nHOMEPATH=\\Users\\cmx\nINTERNINKSTORE_API_KEY=sk-dc57eb0cdf3e5757438ecdc745e7bdc6fe9b9a5db52a6d3a5f6187d20a13ec8d\nLOCALAPPDATA=C:\\Users\\cmx\\AppData\\Local\nLOGONSERVER=\\\\DESKTOP-FDUJ7TN\nNIU_SHELL=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\niu.exe\nNUMBER_OF_PROCESSORS=32\nOLDPWD=/c/Users/cmx\nONEDRIVE=C:\\Users\\cmx\\OneDrive\nOPENCODE=1\nOPENCODE_SESSION_ID=ses_eb8672eebffeowu113VAnlGxZO\nOPENCODE_TERMINAL=1\nOS=Windows_NT\nPATH=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\\usr\\local\\bin;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\\usr\\bin;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\\bin;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd;C:\\Program Files\\SqlCmd\\;C:\\Windows\\system32;C:\\Windows;C:\\Windows\\System32\\Wbem;C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\;C:\\Windows\\System32\\OpenSSH\\;C:\\Program Files (x86)\\Windows Kits\\10\\Windows Performance Toolkit\\;C:\\Program Files\\Microsoft SQL Server\\Client SDK\\ODBC\\170\\Tools\\Binn\\;C:\\Program Files (x86)\\Microsoft SQL Server\\150\\Tools\\Binn\\;C:\\Program Files\\Microsoft SQL Server\\150\\Tools\\Binn\\;C:\\Program Files\\Microsoft SQL Server\\150\\DTS\\Binn\\;C:\\Program Files\\nodejs\\;C:\\Users\\cmx\\.cargo\\bin;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WindowsApps;C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Packages\\Fastfetch-cli.Fastfetch_Microsoft.Winget.Source_8wekyb3d8bbwe;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Packages\\Ninja-build.Ninja_Microsoft.Winget.Source_8wekyb3d8bbwe;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Links;C:\\Users\\cmx\\AppData\\Local\\Microsoft\\WinGet\\Packages\\AlDanial.Cloc_Microsoft.Winget.Source_8wekyb3d8bbwe;K:\\Zed\\bin;C:\\Users\\cmx\\AppData\\Roaming\\npm;C:\\Users\\cmx\\AppData\\Local\\Programs\\codex\\bin\nPATHEXT=.COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC\nPROCESSOR_ARCHITECTURE=AMD64\nPROCESSOR_IDENTIFIER=Intel64 Family 6 Model 143 Stepping 6, GenuineIntel\nPROCESSOR_LEVEL=6\nPROCESSOR_REVISION=8f06\nPROGRAMDATA=C:\\ProgramData\nPROGRAMFILES=C:\\Program Files\nPROGRAMFILES(X86)=C:\\Program Files (x86)\nPROGRAMW6432=C:\\Program Files\nPROMPT=$P$G\nPSMODULEPATH=C:\\Program Files\\WindowsPowerShell\\Modules;C:\\Windows\\system32\\WindowsPowerShell\\v1.0\\Modules;C:\\Program Files (x86)\\Microsoft SQL Server\\150\\Tools\\PowerShell\\Modules\\\nPUBLIC=C:\\Users\\Public\nPWD=/d/repo/rubash-wt-audit-fastpaths\nSESSIONNAME=Console\nSHELL=C:/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nSHLVL=4\nSYSTEMDRIVE=C:\nSYSTEMROOT=C:\\Windows\nTEMP=C:\\Users\\cmx\\AppData\\Local\\Temp\nTERM=xterm-256color\nTHIS_SH=C:/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nTMP=C:\\Users\\cmx\\AppData\\Local\\Temp\nUSERDOMAIN=DESKTOP-FDUJ7TN\nUSERDOMAIN_ROAMINGPROFILE=DESKTOP-FDUJ7TN\nUSERNAME=cmx\nUSERPROFILE=C:\\Users\\cmx\nWINDIR=C:\\Windows\nWINUXCMD_HOME=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\nWINUXCMD_PATH=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd/usr/bin/winuxcmd.exe\nWINUXSH_ROOT=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\nWINUXSH_SHELL=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\niu.exe\nWSLENV=WT_SESSION:WT_PROFILE_ID:\nWT_PROFILE_ID={d434181a-8d38-4f78-89bd-bdef5b5d48b6}\nWT_SESSION=185f336f-9ff6-40af-adc1-a33947be23eb\n` rc=0 | `COMMONPROGRAMFILES(X86)=C:\\Program Files (x86)\\Common Files\nPROGRAMFILES(X86)=C:\\Program Files (x86)\nSHELL=/c/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nNUMBER_OF_PROCESSORS=32\nPSMODULEPATH=C:\\Program Files\\WindowsPowerShell\\Modules;C:\\Windows\\system32\\WindowsPowerShell\\v1.0\\Modules;C:\\Program Files (x86)\\Microsoft SQL Server\\150\\Tools\\PowerShell\\Modules\\\nPROCESSOR_LEVEL=6\nAI_AGENT=opencode\nUSERDOMAIN_ROAMINGPROFILE=DESKTOP-FDUJ7TN\nWT_SESSION=185f336f-9ff6-40af-adc1-a33947be23eb\nPROGRAMFILES=C:\\Program Files\nMSYSTEM=MINGW64\nPATHEXT=.COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC\nWINUXSH_ROOT=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\nPROGRAMDATA=C:\\ProgramData\nOS=Windows_NT\nAGENT=1\nHOMEDRIVE=C:\nCOMMONPROGRAMW6432=C:\\Program Files\\Common Files\nUSERDOMAIN=DESKTOP-FDUJ7TN\nPWD=/d/repo/rubash-wt-audit-fastpaths\nUSERPROFILE=C:\\Users\\cmx\nINTERNINKSTORE_API_KEY=sk-dc57eb0cdf3e5757438ecdc745e7bdc6fe9b9a5db52a6d3a5f6187d20a13ec8d\nOPENCODE_SESSION_ID=ses_eb8672eebffeowu113VAnlGxZO\nBASH=/usr/bin/bash\nALLUSERSPROFILE=C:\\ProgramData\nHOME=/c/Users/cmx\nUSERNAME=cmx\nOPENCODE=1\nPLINK_PROTOCOL=ssh\nCOLUMNS=80\nTHIS_SH=C:/Users/cmx/AppData/Local/Programs/Niubash/niu.exe\nCOMSPEC=C:\\Windows\\system32\\cmd.exe\nAPPDATA=C:\\Users\\cmx\\AppData\\Roaming\nSYSTEMROOT=C:\\Windows\nLOCALAPPDATA=C:\\Users\\cmx\\AppData\\Local\nPROMPT=$P$G\nCOMPUTERNAME=DESKTOP-FDUJ7TN\nDRIVERDATA=C:\\Windows\\System32\\Drivers\\DriverData\nTERM=xterm-256color\nLOGONSERVER=\\\\DESKTOP-FDUJ7TN\nPROGRAMW6432=C:\\Program Files\nONEDRIVE=C:\\Users\\cmx\\OneDrive\nTEMP=/tmp\nWINUXCMD_HOME=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd\nCAPON_TOOLCHAIN_BIN=D:\\CaponAVIS_toolchain\\bin\nSHLVL=3\nPROCESSOR_REVISION=8f06\nCONAN_HOME=K:\\conan2\nCOMMONPROGRAMFILES=C:\\Program Files\\Common Files\nEXEPATH=D:\\Git\\bin\nPROCESSOR_IDENTIFIER=Intel64 Family 6 Model 143 Stepping 6, GenuineIntel\nNIU_SHELL=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\niu.exe\nSESSIONNAME=Console\nWINUXSH_SHELL=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\niu.exe\nWSLENV=WT_SESSION:WT_PROFILE_ID:\nHOMEPATH=\\Users\\cmx\nTMP=/tmp\nPATH=/mingw64/bin:/usr/bin:/c/Users/cmx/bin:/usr/bin:/bin:/c/Users/cmx/AppData/Local/Programs/Niubash/winuxcmd/usr/local/bin:/c/Users/cmx/AppData/Local/Programs/Niubash/winuxcmd/usr/bin:/c/Users/cmx/AppData/Local/Programs/Niubash/winuxcmd/bin:/c/Users/cmx/AppData/Local/Programs/Niubash/winuxcmd:/c/Program Files/SqlCmd:/c/Windows/system32:/c/Windows:/c/Windows/System32/Wbem:/c/Windows/System32/WindowsPowerShell/v1.0:/c/Windows/System32/OpenSSH:/c/Program Files (x86)/Windows Kits/10/Windows Performance Toolkit:/c/Program Files/Microsoft SQL Server/Client SDK/ODBC/170/Tools/Binn:/c/Program Files (x86)/Microsoft SQL Server/150/Tools/Binn:/c/Program Files/Microsoft SQL Server/150/Tools/Binn:/c/Program Files/Microsoft SQL Server/150/DTS/Binn:/c/Program Files/nodejs:/c/Users/cmx/.cargo/bin:/c/Users/cmx/AppData/Local/Microsoft/WindowsApps:/c/Users/cmx/AppData/Local/Programs/Niubash:/c/Users/cmx/AppData/Local/Microsoft/WinGet/Packages/Fastfetch-cli.Fastfetch_Microsoft.Winget.Source_8wekyb3d8bbwe:/c/Users/cmx/AppData/Local/Microsoft/WinGet/Packages/Ninja-build.Ninja_Microsoft.Winget.Source_8wekyb3d8bbwe:/c/Users/cmx/AppData/Local/Microsoft/WinGet/Links:/c/Users/cmx/AppData/Local/Microsoft/WinGet/Packages/AlDanial.Cloc_Microsoft.Winget.Source_8wekyb3d8bbwe:/k/Zed/bin:/c/Users/cmx/AppData/Roaming/npm:/c/Users/cmx/AppData/Local/Programs/codex/bin\nOPENCODE_TERMINAL=1\nWINUXCMD_PATH=C:\\Users\\cmx\\AppData\\Local\\Programs\\Niubash\\winuxcmd/usr/bin/winuxcmd.exe\nCAPON_LLVM_BIN=D:\\CaponAVIS_toolchain\\llvm-21.1.8\\bin\nWINDIR=C:\\Windows\nPROCESSOR_ARCHITECTURE=AMD64\nWT_PROFILE_ID={d434181a-8d38-4f78-89bd-bdef5b5d48b6}\nPUBLIC=C:\\Users\\Public\nSYSTEMDRIVE=C:\nOLDPWD=/c/Users/cmx\n_=/usr/bin/env\nFOO=bar\n` rc=0 |  |
