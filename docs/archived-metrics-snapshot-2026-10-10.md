# 归档基准数字快照（PERF / CORPUS / COMPAT / LINUX）

> **性质声明：本文是历史快照的摘录，不是当前承诺。** 文中数字各自截止于标注日期
> （2026-09-27 至 2026-10-02），引用时必须带上"截至 <日期> 快照"限定语。
> 现状口径一律以 issue tracker、README「Compatibility at a Glance」台账与对应脚本
> （`scripts/run-perf-suite.sh`、`scripts/true-baseline.sh`）实测为准。
>
> 来源：`docs/PERF-BASELINE.md`、`docs/CORPUS-COVERAGE.md`、`docs/COMPATIBILITY-STATUS.md`、
> `docs/LINUX-RUN-STATUS.md` 等 7 个点对点状态文档已于 2026-10-10 归档删除
> （PR #478，commit `47440833`）。全文随 git 保留，取阅：
> `git show 47440833^:docs/PERF-BASELINE.md`（其余同理）。本文件即
> `docs/bash-compat-issues.md` 等处"issue #477 snapshot archive"指针的落点。

## 1. 性能基线：首轮 24 探针（2026-09-27，基线 commit `9bf2df9e`）

来源：归档 PERF-BASELINE.md 首轮表。环境：rubash debug `target/debug/rubash.exe`
vs WSL GNU Bash 5.3.0（`/usr/local/bin/bash`，业主编译版；WSL2 kernel
5.10.16.3-microsoft-standard-WSL2）。

方法（仍然有效）：外层 harness `scripts/run-perf-suite.sh`；wall clock
`date +%s%N`，取 median；每次计时 run 包 `timeout -k 5`；GNU 侧 WSL 内 `--inner`
重入，PATH 净化 `PATH=/usr/local/bin:/usr/bin:/bin`、`HOME=/tmp/rubash-perf-home.$$`；
不用 hyperfine。TSV 落 `target/perf-results/`（可断点续传，外层 harness 会重试不完整的
GNU pass）。已知偏差：msys `timeout.exe` 的常量 spawn 开销抬高 rubash 短探针
（01/02 约 10-20 ms）——启动比值略被高估，全部比值偏保守。

### 首轮中位表（median ms；ratio = rubash/GNU；粗体 = >=10x）

| probe | shape | rubash ms | GNU ms | ratio | status |
|---|---|---:|---:|---:|---|
| 01-startup-empty | startup floor (`exit 0`) | 92 | 7 | **13.1x** | OK |
| 02-startup-fndef | startup + 1 fn def/call | 89 | 7 | **12.7x** | OK |
| 04-loop-true-builtin-x2000 | builtin `true` loop | 1048 | 20 | **52.4x** | OK |
| 05-arith-x5000 | `(( i += 1 ))` loop | 648 | 13 | **49.8x** | OK |
| 06-strconcat-x5000 | `s+=x` loop | 1915 | 25 | **76.6x** | OK |
| 07-fncall-noop-x5000 | no-op fn call loop | 3480 | 37 | **94.1x** | OK |
| 08-cmdsub-true-x1000 | `x=$(true)` loop | 477 | 382 | 1.2x | OK |
| 09-external-uname-x300 | external spawn loop | 316 | 215 | 1.5x | OK |
| 10-pathmiss-x100 | `command -v` miss loop | 261 | 24 | **10.9x** | OK |
| 11-pipeline-yes-head | `yes \| head -100000` | 221 | 8 | **27.6x** | OK |
| 12-pipe-echo-read-x2000 | `echo \| while read` x2000 | 5873 | 1030 | 5.7x | OK |
| 13-readloop-gen-x2000 | gen \| while read | 2549 | 24 | **106.2x** | OK |
| 14-glob-srcrels-x100 | `src/*/*.rs` glob loop | 3365 | 43665 | 0.1x | OK（env-bound，见归档注） |
| 15-expansion-x5000 | `${a#..}`/`${a%%..}` mix | 4492 | 59 | **76.1x** | OK |
| 16-parse-flat8000 | exec 8000 assignments | 2219 | 18 | **123.3x** | OK |
| 17-parse-flat8000-n | `-n` parse only | 510 | 13 | **39.2x** | OK |
| 18-nested-brace-nst1-d200 | one-line nested braces D=200 | 579 | 6 | **96.5x** | OK |
| 19-nested-brace-nst2-d200 | two-line nested braces D=200 | 91 | 6 | 15.2x | RC2（rubash 解析拒收，后续修复） |
| 20-as-fn-mkdir-p-rep40 | autoconf unit x40 defs | 521 | 8 | **65.1x** | OK |
| 21-configure-head1374-n | `-n` bash configure head (1374 L) | 3211 | 9 | **356.8x** | OK |
| 22-configure-full-n | `-n` full bash configure (24753 L) | >120000 | 46 | >2600x | TIMEOUT（rubash，当时） |
| 23-nvm-parse-n | `-n` nvm.sh v0.40.8 | 18748 | 22 | **852.2x** | OK |
| 24-nvm-load | source nvm.sh v0.40.8 | 14402 | 38 | **379.0x** | OK |
| 25-yes-head-read | `yes \| head \| while read` | - | 650 | - | 当时 TIMEOUT；#243 已修正为非挂起（#206 形态） |

