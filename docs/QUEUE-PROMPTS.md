# 排队提示词（交接给 ZCode 之外的 agent 执行）

> **使用说明（外部 agent 必读）**：
> 1. 仓库在 `D:/repo/rubash`（rubash 引擎）与 `D:/repo/niubash`（产品 shell）。每条提示词自包含，按顺序执行，一条一个工作树。
> 2. 开工前必读对应仓库的 `AGENTS.md`（红线：GNU C 源是规范、WSL GNU Bash 5.3.0 脚本文件为唯一 oracle、引用 `file:line`、禁白名单补丁、`src/lexer/continuation.rs` 船长专属禁改、禁 `git stash`、有界运行、结束时杀 rubash.exe 进程）。
> 3. 完成判据（四门禁+推送）：`cargo build` 0 新警告；`cargo test --lib` 全绿；`cargo check --tests --target x86_64-unknown-linux-gnu` 与 `--target aarch64-apple-darwin` 干净；`cargo fmt`；显式路径暂存（禁 `git add -A`）；提交到车道分支后 `git -c http.version=HTTP/1.1 push`；用 `gh` 关对应 issue 并附验证证据；更新 `docs/TASK-BOARD.md`。
> 4. 状态源：`docs/TASK-BOARD.md`（唯一权威任务板）+ `gh issue list --repo unixwin/rubash --state open`。上下文断了先读它们。
> 5. 派发与回收全程受末节《永久闭环调度协议》约束：发现即开单、套件记账防重复、一条一树、搁浅树巡检、清零不下工。

## Q1 — resid2 续作（5 单：#277 #279 #280 #282 #283）
前置：`git -C D:/repo/rubash worktree add D:/repo/rubash-wt-resid2 -b wt7/resid2 master 2>/dev/null; cp -r third_party/bash D:/repo/rubash-wt-resid2/third_party/`（若树已存在且有半成品，先 `git status/diff` 盘点、`cargo build` 验证，能编且对车道的保留，否则 `git checkout -- <文件>` 重置）。
```
Fix lane for rubash (Rust GNU Bash 5.3.0). Work ONLY in D:/repo/rubash-wt-resid2 (branch wt7/resid2). Read D:/repo/rubash-wt-resid2/AGENTS.md first. WSL GNU 5.3.0 script-file oracle (MSYS_NO_PATHCONV=1 wsl /usr/local/bin/bash /mnt/d/repo/rubash-wt-resid2/<probe>.sh); cite GNU file:line; no whack-a-mole; src/lexer/continuation.rs CAPTAIN-EXCLUSIVE (diffs in report); no git stash; bounded runs (timeout-wrap — #282/#283 involve hangs); kill stray rubash.exe at end.
FIX five issues (gh issue view <N> --repo unixwin/rubash each):
1. #277: printf suite residuals — %lc multibyte od dump form, %q quoting shapes, final diagnostic rc (4 stdout lines + rc vs GNU; builtins/printf.def + printf.c family; artifacts under D:/repo/rubash/target/issue-suites/results/).
2. #279: ble.sh --noattach source: rubash rc=0/empty vs GNU rc=1 'not an interactive session' (fixture tests/fixtures/corpus2/; bounded 100s).
3. #280: mkdir wildcard-named operand diagnostic missing the 'mkdir: cannot create directory' prefix (cosmetic).
4. #282: modernish bin/modernish init aborts in the fatal.sh battery (MSH_FTL_DEBUG prints nothing — root-cause why tracing itself is silent, then the abort; artifacts wt6-shell2 references in the issue).
5. #283: bats load.bats/tagging.bats hang inside bats' own machinery (gather-phase DEBUG-trace evaluation / nested run-capture) — root-cause the ENGINE-side hang; genuinely bats-internal parts document for closure. The date-with-colons log path is env-bound.
After fixes: printf/errors true-baseline slices (scripts/true-baseline.sh NAME); modernish .t re-run (expect 166/166 retained); bats filter.bats retained green; cargo test --test regression 24/24.
GATES: build 0 warnings; lib 486 green; regression 24/24; check --tests both targets; fmt; explicit staging. Commit in wt7/resid2 with citations. Report: per-issue root cause/citation/verification, modernish/bats scores, commit hash, any continuation.rs proposals verbatim.
```

