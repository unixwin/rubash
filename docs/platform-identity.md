# Platform Identity & Path Dialect

> Moved from the README on 2026-10-10, when the README was restructured to be
> evidence-first. The short identity summary lives in the README; the persona
> disclosure for AI agents lives in [`SKILL.md`](../SKILL.md) (rubash#154).
> This file keeps the full detail, preserved from the README as of commit
> `7be6a799`.

## Why native matters (Windows)

Shells billed as "bash on Windows" (Git Bash, MSYS2) ship a ported bash that rides on a POSIX emulation layer (`msys-2.0.dll`), with fork emulation and path translation that leak quirks into every script. Rubash has no such layer — one self-contained binary speaking Win32 directly.

## Paths are first-class, one dialect per child

The MSYS model *guesses* which arguments look like paths and rewrites them — which is why every AI agent and script has to set `MSYS_NO_PATHCONV=1` to stop `/flags` from becoming `C:/Program Files/Git/flags`. Rubash inverts the model with a per-child dialect contract (the same line GNU draws: `shell_execve` hands `execve` the raw word bytes and never rewrites argv). A child that owns a POSIX layer — anything resolved out of a WinuxCmd installation, or through the `winuxcmd` dispatcher — receives its argv **verbatim** and resolves `/d/...`, `/tmp`, `/dev/*` itself. A native Windows program instead receives real Win32 spellings for path-shaped operands, translated **uniformly** (never half of one argv translated and half left POSIX), so a native tool always gets a valid Win32 path for every operand, including not-yet-existing targets it is about to create. For emergency rollback the pre-Option-B translation behavior is available by exporting `__RUBASH_ARGV_DIALECT=legacy`.

## Identity and Compatibility (rubash#154)

The engine presents a **selectable platform identity** instead of outsourcing
it to whatever `uname.exe` happens to sit on PATH. `uname` and `arch` are
engine builtins (full option parsing, coreutils/MSYS2 output shapes), and
`OSTYPE`/`MACHTYPE`/`HOSTTYPE` follow the same persona.

- **Default persona: MSYS2-compatible** (disclosed everywhere — this is a
  compatibility mask, not a claim of being an MSYS2 port):
  - `uname -s` → `MSYS_NT-<ver>` — or `MINGW64_NT-` / `UCRT64_NT-` /
    `CLANG64_NT-` … when `MSYSTEM` is set, mapping the value the same way
    the MSYS2 runtime does;
  - `uname -m` / `arch` → `x86_64` / `aarch64` (build arch);
  - `uname -r` → the Windows version string (`10.0-19044`, same string that
    finishes `uname -s`);
  - `uname -o` → `Msys`; `uname -a` → `sysname nodename release version
    machine Msys` (the Git Bash shape);
  - `OSTYPE=msys`, `MACHTYPE=<arch>-pc-msys` (bound `set_if_not`-style, as
    GNU variables.c:723-725 does — an inherited value wins).
  Ecosystem scripts that branch on `case "$(uname -s)" in MINGW*|MSYS*|
  CYGWIN*)` or `$OSTYPE` ∈ {msys, cygwin} take their best-tested path.
- **`RUBASH_IDENTITY=native` switches to the honest-native persona**:
  `uname -s` → `Windows_NT` (the native `%OS%` value), `uname -o` →
  `Windows`, `OSTYPE=windows`, `MACHTYPE=<arch>-pc-windows`. Tests and
  native-first users select this; unset the variable (or set any other
  value) to return to the default.

**Disclosure surfaces** (the persona must never be silent):

- `rubash --help` prints an Identity section (current persona + how to switch);
- `rubash --identity` prints the persona and every effective value
  (`uname -s/-m/-r/-o`, `arch`, `OSTYPE`, `MACHTYPE`);
- the repo-root [`SKILL.md`](../SKILL.md) persona disclosure section (so AI
  agents consuming the repo also know the persona semantics).

`uname`/`arch` are *hidden* fast-path builtins (like `sleep`/`dirname`):
`type`/`enable`/`compgen -b` keep reporting them as external commands, and
they work even when PATH carries no `uname.exe` at all.

## Platform status narrative (2026-09 state)

**Platform status — cross-platform, Windows-first**: Windows is the primary
target and the platform with the full stack: one self-contained binary
solving the classic Windows bash pain points (no POSIX emulation layer, no
path-conversion heuristics, no `MSYS_NO_PATHCONV`, native Win32 paths as
the currency, MSYS2-compatible identity for the bash ecosystem). Linux now
builds and runs natively (`x86_64-unknown-linux-gnu`) with real
getrlimit/chmod/faccessat/uname(2) semantics and signal delivery — verified
by running the same GNU suite corpus on the Linux binary inside WSL (19/24
byte-identical and climbing). macOS
compiles green in CI with coreutils-correct uname arms. The engine's
semantic model (in-process subshells, fd-table semantics, process
boundaries) is deliberately platform-neutral, so the same ledger travels.

Current per-platform status (CI legs, release artifacts, Android check
targets) is kept in the README Platforms table; open gaps are tracked in
[`docs/cross-platform-gap-ledger.md`](cross-platform-gap-ledger.md).
