# mvdan-sh/ — full mvdan/sh snippet corpus + execution-differential ledgers

Pinned for rubash #2906. The eco-2906 lane ran the mvdan/sh corpus at
execution depth out-of-repo; this directory is the in-repo pin of that
differential, exercised by
`tests/issue2906_mvdan_sh_execution_differential.rs`.

## Contents

- `t0000.sh` .. `t0558.sh`, `tSKIP.sh` — all 560 snippets: the FIRST `in`
  string of every `fileTest()` call in mvdan/sh@`aebdf2b8`
  `syntax/filetests_test.go` (fileTests 553 + fileTestsNoPrint 3 +
  fileTestsKeepComments 4 = 560 calls), in file order, with the
  documented historical-numbering exception (fileTests call #169 is
  emitted as `tSKIP.sh`; see the extractor header). Extractor:
  `scripts/mvdan-sh-extract.py` (validation gate: the 16 files under
  `../mvdan-n/` must match the same-named corpus files byte-for-byte —
  asserted by the test).
- `MANIFEST.tsv` — per-snippet provenance (table, call index, variant
  count, first string) for the 559 numbered snippets.
- `gnu-ledger.tsv` — pinned GNU oracle expectations per snippet:
  `number, gnu_parse_rc, gnu_exec_rc|TIMEOUT, gnu_stdout_hex`.
  Recorded with GNU bash 5.3.20 under the chain isolation below.
- `known-divergences.tsv` — snippets whose current rubash behaviour
  diverges from the GNU ledger, with the tracking issue and the recorded
  rubash behaviour: `number, issue, rub_parse_rc, rub_exec_rc|TIMEOUT,
  rub_stdout_hex`. A row must stay byte-exact AND stay divergent; when
  the tracked fix lands, delete the row (the snippet then has to match
  the GNU ledger).

## Chain isolation (how the ledgers were recorded; how the test runs)

Per snippet and shell: fresh empty cwd+HOME, environment exactly
`{PATH=/usr/bin:/bin, HOME=<scratch>}`, stdin `/dev/null`, 5 s timeout
(timeout = still running at the limit, recorded as `TIMEOUT`). Parse
level: `<shell> -n <snippet>` exit code. Exec level: stdout bytes +
exit code. stderr is never compared (advisory axis; wording divergences
are filed separately).

Outputs are path-normalized before comparison — shell binary path →
`<SHELL>`, snippet path → `<SCRIPT>`, scratch/HOME path → `<HOME>` —
so the ledgers do not depend on the checkout location (the eco chain's
"path artifact" exclusion; it is what keeps the `typeset` listing
snippet t0447 comparable across trees). Variable dumps additionally
have the shell's own PPID masked to `<PPID>` (both the bare `PPID=…`
form in t0447 and the `declare -ir PPID="…"` form in t0449): a PPID
names the parent process of whoever ran the recording, not shell
behaviour, so without the mask those two rows could only ever pass
under the recorder's own parent process.

## State at pinning (master `88047f50`, 2026-11-08)

- 560 snippets, parse DIFF 0, exec SAME 537, tracked divergences 23:
  #3019 ×17 (bad-substitution exit status), #3020 ×3 (hash-leading
  command word), #3021 ×2 (empty-word command + heredoc status),
  #3025 ×1 (`[[ -t a && -z b ]]` exit status).
- Reconciliation against the eco-2906 canonical run (base `6eb861b8`,
  27 exec DIFF): t0284 (#3022), t0498 (#3023), t0538 (#3024) turned
  SAME via merged fixes; t0447 (#3026) no longer reproduces in the
  debug-binary runs that recorded these ledgers (still open; the
  release-binary face is the captain's call). The GNU side of this
  ledger reproduces the eco canonical GNU rcs for all 560 snippets.

## Regeneration

Re-extract with `scripts/mvdan-sh-extract.py` against the pinned
upstream file, then re-run both shells under the isolation above
(the ecosuite chain driver `run-mvdan-sh-chain.py` implements the same
isolation) and re-derive the two ledgers. Do not hand-edit ledger rows.

## Boundary vs #3033

Whether this differential also runs as a CI job (or only as a periodic
eco chain) is #3033's decision. This pin deliberately adds no CI job;
it runs wherever the full integration-test set runs:

```
cargo test --test issue2906_mvdan_sh_execution_differential
```
