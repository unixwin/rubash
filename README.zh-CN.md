# Rubash

用 Rust 从零实现的可嵌入 GNU Bash 兼容 shell 引擎。

[English](README.md)

[![CI](https://github.com/unixwin/rubash/actions/workflows/ci.yml/badge.svg)](https://github.com/unixwin/rubash/actions/workflows/ci.yml)
[![Rust Version](https://img.shields.io/badge/rust-1.70+-blue)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

## 什么是 Rubash

Rubash 是用 Rust 对 GNU Bash 语义的从零重实现，以**可嵌入的无头引擎**形态交付——词法分析、解析器、展开引擎、执行器、内建命令，全部重写。目标是与 GNU Bash 5.3.0 逐字节兼容，原生运行在 Windows 上。

Rubash 本身不是 shell 产品。它自带一个参考 CLI（供兼容性 harness 和工具链使用），而交互式 shell 构建于引擎**之上**：niubash 嵌入 Rubash 承担全部 bash 语义，自己只负责行编辑、prompt 渲染与补全。

**实测而非宣称**：兼容性用 GNU Bash 自己的 83 套上游测试语料验证——当前 81 套件逐字节一致，每一条残余差异行都经过逐行审计（台账见下）。

**原生的意义**：打着"Windows 上的 bash"旗号的方案（Git Bash、MSYS2）装的是移植版 bash，骑在 POSIX 模拟层（`msys-2.0.dll`）上——fork 模拟、路径翻译的怪癖会渗进每一个脚本。Rubash 没有这层：一个自包含二进制，直接对话 Win32。

**路径是一等公民，不是转换对象**：MSYS 的模型是*猜*哪些参数像路径然后改写——这就是为什么每个 AI agent 和脚本都得设置 `MSYS_NO_PATHCONV=1`，防止 `/flag` 被改成 `C:/Program Files/Git/flag`。Rubash 把模型反过来：Windows 路径是原生货币。POSIX 风格和 WSL 风格的路径都接受输入、解析成真实的 Windows 路径，原生 Windows 程序拿到的永远是合法的 Win32 路径——没有转换启发式、不需要 `MSYS_NO_PATHCONV`、进程边界零意外。

**平台状态**：当前阶段的中心是 Windows，也是唯一拥有完整技术栈的平台。macOS 与 Linux 适配在计划中，尚未开始。引擎的语义模型（进程内子壳、fd 表语义、进程边界）刻意保持平台中立，设计目标是让同一本 83 套件账本未来可以携带到其他平台。

## 兼容性一览

```
GNU Bash 5.3.0 测试套件 — 83 个文件，true-baseline 实测
（台账：2026-09-24，master 7b562024——无 upstream 脚本桩、
 niu 挂载 /bin/sh 夹具、逐套件 TMPDIR 隔离、前台进程组超时；
 环境绑定差异经逐行审计后归零计）

  零差通过：      81 套件  ████████████████████████████░░  98%
  残余差异：       2 套件  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░   2%
  ────────────────────────────────────────────────────────────────
  残余：          nameref 1（内部 RO_PID 泄漏进 declare -r）
                 trap    1（ERR trap 多触发一次，时序边界）
  归零的环境差：  glob extglob test type ifs-posix read coproc
                 nquote errors intl quotearray——NTFS 文件名/属性位、
                 locale、宿主工具输出、/dev/tty、路径形式
  9 月 9 日为 3427 行 → −99%+
```

### 完全通过的套件（零差异，环境差除外）

`alias` `appendop` `arith` `arith-for` `array` `assoc` `attr` `braces` `builtins` `case` `casemod` `complete` `comsub-eof` `comsub-posix` `comsub` `comsub2` `cond` `coproc` `cprint` `dbg-support` `dbg-support2` `dstack` `dstack2` `dynvar` `errors` `exp` `exportfunc` `extglob` `extglob2` `extglob3` `func` `getopts` `glob-bracket` `glob` `globstar` `heredoc` `herestr` `histexp` `history` `ifs-posix` `ifs` `intl` `invert` `invocation` `iquote` `jobs` `lastpipe` `mapfile` `more-exp` `new-exp` `nquote` `nquote1` `nquote2` `nquote3` `nquote4` `nquote5` `parser` `posix2` `posixexp` `posixexp2` `posixpat` `posixpipe` `precedence` `printf` `procsub` `quote` `quotearray` `read` `redir` `rhs-exp` `rsh` `set-e` `set-x` `shopt` `strip` `test` `tilde` `tilde2` `type` `varenv` `vredir`

## 架构

```
src/
├── lexer/           词法分析器（引号、转义、heredoc、续行）
├── parser/          递归下降（简单命令、管道、case、arith-for、[[ ]]）
├── executor/        命令执行、内建命令、展开、glob、数组、trap
├── builtins/        40+ 内建命令实现（declare、read、printf、kill、...）
└── lib.rs           核心类型和错误处理
```

- **词法分析器**：Bash 风格引号、转义、注释、变量、命令替换、算术展开、here-doc/here-string token、常见重定向。
- **解析器**：简单命令、管道、AND/OR 列表、函数、花括号/子 shell 组、`if`、`for`、算术 `for`、`while`、`until`、`case`、`select`、`[[ ... ]]`、`coproc`、`time` 前缀。
- **执行器**：外部命令、管道、重定向、临时赋值、函数调用、`source`/`.`、`eval`、无 shebang 脚本回退、Windows/Git Bash 路径桥接。
- **展开系统**：变量、位置参数、索引/关联数组、命令替换、算术展开、花括号展开、tilde 展开、路径名 glob、`${parameter...}` 操作符、大小写/替换变换。
- **内建命令**：`alias`、`cd`、`declare`/`typeset`/`local`、`echo`、`eval`、`exec`、`export`/`readonly`、`getopts`、`hash`、`jobs`、`kill`、`let`、`mapfile`、`printf`、`pushd`/`popd`/`dirs`、`read`、`return`、`set`、`shopt`、`source`、`test`/`[`、`trap`、`type`、`ulimit`、`umask`、`unset`、`wait` 等。

### 无 fork 的子壳语义

POSIX `fork()` 没有 Win32 等价物。仿真层（MSYS2、Cygwin）在系统调用层面硬造它——昂贵、脆弱，也正是它们最出名怪癖的来源。Rubash 改为在**语义**层面复刻 fork，分三层：

1. **进程内子壳**。`( list )` 和 `$( )` 从不创建进程。`ShellState::clone` 产出子壳的变量、别名、函数、trap 和历史——即 fork 的内存侧语义——子壳结束时副本直接丢弃。
2. **带 POSIX `dup` 语义的真实句柄 fd 表**。槽位持有原生 Windows `HANDLE`，而 `DuplicateHandle` 复制的句柄共享同一内核文件对象——因此共享同一文件偏移。这正是 POSIX "dup 共享 open file description" 的语义，落地前在 POC 中实证过。`fork_table()` 逐句柄复制整表，正如 `fork` 复制 fd 表而不复制文件对象。
3. **真实进程只出现在真实进程边界**。外部命令与管道成员走 `CreateProcess` + `os_pipe`；后台作业与 coproc 拥有独立进程，作业控制基于 Job Objects，SIGCONT 通过 `ResumeThread` 投递。

结果：子壳与命令替换的创建成本为零，而脚本可观测的一切——退出码、fd 继承、共享偏移、信号处置——都与 GNU Bash 一致。

## 快速开始

### 从源码构建

> 完整功能当前需要 Windows。

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

```bash
# 全量 83 套件测量（需要 WSL + GNU Bash 5.3.0）
MSYS_NO_PATHCONV=1 wsl bash scripts/true-baseline.sh

# 单个套件
MSYS_NO_PATHCONV=1 wsl bash scripts/true-baseline.sh array
```

## 测试

```bash
# 单元 + 集成测试
cargo test --lib

# bashdb 兼容性
cargo test --test cli_tests bashdb_compat -- --nocapture

# source 展开
cargo test --test cli_tests source_expands -- --nocapture
```

引擎还可以端到端运行 [bashdb](https://github.com/Trepan-Debuggers/bashdb) 核心调试闭环（list、step、next、where、continue、quit）。

## 文档

- [`docs/COMPATIBILITY-STATUS.md`](docs/COMPATIBILITY-STATUS.md) — **唯一权威来源**，Rubash ↔ GNU Bash 兼容性状态
- [`docs/PROVENANCE.md`](docs/PROVENANCE.md) — 来源声明：Rubash 与 GNU Bash 源码的关系，以及贡献者方法论规范
- [`docs/builtins.md`](docs/builtins.md) — 内建命令清单和分发模型
- [`docs/bashdb-debugging-rubash.md`](docs/bashdb-debugging-rubash.md) — bashdb fixture 设置和 smoke test
- [`docs/bash-upstream-tests.md`](docs/bash-upstream-tests.md) — 如何运行 GNU Bash 上游测试

## 开发原则

- 按 Bash 语义的 root cause 修 Rubash 子系统，不按单条 expected output 打补丁。
- bashdb 保持外部 clean 工具；临时 instrumentation 仅用于诊断。
- 每个失败的 bashdb 命令都是发现和修复 Rubash 兼容性缺口的机会。
- 兼容性基线为 GNU Bash 5.3.0（业主编译于 `/usr/local/bin/bash`）。

## 来源与实现方式

Rubash 是对 GNU Bash 语义的 **Rust 从零重写（rewrite）**——不是对 GNU Bash C 代码的移植或翻译。GNU Bash 的任何源码都没有被编译进、链接进或复制进 Rubash 自身代码。兼容性以 GNU Bash 5.3.0 的**可观测行为**为定义，并通过黑盒差分测试验证。vendored 的 GNU Bash 源码位于独立的 `third_party/bash` 子模块，保持其原始 GPL-3.0-or-later 许可证，仅作为语义参考与测试 oracle 使用。完整声明与贡献者规范见 [`docs/PROVENANCE.md`](docs/PROVENANCE.md)。

## 许可证

MIT — 详见 [`LICENSE`](LICENSE)。

## 贡献

欢迎提交 issue、兼容性复现、focused regression tests 和实现补丁。贡献前请阅读 [`AGENTS.md`](AGENTS.md)。

## 致谢

- GNU Bash 团队 — 其可观测行为定义了我们兼容性目标的参考实现
- Trepan-Debuggers/bashdb — 外部调试器和兼容性压力测试
- Rust 社区 — 语言和工具链

---

*最后更新：2026-09-23*