## Q2 — niubash 交互式框架收尾（状态更新 2026-09-28：框架已建成并提交 f7a5b69 于 wt/interactive——20/28 过；续作=修 driver 的 ctrl_c 用例硬超时 kill、判别 #287 壳侧还是驱动侧、修 #286 或把两单转 rubash 修复车道；发现已开单 rubash#286/#287）
前置：树已存在于 D:/repo/niubash-wt-interactive（branch wt/interactive，含提交 f7a5b69）。
```
Test-infra lane for the niubash repo (interactive shell product; engine=rubash). Work ONLY in D:/repo/niubash-wt-interactive (branch wt/interactive). Read the repo's AGENTS.md/CLAUDE.md + tests/ layout first. MISSION: systematic INTERACTIVE-mode regression harness — Rust-native via the expectrl crate (pexpect port, Windows ConPTY) as a new tests/interactive.rs target wired into CI; keep the existing Python ConPTY scripts (scripts/test_setup_wizard_pty.py, ctrl_c_conpty_check.py) as one-off diagnostics, do not extend them.
HARNESS: small driver (spawn niu with a sized PTY, send line, expect regex with timeout, capture bytes); per-case timeouts (10s default, 30s heavy); deterministic env (fresh HOME temp dir, fixed TERM/NIU_* vars, no user rc); structural PS1 matching (theme escapes vary — assert echoed command + output, not full prompt bytes).
COVERAGE MATRIX (each a named test; skip-with-reason only where feature genuinely absent, then file the gap):
1. Continuation: unclosed "/'/{/(/backslash-newline/heredoc-in-progress → PS2 shown, completion on close, Ctrl-C abandons to PS1.
2. Termination: Ctrl-C on empty line / mid-typed / mid-continuation / mid-running external (status 130, prompt returns); Ctrl-D empty (exit) / mid-line (no exit) / with jobs (warning); Ctrl-Z suspend + fg/bg if job control exists (else gap).
3. Interactive-only expansion: history !!/!$ live, alias defined-then-used, set -H flip, PS1 re-render.
4. TAB completion basics, session history persistence.
5. Theme PS1 smoke (ANSI present).
6. Robustness: input bursts, multiline paste (bracketed paste or gap), COLUMNS resize no-crash.
Every divergence (crash/hang/wrong prompt/lost input): minimize, classify engine-vs-UI, file gh issue --repo unixwin/rubash for engine semantics; niubash-side gaps documented in the report.
GATES: cargo build --locked green; cargo test --workspace --locked green (known base failure script_file_while_read_redirect — note only, don't fix); the interactive target actually RUNS (if PTY unavailable in the environment, gate behind NIU_INTERACTIVE_TESTS=1 and verify with it set + give the exact command); fmt per repo; explicit-path commit. Report: harness design, coverage vs matrix, divergences + issue numbers, run command, commit hash.
```

## Q3 — resid3 新车道（#284 #285）
前置：`git -C D:/repo/rubash worktree add D:/repo/rubash-wt-resid3 -b wt8/resid3 master; cp -r third_party/bash D:/repo/rubash-wt-resid3/third_party/`
```
Fix lane for rubash (Rust GNU Bash 5.3.0). Work ONLY in D:/repo/rubash-wt-resid3 (branch wt8/resid3). Read AGENTS.md there. WSL GNU 5.3.0 script-file oracle; cite GNU file:line; no whack-a-mole; src/lexer/continuation.rs CAPTAIN-EXCLUSIVE (diffs in report); no git stash; bounded runs; kill strays.
FIX: #284 — f1=$(case z in esac) (empty case in comsub) dies 'unexpected end of file' while GNU yields an empty result rc=0 — the unclosed-comsub detection in the driver-side scanner family; if the fix belongs in continuation.rs, write the diff verbatim in your report and fix everything AROUND it (so the captain only applies one file). #285 — near-token syntax errors (e.g. '{ :; } }') echo a RECONSTRUCTED source line instead of the physical input line (parse.y:6813 print_offending_line prints the real line).
Verify: shape matrices vs GNU; comsub/errors true-baseline slices; cargo test --test regression 24/24.
GATES: build 0 warnings; lib 486 green; regression 24/24; check --tests both targets; fmt; explicit staging. Commit in wt8/resid3. Report: root causes, citations, matrices, any captain diff verbatim, commit hash.
```

