# Rubash GNU Bash Compatibility Skill

## Scope

Use this skill for GNU Bash 5.2.21 compatibility work in this repository. Fix semantic owners, not individual expected-output lines or upstream bridge artifacts.

## Required evidence

1. Read the compatibility plan, suite diff analysis, issue ledger, and GNU source map before editing.
2. Compare script files with WSL GNU Bash 5.2.21. The authoritative verdict is:
   `MSYS_NO_PATHCONV=1 wsl bash tests/gnu-compat/run-83.sh check <NAME>`.
3. Keep raw outputs under `target/issue-suites/results/`; record interpretation in `docs/`.
4. Record the GNU source/test body, Rust semantic owner, minimal probe, exit status, stdout, stderr, and timeout before claiming a fix.
5. Never use Git Bash, the winuxsh `bash` shim, raw `.right` parity, or relay output as the sole baseline.

## Shared-tree handoff rules

- Read the current tree before editing. Do not stash, reset, or overwrite another agent's work.
- `src/lexer/continuation.rs` is captain-exclusive; send proposed changes instead of editing it.
- Every edit must leave `cargo build` clean. Do not leave debug prints, partial migrations, or generated junk.
- Agents report files, ownership, before/after counts, and verification; the captain groups commits.
- Do not regenerate expected outputs unless a fresh WSL GNU capture proves the old artifact is stale. Label environment/shim artifacts explicitly.
- Check for stuck `rubash.exe`, `bash.exe`, suite runners, and `cargo` processes after bounded tests.

## Known footguns

- Pin `*.sh` and `*.rs` to LF; if CRLF appears, use Git normalization rather than patching scripts.
- WSL interop does not reliably forward arbitrary Linux environment variables to Windows children; use `WSLENV=PATH` where required and prefer `current_exe()` for `THIS_SH`.
- Do not use `wsl bash -c` for multi-level quoting or doubled backslashes; pass a script file.
- Never put backticks or `$()` in commit messages; write a message file and use `git commit -F`.
- Verify `git add`, commits, and pushes with `git diff --cached`, `git log`, and remote status; shims may swallow output.
- A failed push is not a successful handoff. Report the exact transport error and leave the local commit state visible.

## Globstar handoff

The current high-value globstar evidence is `target/multi-gnu.out`, `target/multi-rub.out`, and `target/multi-diff.txt`; preserve the verified adjacent-collapse, trailing-slash, and single-`**` fast paths while investigating multiplicity. Do not claim completion without an authoritative `check globstar` PASS.

## Casemod handoff

For bare associative expansions such as `AA1^^` and `([FOO]=BAR)`, compare against the matching GNU `subst.c` path and a minimal WSL script before changing expansion routing. Treat environment-sized `declare -p` differences as harness artifacts unless a constrained environment reproduces them.

## Identity persona disclosure (rubash#154)

The engine answers `uname`/`arch` itself (hidden fast-path builtins — full
coreutils option parsing; `type` still reports them like external commands).
Identity is a **persona**, selected by `RUBASH_IDENTITY`:

- default (no variable / any value other than `native`): **MSYS2-compatible**
  — `uname -s` = `MSYS_NT-<winver>` (prefix follows `MSYSTEM`:
  `MINGW64_NT-`/`UCRT64_NT-`/…), `uname -m`/`arch` = build arch,
  `uname -r` = Windows version (`10.0-19044`), `uname -o` = `Msys`,
  `OSTYPE=msys`, `MACHTYPE=<arch>-pc-msys`. This is a compatibility mask
  for the MSYS/Cygwin script ecosystem, not a claim of being an MSYS2 port.
- `RUBASH_IDENTITY=native`: honest-native — `uname -s` = `Windows_NT`,
  `uname -o` = `Windows`, `OSTYPE=windows`, `MACHTYPE=<arch>-pc-windows`.

Query the active persona and effective values with `rubash --identity`;
`rubash --help` carries the same disclosure. Agents diffing output against
WSL GNU Bash must remember the WSL baseline reports `linux-gnu`/`Linux` —
identity-driven lines (uname, OSTYPE, MACHTYPE) are persona-owned, not
engine bugs.