### 当时的读数（历史结论，勿当作现状）

- 近平价在子壳/进程创建（08=1.2x、09=1.5x）：成本集中在每命令解释器工作，不在进程创建。
- 12（echo\|read 5.7x）vs 13（read loop 106x）：内建管道本身仅 ~6x；read builtin /
  simple-command dispatch 是放大器。
- probe 03（`$SH -i -c exit`）退役：GNU bash 在 WSL 无 tty 交互调用永挂，环境绑定不可比。
- probe 25 非挂起：#243 修正。
- probe 14 env-bound（GNU 侧 43.7 s，宿主 glob 差异），非引擎可比项。

## 2. 后续轮次与优化结论（摘自 PERF-BASELINE.md 各轮小节，均截至归档时）

- perf7/perf10 轮后：nvm `-n` 315.4 → 230.8 ms（**-26.6%**）；nvm load
  411.8 → 323.0 ms（**-21.6%**）；最终洁-binary 复验 `-n` **-24.7%** vs master，
  canary 12/12。
- 历史轨迹（nvm `-n` 比值）：852x → 719.8x → 644.9x → 137.2x → 65.7x
  （各轮原文见归档），对应 #241/#281 车道。
- store 三克隆评估：`store_self` = 3.1-3.3 ms / nvm `-n`（6,723 词，约
  470 ns/word），上限约 1.3% 墙钟——**REJECTED**（低于诚实门槛），思路归入
  token 架构大轮。
- body-reparse 缓存：实测函数体/循环体均单次解析，文本键缓存零命中——
  **REJECTED**；结构性修法指向 parse20 inline-section 扩到
  case/for/loop/brace/subshell/function 六处。
- 归档后的 perf 落地（2026-10-10/12 合并，数字以 PR 与 tracker 为准，本文不续写）：
  #486（#281 opener-state checkpoint）、#501（#242 hot-path residue）、
  #484 + #562（#375 dispatch-floor）、#554（#241 parse-throughput wins）。

## 3. 语料覆盖：防重跑清单（2026-09-27 建立，摘自 CORPUS-COVERAGE.md）

已覆盖语料（**勿重跑**；逐项 issue/工件路径见归档原文表格
`git show 47440833^:docs/CORPUS-COVERAGE.md`）：

- GNU 83 套件全量（true-baseline；09-25 口径 82 零差 / 1 残余；工件
  `target/issue-suites/results/true-baseline*`；2026-10-12 复核 83/83，PR #506）
- oils spec / mksh check.t / ksh93 / busybox ash（#20–#24, #37–#57）
- linux-smoke（#138–#140）、bash-completion 2.18.0（#138 #144 #146 #147 #153）、
  git-completion.bash（#142）
- oh-my-bash 全仓 + 主题矩阵（#143 #148 #149 #160）、bash-it 全仓（#161）
- 版本管理器全家：nvm（#130 #155 #162 #169）、rustup（#205）、rbenv/jenv
  （#166 #171 #184 #191 #193）、ruby-build（#171 #185）、pyenv（#154/#171 级联
  已覆盖）、conda、sdkman、gvm、asdf、volta（ecosys5）
