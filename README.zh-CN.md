# Rubash

一个使用 Rust 编写的 GNU Bash 兼容 Shell 实现。

[English](README.md)

[![CI](https://github.com/unixwin/rubash/actions/workflows/ci.yml/badge.svg)](https://github.com/unixwin/rubash/actions/workflows/ci.yml)
[![Rust Version](https://img.shields.io/badge/rust-1.70+-blue)](https://www.rust-lang.org)
[![Crates.io](https://img.shields.io/crates/v/rubash)](https://crates.io/crates/rubash)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

## 概述

Rubash 是一个正在开发中的 GNU Bash 兼容 Shell，使用 Rust 从零实现 Bash 的词法、解析、展开、执行和内建命令语义。项目已经超过早期骨架阶段，当前重点是继续补齐 GNU Bash 细节、扩大上游测试覆盖，并服务 Winuxsh 等 Windows-native shell 场景。

**当前定位**: Rubash 适合兼容性开发、测试、研究和作为 Winuxsh/Rubash API 的执行引擎验证；它尚未声明为生产登录 shell 或关键脚本运行时。

## 当前进度

当前源码包版本为 `1.0.0`。兼容性状态以
[`docs/COMPATIBILITY-STATUS.md`](docs/COMPATIBILITY-STATUS.md) 为唯一权威
来源，只有对 `third_party/bash/tests/` 下 GNU 官方测试文件真实复现后才更新，
比对基线为 WSL GNU Bash 5.2.21；`docs/` 下其余带日期的分析快照仅作历史归档，
不得用于判定当前 parity。上游 `run-*` 使用旧 `.right` 文件得到的 `87/87`
runner 结果，不应与真实输出 parity 混用。

隔离场景下的 GNU Bash 语义——数组、关联数组、算术、条件、nameref、mapfile、
POSIX 命令替换、前缀花括号（`foo{a,b}`）、转义花括号/转义逗号——已与 GNU
Bash 一致。剩余高优先级缺口按测试文件在该状态文档中跟踪（`posixexp2`、
`cond`、`mapfile`、`comsub-posix`、`braces` 序列细节）。

Rubash 的 GNU Bash 语法和运行时支持已经推进到可以运行较复杂 Bash 程序的阶段：clean 外部 `bashdb` 的核心调试闭环已经能在 `target/debug/rubash.exe` 下工作。

已验证的 bashdb 核心命令包括：

- `list`: 显示被调试脚本源码。
- `step`: 进入 shell 函数体。
- `next`: 前进到下一行。
- `where`: 打印调用栈。
- `continue`: 继续运行被调试脚本。
- `quit`: 退出调试器。

这说明 Rubash 已经覆盖了 bashdb 依赖的一批关键 Bash 语义，包括 `source`/`.`、`eval`、数组和关联数组、`DEBUG`/`RETURN`/`EXIT` trap、`BASH_SOURCE`/`BASH_COMMAND`、函数栈、`functrace`/`extdebug`、路径展开、重定向、`/dev/stdin`、`tty`、动态 fd、参数展开和算术命令等。

同时需要保持清晰边界：**bashdb 核心工作流可用，不等于 bashdb 全部命令和交互功能已经认证完成**。项目目标是继续推进到 bashdb 全功能可用，并把 bashdb 作为真实 Bash 应用压力测试来发现更多 Rubash 兼容性问题。

## 功能概览

- **词法分析器**: 支持 Bash 风格引号、转义、注释、变量、命令替换、算术展开、here-doc/here-string 和常见重定向 token。
- **解析器**: 覆盖简单命令、管道、AND/OR 列表、函数、brace/subshell、`if`、`for`、算术 `for`、`while`、`until`、`case`、`select`、`[[ ... ]]`、`coproc` 和 `time` 前缀。
- **执行器**: 支持外部命令、管道、重定向、临时赋值、函数调用、`source`/`.`、`eval`、shebangless 脚本回退执行，以及 Windows/Git Bash 路径桥接。
- **展开系统**: 支持变量/位置参数、数组和关联数组、命令替换、算术展开、花括号展开、tilde、pathname glob、常见 `${parameter...}` 操作和大小写/替换类参数变换。
- **数组语义**: 支持 indexed/associative arrays、复合赋值、元素赋值/追加、负下标、数组切片、`${arr[@]}`/`${arr[*]}`、`declare`/`local`/`export`/`readonly` 交互中的常见数组行为。
- **内建命令**: 已实现或接入常用 Bash builtins，包括 `alias`/`unalias`、`builtin`、`cd`、`command`、`declare`/`typeset`/`local`、`echo`、`enable`、`eval`、`exec`、`exit`、`export`/`readonly`、`getopts`、`hash`、`help`、`jobs`、`kill`、`let`、`mapfile`/`readarray`、`printf`、`pushd`/`popd`/`dirs`、`pwd`、`read`、`return`、`set`、`shift`、`shopt`、`source`/`.`、`test`/`[`、`times`、`trap`、`type`、`ulimit`、`umask`、`unset`、`wait` 等。
- **仍在补齐**: 完整 job control、交互式 readline/history、进程组/终端控制、信号边界、Bash 精细解析/别名重读细节、bashdb 全命令面、所有上游兼容角落案例。

## 快速开始

### 从源码构建

```bash
git clone https://github.com/unixwin/rubash.git
cd rubash
cargo build
target/debug/rubash --version
```

### 安装发布版

```bash
cargo install rubash
```

Windows-native 安装由 Winuxsh/WinuxCmd 安装器和 WPM 负责，`cargo install` 只适合 Rust 开发环境。安装器应在选定的 WinuxCmd 根目录创建 `usr/bin/`、`bin/` 和 `usr/local/bin/`，并将 `rubash.exe` 与 `bash.exe` shim 放入 `usr/bin/`；`bash.exe` 仅转发到同一安装中的 `winuxsh.exe`，不应放入 `.wpm/` 私有状态目录。WPM 负责包载荷和目标目录同步，Winuxsh 负责将真实 bin 目录加入 `PATH`。

当前源码 checkout 为 `D:/repo/rubash`；历史记录中的 `J:/caponAVIS2019` 仅用于追溯，不是安装器、WPM 或运行时应依赖的路径。

### 运行脚本

```bash
target/debug/rubash path/to/script.sh
target/debug/rubash -c 'echo hello from rubash'
```

## 使用 bashdb 调试脚本行为

bashdb 是外部 Bash 脚本调试器。Rubash 不内置也不 vendor bashdb；当前验证使用 clean bashdb fixture：

```bash
target/bashdb-clean/bashdb-generated
```

核心 smoke test：

```bash
export TERM=xterm DARK_BG=0
printf 'list\nstep\nnext\nwhere\ncontinue\nwhere\nquit\n' | \
  target/debug/rubash.exe target/bashdb-clean/bashdb-generated --no-highlight target/bashdb-probe-target.sh
```

通过标准：退出码为 `0`，stderr 为空，`list` 能显示目标脚本，`step` 能进入函数，`where` 能显示调用栈，`continue` 能运行到 `42` / `done`。

后续目标是让 bashdb 的完整命令面尽可能都能在 Rubash 下使用。新增 bashdb 命令失败时，应优先视为 Rubash Bash 兼容性缺口进行 root-cause 分析，而不是 patch bashdb。更多设置说明见 `docs/bashdb-debugging-rubash.md`。

## 测试

常用窄测试：

```bash
cargo test --test cli_tests bashdb_compat -- --nocapture
cargo test --test cli_tests source_expands -- --nocapture
cargo test --test cli_tests script_bash_source -- --nocapture
```

完整测试仍在持续扩展中。开发兼容性功能时优先使用 focused tests 和受限范围的 GNU Bash 上游测试切片，避免无界运行大型套件。

## Windows 提权

Rubash 的 Windows `sudo` builtin is an embedding API, not a complete Windows
elevation product. It requires the embedding host to register an elevation
handler. Winuxsh disables it by default and recommends the WPM `gsudo` package
for UAC elevation. Hosts may disable a builtin through
`Executor::set_builtin_disabled("sudo", true)`; users can use
`enable -n sudo` and restore it with `enable sudo`.

## 文档

- `docs/COMPATIBILITY-STATUS.md`: Rubash ↔ GNU Bash 兼容性权威状态（唯一事实来源）。
- `docs/bashdb-debugging-rubash.md`: bashdb fixture、launcher/libdir 说明、smoke test 和 fresh checkout 用法。
- `docs/gnu-bash-compatibility-implementation-plan.md`: GNU Bash 兼容性实现路线。
- `docs/issue-suite-diff-analysis.md`: 上游测试差异分析。
- `docs/bash-compat-issues.md`: 兼容性问题清单。
- `docs/bash-source-map.md`: Bash 源码/语义映射。
- `docs/typed-expansion-migration-checkpoint.md`: typed expansion 迁移检查点。
- `docs/source-layout.md` 与 `docs/semantic-ownership.tsv`: 源码布局与 GNU 语义归属映射。

## 开发原则

- 按 Bash 语义的 root cause 修 Rubash 子系统，不按单条 expected output 打补丁。
- bashdb 必须保持外部 clean 工具；临时 instrumentation 用完要恢复。
- bashdb 当前是高层脚本行为调试器，不是 Rust 源码调试器。调试 `src/**/*.rs` 仍使用 Rust tooling、日志、instrumentation 和 focused tests。
- bashdb 全功能可用是后续目标；每个失败命令都是发现 Rubash 兼容性缺口的机会。

## 许可证

Rubash 采用 MIT 许可证。详见 `LICENSE`。

## 贡献

欢迎提交 issue、兼容性复现、focused regression tests 和实现补丁。贡献前请阅读 `AGENTS.md` 中的开发规则。

## 联系方式

- GitHub Issues: https://github.com/unixwin/rubash/issues
- 讨论区: https://github.com/unixwin/rubash/discussions

## 致谢

- GNU Bash 团队 - 原始 Bash 的创造者
- Trepan-Debuggers/bashdb 项目 - 外部 Bash 调试器和兼容性压力测试来源
- Rust 社区 - 语言和工具链

---

*最后更新: 2026-08-29*
