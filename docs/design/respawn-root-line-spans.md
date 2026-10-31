# Respawn inline TEXT bodies: root-command line-span transport (rubash#2332)

## Problem

#2202 (PR #2327) restored per-command `$LINENO` inside transported
**function** bodies: the parent stamps `__RUBASH_FORK_FN_SPANS` with the
per-function span vectors of the bodies it re-renders, and the child's
definition paths (prelude text, BASH_FUNC import) re-seed the re-parsed
bodies from them. The #2058 census rose to 15/21 byte-identical.

The remaining 6 cells are one residual mechanism, filed as #2332: when
the respawned child's **main source is an inline TEXT body itself**
(an async `( list ) &` / `{ list; } &`, or a coproc compound body),
the body's first command reports its exact line (the #2058
`__RUBASH_LINE_OFFSET` seed), but every later command still reports
lines collapsed onto the seed (probes: GNU 3/4/6 vs rubash 3/3/3 on the
same body; see `tests/` pins named `issue2332`).

## GNU semantics (source first)

- Every `COMMAND` node carries the line it was parsed at:
  `make_cmd.c:196` (`temp->line = lineno` — the same `lineno`
  parameter threads every `make_*` constructor).
- Execution stamps the current line from the executing node:
  `execute_cmd.c:936` (`SET_LINE_NUMBER (command->value.Simple->line)`,
  macro at `execute_cmd.c:295`).
- `$LINENO` is a dynamic variable bound to that line:
  `variables.c:1861` (`INIT_DYNAMIC_VAR ("LINENO", ..., get_lineno,
  ...)`; `get_lineno` at `variables.c:1462` returns
  `executing_line_number()`).

A GNU fork child therefore needs no line transport at all: it executes
the parent's already-parsed tree, and each body command reports its
own parse line. rubash's respawn child instead re-parses re-rendered
text (`bash_command_sequence_text` collapses the body's line
structure onto one rendered line), so per-node lines must cross the
spawn boundary explicitly — exactly the gap `__RUBASH_FORK_FN_SPANS`
closed for function bodies. This design closes it for the root body
with the same machinery.

## Design

### Carrier

One new fork-transport key, sibling to `__RUBASH_FORK_FN_SPANS`:

- `__RUBASH_FORK_ROOT_SPANS` — an indexed-array-codec value with two
  elements: element 0 is the count of prelude function-definition
  commands the child's `-c` source starts with (the async site renders
  `background_function_prelude()` ahead of the body; the coproc site
  renders no prelude, so 0), element 1 is the comma-joined
  `line.end_line.logical_line` triple vector of the root body, in
  `collect_command_line_spans` traversal order — the identical
  encoding the FN_SPANS entries use.

### Parent sites (the two in #2332's scope)

- **Async `&`** (`execute_background_ast_command`): after the child
  source is built, collect spans over exactly the nodes that were
  rendered into it — the absorbed `( list )` inner body list
  (`background_command_source_for_body` form) or the single transported
  command node (post redirect-strip; `background_command_source`
  form) — with the prelude count taken from the same
  `is_exportable_function_name` filter `background_function_prelude`
  iterates. The entry is stamped unconditionally (spans are absolute
  parent-source lines; unlike the line-offset seed they need no
  script-identity gate).
- **Coproc compound body**: collect spans over `coproc_cmd.body`, the
  list `bash_command_sequence_text` renders into the child's `-c`
  string; prelude count 0. Simple-command coprocs carry no body
  statements and are skipped (the command IS the body; it keeps the
  node-line seed it already has).

The output/input procsub spawn sites share the shape but are **not**
in this change: their body text reaches the spawn helper as a
pre-rendered `&str` with the body Ast at a different layer, so they
adopt the same carrier in a follow-up rather than by analogy here.

### Child install

`init.rs`, in the same provenance-gated block that decodes
`__RUBASH_FORK_FN_SPANS` (`internal_respawn` only; a forged carrier on
an ordinary invocation is ignored and, like every `FORK_*` key,
stripped unconditionally so it never surfaces as a shell variable):
decode into `ShellState::respawn_root_line_spans` plus a skip count
(prelude definitions still to pass over) and a cursor (spans consumed
so far).

### Patch point (the part #2332 flagged as needing design)

The `-c` grouped driver parses incrementally, so there is no single
initial Ast for the whole string — but both `-c` drivers (the
whole-source fast path and the grouped/history path) funnel every
reader unit through `run_source_impl`, which holds the unit's complete
parsed `Ast` immediately before `execute_ast`. That is the patch
point, the same shape as `define_function`'s reseed for the FN carrier:

- `main.rs` arms a one-shot flag on the executor around the `-c`
  driver call only (startup files and `BASH_ENV`-class runs before it
  are never armed; ordinary shells never carry the install anyway).
- `run_source_impl` applies only on its outermost entry (an entry
  guard restores the prior flag on exit; nested `source`/`eval`/
  diagnostic-prefix runs during a unit's execution see the guard set
  and never consume spans).
- Application walks the unit's top-level commands: skip the stamped
  prelude-definition count, then —
  - if the remaining commands' span count exactly consumes the
    transport, apply the remainder in one `apply_command_line_spans`
    call (the #2202 all-or-nothing count check is inherited: a
    structure-diverged re-render refuses and keeps its parse lines);
  - if it consumes only part (a grouped unit), apply per top-level
    command from the cursor, stopping permanently at the first
    command whose span count would overrun the transport;
  - if it would overrun outright, drop the install unapplied.

`$LINENO`, `BASH_LINENO` call lines, and `script: line N:`
diagnostics inside the body all read node lines, so they follow the
re-seeded tree with no further changes; a body that itself respawns
stamps its children from the re-seeded tree, so lines stay in the
original source's line space at every depth.

## Safety posture

Inherited from #2202: spans are consumed only by provenance-gated
respawn children; counts gate every application; a misaligned
transport degrades to today's behavior (seed + rendered-line drift),
never to sprayed lines. The carrier is stripped at child birth and
never enters the variable table.
