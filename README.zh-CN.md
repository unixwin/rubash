<p align="center">
  <img src="assets/rubash-lockup.svg" alt="rubash — 可嵌入的 bash 兼容引擎" width="620" />
</p>

# Rubash

**用 Rust 从零实现的 GNU Bash 5.3 兼容 shell 引擎。** 可嵌入为库，Windows 原生运行、**无 POSIX 模拟层**——它是 [niubash](https://github.com/unixwin/niubash) 的引擎，也是 Git Bash / MSYS2 移植版 bash 的从零替代。

[English](README.md) · [unixwin.dev](https://unixwin.dev)

[![CI](https://github.com/unixwin/rubash/actions/workflows/ci.yml/badge.svg)](https://github.com/unixwin/rubash/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/unixwin/rubash)](https://github.com/unixwin/rubash/releases)
[![Rust Version](https://img.shields.io/badge/rust-1.70+-blue)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

Rubash 是用 Rust 对 GNU Bash 语义的从零重实现——词法分析器、解析器、展开引擎、执行器、内建命令——以**可嵌入的无头引擎**形态交付。目标是与 GNU Bash 5.3.x 逐字节兼容（vendored 上游树已到 Bash-5.3 patch 20 层级，#957），原生运行在 Windows、Linux 与 macOS 上。

Rubash 本身不是 shell 产品。它自带一个参考 CLI（供兼容性 harness 和工具链使用），而交互式 shell 构建于引擎**之上**：niubash 嵌入 Rubash 承担全部 bash 语义，自己只负责行编辑、prompt 渲染与补全。

## 快速开始

### 获取

每个 [release](https://github.com/unixwin/rubash/releases) 附 Linux（x86_64）与 macOS（aarch64）预编译 tarball（最新：v1.4.3，截至 2026-10-30）。Windows 上引擎随 [niubash](https://github.com/unixwin/niubash) 分发——要装的是那个二进制；本仓不单独发 Windows 二进制。

### 从源码构建

> 跨平台：Windows 是全栈主平台；Linux 原生构建并运行，带 CI 测试车道；macOS 在 CI 跑库测试套件（见「为什么是 Rubash」下的平台状态）。

```bash
git clone https://github.com/unixwin/rubash.git
cd rubash
cargo build
target/debug/rubash --version
```

### 运行脚本

```bash
target/debug/rubash path/to/script.sh
target/debug/rubash -c 'echo hello from rubash'
```

### 运行兼容性测试套件

```sh
# GNU 上游语料回放（CI 车道运行器，Linux）：
scripts/run-bash-upstream-tests.sh        # 写 target/bash-upstream-tests/summary.md

# 全量 83 套件 true-baseline 测量（台账背后的 harness；
# Windows 侧用法照 scripts/true-baseline.sh 文件头）：
MSYS_NO_PATHCONV=1 wsl bash /path/to/rubash/scripts/true-baseline.sh          # 全部 83 套
MSYS_NO_PATHCONV=1 wsl bash /path/to/rubash/scripts/true-baseline.sh array    # 单个套件
```

## 为什么是 Rubash

**实测而非宣称。** 这里的兼容性靠三把机械的尺子，不靠形容词：

- **自研差分套件**——246 例原创用例对本地自编译的 GNU Bash 5.3.20 逐字节比对（stdout、stderr、退出码）：**244/246 一致**，较首次审计（2026-10-22）的 222/246 上行。剩余 2 例是已记录的平台性差异。
- **GNU 官方语料**——以 GNU 自家 83 套上游测试集为一致性标尺；完整台账见下。
- **真实生态**——nvm、bats-core、bash-it、oh-my-bash、bash-completion、modernish 逐项真跑。每处分歧立公开 issue 并固化成回归测试：截至 2026-10-30，rubash 累计 1,440 个 issue、已关 1,408。

**原生的意义。** Git for Windows 对自己 shell 的定性原话是「a BASH emulation used to run Git from the command line」——为跑 Git 而设的模拟。它的 bash 是移植版，骑在 POSIX 模拟层（`msys-2.0.dll`）上：fork 模拟、参数改写、设备怪癖，全都渗进每一个脚本。Rubash 没有这层：一个自包含二进制，直接对话 Win32。

**路径是一等公民，一个子进程一种方言。** MSYS 的模型是*猜*哪些参数像路径然后改写；MSYS2 官方文档自承这种自动转换「并不完美」，会改写根本不是路径的参数——这就是为什么每个 AI agent 和脚本都得设置 `MSYS_NO_PATHCONV=1`，防止 `/flags` 被改成 `C:/Program Files/Git/flags`。Rubash 用**逐子进程方言契约**把模型反过来（与 GNU 画的是同一条线：`shell_execve` 把原始词字节原样交给 `execve`，从不改写 argv）。自带 POSIX 层的子进程——凡是从 WinuxCmd 安装解析出来的、或经 `winuxcmd` 分发器解析的——保留 shell 拼写，`/tmp`、`/dev/*` 与 shell 树由它自己解析；一处有据可查的例外：盘符形态操作数（`/d/...`）由引擎漏斗对所有子进程类统一翻译（[#592](https://github.com/unixwin/rubash/issues/592)）。原生 Windows 程序则拿到经分类的路径形操作数的真实 Win32 拼写——虚拟根、`/mnt/X` 与带分量的盘符形态（如 `/d/repo`），目标存不存在都翻译；而有据可查的 verbatim 类按原样过界：裸 `/`、盘根形态（`/d`、`/d/`）、附着值（`--opt=/d/x`）与语法数据操作数（[#748](https://github.com/unixwin/niubash/issues/748)、[#779](https://github.com/unixwin/niubash/issues/779)）。契约正本见 niubash 的 `docs/src/windows-path-contract.md`。翻译面刻意收窄：引号参数永不改写，唯一的设备翻译是精确 token `/dev/null` → `nul`（[#1506](https://github.com/unixwin/rubash/issues/1506)）。历次被套件抓到的过度翻译（[#31](https://github.com/unixwin/rubash/issues/31)、[#59](https://github.com/unixwin/rubash/issues/59)、[#60](https://github.com/unixwin/rubash/issues/60)）全部立公开 issue 并固化回归。紧急回退：导出 `__RUBASH_ARGV_DIALECT=legacy` 可恢复 Option-B 之前的翻译行为。

**Bash 5.3，不是 5.2。** Git for Windows 捆绑的是 Bash 5.2 系。rubash 以 5.3 语义为目标，包括 5.2 里结构性不存在的语法——如无 fork 命令替换 `${ cmd; }`（[#195](https://github.com/unixwin/rubash/issues/195)）。

**无 fork 的子壳。** `( list )` 与 `$( )` 在**进程内**运行——零进程创建、零 fork 模拟——而脚本可观测的一切（退出码、fd 继承、共享文件偏移、信号处置）都与 GNU Bash 一致。三层设计见[架构](#架构)。

**为可嵌入而设计。** Rubash 是带参考 CLI 的 Rust 库，不只是一个可执行文件：niubash 今天就把它进程内链接承担全部 shell 语义，1.5.0 线（见[路线图](#路线图)）把这面正式化为稳定、带版本号的引擎 API。

**平台状态——跨平台、Windows 优先。** Windows 是主战场，拥有完整技术栈：单一自包含二进制直面 Windows 的 bash 痛点（无 POSIX 模拟层、无路径转换猜测、无需 MSYS_NO_PATHCONV、原生 Win32 路径为通货、为 bash 生态提供 MSYS2 兼容身份）。Linux 原生构建并运行（x86_64-unknown-linux-gnu）——真 getrlimit/chmod/faccessat/uname(2) 语义与信号投递——CI 在 Ubuntu 上跑引擎测试车道（库测试、兼容性 `cli_tests`、marker goldens），并且每次推送都回放 GNU 上游语料（`Bash upstream test progress` job，`scripts/run-bash-upstream-tests.sh`）；更早在 Linux 二进制上人工跑同一语料的成绩为 17/24 套字节一致（2026-09-27，台账存档于 #477/#581），剩余差距在 issue tracker 跟踪。macOS 在 CI 跑库测试套件（`macos-latest`），另有 cross-target check。引擎语义模型刻意平台中立，账本随行。

## Rubash vs Git Bash / MSYS2

| | **Rubash** | **Git Bash / MSYS2** |
| --- | --- | --- |
| 实现 | Rust 从零重实现 bash 语义，直对 Win32 | GNU Bash 移植到 `msys-2.0.dll` POSIX 模拟层之上 |
| Bash 语言层级 | **GNU Bash 5.3** 语义——含 5.3 独有的无 fork 命令替换 `${ cmd; }`（[#195](https://github.com/unixwin/rubash/issues/195)） | GNU Bash **5.2**（Git for Windows 捆绑版本） |
| 参数路径处理 | 确定性的逐子进程方言契约——按子进程类翻译经分类的拼写、有据可查的 verbatim 类原样过界；引号参数永不改写；不靠猜、没有总开关变量 | 对路径形参数的启发式改写——其官方文档自承「并不完美」；`MSYS_NO_PATHCONV` / `MSYS2_ARG_CONV_EXCL` 是官方给的逃生舱 |
| 子壳与命令替换 | 进程内——零进程创建 | 运行时内的 `fork()` 模拟 |
| 控制台设备重定向 | 与 GNU Bash 一致——引擎把 `CON` 拆为 `CONIN$`/`CONOUT$`（[#1312](https://github.com/unixwin/rubash/issues/1312)、[#1314](https://github.com/unixwin/rubash/issues/1314)） | 在 MSYS 层直接以 `Invalid argument` 失败（项目自测口径） |
| 给原生程序的设备路径 | 原生程序参数中的 `/dev/null` 统一译为 `nul`，与同一 argv 的其余部分一致（[#1506](https://github.com/unixwin/rubash/issues/1506)） | 在模拟层内部翻译 |
| 可嵌入性 | Rust 库；宿主进程内链接引擎（niubash 是参考宿主） | 独立可执行文件；无可嵌入引擎 |

在位三家里的 **Cygwin** 是同一种架构的自我定义：其官网称核心是一个「提供大量 POSIX API 功能」的 DLL，并明说应用「必须从源码重编」才能在它的世界里运行。源码重编出来的平行世界，隔着一整层翻译——rubash 是原生构建，没有这层。

## 同一把尺子，量一量其他宣称者

自称 bash 替代的 shell，值得用同一把尺子量。Rust 时代的同行里：hush 是一门基于 Lua 的*新语言*，从没自称 bash 替代；Oils/OSH 自称「从 bash 升级出去的路径」，不是无感 drop-in——这两家都没宣称我们要量的那件事。**brush** 宣称了：「Run it as your everyday shell」，其兼容页自评「Production-ready for most use cases」，同时把约 125 条已知失败定性为「mostly edge cases」。用我们的 306 例差分语料（对 GNU Bash 5.3.20 逐字节比对）实测：brush 发行版 v0.4.0 是 **181/306**；其 master 最新提交源码构建复测 **183/306**，同一失败剖面。rubash 在同一语料上是 **304/306**。差距的形状比数字更说明问题（最小复现见宣传线证据文件）：

- alias 展开时机模型与 GNU 相反：非交互未开 `expand_aliases` 也展开、定义同行即生效、函数体内按调用时展开——20 个专项探针 6 条实质分歧。alias 是 GNU 语义里最简单的机制之一，时机模型反了，真实 `.bashrc` 写法会静默走样；
- `printf x y`（无转换符格式串）在发行版上无限循环，直接挂死脚本（master 已修复此条）；
- 脚本模式下作业控制是空壳：`wait $!` 报「not yet implemented」，29 例作业用例挂 27；
- 信号 trap 不触发、子 shell 的 EXIT trap 丢失；globstar 的 `**` 不递归——而其兼容页把 globstar 列在「Fully Supported」；
- nameref 半成品：`unset` nameref 报未实现，真实生态（oh-my-bash 加载）正好撞上；
- 退出码与诊断文案全线自家格式（rc 99、管道里带 ANSI 颜色码）——按退出码与 stderr 做决策的程序和 agent 会被静默误导；
- 它主打的平台缺席：Windows 支持自述为「Preview」，v0.4.0 发行包根本没有 Windows 二进制。

轻量不是残疾的遮羞布。rubash 的 385,618 行（静态审计口径，2026-10-25）——连同 niubash 的 89,367 行共 474,985 行——是两件巨量工程的完工价：不用 fork 逐路径复刻完整 bash 语义，与跨平台落地本身。以约五分之一的引擎体量宣称同一件事、又过不了这些基础机制的实现，既无法成为 bash 的替代产物，也无法成为可靠的登录态 shell。

## 兼容性一览

```
GNU Bash 5.3.x 测试套件 — 83 个文件，true-baseline 实测
（台账：2026-10-12 全量重跑，master 764b9dde，已剥离宿主 WINUXSH_ROOT
 钉扎——无 upstream 脚本桩、niu 挂载 /bin/sh 夹具、逐套件 TMPDIR 隔离、
 前台进程组超时；环境绑定差异经逐行审计后归零计；更早切片：
 2026-09-25，71c933eb）

  通过：          83/83 套件  ██████████████████████████████  100%
  ────────────────────────────────────────────────────────────────
  原始值：        62/83 套件 Windows 上 stdout 逐字节一致——
                 字面 0 差异的运行，scripts/true-baseline.sh
  环境归类：      +20 套仅在环境绑定行上有差异（宿主工具输出、
                 NTFS 文件名/属性位、locale、/dev/tty、路径形式、
                 信号表、coproc 收割时序）——台账逐行归因见下
  净化值：        82/83 套经逐行环境审计后无 rubash 引起的差异
  nameref：       1 行（coproc 后 declare -r 可见 RO_PID）——证实为
                 GNU 5.3.0 自身 coproc 收割时序竞争：GNU 自我对跑在
                 harness 文件系统上 59/60↔1/60 翻转，ext4 上 60/60
                 确定性且与 rubash 输出逐字节一致（stdout 与 stderr）
                 ——docs/nameref-suite-residual-gnu-race-20261012.md
  9 月 9 日为 3427 行 → −99%+
```

2026-10-12 全量重跑早于 v1.4.2 tag。

### 完全通过的套件（零差异，环境差除外）

净化套件集派生自台账的逐套件分类，并非手工维护：仅当某套件的每一差异行都归因于环境（按 AGENTS.md 的 rubash 引起 vs 环境引起划分，并对应覆盖 issue）时才列入此处。上方原始计数（62）+ 环境归类集（20）+ nameref 已证实的 GNU 竞争 = 全部 83 套。

`alias` `appendop` `arith` `arith-for` `array` `assoc` `attr` `braces` `builtins` `case` `casemod` `complete` `comsub-eof` `comsub-posix` `comsub` `comsub2` `cond` `coproc` `cprint` `dbg-support` `dbg-support2` `dstack` `dstack2` `dynvar` `errors` `exp` `exportfunc` `extglob` `extglob2` `extglob3` `func` `getopts` `glob-bracket` `glob` `globstar` `heredoc` `herestr` `histexp` `history` `ifs-posix` `ifs` `intl` `invert` `invocation` `iquote` `jobs` `lastpipe` `mapfile` `more-exp` `new-exp` `nquote` `nquote1` `nquote2` `nquote3` `nquote4` `nquote5` `parser` `posix2` `posixexp` `posixexp2` `posixpat` `posixpipe` `precedence` `printf` `procsub` `quote` `quotearray` `read` `redir` `rhs-exp` `rsh` `set-e` `set-x` `shopt` `strip` `test` `tilde` `tilde2` `trap` `type` `varenv` `vredir`

（第 83 套 nameref 见上方台账——其唯一残差行经逐字节证实归因 GNU 自身 coproc 收割竞争，见 `docs/nameref-suite-residual-gnu-race-20261012.md`。）

### GNU 套件之外——混合评测套件与真实生态

同一 true-baseline 方法（脚本文件、对 GNU 5.3.x 逐字节 diff）也用在专门评测套件与真实语料上（完整清单与裁决归档于 issue #477）：

| 面 | 结果 |
| --- | --- |
| **modernish** 能力套件（跨 shell 怪癖探针） | **166/166 rc 向量一致** |
| **mvdan/sh 解析语料** | 559 个片段，操作符类修复后 parse-accept/exit 对齐 |
| **ble.sh**（29,601 行行编辑框架） | `bash -n` **rc=0 对齐**；source 模式可终止 |
| **bash-completion** | 452/452 矩阵；`bash -n` 金丝雀逐字节一致 |
| **git-completion** | 140/140 |
| **oh-my-bash** | 22 个 lib 加载，**252/252 函数**环境对齐，PS1 逐字节一致（5 款主题） |
| **nvm.sh**（v0.40.x loader） | 加载逐字节干净，含 Windows 原生空 `NVM_DIR` 自动使用（`$PWD` POSIX 人设，#537）；自有测试切片的引擎侧失败已修 |
| **FFmpeg / OpenSSH / PHP / ltmain / cmake configure 脚本** | `--help`/错误路径输出**逐字节一致** |
| **git 自家测试框架**（t/test-lib.sh） | 在引擎下端到端运行 |
| **GNU 83 套件 · Linux 原生二进制** | 17/24 套字节一致（2026-09-27 wt4/linuxrun 车道，台账存档于 #477/#581）；该语料现由 CI 每次推送回放（`Bash upstream test progress`，Ubuntu） |

以上全部钉为回归夹具（`tests/` 下 906 个集成测试文件，截至 2026-10-30，含 `tests/regression.rs`——GNU golden 矩阵、套件切片快照、perf 金丝雀），CI 每次推送都跑——外加一道 C 源码逐行审计（`docs/SOURCE-AUDIT.md`，已盘点 336 个行为），总在套件踩到之前把缺口找到并关掉。

## 性能

实测而非宣称——同一规程、同一资产，好看难看都发。参考资产是 2026-10-25 基线（PERF-BASELINE-2026-10-25，存于公开的 [shellbench](https://github.com/unixwin/shellbench) 仓）：release 构建、Linux、对自编译 GNU Bash 5.3.20；仓内台账在 [`docs/performance.md`](docs/performance.md)，perf 金丝雀随 CI 回归套件跑。

**先说诚实结论。** 解释器受限的微观负载上，现行 release 构建对 GNU Bash 5.3.20 付一层近似均匀的 **~3× 地板税**——算术、函数调用、赋值、read 循环的单操作成本落在 2.8–3.2×。它是一个机制（dispatch 成本），不是几百个散点；「全表超越 GNU」是本项目公开的性能目标、是 1.5.0 之后旗舰版的硬门，对照的正是这份已发布的基线。这份基线里最差的单一形状 `$((a[i]))` 数组元素算术直读（约 11×）已收至 2.99×（2026-10-28 轮量测、同协议），回到地板量级。

**进程模型赢的地方。** 成本在进程创建上的负载正好反过来，因为管道段与命令替换在进程内执行：管道基准是 GNU 耗时的 **0.05×**，命令替换循环是 **0.11×**；shellbench 409 例广度表里，命令替换一类的中位比值是 0.538×。今日广度（408 例可比）：领先 53、接近 17、落后 338——落后集就是上面那层地板税，明说。

**启动——唯一可引用的口径。** `-c 'true'`，50 样本交错采样取 p50，release 构建，Linux：**rubash 3.43 ms，GNU Bash 5.3.20 2.59 ms**。Windows 面的启动数字在真机按同一规程复测之前，一律不引用。

## 路线图

以下两项均在公开 issue 与设计文档中定版，随 **1.5.0** 落地。

- **可嵌入引擎 API**（[#595](https://github.com/unixwin/rubash/issues/595)）：稳定的、带 semver 的 Session 级公开契约——输入注入、输出流、生命周期、事件回调、rc 策略——niubash 已全量迁移其上、为参考宿主。定版设计：[`docs/design/1.5.0-session-api.md`](docs/design/1.5.0-session-api.md)；RFC 草案 v1：[`docs/design/1.5.0-workspace-and-engine-api.md`](docs/design/1.5.0-workspace-and-engine-api.md)。
- **原生脚本调试器**（[#497](https://github.com/unixwin/rubash/issues/497)）：`rubash --debug`（交互式、gdb 文法前端，目标是替代 bashdb 工作流）与 `rubash --dap`（同一调试会话走 Debug Adapter Protocol，供编辑器）是同一引擎事件流的两个视图——引擎 API 的第二个真实消费者。定版设计：[`docs/design/debugger-full-design.md`](docs/design/debugger-full-design.md)；M0 设计：[`docs/design/debugger-m0.md`](docs/design/debugger-m0.md)。

## 里程碑

<p align="center">
  <img src="assets/milestone-timeline.svg" alt="The Climb——rubash 里程碑时间线：从 2026-09-09 的 3,427 行 GNU 套件差异到 2026-10-12 的 83/83 全数通过" width="880" />
</p>

从 `chore: init` 到 GNU 官方语料 83/83，历时四个半月——身后是钉进 CI 夹具的生态墙（bashdb、ble.sh、oh-my-bash、bash-completion、nvm、git 自带的 `t/test-lib.sh`）。两位核心维护者同时也是微软 Windows 版 Coreutils 打包项目 [microsoft/coreutils](https://github.com/microsoft/coreutils) 以及 vcpkg/winget-pkgs 的上游贡献者。

## 架构

```
src/
├── bin/             `rubash` 参考 CLI
├── lexer/           词法分析器（引号、转义、heredoc、续行）
├── parser/          递归下降（简单命令、管道、case、arith-for、[[ ]]）
├── expand/          展开引擎（参数、glob、花括号、tilde、算术）
├── executor/        命令执行、内建命令、分发、数组、trap、身份
├── builtins/        62 个引擎内建（declare、read、printf、kill、...）
├── fd/              fd 表与传输层
├── jobs/            作业控制、coproc、进程替换
├── shell/           ShellState、子壳克隆、初始化
└── lib.rs           核心类型和错误处理
```

- **词法分析器**：Bash 风格引号、转义、注释、变量、命令替换、算术展开、here-doc/here-string token、常见重定向。
- **解析器**：简单命令、管道、AND/OR 列表、函数、花括号/子 shell 组、`if`、`for`、算术 `for`、`while`、`until`、`case`、`select`、`[[ ... ]]`、`coproc`、`time` 前缀。
- **执行器**：外部命令、管道、重定向、临时赋值、函数调用、`source`/`.`、`eval`、无 shebang 脚本回退，以及逐子进程 argv 方言契约（POSIX 层子进程拿 shell 拼写，原生子进程拿经分类翻译的 Win32 拼写——见 `docs/windows-path-contract.md`）。
- **展开系统**：变量、位置参数、索引/关联数组、命令替换、算术展开、花括号展开、tilde 展开、路径名 glob、`${parameter...}` 操作符、大小写/替换变换。
- **内建命令**：62 个引擎内建——机器核对的清单在 [`docs/builtins.md`](docs/builtins.md)（由 `src/executor/builtin_names.rs` 上的测试守护同步），外加有意隐藏的 fast-path 内建（`sleep`、`dirname`、`basename`、`uname`、`arch`），让 `type`/`compgen -b` 照报外部命令，与 GNU 完全一致。

### 无 fork 的子壳语义

POSIX `fork()` 没有 Win32 等价物。仿真层（MSYS2、Cygwin）在系统调用层面硬造它——昂贵、脆弱，也正是它们最出名怪癖的来源。Rubash 改为在**语义**层面复刻 fork，分三层：

1. **进程内子壳**。`( list )` 和 `$( )` 从不创建进程。`ShellState::clone` 产出子壳的变量、别名、函数、trap 和历史——即 fork 的内存侧语义——子壳结束时副本直接丢弃。
2. **带 POSIX `dup` 语义的真实句柄 fd 表**。槽位持有原生 Windows `HANDLE`，而 `DuplicateHandle` 复制的句柄共享同一内核文件对象——因此共享同一文件偏移。这正是 POSIX "dup 共享 open file description" 的语义，落地前在 POC 中实证过。`fork_table()` 逐句柄复制整表，正如 `fork` 复制 fd 表而不复制文件对象。
3. **真实进程只出现在真实进程边界**。外部命令与管道成员走 `CreateProcess` + `os_pipe`（Unix 上原生 `fork`/`exec`）；后台作业与 coproc 拥有独立进程，作业控制基于 Job Objects，Windows 上 SIGCONT 通过 `ResumeThread` 投递。

结果：子壳与命令替换的创建成本为零，而脚本可观测的一切——退出码、fd 继承、共享偏移、信号处置——都与 GNU Bash 一致。

## 身份与兼容性（rubash#154）

引擎提供**可选的平台身份**，而不是把它外包给 PATH 上恰好躺着的那个 `uname.exe`。`uname` 与 `arch` 是引擎内建命令（完整选项解析，coreutils/MSYS2 输出形状），`OSTYPE`/`MACHTYPE`/`HOSTTYPE` 跟随同一人设。按 `case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*)` 或 `$OSTYPE` 分叉的生态脚本——包括会在陌生身份下直接拒跑的 bash-completion——走它们测试最充分的那条路，而不是直接退出。

- **默认人设：MSYS2 兼容**（Windows 默认；处处披露——这是兼容性面具，不是"MSYS2 移植版"的宣称。非 Windows 构建永远报诚实原生身份——MSYS 人设的存在意义是把 Windows 留在 MSYS/Cygwin 脚本生态里，其他平台不可达）：
  - `uname -s` → `MSYS_NT-<ver>`——设了 `MSYSTEM` 时按 MSYS2 运行时的同样映射规则给 `MINGW64_NT-` / `UCRT64_NT-` / `CLANG64_NT-` …；
  - `uname -m` / `arch` → `x86_64` / `aarch64`（构建架构）；
  - `uname -r` → Windows 版本串（`10.0-19044`，与 `uname -s` 结尾同一串）；
  - `uname -o` → `Msys`；`uname -a` → `sysname nodename release version machine Msys`（Git Bash 形状）；
  - `OSTYPE=msys`、`MACHTYPE=<arch>-pc-msys`（`set_if_not` 风格绑定，同 GNU variables.c:723-725——继承值优先）。
- **`RUBASH_IDENTITY=native` 切到诚实原生人设**：`uname -s` → `Windows_NT`（原生 `%OS%` 值），`uname -o` → `Windows`，`OSTYPE=windows`，`MACHTYPE=<arch>-pc-windows`。测试与原生优先的用户选这个；unset 该变量（或设为其他任何值）即回默认。

**披露面**（人设绝不沉默）：

- `rubash --help` 打印 Identity 段（当前人设 + 切换方法）；
- `rubash --identity` 打印人设与每个生效值（`uname -s/-m/-r/-o`、`arch`、`OSTYPE`、`MACHTYPE`）；
- 本 README 的这一节与仓库根 `SKILL.md` 的披露节（让消费本仓的 AI agent 也知道人设语义）。

`uname`/`arch` 是*隐藏的* fast-path 内建（同 `sleep`/`dirname`）：`type`/`enable`/`compgen -b` 照报外部命令，且 PATH 上一个 `uname.exe` 都没有时照样能跑。

## 测试

```bash
# 单元 + 集成测试（tests/ 下 906 个集成测试文件，截至 2026-10-30）
cargo test --lib
cargo test --test cli_tests

# 回归套件：GNU golden、生态快照、perf 金丝雀
cargo test --test regression

# bashdb 兼容性
cargo test --test cli_tests bashdb_compat -- --nocapture

# source 展开
cargo test --test cli_tests source_expands -- --nocapture

# 无 console 会话门禁（一个会话一个隐藏 console，零可见窗）
cargo test --test console_window_gate
```

引擎还可以端到端运行 [bashdb](https://github.com/Trepan-Debuggers/bashdb) 核心调试循环（list、step、next、where、continue、quit）。

兼容性由四道机械防线守住：

- **GNU 官方语料**——83 套上游测试套件，CI 每次推送回放，台账节律全量重测。
- **独立差分套件**——246 例手写用例对本地自编译 GNU Bash 5.3.20 oracle，stdout、stderr、退出码逐字节比对：当前 master **244/246**（首次审计 2026-10-22 为 222/246；残差立 issue 跟踪）。
- **回归矩阵**——每个已修 issue 在同一次变更里带一条以 issue 命名的回归测试，断言 GNU 行为。没有钉住的测试，修复不算完成。
- **生态夹具**——上方表格里的真实语料（oh-my-bash、bash-completion、nvm、git 的测试框架…）钉为 CI 夹具，语义变更弄坏生态时挂的是构建，不是用户。

## 文档

- GitHub issue tracker — **唯一权威来源**，Rubash ↔ GNU Bash 兼容性状态（快照台账已于 2026-10-10 归档至 issue #477）
- [`docs/PROVENANCE.md`](docs/PROVENANCE.md) — 来源声明：Rubash 与 GNU Bash 源码的关系，以及贡献者方法论规范
- [`docs/builtins.md`](docs/builtins.md) — 内建命令清单和分发模型
- [`docs/bashdb-debugging-rubash.md`](docs/bashdb-debugging-rubash.md) — bashdb fixture 设置和 smoke test
- [`docs/bash-upstream-tests.md`](docs/bash-upstream-tests.md) — 如何运行 GNU Bash 上游测试

## 开发原则

- 按 Bash 语义的 root cause 修 Rubash 子系统，不按单条 expected output 打补丁。
- bashdb 保持外部 clean 工具；临时 instrumentation 仅用于诊断。
- 每个失败的 bashdb 命令都是发现和修复 Rubash 兼容性缺口的机会。
- 兼容性基线为 GNU Bash 5.3.x——vendored 上游树（`third_party/bash`）在 Bash-5.3 patch 20 层级（#957），差分 oracle 为本地与 WSL 的 GNU 5.3.20 基线（#978）。

## 来源与实现方式

Rubash 是对 GNU Bash 语义的 **Rust 从零重写（rewrite）**——不是对 GNU Bash C 代码的移植或翻译。GNU Bash 的任何源码都没有被编译进、链接进或复制进 Rubash 自身代码。兼容性以 GNU Bash 5.3.x 的**可观测行为**为定义，并通过黑盒差分测试验证。vendored 的 GNU Bash 源码位于独立的 `third_party/bash` 子模块，保持其原始 GPL-3.0-or-later 许可证，仅作为语义参考与测试 oracle 使用。完整声明与贡献者规范见 [`docs/PROVENANCE.md`](docs/PROVENANCE.md)。

## 许可证

MIT — 详见 [`LICENSE`](LICENSE)。

## 贡献

欢迎提交 issue、兼容性复现、focused regression tests 和实现补丁。贡献前请阅读 [`AGENTS.md`](AGENTS.md)。

## 致谢

- GNU Bash 团队 — 参考实现，其可观测行为定义了我们的兼容性目标
- Trepan-Debuggers/bashdb — 外部调试器和兼容性压力测试
- Rust 社区 — 语言和工具链

---

*最后更新：2026-10-30*
