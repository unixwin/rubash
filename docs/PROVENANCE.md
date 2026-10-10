# Provenance & Implementation Methodology

> Status: **current**. This is the authoritative statement of what Rubash is
> relative to GNU Bash, how its compatibility work is done, and what rules
> contributors must follow. Established 2026-09-23.

中文摘要见文末 [中文摘要](#中文摘要)。

## 1. What Rubash is

Rubash is a **from-scratch rewrite of GNU Bash semantics in Rust**.

- No GNU Bash source code is compiled into, linked with, or copied into
  Rubash's own code (`src/`, `tests/`, `scripts/`).
- Rubash's expression is its own: idiomatic Rust data structures, module
  organization, error handling, and control flow. The codebase does not
  translate GNU C files file-by-file or function-by-function.
- The MIT license covers Rubash's own code only.

## 2. How compatibility is defined and measured

The compatibility target is the **observable behavior** of GNU Bash 5.3.0,
never its source text.

- The oracle is black-box differential testing: the harness in
  `scripts/true-baseline.sh` runs GNU Bash's own 83-suite upstream test
  corpus against GNU Bash 5.3.0 (WSL, owner-compiled) and Rubash, and
  byte-compares stdout/stderr. See the rubash README "Compatibility at a Glance"
  ledger (snapshot archive: issue #477) for the current ledger and
  methodology history.
- Day-to-day development uses the same discipline: probes are run against
  GNU Bash and Rubash implements the observed behavior. The ledger's
  per-suite diff audits attribute each residual difference to an observable
  behavior family, verified by output probes.
- Vendored GNU Bash test fixtures live inside the `third_party/bash`
  submodule and are **executed in place**. They are never copied into
  Rubash's MIT-licensed test data; where Rubash records a fixture, it
  records *observed outputs* of the reference shell, which are not
  copyrightable subject matter.

## 3. Role of the vendored GNU source

The GNU Bash source tree is vendored as a separate git submodule
(`third_party/bash`) under its original **GPL-3.0-or-later** license.

- It is used to **understand semantics** (which behavior a construct has,
  which error class a case falls into) and as the build source for the test
  oracle. It is excluded from published packages (`Cargo.toml` `exclude`).
- Source citations that appear in Rubash comments and commit messages
  (e.g. `jobs.c:3705-3797`) are **traceability pointers**: they record
  *which GNU semantics* a behavior family corresponds to. They are
  provenance of the *contract*, not claims of code derivation.
- The submodule is and remains a separate GPL-3.0-or-later project. Nothing
  in this repository relicenses, modifies, or redistributes it.

## 4. Contributor rules (binding)

1. **Implement from observed behavior.** Run the probe against GNU Bash
   5.3.0 first; make Rubash match the observation. Consulting the vendored
   source to understand *what the behavior is* is fine; the deliverable is
   the behavior, expressed in idiomatic Rust.
2. **Describe work as behavior alignment, not code porting.** Commit
   messages and comments should say what GNU behavior is matched and how it
   was probed:
   - Prefer: `fix(jobs): wait interrupted by trapped signal, status 128+sig
     (GNU jobs.tests jobs9.sub; byte-verified vs 5.3.0)`
   - Avoid: `ported from jobs.c:NNN` as the primary description of a
     change. Historical commits are not rewritten; new ones set the tone.
3. **Keep expression independent.** New code follows Rust idioms and
   Rubash's existing module structure; do not mirror GNU C control flow,
   layout, or naming.
4. **Keep GPL material inside the submodule.** Test data, fixtures, and
   docs copied from the GNU tree do not enter MIT-licensed paths.

## 5. Scope of this statement

This is an engineering methodology statement about how Rubash is built and
verified. It is not legal advice, and it does not modify any license. For
licensing questions, contact the maintainers.

---

## 中文摘要

**Rubash 是什么**：对 GNU Bash 语义的 Rust 从零重写（rewrite），不是对 GNU Bash C
代码的移植或翻译。GNU Bash 源码未被编译进、链接进或复制进 Rubash 自身代码；
`src/`、`tests/`、`scripts/` 的表达是独立的 Rust 惯用法，适用 MIT 许可证。

**兼容性如何定义与验证**：以 GNU Bash 5.3.0 的**可观测行为**为目标，通过黑盒
差分测试验证——harness 用 GNU Bash 自己的 83 套官方语料对两个 shell 做 byte 级
输出对比；现行台账见 rubash README「Compatibility at a Glance」及 issue #477 归档）。

**vendored 源码的角色**：`third_party/bash` 子模块保持其原始 GPL-3.0-or-later
许可证，仅用于**理解语义**和构建测试 oracle，已从发布包排除。代码注释与 commit
中的 GNU 源码行号引用是**语义可追溯性指针**，标注"实现的是哪条 GNU 行为契约"，
不是代码派生声明。

**贡献者规范**：从观测行为出发实现（先跑 GNU 探针，再让 Rubash 对齐观测值）；
commit/注释用"行为对齐"语言而非"移植自 X.c:NNN"；新代码保持 Rust 惯用表达；
GPL 物料（测试数据/夹具/文档）不进入 MIT 路径，夹具只记录参考 shell 的**输出**。
