# #2036 S5 three-way sync corpus

Case data for the 34 names that both the engine (on some face) and
WinuxCmd (as an applet) claim — the audit §2.2 conflict universe.
Runner: `tests/issue2036_s5_sync_corpus.rs` (Linux CI gate; unix-only).
The Windows three-way run of this same case set (engine × applet × GNU
on a real Windows host, where the cfg(windows) faces are live) is the
audit's win-diff battery (pkg10 H cell).

## Anchors

Every expectation was measured on 2026-10-29 by running each case as a
script file under GNU bash 5.3.20 (owner-built) with GNU coreutils 9.4
and GNU diffutils 3.10. GNU is the shared sync target: the engine is
pinned to it here, and WinuxCmd's applets are pinned to it by the
WinuxCmd repo's own differential CI — that is what makes the corpus
"three-way" without a Windows host in this repo's CI. To re-anchor a
case, dual-run its `script` in a fresh directory under the GNU shell
and the engine and compare rc / stdout / stderr byte-for-byte; do not
transcribe expectations from the engine's current output for
`status: "gnu"` cases.

## Case schema (corpus.json)

- `id`, `name` (one of the 34), `face` (claiming face), `ref`
  (audit section / issue / ruling the case answers to).
- `files`: scratch files to create (`path: content`); `fakes`: PATH
  script bodies (`name: body`, a `#!/bin/sh` header is added);
  `elf_fakes`: names to plant as a copy of `/bin/true`.
- `path`: `system` (fixed `/usr/local/bin:/usr/bin:/bin`), `fakes`
  (fakebin first, then system), or `nonexist`.
- `env`: extra environment; an empty `TERM` value removes TERM. The
  runner otherwise fixes `TERM=xterm`, `LC_ALL=C`, `HOME` = scratch.
- `script`: run as a file (`case.sh`) in the scratch dir, stdin null,
  20s hard bound.
- `expect`: `rc` plus `stdout` (exact) or `stdout_contains`, and
  optionally `stderr` (exact) or `stderr_contains`; an absent stderr
  field is unchecked (e.g. timing text). `{DIR}` in an expectation is
  replaced by the scratch path.
- `status`: `gnu` — `expect` is the GNU measurement and the engine
  must match it. `registered` — `expect` pins the engine's current
  measured behavior for a divergence that is adjudicated-keep (a ruled
  stand_down Replace cell) or not yet ruled (#2046 B2/B3/C1/C2); the
  GNU measurement is recorded in the case's `gnu` field. A registered
  case may only change with the ruling that owns it.
- Host-tool rule: for shapes the engine *declines* to the real body
  (e.g. `cat --version`, no-operand `chmod`/`rmdir`), the output text
  belongs to the host toolset, which is not part of the engine's
  contract — GNU coreutils on one machine, uutils on the ubuntu CI
  image. Those cases anchor on status plus a toolset-independent
  substring only, with the locally measured GNU text kept in the
  case's `note`. Expectations produced by the engine itself (builtin
  and emulation diagnostics) are pinned verbatim.

## Attribution correction (S4 four-quadrant measurement)

Audit §4 / S3 §3.2-a described cat as "yields to a script fake on the
simple-command face, replaced only for ELF". Measured on master
03ae580d in all four quadrants, that is not what the code does: the
simple-command face replaces **both** shapes (shape-blind Replace),
and the pipeline face yields **both** shapes because the concurrent
direct-spawn planner claims any resolvable stage member before the
inline arm runs. The audit's contrary reading compared a pipeline
measurement (its shape 1 cat leg was `cat f1.txt | head -1`) against a
simple-command one. The corpus cases `cat-simple-fake`,
`cat-simple-elf`, and `cat-pipe-fake` pin the corrected attribution.