- starship（#202）、fzf（0 diff）、atuin/direnv-stdlib/zoxide（#202/#203 族）
- 安装器：get-docker / deno / bun / starship install（env-bound，无 rubash 因素）
- configure 巨物：bash53 (24,753 L) / git / ltmain.sh / cmake bootstrap /
  php-8.3.16（#175 #177 + perf #155 #130 #176 #178）
- Homebrew install.sh（#166 族）、MSYS2/GfW profile 链（#163）、WSL profile.d（0 diff）
- makepkg（#165 #166）、shellspec 自测（#167 #166）、bats-core 自测（#172 #166）、
  portage ebuild（#174）、bashdb
- mvdan/sh（shfmt）解析语料：559 snippets / 16 divergences（归档原文 §2）
- 主题 sweep（83 主题，OMB 全仓）：load 60 PASS / 4 FAIL-engine（#416 #417 #418）
  / 14 FAIL-env / 2 FAIL-env-suspected / 3 VOLATILE；render（82）18 EQUIVALENT /
  64 DIVERGENT（33 wrap/width → #420；22 redraw-glyph → #422；其余见归档原文）

### 工具注意事项（仍然有效）

- pyte 0.8.2 吃 SOH 后文本：喂 pyte 前先从 rubash 非 tty 提示符流剥离 SOH/STX。
- pywinpty ConPTY 默认回显输入行：reader 须关 echo（relay 用 `read -rs`），
  否则屏幕出现幻影行。
- WSL `bash -i` 非 tty 父进程需 `setsid`（新进程组）。
- GNU 侧"稳定性"重跑必须换新 HOME：OMB 主题在 `$HOME` 缓存随机态，制造假确定性。
- ConPTY 下 rubash 走占位 REPL（#419）：render 探针必须管道 stdin（relay）才能
  到达 PS1 渲染 reader。

## 4. 兼容性台账末次口径（摘自 COMPATIBILITY-STATUS.md）

**最后核对 2026-10-02**（wt42/rebaseline 全量重基线车道，master `dc3e799b`）：
**63 零差 / 20 有 DIFF / 285 原始行**（83 套件 true-baseline）。
更早口径：2026-09-25（master `71c933eb`，env 归零）82 零差 / 1 残余 / 83 套件。

⚠️ **该口径在归档时即已过时**：其后 #453–#476、Wave1–4b（#552–#579）等修复已落地；
2026-10-12 复核为 GNU 83/83（PR #506）。现状一律以 issue tracker 与
`scripts/true-baseline.sh` 实测为准；现役权威是 README「Compatibility at a Glance」
台账（PR #502 已把旧文档引用指向该处）。

## 5. Linux 原生（摘自 LINUX-RUN-STATUS.md，2026-09-27，lane wt4/linuxrun）

- 结论（当时）：**Linux target 真构建真运行**——`x86_64-unknown-linux-gnu`
  冷构建 59.7 s 绿；ELF 作为脚本 shell 的冒烟矩阵（smoke_basic/loops/funcs/
  pipes/glob/arith/subst + p0 系列）逐字节对齐 WSL GNU Bash 5.3.0（两侧同 WSL
  实例对跑，零 Windows 噪声）。基准 `9bf2df9e` + 车道三修复；原始工件
  `target/wslrun/`。
- 当时剩余：trap/redir 套件级挂起（SA_RESTART 信号后端使其不可杀故可见）、
  locale 排序 glob、小诊断形态差（`kill -l 32` #230、wait race #228、
  signal-death notices #229）。
- 后续提交已部分修复；现行口径以 README 与 tracker 为准。

---

*摘录自 #477（2026-10-10 docs 审计）。归档原文 7 件：PERF-BASELINE、
CORPUS-COVERAGE、COMPATIBILITY-STATUS、LINUX-RUN-STATUS、QUEUE-PROMPTS、
TASK-BOARD、TASKBOARD，均在 `git show 47440833^:docs/<NAME>.md`。*
