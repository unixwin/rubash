# Rubash

一个使用 Rust 编写的 GNU Bash 重新实现。

[![CI](https://github.com/unixwin/rubash/actions/workflows/ci.yml/badge.svg)](https://github.com/unixwin/rubash/actions/workflows/ci.yml)
[![Rust Version](https://img.shields.io/badge/rust-1.70+-blue)](https://www.rust-lang.org)
[![Crates.io](https://img.shields.io/crates/v/rubash)](https://crates.io/crates/rubash)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-orange)](LICENSE)

## 概述

Rubash 是一个正在开发中的 GNU Bash 兼容 Shell，使用 Rust 语言从零编写。项目目标是逐步复刻 Bash 的解析、展开、执行和内建命令行为，并用 GNU Bash 上游测试和本仓库回归测试持续校验兼容性。

**注意**: Rubash 已经超过早期骨架阶段，但仍处于活跃兼容性补齐期。它适合用于测试、研究和兼容性验证，暂不建议作为生产登录 shell 或关键脚本运行时。

## 特性

- ✅ **词法分析器**: 支持 Bash 风格引号、转义、注释、变量、命令替换、算术展开、here-doc/here-string 和常见重定向 token。
- ✅ **解析器**: 生成结构化 AST，覆盖简单命令、管道、AND/OR 列表、重定向、函数、brace/subshell、`if`、`for`、算术 `for`、`while`、`until`、`case`、`select`、`[[ ... ]]`、`coproc` 和 `time` 前缀。
- ✅ **执行器**: 支持外部命令、管道、重定向、临时赋值、函数调用、source/eval、shebangless 脚本回退执行，以及 Windows/Git Bash 路径桥接。
- ✅ **展开系统**: 覆盖变量/位置参数、数组和关联数组、命令替换、算术展开、花括号展开、tilde、pathname glob、常见 `${parameter...}` 操作和大小写/替换类参数变换。
- ✅ **数组语义**: 支持 indexed/associative arrays、复合赋值、元素赋值/追加、负下标、数组切片、`${arr[@]}`/`${arr[*]}`、declare/local/export/readonly 交互中的常见数组行为。
- ✅ **内建命令**: 已实现或接入常用 Bash builtins，包括 `alias`/`unalias`、`builtin`、`cd`、`command`、`declare`/`typeset`/`local`、`echo`、`enable`、`eval`、`exec`、`exit`、`export`/`readonly`、`getopts`、`hash`、`help`、`jobs`、`kill`、`let`、`mapfile`/`readarray`、`printf`、`pushd`/`popd`/`dirs`、`pwd`、`read`、`return`、`set`、`shift`、`shopt`、`source`/`.`、`test`/`[`、`times`、`trap`、`type`、`ulimit`、`umask`、`unset`、`wait` 等。
- ✅ **控制流和函数**: `if`/`while`/`until`/`for`/算术 `for`/`case`/`select` 主体执行路径、函数定义、局部变量、返回状态、break/continue/return 等常见控制语义正在回归测试覆盖中。
- 🚧 **仍在补齐**: 完整 job control、交互式 readline/history、进程组/终端控制、信号边界、Bash 精细解析/别名重读细节、所有上游兼容角落案例。

当前开发重点是继续补齐 Bash 行为细节、扩大 GNU Bash 上游测试覆盖面，并保持跨平台执行语义一致。

## 快速开始

### 安装

从 crates.io 安装当前发布版:

```bash
cargo install rubash
```

从源码构建当前仓库版本:

```bash
# 克隆仓库
git clone https://github.com/unixwin/rubash.git
cd rubash

# 构建项目
cargo build --release

# 运行
./target/release/rubash
```

### 使用

```bash
$ echo "Hello, World!"
Hello, World!

$ ls -la
total 64
drwxr-xr-x 2 user user 4096 Jun 11 00:00 .

$ pwd
/home/user/projects/rust_shell

$ export MY_VAR=hello
$ echo $MY_VAR
hello
```

## 开发

### 依赖

- Rust 1.70 或更高版本
- Cargo

### 运行测试

```bash
# 运行所有测试
cargo test

# 运行特定测试
cargo test --test lexer_tests
cargo test --test parser_tests
cargo test --test executor_tests

# 带详细输出
cargo test -- --nocapture
```

当前本地 Rust 测试套件包含 1800+ 条可执行用例，覆盖 lexer、parser、executor、CLI、Bash 示例脚本和参数/数组/重定向等兼容性路径。

### GNU Bash 上游测试进度

本仓库通过 `third_party/bash` submodule 固定 GNU Bash 上游源码，并用
`scripts/run-bash-upstream-tests.sh` 跑上游 `tests/run-*` 套件。该 job 已纳入
GitHub Actions；每个 PR 都会生成当前兼容性进度 summary 和日志 artifact。

```bash
git submodule update --init --depth 1 third_party/bash
scripts/run-bash-upstream-tests.sh
```

必须通过这个 runner 运行上游测试，不要直接在 `third_party/bash/tests` 或用户目录
里执行 `run-*`。Runner 会拒绝以 `/`、`$HOME`、桌面、下载、文档等位置作为仓库
根目录；每个上游测试都会被复制到 `target/bash-upstream-tests/work/<runner>/`
下面运行，并使用隔离的 `HOME`/`TMPDIR`。测试中的 `rm`、`touch`、`mkdir`、
`cp`、`mv`、`ln` 会被 wrapper 拦截，路径不在当前测试 workdir 内就直接失败。

当前基线:

| 环境 | 总数 | 通过 | 失败 | 通过率 |
|------|------|------|------|--------|
| Windows + Git Bash 本地 upstream run | 87 | 87 | 0 | 100.00% |

`Bash upstream test progress` CI job 默认不阻塞 PR，用来追踪兼容性曲线；当前
本地基线已覆盖 `third_party/bash/tests` 下除聚合入口 `run-all` 外的实际
`run-*` runner。需要把上游失败作为硬门禁时，可设置:

```bash
BASH_UPSTREAM_STRICT=1 scripts/run-bash-upstream-tests.sh
```

### 代码结构

目录结构决策见 [docs/source-layout.md](docs/source-layout.md)，GNU Bash 源码到
Rubash 模块的对应关系见 [docs/bash-source-map.md](docs/bash-source-map.md)。

```
src/
├── builtins/        # Bash builtin 实现
├── executor/        # 展开、执行、变量状态、管道、函数和重定向
├── expand/          # brace/glob/tilde/arithmetic 等展开组件
├── input/           # 交互式输入/readline 相关骨架
├── lexer/           # 词法分析器
├── parser/          # 解析器和 AST 元数据
├── shell/           # Bash 运行时数据结构映射
├── sys/             # Bash 兼容辅助模块
├── lib.rs           # 库入口
└── main.rs          # CLI 入口

tests/
├── cli_tests.rs                  # CLI 和脚本入口测试
├── executor_command_chaining/    # 执行器兼容性回归测试
├── lexer_*                       # 词法分析器测试
├── parser_*                      # 解析器和结构化 AST 测试
└── parameter_transform_tests/    # 参数变换测试
```

## TDD 开发方法

本项目采用测试驱动开发 (TDD) 方法：

1. 先编写测试
2. 实现功能直到测试通过
3. 重构代码使其更简洁
4. 重复以上步骤

详见 [CONTRIBUTING.md](CONTRIBUTING.md)

## 贡献

欢迎贡献！请查看 [CONTRIBUTING.md](CONTRIBUTING.md) 了解如何参与。

## 许可证

本项目采用 GPL-3.0 许可证。详见 [LICENSE](LICENSE)。

## 行为准则

我们遵循 [Code of Conduct](CODE_OF_CONDUCT.md)。请阅读并遵守。

## 联系方式

- GitHub Issues: https://github.com/unixwin/rubash/issues
- 讨论区: https://github.com/unixwin/rubash/discussions

## 致谢

- GNU Bash 团队 - 原始 Bash 的创造者
- Rust 社区 - 优秀的语言和工具链

---

*最后更新: 2026-07-17*
