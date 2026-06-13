# Rubash

一个使用 Rust 编写的 GNU Bash 重新实现。

[![CI](https://github.com/unixwin/rubash/actions/workflows/ci.yml/badge.svg)](https://github.com/unixwin/rubash/actions/workflows/ci.yml)
[![Rust Version](https://img.shields.io/badge/rust-1.70+-blue)](https://www.rust-lang.org)
[![Crates.io](https://img.shields.io/crates/v/rubash)](https://crates.io/crates/rubash)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-orange)](LICENSE)

## 概述

Rubash 是一个正在开发中的 POSIX 兼容 Shell，使用 Rust 语言从零编写。它旨在提供一个安全、快速的 Bash 替代方案，同时保持与现有 bash 脚本的兼容性。

**注意**: 此项目目前处于 alpha 阶段，不建议用于生产环境。

## 特性

- ✅ **词法分析器**: 支持引号、变量、命令替换、花括号展开
- ✅ **解析器**: AST 生成、管道、重定向、命令分隔
- ✅ **执行器**: 内建命令和外部命令执行
- 🚧 **变量展开**: 环境变量和参数展开
- 🚧 **控制流**: if/while/for/case 语句
- 🚧 **管道实现**: 进程间通信
- 🚧 **函数定义**: function 关键字
- 🚧 **作业控制**: job control
- 🚧 **命令历史**: readline 集成

## 快速开始

### 安装
从cargo
```bash
cargo install Rubash
```


```bash
# 克隆仓库
git clone https://github.com/unixwin/rubash.git
cd rubash

# 构建项目
cargo build --release

# 运行
./target/release/rust-shell
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
| Windows + Git Bash 本地 upstream run（分批执行） | 86 | 11 | 75 | 12.79% |

`Bash upstream test progress` CI job 默认不阻塞 PR，用来追踪兼容性曲线。需要把
上游失败作为硬门禁时，可设置:

```bash
BASH_UPSTREAM_STRICT=1 scripts/run-bash-upstream-tests.sh
```

### 代码结构

目录结构决策见 [docs/source-layout.md](docs/source-layout.md)，GNU Bash 源码到
Rubash 模块的对应关系见 [docs/bash-source-map.md](docs/bash-source-map.md)。

```
src/
├── lexer/mod.rs     # 词法分析器
├── parser/mod.rs    # 解析器
├── executor/mod.rs  # 命令执行器
├── lib.rs           # 库入口
└── main.rs          # CLI 入口

tests/
├── lexer_tests.rs   # 词法分析器测试
├── parser_tests.rs  # 解析器测试
└── executor_tests.rs # 执行器测试
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

*最后更新: 2024-06-11*
