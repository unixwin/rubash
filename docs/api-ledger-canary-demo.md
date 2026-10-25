# API ledger canary — demonstration record

Date: 2026-10-25. Basis: rubash `abd9be95b556556149ab44c45d0e80d27eefa8f6`,
niubash `b53e6bffcfa4abe25a1e2176b16b63cdc24f487b`. Demonstrator: the
150-a cluster (1.5.0 plan, row 1). This record is the acceptance
evidence PLAN row 1 requires: the canary must be shown to go red when
a ledger symbol is deleted and green when it is restored.

## What was demonstrated

The ledger tripwire half of the canary (`scripts/check-api-ledger.py`,
see `docs/api-ledger.md` §5) against a scratch copy of the tree
(`src/` + `docs/` + `scripts/` copied out; the real tree was never
modified). The deleted symbol is `note_console_ctrl_c` —
deliberately the `builtins::kill` console hook whose disappearance
once broke niubash master undetected (RFC §3.2 / §6.4).

## Transcript

Baseline, unmodified tree:

```text
$ python3 scripts/check-api-ledger.py --repo .
check-api-ledger: all 100 ledger symbols present
rc=0
```

Symbol deleted in the scratch copy
(`pub fn note_console_ctrl_c()` renamed in `src/builtins/kill.rs`):

```text
$ python3 scripts/check-api-ledger.py --repo .
check-api-ledger: 1 ledger symbol(s) missing:
  MISSING item rubash::builtins::kill::note_console_ctrl_c (not found in tree)
rc=1
```

Symbol restored:

```text
$ python3 scripts/check-api-ledger.py --repo .
check-api-ledger: all 100 ledger symbols present
rc=0
```

Checker self-test (the checker must itself be able to fail; run in CI
before the real check):

```text
$ python3 scripts/check-api-ledger.py --self-test
check-api-ledger: 1 ledger symbol(s) missing:
  MISSING item rubash::executor::NoSuchSymbol (not found in tree)
self-test: bogus symbol correctly reported missing
rc=0
```

## Scope note

This demonstrates the tripwire half. The authoritative half — the
pairing job compiling niubash against this tree
(`.github/workflows/niu-canary.yml`, job `pairing`) — runs in CI,
where a full niubash `cargo check --workspace --locked` belongs; it
was not run on the authoring machine (no heavy builds there, per the
1.5.0 plan's build-window discipline). The pairing job's first CI run
is its own green evidence; if it surfaces a real incompatibility at
that point, that is the canary working, and the finding goes to the
ledger's next update.
