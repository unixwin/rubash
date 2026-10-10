# Rubash

用 Rust 从零实现的可嵌入、跨平台 GNU Bash 兼容 shell 引擎。Windows 优先，同时提供原生 Linux 与 macOS 构建。

[English](README.md)

[![CI](https://github.com/unixwin/rubash/actions/workflows/ci.yml/badge.svg)](https://github.com/unixwin/rubash/actions/workflows/ci.yml)
[![Rust Version](https://img.shields.io/badge/rust-1.70+-blue)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

Rubash 以可嵌入的无头引擎形态交付——词法分析、解析器、展开引擎、执行器、内建命令——对齐 GNU Bash 5.3.0 的可观测行为。它本身不是 shell 产品：仓库自带参考 CLI，交互式 shell（如 niubash）嵌入引擎承担全部 bash 语义，行编辑、prompt、补全由宿主自己实现。

<!-- TODO(golden-lane, ci490)：最后一条上游切片快照重录落地 CI 后，把本注释替换为头条行：
**83/83 GNU goldens 全绿** —— 上游语料全量逐字节一致（台账：#477）。
当前实测为 82/83，不得提前写。 -->

## 证据

### 能 source 真实世界脚本

这些都是引擎真实 source 过的目标。每个数字都是逐字节实测，每行都链到记录它的 issue。

| 真实世界目标 | 实测结果 | Issue |
| --- | --- | --- |
| nvm.sh v0.40.x 全文 | 加载逐字节干净 | #130 #155 #162 #169 |
| bash-it 全仓 | 221/221 组件加载；完整框架端到端加载 | #316 #161 |
| oh-my-bash 全仓 + 主题矩阵 | 22 libs 加载、252/252 functions 环境一致、5 主题 PS1 逐字节一致 | #143 #148 #149 #160 |
| bash-completion 2.18.0 | 452/452 矩阵；`bash -n` canary 逐字节一致 | #138 |
| git-completion | 140/140 | #142 |
| modernish capability suite | 166/166 rc-vector 一致 | #477 |
| FFmpeg / OpenSSH / PHP / ltmain / cmake configure | `--help` 与错误路径输出逐字节一致 | #477 |
| git 自家测试框架（t/test-lib.sh） | 在引擎下端到端运行 | #477 |

以上结果钉为回归夹具（`tests/regression/`，24 个测试，含 GNU-golden 矩阵、套件切片快照、perf canary）随每次 push 进 CI，并有 C 源码行级审计（[`docs/SOURCE-AUDIT.md`](docs/SOURCE-AUDIT.md)，336 项行为清点）在套件撞上之前找缺口、关缺口。

### 对 GNU Bash 自家语料

兼容性基线是 GNU Bash 5.3.0 的上游测试套件：83 个文件，true-baseline 实测——无上游脚本桩、逐套件 TMPDIR、前台超时，环境绑定差异逐行审计后归零。

```
零差通过：      82 套件
残余差异：       1 套件 —— nameref，1 行已审计（coproc 收割时序
                 竞争，GNU 自身也会出现）
```

9 月 9 日原始差异 3427 行，如今降幅超 99%。完整台账与清单归档于 [#477](https://github.com/unixwin/rubash/issues/477)；完全通过的套件名单在台账里，不占本 README。

## 性能

数据来自 `scripts/run-perf-suite.sh`（探针收在 `benchmarks/`，对 GNU Bash 5.3.0 取中位墙钟）。逐探针数据见所链 issue。

- 数组追加曾是 O(n²)。摊还扩容修复后，n=3000 的 `a+=(i)` 从 771 ms 降到 52 ms，14.8x 提速（#437）。
- nvm.sh 解析：首轮基线是 GNU 的 852x（#241）；解析与增量读取两轮之后进入个位数倍（#241、#281）。
- 近平价：命令替换 1.2x、外部进程 spawn 为 GNU 的 0.9-1.5x（#242）。
- 解释器循环热路径（read 循环、展开、函数调用）仍落后 GNU。开口子的逐探针数字与攻坚队列在 #241、#242。

## 平台

| 平台 | 现状 |
| --- | --- |
| Windows x64（主平台） | 完整技术栈，单一自包含二进制直接对话 Win32，为 bash 生态提供 MSYS2 兼容身份；GNU 回归 golden 与 canary 在 Windows runner 上运行 |
| Linux `x86_64-unknown-linux-gnu` | 引擎与测试套件进 CI；发布 tarball；真 getrlimit/chmod/faccessat/uname(2) 语义与信号投递；GNU 语料在原生 Linux 二进制实测 19/24 逐字节一致（WSL 内测量） |
| macOS `aarch64-apple-darwin` | 库测试在 macOS runner 上真实执行；cross-target check 全绿；uname 臂 coreutils 正确；发布 tarball |
| Android `aarch64-linux-android`、`armv7-linux-androideabi` | CI 交叉编译 check 腿（当前仅 check，无 NDK 链接） |

引擎语义模型（进程内子壳、fd 表语义、进程边界）刻意平台中立，同一份台账随行。Windows 仍是主目标、拥有最完整技术栈；平台缺口在 [`docs/cross-platform-gap-ledger.md`](docs/cross-platform-gap-ledger.md) 跟踪。

## 快速开始

### 从源码构建

```bash
git clone https://github.com/unixwin/rubash.git
cd rubash
cargo build
target/debug/rubash --version
```

完整技术栈当前在 Windows。引擎同样可在 Linux 与 macOS 构建运行（见[平台](#平台)）。

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

引擎测试：`cargo test --lib`（完整矩阵见 [`docs/architecture.md`](docs/architecture.md)）。

## 身份

`uname`、`arch`、`OSTYPE`、`MACHTYPE` 是引擎内建命令，背后是可选 persona：默认 MSYS2 兼容（处处披露：这是兼容面具，不是移植声明），`RUBASH_IDENTITY=native` 切换到诚实原生 Windows。`rubash --identity` 打印当前 persona 与全部生效值。完整语义：[`SKILL.md`](SKILL.md)（rubash#154）与 [`docs/platform-identity.md`](docs/platform-identity.md)。

## 常见问题

### Rubash 需要 WSL 吗？

不需要。Rubash 是单一原生二进制；Windows 上直接对话 Win32，没有 POSIX 模拟层，也不依赖 WSL。WSL 只出现在开发流程里——作为运行 GNU Bash 5.3.0 的测量 oracle。

### 兼容性是实测的还是宣称的？

实测：GNU Bash 5.3.0 自家 83 套上游语料，当前 82 套逐字节一致，每条残余差异行都逐行审计（见[证据](#证据)与 #477）。

### 可以嵌入吗？

可以。引擎以 Rust 库 crate 交付（词法、解析、展开、执行器、内建命令全部进程内），`rubash` CLI 只是参考二进制。niubash 嵌入它承担全部 bash 语义。

## 文档

- Issue tracker — **唯一权威来源**，Rubash ↔ GNU Bash 兼容性状态（测量台账归档于 [#477](https://github.com/unixwin/rubash/issues/477)）
- [`docs/PROVENANCE.md`](docs/PROVENANCE.md) — 来源声明：Rubash 与 GNU Bash 源码的关系，以及贡献者方法论规范
- [`docs/architecture.md`](docs/architecture.md) — 子系统、无 fork 子壳设计、引擎测试
- [`docs/platform-identity.md`](docs/platform-identity.md) — persona 全表、argv/路径方言契约、原生与 MSYS 对比
- [`docs/cross-platform-gap-ledger.md`](docs/cross-platform-gap-ledger.md) — 平台缺口台账
- [`docs/builtins.md`](docs/builtins.md) — 内建命令清单和分发模型
- [`docs/bash-upstream-tests.md`](docs/bash-upstream-tests.md) — 如何运行 GNU Bash 上游测试
- [`docs/bashdb-debugging-rubash.md`](docs/bashdb-debugging-rubash.md) — bashdb fixture 设置和 smoke test

## 来源与实现方式

Rubash 是对 GNU Bash 语义的 **Rust 从零重写（rewrite）**——不是对 GNU Bash C 代码的移植或翻译。GNU Bash 的任何源码都没有被编译进、链接进或复制进 Rubash 自身代码。兼容性以 GNU Bash 5.3.0 的**可观测行为**为定义，并通过黑盒差分测试验证。vendored 的 GNU Bash 源码位于独立的 `third_party/bash` 子模块，保持其原始 GPL-3.0-or-later 许可证，仅作为语义参考与测试 oracle 使用。完整声明与贡献者规范见 [`docs/PROVENANCE.md`](docs/PROVENANCE.md)。

## 许可证

MIT — 详见 [`LICENSE`](LICENSE)。

## 贡献

欢迎提交 issue、兼容性复现、focused regression tests 和实现补丁。贡献前请阅读 [`AGENTS.md`](AGENTS.md)。

## 致谢

- GNU Bash 团队 — 参考实现，其可观测行为定义了我们的兼容性目标
- Trepan-Debuggers/bashdb — 外部调试器和兼容性压力测试
- Rust 社区 — 语言和工具链

---

*最后更新：2026-10-10*