## Q4 — 船长保留（勿派）：已全部完成
- ~~continuation.rs 零拷贝 diff~~ 已应用（15f94b9d）
- niubash↔rubash linux 交叉编译钉版本：若外部 agent 有余力可查（niubash 依赖的 rubash 在 x86_64-unknown-linux-gnu 下 4 个编译错；先看 Cargo.toml 钉的 rev 是否落后于 rubash master）

## Q5 — 全量回归轮（开放板清零后的收官门）
```
Full-regression audit for rubash at master. Work in D:/repo/rubash directly (read-only + report; no src edits without a filed issue). Run EVERYTHING and produce docs/FULL-REGRESSION.md:
(1) cargo build + check --tests both targets + fmt --check;
(2) cargo test --lib + --test regression + full cli_tests (bashdb fixtures per docs);
(3) scripts/true-baseline.sh for the 24 Linux suites (WSL build per docs/LINUX-RUN-STATUS.md) + Windows ledger suites, bounded sequential;
(4) scripts/run-perf-suite.sh full table vs docs/PERF-BASELINE.md (flag any ratio regression >20%);
(5) pinned corpora smoke: tests/fixtures/{corpus2,corpus3,ecosuite,audit} reproducers all green; ble.sh -n rc=0; modernish .t 166/166.
Every red = minimize + classify + gh issue (do not fix). This is the closure gate: green across the board = the round is done; single-page scoreboard at the end.
```

## 附录 — 永久闭环调度协议（派发方铁律）

> 本节约束派发方（船长/Devin/调度 agent）本身，与 TASK-BOARD「闭环规则」同源且更细；与单条提示词冲突时以较严者为准。核心：**不允许停止**——开放板清零前，每一轮必须以「再派发 / 复活 / 下潜」三者之一收尾。

1. **发现即开单，issue 生 issue（无限闭环）**。派遣、巡检、回收子 agent 的任何环节中暴露的分歧、回归、可疑行为、环境坑，一律当场 `gh issue create --repo unixwin/rubash`（niubash 侧开到产品仓），附最小复现 + GNU oracle 对比 + 疑似 owner 子系统；不许「先记着」。issue 里再发现 issue 是预期形态而非失败。开完单登记 TASK-BOARD 排队栏。
2. **套件记账防重复**。每条提示词的 GATES 与每份回收报告必须列出本次实际跑过的套件/切片/探针及结果（如 `true-baseline.sh comsub2`、`modernish .t 166/166`、`regression 24/24`、`run-perf-suite.sh` 比值表）。新语料登记 `docs/CORPUS-COVERAGE.md`，83 套件账本更新 `docs/COMPATIBILITY-STATUS.md`。派发前先查这两份 + TASK-BOARD 战绩栏，已测过的组合不重复派。
3. **一条一树，隔离 worktree**。命名 `D:/repo/rubash-wt-<lane>`（分支 `wt<N>/<lane>`），vendored 依赖按需拷贝（`cp -r third_party/bash`）；禁止两车道共树、禁止在主仓直接改（只读审计轮除外）；回收/废弃后 `git worktree remove` 释放。niubash 车道同规（`D:/repo/niubash-wt-<lane>`）。
4. **搁浅/外部 worktree 巡检**。每轮调度先扫 `git worktree list` + `D:/repo/*-wt-*`：有推进但长时间无心跳（无新提交、脏文件无新 mtime）→ `git status/diff/log` 盘点，能编则续作接管，半成品有价值则接收（cherry-pick/合入并过五门禁），无价值记录后清树；撞配额墙的树按续作协议复活（audit→build→续/弃），不按死树清；非 git 残留目录确认无独立产物后删除。
5. **清零后的下潜顺序（无事可做 ≠ 停工）**。开放板=0 时依次：(a) 挖新语料/新套件——先查 CORPUS-COVERAGE 缺席的真实生态（configure 族、rc 框架、包管理器、CI 脚本、`third_party/bash/tests/` 未跑项），跑出差异按第 1 条开单；(b) 性能轮——按 `docs/PERF-BASELINE.md` 最差比值逐个攻；(c) 全量回归轮（Q5 剧本）。
6. **兜底 = 性能**。连新套件也挖不到差异时专职性能（解析吞吐 / 执行器热路径 / 启动 / spawn），唯一约束零回归：每笔性能改动过五门禁 + `run-perf-suite.sh` 全表对比，单项比值回退 >20% 必须解释或回滚。
