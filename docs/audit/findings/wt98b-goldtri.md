# wt98b/goldtri — ecosystem gold-candidate triage (2026-10-09)

Lane wt98b/goldtri (retry). Baseline: branch `wt98b/goldtri` fast-forwarded to
master `a157656d` (includes fa87622b multi-line dparen #435 and 567acfd8
captured pipeline-stage stdin #436), fresh `cargo build --release`.
Input: `D:/eco-harvest/results/test-results.jsonl` (6336 rows), gold targets
SYNTAX-REJECT-RUBASH-ONLY 54 + HANG 101 + EXITED 17 = 172 rows.

Machine-readable result: `wt98b-goldtri-triage-results.json` (every target
hash classified; 172 rows → 139 unique content hashes after LAST-WINS).
Raw drivers + per-asset evidence: `target/goldtri/` (untracked).

## Environment notes (affect evidence provenance)

- **WSL is DOWN on this machine right now** (`WslService` stopped since boot,
  start denied without admin; Store wsl.exe reports
  `Wsl/WSL_E_WSL_OPTIONAL_COMPONENT_REQUIRED`). The GNU side of every
  classification therefore comes from **recorded oracle data**: the harvest
  rows themselves carry `gnu_syntax_ok` / `gnu_parity` / `gnu_detail`
  captured live under WSL GNU Bash 5.3.0 on 2026-10-04/05, plus the wt97b
  triage table (`docs/eco-harvest-resume-wt97b.md`, niubash repo). GNU
  behavior is static, so recorded parity stays valid; fresh-GNU
  re-verification is the one blocked step. Git Bash 5.2 grammar was used as
  a non-authoritative *corroboration* only, and every new issue says so.
- The ConPTY harness (`scripts/harvest/eco-test.py`, imported from the
  niubash repo) sends a `^U` (WAKE) prefix before each probe. rubash.exe's
  engine REPL does **not** implement `^U` kill-line (that is the niu product
  layer — the original harvest drove `niu -i`), so the WAKE glues into the
  first word and kills every probe (`$'\025command': command not found`).
  This lane disabled WAKE (`et.WAKE = ""`) and kept everything else
  (marker-verified phases, settle, hard caps, GNU ladder logic).

## Row accounting

- 172 target rows → 139 unique hashes still carrying a target verdict in the
  LAST-WINS view; 66 hashes were already superseded by the later harvest
  re-test passes (30 → GNU-ALSO-FAILS, 17 → OK, 3 → SLOW, 16 flipped between
  target families). All 139 were retested here against the fresh build;
  172-row coverage is via `original_verdicts` per hash.
- 11 assets could not be re-fetched in earlier runs but all 89 live targets
  re-fetched fine this run (0 fetch failures).

## Classification (139 hashes)

| class | n | meaning |
| --- | --- | --- |
| new-engine-bug | 5 | fresh build still rejects; GNU accepts → issues #460 #461 #462 #463 #465 |
| already-fixed | 27 | 6 syntax (ble.sh → #435/fa87622b) + 21 ConPTY passes (incl. #425 vscode.theme.sh, #436 nox.bash) |
| already-passed-later-run | 20 | superseded by later harvest runs (OK/SLOW) |
| gnu-also-fails | 28 | later runs re-stamped GNU-ALSO-FAILS (old `-n`-only oracle artifacts, zsh content, non-shell files) |
| known-family-zsh-content | 2 | zsh-only syntax GNU rejects too (#142 family context) |
| conpty-harness-stdin | 40 | hang only under ConPTY: `cat`/`readarray`/`read`/REPL/menu/terminal-query children never see EOF; each finishes ≤2s non-interactively (stdin=DEVNULL); GNU interactive blocks identically (wt97b harness class) |
| upstream-heavy-external | 12 | still >15s non-interactively: git clone / make / pkg-install / `nc -l` servers / sudo-loop / 3× Android `envsetup.sh` (platform exec latency × hundreds of external calls) |
| upstream-exit | 5 | asset itself runs `exit` on this environment; sourcing `exit` must end the interactive shell (GNU identical) |

## Already-fixed confirmations (fresh master a157656d)

1. **rubash#435 (multi-line `(( ))`)** — all 6 ble.sh files (core-syntax,
   init-term, keymap.vi, canvas, color, edit) now pass `rubash -n`; fixed by
   fa87622b. wt97b's gold group is fully resolved.
2. **rubash#436 / #426** — `scop/bash-completion/completions-fallback/nox.bash`
   (the #436 evidence file) now passes the full ConPTY journey; fixed by
   567acfd8.
3. **rubash#425 (vscode theme)** — `ohmybash/oh-my-bash/themes/vscode/vscode.theme.sh`
   no longer wedges on fresh master (was still open at wt97b). Full journey
   OK; evidence comment added to #425.
4. 19 further dotfiles/scripts that hung under the older builds pass clean.

## NEW engine bugs filed (one per distinct root cause)

All five: `rubash -n` rejects on fresh master a157656d while GNU 5.3.0
accepts (recorded harvest oracle + wt97b ladder; Git Bash 5.2 rc=0 as
non-authoritative corroboration). Three of the five also show rubash's own
runtime sourcing the file cleanly — the `-n` parse path is the strict side.

| issue | construct | real-world asset |
| --- | --- | --- |
| [#460](https://github.com/unixwin/rubash/issues/460) | terminator-less `then` directly after `]]` **inside a function body** (top level parses): `f() { if [[ x == y ]] then … fi }` | shell_function_n_shortcuts/.shortcut |
| [#461](https://github.com/unixwin/rubash/issues/461) | `=~` regex word with unquoted `\|` + reserved word (`case`/`if`) in `while`/`until` condition — #322 residue: `while [[ x =~ x\|case ]]; do` | irrequietus/shellapi odsel.shellapi.bash:59 |
| [#462](https://github.com/unixwin/rubash/issues/462) | function-definition name word containing unquoted `$( )`: `f$()g() { :; }` (quoted `f"$()"g()` passes) | gingerpaledale/zZshFramework files.sh:5 |
| [#463](https://github.com/unixwin/rubash/issues/463) | word-attached multi-line process substitution `word<(` NL … NL `)` (space-separated multi-line passes) | h4l/json.bash examples/jb-cli.sh |
| [#465](https://github.com/unixwin/rubash/issues/465) | compound-group state (`[[ ]] && { (( )) && { cmd <<< "" } }`) leaks into a later nested `function NAME {` definition → EOF from `{` | Klapptnot/dotf-old mirkop.sh:265 |

GNU C anchors per issue are in the issue bodies (parse.y line numbers against
the vendored tree @b4608166): #460 → if_command/compound_list/list1 →
pipeline_command + reserved_word_acceptable (COND_END); #461 → PST_REGEXP
folding at read_token_word (`(`,`|` special-case) + `goto tokword`; #462 →
`function_def: WORD '(' ')'`; #463 → read_token procsub fallthrough into
word-reading; #465 → group_command + function_def state separation.

## Harness-class detail (no engine bug)

The 40 ConPTY-stdin hangs include the wt97b-known set (bats-format-cat,
lintorama-stop.sh, statusline.sh ×2, bash-cat-with-cat cat.sh, doitlive
walkthrough) plus the pash test six-pack (`cat $IN | …` with `$IN` unset →
bare `cat`), shellspec pretty.sh (`eval "__dummy__() { $(cat -) }"`),
json.bash trap_bench (`readarray`), mForce "Paste your code below" read
loops, Astro menu scripts, and terminal-query scripts (stty `\033[6n` cursor
reads — D1665.bashrc2). None completes differently from GNU in an
interactive terminal; none shows a semantic wedge: every one finishes ≤2s
under `rubash file </dev/null`.

No rubash-only execution wedge was found in the HANG population on this
build — the two wedges wt97b filed (#435/#436) are fixed, and #425 no longer
reproduces.

## Reproduction pointers

- Fresh retest verdicts: `target/goldtri/retest-results.jsonl` (89 rows)
- Non-interactive discriminators: `target/goldtri/nonint.json`
- Content-shape analysis: `target/goldtri/hang-shapes.json`
- Minimal repros: `target/goldtri/repro/`, `mirkop-min.sh`, `shrink.sh`
- Triage join (all 139 hashes): `wt98b-goldtri-triage-results.json`

No product code was changed in this lane.
