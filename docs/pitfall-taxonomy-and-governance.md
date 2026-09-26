# 坑分类学与治理机制（Pitfall Taxonomy & Governance）

> 生成日期：2026-09-20。数据来源：全量 git log（2566 条提交）主题聚类、
> 热点文件统计、docs/COMPATIBILITY-STATUS.md、docs/issue-suite-diff-analysis.md、
> GitHub issues（gnu-compat G1–G26、niubash #100–#130）。
> 本文档的目的：**让"同一类坑反复踩"变得可见、可防、可测**。
> 新 agent 进场必读；修改词展开/标记/重定向相关代码前先查第五节的禁令。

## 一、事故家族总表（按根因分组）

### 甲类：标记/载体字节（带内哨兵机制）

rubash 用带内哨兵（C0 字节 \x11–\x1f、PUA 码点 E000–E317、命名串
`__RUBASH_CA1__`/`__RUBASH_HD1__`/`__RUBASH_CSB1__`）在词展开管线里承载
引号/来源信息，对标 GNU 的 CTLESC/CTLNUL。

| # | 根因 | 症状 | 证据（示例） |
|---|------|------|------------|
| M1 | **编码后消费点漏解码（主模式）** | 每新增一种标记/一条路径需人工补 5–10 处 decode，漏一处即泄漏 | eda3a734 系列五连补；613e7089(recho)、cb39127a(echo)、5eaae06c(eval)、f3108992(declare)、d45619ee |
| M2 | 标记与用户数据碰撞 | 用户数据含控制字节/PUA 被误认成标记 | 049d6d9d、118c5ee3、0a4ba289、f39669b3(E000 碰撞) |
| M3 | 同字节双语义 / 词表不统一 | ANSI-C 解码出的真实字节被载体占用；常量多处重复定义 | d45619ee、358c6a21、4c8f543f、c282a850 整编；**现存 E10A 双占用**（SQ_DOLLAR_DATA @assignment_expansion.rs:182 vs FAILED_SUBSCRIPT_SENTINEL @types.rs:53） |
| M4 | restore 站点反向漏调 | 有解码要求但漏调用 → 标记被剥/泄漏 | 649b2ac1、1d405365、80ee05a3、af987f35 |
| M5 | merge/rescue 丢载体修复 | 合并静默吞掉已修语义 | ba2e7c19(#103)、0e44b432、907d2d99、8c9a4ee8 |
| M6 | 标记泄漏到用户可见输出 | echo/xtrace/declare -p/recho 出现内部标记 | COMPAT-STATUS:640/661/1074、:147、:471 |
| M7 | 快路径白名单漏载体 | 词级 fast path 不认识新载体形态 | 3f23f8dc(niubash#103)、7ab91ffd、2e149678 |

当前词表热点面：`\x1f` 解码散布 **50+ 文件**、`\x1d` **39 文件**——
这是 M1 持续发生的结构性原因。

### 乙类：展开/解析语义（非标记）

| # | 家族 | 规模 | 热点文件 | 深层根因 |
|---|------|------|---------|---------|
| S1 | **词层快速路径引号/转义失真** | ~242 提交（#116/#117、niubash#103…） | executor/mod.rs、assignment_expansion.rs | fast path 在文本层重实现词法语义，准入靠 contains() 黑名单（#117 已定性） |
| S2 | **数组复合赋值/下标展开** | 223+ 提交，两次 merge 丢修复 | declare.rs、executor/mod.rs | 引号去除→转义解码→字段拆分的阶段顺序无唯一实现；declare/read/alias/直接赋值四入口各一份 |
| S3 | 重定向/fd 模型 | ~206 提交（#118/#122） | command_prepare.rs | Windows 无真实 fd；内建输出通路 N 条各自打补丁 |
| S4 | heredoc 收集 | ~101 提交（heredoc3/7/9/10 变体各自失败） | lexer/mod.rs | 独立扫描器而非 parser 状态机（GNU PST_EOFTOKEN 模型） |
| S5 | 命令替换 body 再解析 | ~129 提交 | command_substitution.rs | 文本切片+再解析替代 parse_and_execute 一体化 |
| S6 | 子壳隔离 | 20+ 提交（#100 两修） | ast_exec.rs | 非 fork，靠手工快照/恢复清单，新增状态即新泄漏点 |
| S7 | 作用域/varenv/nameref | 100+ 提交 | declare.rs、varenv | 与 variables.c frame/tempenv 不同构，正成批补语义 |
| S8 | errexit/status 传播 | ~30 提交 | executor/mod.rs | `$?` 曾是裸整数，c5c97683 才统一 ExitCode |
| S9 | trap/job/signal | 45+ 提交，coproc 三次 revert | trap_exec.rs、pipeline_exec.rs | 无子进程所有权模型 |
| S10 | Windows 进程/路径 | ~104 提交 | path.rs、main.rs | 平台基建债，收敛中 |
| S11 | stderr 顺序 | AGENTS.md 已记录 | 多处 eprintln | line-buffered vs 逐写 flush |
| S12 | 字段拆分/IFS | ~44 提交 | executor | 拆分入口不唯一（read/declare/array/comsub 各有路径） |
| S13 | 流程性：merge 丢修复 + 假绿基线 | 10+ 条 | 全仓 | 双线开发共享树；历史上用 5.2.21/Git Bash/仿真层判定 PASS |

## 二、结构性结论

1. **两个根因喂养了大半个事故库**：S1（词层 fast path）同时喂养 S2/S5/S12；
   S2（复合赋值展开管线不唯一）是 #103→回归→#109→revert→"four lost fixes
   re-implemented" 循环的源头。
2. **标记机制的症状（甲类）大多也是 S1/S2 的投影**：fast path 不认识载体（M7）、
   多入口展开各自带解码（M1/M4）。
3. **平台债（S10/S11）是一次性的**，已在收敛，不再新增。
4. **流程漏洞（M5/S13）用规则+CI 补**：merge 前聚焦回归、PASS 只认 WSL 5.3.0
   script-file 基线——规则已存在，需强制化。

## 三、治理机制（新代码必须遵守）

### 3.1 标记注册表（markers.rs，已建 — M1 落地 2026-09-22）

- 所有哨兵字节/PUA 码点/命名标记串**只允许**在 `src/executor/markers.rs`
  声明一次；业务代码禁止裸写 `\x14`、`E10A` 等字面量，一律 `use markers::*`。
- 每个标记声明必须包含：字节值、含义、**编码函数**、**解码函数**（同文件相邻、
  成对导出）、消费者边界清单（输出/存储/重解析三类边界各写明谁负责还原）、
  **用户可达性**（`user_reachable`：用户同字节入口必须在同点编码，
  否则字面量字节被误当标记——B1 \x05 教训）。
- 新增标记 = 新增一对 encode/decode + 一条"标记值不得出现在 stdout、
  `declare -p`、xtrace 输出"的金标断言测试。
- **E10A 双占用已拆**（前置已落地）：FAILED_SUBSCRIPT_SENTINEL = U+E200。
- **E1xx/UTF-8 混排歧义已解（M1）**：DATA_* 哨兵族与
  COMPOUND_EXPANSION_WS_TAG 从 E101-E10C 迁入专属注册块
  **U+E301..=U+E30C**（原码点落在 BYTE_CHAR_BASE 动态字节字符区
  E100-E1FF 内，与模式字节字符 0x01-0x0C 同值碰撞）；PATSUB 族
  （\x0b/\x0c/\x0e/\x0f）与四个函数级保护哨兵（parameter_words \x0e、
  command_prepare \x1e、preserve_prompt_escapes \x15、case-pattern \x15）
  迁入 **U+E310..=U+E317**，彻底脱离用户可达字节域。
- M1 已完成：注册表 + 全部具名 const 别名化 + 碰撞区重编号 + 注册表
  自洽测试（无 E1xx 内哨兵、PUA 唯一性、char/_STR 一致性、pair roundtrip）。
- **M2 已完成（c84bde92）**：`\x1f`/`\x1d` 高频字面量全部走注册表；
  修复脚本迁移引入的双前缀 bug（`\x1d\x1d`）；**ARRAYREF_FLAG \x02→U+E318**
  （与 PROMPT_IGNORE_END 撞码，注册表唯一性测试捕获）；decoder 补齐
  PATSUB/guard/FAILED_SUBSCRIPT/ARRAYREF 映射。
- **M3 已完成（34e204bf）**：其余 C0 字面量（\x11-\x1e、\x10、\x03、\x05、
  \x12、\x01/\x02）全部迁移约 300 处；同码点异协议登记为诚实别名
  （PATTERN_LITERAL_BACKSLASH=\x18 pattern 域、PARSE_ERROR_FIELD_SEP 与
  HEREDOC_WARNED_BODY_PREFIX=\x1e）；数据字节（printf/ANSI-C 转义表、
  [:space:] 字符类）保持字面量不误迁。
- **M4 已完成**：三边界入口在注册表文档列明（输出=decode_to_visible_text、
  存储=substitution_metadata 入口编码、重解析=词法器原生识别）；**字面
  字符转义落地**——`E000+<非载荷字符>` 恢复字面字符，ANSI-C `\uXXXX`
  入口对注册区码点（E000-E3FF）加 E000 前缀（push_literal_char），
  解决用户 PUA 输入与标记的混排歧义（$'\uE314' 往返无损）。
- **Typed-Carrier 迁移批次（Golden Assertions 方案，2026-09-22 完成）**：
  - Batch 1: PREEXPANDED_STDIN_BODY（\u{5}）完整 typed-carrier 迁移（StdinBody enum）
  - Batch 2: PATSUB 族（U+E310-E313）金标断言
  - Batch 3: 函数本地守卫（U+E314-E317）金标断言
  - Batch 4: ASSIGN_DATA_* 族（U+E301-E30C）金标断言
  - Batch 5: CTLESC（U+0011）金标断言
  - Batch 6: 命名字符串标记（__RUBASH_HD1__/CSB1__/CA1__）金标断言
  - 全部批次验证：cargo test 418 passed + 83 套件 true-baseline 无回归
  - 迁移计划文档已完成使命并于 2026-09-26 清理删除（git 历史可查）。
    **治理裁决（收尾口径）**：剩余标记族（C0 族除 CTLESC 外的 14 个、PUA 族
    6 个、C0 执行标记 4 个）**保留带内协议**，由 markers.rs 注册表 + 金标断言
    治理，不再做全量 typed-carrier 重构——CTLESC 全迁移需重造整个引号状态
    追踪（对齐 parse.y:5694-5706 / subst.c:4692 / subst.c:4807 的成本远超收益），
    函数本地标记本就无泄漏面。新增标记仍按 3.1 禁令一律禁止。

### 3.2 解码收口

- 按三类边界收口解码：**输出边界**（echo/declare/-p/xtrace 前统一还原）、
  **存储边界**（变量/数组赋值前统一还原）、**重解析边界**（eval/alias/comsub
  再解析前统一还原）。目标是把 50 文件的散布 decode 收敛到每边界一个入口。
- 新路径接入时只允许调用边界入口，不允许自带 decode。
- **宿主边界入口已落地（2026-09-22，Phase 0）**：`rubash::decode_to_visible_text`
  （src/locale.rs，crate 根 re-export）是面向宿主的"内部传输文本→用户可见文本"
  公开解码函数，供 niubash dump-strings 与 A1/A2 宿主改写删除使用。单遍解码
  CTLESC、C0 数据载体、词级前缀标记（\x1b/\x1c/\x1d）、ANSI-C PUA 标记、
  赋值 DATA_* 哨兵、raw-byte 标记对与条件模式字节字符；载体表见函数文档。
  `bad_substitution_display` 已收敛到它。GNU 锚点：locale.c:550
  `dump_translatable_strings`、shell.c:507-509、parse.y:5694-5706 CTLESC。
- **C1 遗留登记**（niubash 计划文档同步）：c16 未闭合 `$"` 需要 EOF 诊断
  （对齐 GNU parse.y 未终结引号 EOF 报告）；c20 `$"..."` 体内嵌套
  `$(...)`/`${...}` 的 dump 需要 AST→源码 body 序列化器（与 niubash C2
  `pretty_print_script` 同前置）。
- **金标断言已落地（M5，tests/marker_leak_golden.rs）**：6 个进程级断言——
  echo/`declare -p`/xtrace/reparse 输出不得出现 PUA 注册区码点
  （U+E000-U+EFFF UTF-8 序列）与 `__RUBASH_*` 命名串；用户 PUA 字符
  （`$'\uE314'`）必须逐字往返。C0 载体字节在 stdout 中与合法用户数据
  不可区分，故金标针对无歧义类别（PUA 码点与命名串）。
- **Typed-Carrier 金标断言（2026-09-22 完成）**：locale.rs::decode_to_visible_text
  增加生产环境断言，确保以下标记族不泄漏到输出：
  - 函数本地守卫（U+E314-E317）
  - ASSIGN_DATA_* 族（U+E301-E30C）
  - CTLESC（U+0011）
  - 命名字符串标记（__RUBASH_HD1__/CSB1__/CA1__）
  - 断言在测试模式下跳过（单元测试可能直接传递原始标记字符串）

### 3.3 语义唯一入口（按杠杆排序）

1. **消灭词层 fast path**（S1）：准入改白名单（纯字面量才短路）；
   CI 静态检查禁止词层函数新增 `contains(`/`starts_with(` 准入条件（#117 裁决落地）。
2. **复合赋值唯一管线**（S2）：抽 `expand_compound_assignment_word()`，
   对齐 GNU expand_word_internal 阶段顺序，declare/read/alias/直接赋值四入口强制走它；
   属性测试覆盖 赋值×引号×转义×IFS 矩阵。
3. **重定向单一 RedirectionEngine**（S3）：内建禁止直写 std handle。
4. **heredoc 进 parser 状态**（S4）：对齐 PST_EOFTOKEN，所有解析入口免费获得正确行为。
5. **子壳隔离 ShellState 化**（S6）：可变状态全收进一个结构，子壳=克隆整结构，
   新字段自动被隔离。
6. **ExitCode 贯通**（S8）、**子进程所有权注册表**（S9）、**stderr 单通道逐写 flush**（S11）。

### 3.4 流程防线

- **修 issue 必须固化回归测试（资产化，硬规则）**：每个 issue 修复提交必须同时
  包含一个能在旧代码上失败、在新代码上通过的测试（`tests/` 下，命名引用 issue
  号，如 `issue120_*.rs`）。判定标准：revert 修复提交后该测试必须红。多次
  "丢失的修复重做"（ba2e7c19、0e44b432、907d2d99、8c9a4ee8）证明仅修代码不留
  钉子的修复等于没修。
- merge 前跑聚焦回归；rescue merge 后必须重跑相关族的探针（M5 四次事故的教训）。
- PASS 断言只认 `true-baseline.sh` + WSL GNU 5.3.0 script-file 口径；
  建议 CI 强制而非靠纪律（S13）。
- 合并跨 agent 分支时：先通读对方分叉点以来的全部提交再解冲突；
  同一 GNU 行为两边都实现的，保留更贴 C 源码、更少特判的一边。

## 3.5 八三套件与"百分百兼容"的关系（口径声明）

**83 套件零差 ≠ 100% 兼容。** 它是必要条件，不是充分条件。覆盖盲区：

1. **测试面**：83 个 `.tests` 是 GNU 自带的回归测试（约几千个断言），针对
   历史 bug，不是穷尽一致性规范。GNU bash 的内建选项矩阵、组合语义空间
   远大于此。
2. **观测面**：true-baseline 主要比对 stdout（stderr 单独捕获但不进主台账），
   exit code、时序、信号投递时机、内存/资源行为均不在字面 diff 内。
3. **域盲区**：交互/readline/PS1/completion 基本无自动覆盖；多字节/ locale
   组合稀疏；`bashdb`、大型真实脚本（bash-completion 等）是抽样而非系统覆盖；
   POSIX conformance test suite 未接入。
4. **环境面**：部分剩余 diff 是 Windows 平台噪声（AGENTS.md 口径单独记账），
   反之 GNU 的 Linux 特有行为（如真实 fork、信号语义）在 Windows 上只能模拟，
   零差也可能只是该场景恰好没被触发。

**因此补强方向**（全部只对齐唯一权威 WSL GNU bash 5.3.0，不引入第二口径——
POSIX conformance suite 等一律不用，POSIX 与 GNU 存在分歧，对齐标准会偏离 GNU）：
① 修 issue 固化回归测试（上文硬规则）；② 差分模糊测试（随机脚本对 WSL GNU
5.3.0 逐字节比对，自动发现 83 套件外的分歧）；③ 真实世界脚本语料作为更高强度
的 GNU 对照（autotools `./configure` 双侧跑 diff 产物、bash-completion、发行版
脚本——判定标准仍是与 GNU 5.3.0 输出一致，不是"能跑通"）；④ 补齐第四节数的
套件缺口（解析器族无专属覆盖）。

### 3.6 POSIX fork/exec 模型的实现策略（架构决策记录，2026-09-20）

**决策**：不追 fork 的机制，只复刻 fork 的语义。GNU 用 fork 是因为 Unix 内核
给了这个原语；fork 机制本身在 Windows 上的模拟（Cygwin/MSYS 的内存循环拷贝）
又慢又脆，不值得模仿。要对齐的语义只有四条：①子壳状态完整隔离复制；②独立
执行流（后台 `&`、coproc）；③exit status/信号投递回传；④fd 表按序复制。

**三层混合实现**（分场景，不追单一机制）：

| 场景 | 机制 | 说明 |
|---|---|---|
| 普通 `( )` 子壳 | 串行模拟（现状保留）+ **ShellState 结构化**：全部可变 shell 状态收进一个结构，子壳 = 克隆整个结构 | 类型系统保证新增字段自动被隔离，消灭 S6 的手工清单漏项 |
| 后台 `&` / coproc | **线程池 + 子进程所有权注册表**（谁 spawn、谁 reap、trap 归谁） | 唯一语义上必须并发的场景；即 AGENTS.md 挂账的"后台任务线程化"深水区，值得立项 |
| fd 表复制/排序 | **Win32 精确句柄继承**（`PROC_THREAD_ATTRIBUTE_HANDLE_LIST` + Job Objects） | 修 `read v <&3 3<<EOF` 排序、exec fd 中毒等 redir/read 族架构项，不动执行模型 |

**明确排除**：全量线程化 fork（引擎全量线程安全化代价大且普通子壳不需要）、
子进程序列化 fork（引擎状态不可整体序列化）、WSL 桥接（破坏 Windows 原生定位）。

**验收口径**：每层独立验证——子壳快照完整性用属性测试（随机状态组合下克隆
隔离性）、并发用 ownership registry 单测、fd 用定向探针对拍 WSL GNU 5.3.0。

**实测校准（2026-09-20，`exp/fork-hybrid` @74bf11dc，产物在
`D:/repo/rubash-exp-fork/experiments/fork-hybrid/`）**：

1. **fd 语义现状**：12 探针 6 个真实分歧，单一根因——fd 层是"逐条重定向重开
   模拟"而非 open file description 表受控复制（典型：fd 偏移量不共享、子壳
   `exec 3<&-` 穿透父壳、后台不继承 fd3）。`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`
   POC 通过（白名单继承、rogue 句柄不泄漏）。**校准**：HANDLE_LIST 只修继承面；
   偏移量共享/关闭隔离需把 FdTable 改为记录真实 HANDLE——规模是 FdTable 重构
   （中等工程），大于本节原估的"修排序"。
2. **并发 registry POC 通过**（spawn <100ms 不阻塞、wait 回收 exit code、
   shutdown 不挂死，5/5 测试）。**校准**：引擎 `&mut self` 单线程借用模型是
   真正成本，建议先 registry 化记账、后评估状态共享；且与第 3 层有顺序依赖
   ——必须先 HANDLE_LIST 化再线程化，否则 stdio 手术补偿的单线程 spawn 安全
   论证失效。
3. **ShellState 量化**：Executor 88 字段；`( )` 子壳 save/restore 清单仅 7 项、
   命令替换 fresh-Executor 手工清单约 55 项——**两份清单已漂移**；实测打穿：
   **alias 和 function 从子壳泄漏到父壳（GNU 干净）**，S6 家族现行证据。
   工作量约 60 项语义状态 + 28 项瞬态分离，估 3–5 天机械重构 + 83 全量回归
   （载体字节/$'...' 重点）。

**优先级修订**：第一层（ShellState）提升为最高——alias/function 泄漏是现行
bug 且不依赖其他层；fd 层规模上修；线程化必须在句柄层之后。

**前人方案对比（2026-09-20 调研）与自研可行性**：

| 前人方案 | 思路 | 对我们的结论 |
|---|---|---|
| Cygwin/MSYS2 fork 模拟 | 硬模拟 fork 机制 | 拒绝——慢且脆，Git Bash 语义怪癖的来源 |
| MSR《A fork() in the road》HotOS'19 | shell 不需要 fork 拷贝语义，spawn 式更贴近真实用法 | 直接背书本 ADR 方向 |
| libuv (Node) uv_spawn | stdio 管道 + 受控句柄继承 | 与句柄层 POC 同思路，实现细节可参考 |
| Rust std | 历史上 bInheritHandles=TRUE + 进程级互斥锁；#73281 提案 HANDLE_LIST；新版开放 `raw_attribute` 自定义 ProcThreadAttributeList | **句柄层实现优先用 std `raw_attribute`**（上游维护），POC 的裸 Win32 代码可减半 |
| fish/nushell/elvish | 避开 fork，内部任务模型 | 语义优先路线的同类实践 |

**自研可行性判断**：可行，且有一个关键技术支点——**Windows 的
`DuplicateHandle` 复制出的两个 HANDLE 指向同一 FILE_OBJECT，天然共享文件
偏移量**。这意味着 POSIX `dup()` 的"偏移量共享"语义在 Win32 上是原生免费的：
实现一个引擎内 FdTable（槽位数组 + open/dup/close/子壳时整表 DuplicateHandle），
六个实测分歧里的偏移量共享、关闭隔离、子壳穿透三类就随语义自然修复，不需要
hack。spawn 侧用 std `raw_attribute` 白名单传递。预估核心 FdTable 数百行 +
接入 redirect/spawn 路径，属 1–2 周工程；风险点在内建读 fd（read/mapfile）与
stdio 载体路径的接线，需定向探针护航。

**阶段 2 POC 实证（2026-09-20，`exp/fork-hybrid` @1c1108b9，
`experiments/fork-hybrid/fdtable-poc/`）**：

- FdTable 原型单测 8/8：dup 偏移量共享、close 隔离、整表复制不穿透、seek 跨槽
  联动、管道缓冲跨 dup、dup2 替换即关闭、继承位——**DuplicateHandle 偏移量共享
  假设实测成立**（dup 槽交叉读写互推偏移量，fork_table 后子表推进父表）。
- 12 探针中 6 个失败场景在 FdTable 模型上重放 **6/6 消除**。
- **修正**：std 的 `raw_attribute`/`spawn_with_attributes` 至今**从未进 stable**
  （feature `windows_process_extensions_raw_attribute`，#114854）；stable 路径
  手写 `STARTUPINFOEXW` 回退已验证 6/6。已知三坑：① attribute 传值必须裸
  HANDLE 数组（传 Vec 结构体本身 → ERROR_87）；② 白名单上每个句柄须先标
  可继承；③ `Stdio::piped()` 内部句柄进不了白名单（跨进程结果须经文件回传）。
- 附带：匿名管道 EOF 时 `ReadFile` 返回 `ERROR_BROKEN_PIPE(109)` 而非 0 字节，
  fd 层必须映射为 EOF。
- **接入清单（约 1 周）**：① FdTable 移入引擎作 `fd` 模块（POC 直迁，~300 行）；
  ② redir 打开路径删"重开模拟"改槽上真句柄（最大面，2–4 天 + redir 套件回归）；
  ③ spawn 层白名单属性表替换 `spawn_with_isolated_std_handles` 补偿（顺带删除
  单线程 spawn 假设、解锁线程化）；④ 后台/coproc 记账迁 FdTable + `fork_table`；
  ⑤ 12 探针转正为引擎差分测试。

**S6 实施记录（2026-09-21，`master-fix` 分支）**：

- `src/shell/state.rs` 扩为完整隔离边界：约 30 个语义字段（env_vars、aliases、
  functions、function_def_* 三表、positional、pipestatus、local_*_scopes、
  expanding_aliases、loop/function_depth、random_state、subshell_depth、
  background_jobs、coproc_names、completion_specs、session_history 等）全部
  移入 `ShellState`；Executor 只留 fd/进程资源与单命令瞬态。
- 三处子壳路径（flat subshell region、嵌套 `( list )` 节点、命令替换
  fresh-Executor）全部改为 `shell_state.clone()` 整快照恢复；两份手工清单
  （7 项 / ~55 项）已删除。GNU 锚点：`execute_cmd.c:1576 execute_in_subshell`
  ——fork 进程拷贝天然隔离全部语义状态。
- 探针验证（WSL GNU 5.3.0 逐字节）：alias/function 定义、覆写、嵌套子壳均不
  再泄漏；回归测试 `tests/issue_s6_subshell_state_isolation.rs`（8 例）。
- 顺手拆分 E10A 码点双占用：`FAILED_SUBSCRIPT_SENTINEL` 迁至 U+E200（E100–E1FF
  段已被 conditional/pattern.rs 的 BYTE_CHAR_BASE 字节编码占用）。
- 基线暴露并修复一个 ab811d04 引入的回归：`word_level_quote_syntax` 门控正确化
  后暴露了 `${x-word}` 默认值词在非双引号上下文被 `decode_double_quotes_in_
  quoted_parameter_word` 误作 dq 句子解码（`o=${x-' '}` 存字面 `' '`、
  `${f-'$HOME'}` 内 `$HOME` 被展开）。修法按 `subst.c:7663
  parameter_brace_expand_word`：词按自身引号上下文展开——仅 DoubleQuoted/
  HereDocument（Q_HERE_DOCUMENT 对 `${}` 词呈 dq 语义）走 dq 解码，非引号
  上下文交回 walker 引号剥离。`embedded_mutations.rs` 通用 `\` 臂补非双引号
  `'` → ANSI_C_QUOTE_MARKER 数据载体（`o=${c='q'}` → `'q'`）。
  precedence 套件 +24 → 0，posixexp +2 → 0，quote/nquote/dstack/rhs-exp/
  braces/new-exp/more-exp/posixexp2/exp 全部持平。
- 已知残差（预存，非本次回归）：heredoc 体内 `${x-'q'}` 非 mut 路径
  丢 heredoc 上下文（GNU 出 `'q'`，RB 出 `'q'`）；`declare -A 'a[$q]=v'`
  空键接收 vs GNU bad-subscript。

**台账修正记录（2026-09-22，`master-fix` 分支）**：

- 用户复核发现 master `7e57003e` 上 `precedence`（40 行）与 `posixpipe`
  （2 行）确定性分歧，而 9-21 台账记二者零差。根因查明：precedence 由
  `ab811d04` 的 `${x-word}` 引号门控回归造成（已在 `6e29c031` 修复，非
  pipeline stdin cursor 工作的副作用）；posixpipe 是 6-19 遗留的脚本名
  硬编码 hack（`is_this_shell_posixpipe_time_count` 家族直接 `println!("4")`
  并跳过管道），`tests/` 种子目录缺 `test-glue-functions` 等文件时套件
  双侧同报 "No such file" 形成**假零差**掩盖了它——旧台账的 57 零差
  因此作废。
- 修复与真实缺口补齐：① 删除整个 posixpipe 硬编码 hack 族
  （external_finish.rs/command_dispatch.rs/external_inner.rs/
  pipeline_exec.rs/types.rs），真实管道机制实测零差；② `kill -n9`/
  `-sNAME` 黏连信号规格（GNU `builtins/kill.def:134-142`）——缺失曾使
  `jobs2.sub` 的 `kill -9` 无效、`wait` 空挂；③ `set -m` 映射 monitor
  选项（support_names.rs `short_set_flag_option` 缺 'm' 项）；④ `fg`/`bg`
  两级作业控制检查——shell 级 monitor 关→"no job control"，作业级
  `J_JOBCONTROL` 未置→"job N started without job control"
  （`builtins/fg_bg.def:108-113,154-160`，`JobEntry.job_control` 按注册时
  monitor 状态打标）；⑤ `${THIS_SH}` 同进程子壳作业表进程边界
  （`execute_cmd.c:6139-6233`：exec 模型清表、fork 模型快照恢复，
  `background_children` 句柄停泊防子壳收割父进程）。
- **修正后诚实台账：55 零差 / 799 原始行**；其中 `jobs` 31 + `history`
  173 = 204 行为 harness 40s 超时双侧截断（GNU 侧 rc=124/137 同样被杀，
  jobs.tests 含真实秒级 sleep、history 在 `env -i` 下交互挂起），真实
  语义差 ≈595 行 / 26 套件。零差名单较作废台账：−nameref(1, coproc
  时序噪音)、−posixexp(1)、−procsub(13, 种子补齐后暴露真差)、+mapfile(0)。
- `jobs` 套件另观测到一次时序性静默退出（rb 进程于 `wait` 边界消失、
  tasklist 无残留、孤儿 `sleep.exe` 存活），疑似后台子进程生命周期竞态，
  与本次修复无关，需后续专项调查。
- harness 加固：`scripts/true-baseline.sh` 改为每轮全量 gap-fill 种子
  同步 + ELF magic 守卫（`recho` 二进制不再被 `tr` 误伤；`file` 对高位
  字节文本误判 data，故改用 od 魔数判定）。

### 3.7 双层测试口径（引擎层 + 产品层）

**真正的 shell 层是 niubash**（`D:/repo/niubash-*`，crate `niubash`，依赖
`rubash = { git = ".../rubash.git", branch = "master" }` + winuxcmd），用户
摸到的是它。因此：

1. **引擎层**：rubash 二进制跑 83 套件 true-baseline（现行口径）。
2. **产品层**：niubash 二进制跑同一 83 套件 + niubash 专属回归（其 issue 系列的
   固化测试）。发布前产品层必须过，因为 CLI 解析、readline、默认值、AI 粘合层
   完全可能在引擎零差之上引入自己的分歧（实例：niubash#129 脚本模式无条件
   expand_aliases 是产品层语义 bug，不是引擎的）。
3. **分层纪律**：任何语义修复落在 rubash 引擎，niubash 保持薄壳——niubash
   里出现语义补丁即是架构异味，应下沉引擎。
4. 注意：AGENTS.md "不要用 niubash 做测量"指的是**不要拿它当工具 shell 跑
   harness**（它 mangle glob/引号），不是"不测它"——它本身是被测对象。
5. 时序约束：niubash 依赖 rubash master git 分支，因此产品层基线只能在引擎
   合并后刷新；分支开发期以引擎层基线为准，合并后补产品层。

### 3.8 引擎去 winux 化（embedding hygiene，2026-09-20 决策；低优先级）

**动机**：不是为第三方嵌入（大概率没有别家用），而是**引擎里的 winuxsh 痕迹
刺眼**——产品关切不该污染引擎层，且 `WINUXSH_ROOT` 泄漏进测试环境会作废整轮
基线（AGENTS.md 已有记录）。**例外**：`bin/bash.rs` 的 bash shim 保留——它
就是给 AI 调用方用的入口，属有意设计，feature-gate 标注清楚即可。

**优先级：低。** `WINUXSH_ROOT` 被 niubash 大量使用（引擎 path.rs 根解析/
路径翻译、pwd.rs，以及 niu 侧 completion/syntax_highlighting/shell 多处读取
root 定位），**不允许直接删**；做法是**改名迁移**，且只在任意批次顺手携带，
不单独立项、不占架构项排期。

**改名迁移步骤**（每步独立可验证，零功能变化）：

0. **全量盘点（2026-09-20 grep 全仓）**：11 个源文件含 winux 痕迹，逐个归类——

| 痕迹 | 位置 | 处置 |
|---|---|---|
| `WINUXSH_ROOT` 导出+读取 | public_accessors.rs:110、path.rs:96/1197、pwd.rs:202 | 上文改名迁移 |
| `WINUXCMD`/`WINUXCMD_PATH` env | path.rs:98-99（与 ROOT 同一张表） | **概念升级为可注入 coreutils provider**：引擎只认中性的 coreutils 注入点（如 `COREUTILS_PATH`/`SHELL_COREUTILS_DIR`），宿主注入任何 coreutils 集合——现阶段注入的是 winuxcmd，跨平台时可直接注 GNU coreutils。旧名转兼容别名随迁移期淘汰。产品名 winuxcmd 本身不改 |
| `WINUXSH_SHELL_PATH_STYLE` | init.rs:7、builtins/cd/paths.rs:113 | 改宿主注入配置（如 `__RUBASH_PATH_STYLE`） |
| `WINUXSH_HIST_IGNORE_DUPS/_SPACE` | zsh_options.rs:111/117 | 改名迁移（zsh→bash 迁移期兼容选项） |
| `~user` 读 winuxcmd passwd 数据 | expand/tilde/tilde.rs:67-70 | **功能依赖**：改挂 coreutils provider 的 passwd 数据接口——provider 换成 GNU coreutils 时用 /etc/passwd，零引擎改动 |
| path.rs 内嵌 winux/winuxcmd 路径处理串 | 479/512/521/524/1288/1841/1849/2152 | 逐个审：平台路径翻译逻辑（正当）但命名去 winux 化 |
| 注释引用 winuxcmd | command_substitution_pipelines.rs:605、pipeline_exec.rs:757/984、path.rs:27、init.rs:250 | 允许存在（注释），措辞顺手中性化 |
| bash shim | bin/bash.rs | 保留（AI invoker 入口，feature-gate + 注明用途） |
| 测试夹具串 | support_names.rs:593、tests/ 若干 | 保留（fixture） |

1. 定新名（如 `NIU_SHELL_ROOT` 或中性 `SHELL_ROOT_DIR`），由宿主注入；
   引擎 `set_shell_root` 的导出改挂新名，`WINUXSH_ROOT` 转为兼容别名继续
   导出一个迁移期。
2. 引擎内部读取（path.rs 三别名表收敛、pwd.rs）切换到新名。
3. niubash 侧的读取点（completion/path.rs:231、shell.rs:4972、
   syntax_highlighting.rs:586 的 "deprecated rubash bridge"）同步切换。
4. 迁移期满删兼容别名。

**相关决策**：winuxcmd 维持 PATH 级集成，不做 FFI（进程内绑定破坏 GNU 的
"外部命令=独立进程"模型边界，维护成本换不来语义收益）；**概念上引擎只认
"coreutils provider"**——中性注入点 + passwd 数据接口，winuxcmd 是现阶段的
提供者，跨平台时换注 GNU coreutils 即可，产品名 winuxcmd 本身不改；引擎对
winuxcmd 的引用仅允许存在于注释。

## 四、热点文件提示（改动需extra谨慎）

`executor/mod.rs`(677 次)、`command_prepare.rs`(123)、`command_substitution.rs`(98)、
`compound_exec.rs`(92)、`embedded_mutations.rs`(81)、`lexer/mod.rs`(79)、
`trap_exec.rs`(77)——这些是历史事故密集区，改动前先查本文件第一节对应家族，
改完跑对应族探针。

## 五、快速禁令（code review 自查）

1. 禁止新增词级 `contains()`/`starts_with()` 语义准入（#117）。
2. 禁止裸写哨兵字节/码点字面量；必须走 markers.rs。
3. 禁止在业务文件自定义标记常量副本（历史上 E102/E107/E108 曾 4 处重定义）。
4. 禁止新增"文本切片+再解析"式 comsub 捷径。
5. 禁止内建绕过重定向引擎直写 std handle。
6. 新增可变 shell 状态必须进 ShellState（或明确记入子壳隔离清单）。
7. 新增 POSIX 分支必须集中在唯一的 posix 判定点，不许散落。

## 六、问题热点索引（issue/PR 交叉版，2026-09-20）

数据来源：rubash 全量 118 issues + 28 PRs、WinuxCmd/niubash 关联 issue、
git log 交叉核对。按"反复出问题"频次排序。

| # | 热点主题 | issue/PR 证据 | 家族 | 套件 | 主要源文件 |
|---|---|---|---|---|---|
| 1 | 词层 fast path / comsub 捷径漏语义（黑名单守卫不断被打穿）——**#117 母 issue 已解决（2026-09-22，2d7c973a 白名单正解），区域已收敛（见第八节注记）** | #68 #69 #70 #116 #117、PR#101/102/116；niubash#119 | S1+S5 | 全套 | command_substitution*.rs、parameter_core.rs |
| 2 | 数组/关联数组复合赋值、下标、declare 回显 | #24 #77(已解决待关) #79 #109 #111；niubash#72 #78 | S2 | assoc/array/quotearray——**三套件已全部 0 差（2026-09-21 true-baseline），区域已收敛** | builtins/declare*、arrays |
| 3 | 载体字节泄漏到用户可见层 | #64 #95 #96 #97 #109、PR#104/114；niubash#92 #103 #124 | M 类 | quotearray/posixexp/varenv | lexer/quotes.rs、eval_source_for_reparse |
| 4 | comsub 捕获/重解析 | #69 #70、PR#6/7/9/102/104/115；niubash#76 #120 | S5 | heredoc/comsub、bashdb | command_substitution.rs |
| 5 | varenv/nameref/tempenv | #24 #78 #86；audit C2–C15 批次 | S7 | nameref11/varenv | varenv、nameref |
| 6 | 解析器：同行 `#` 尾注释 / CRLF / `{` 未闭合（三仓库各报一次） | #118(PR#119)、#31；niubash#106 #130 | S1 邻域 | **无专属套件（缺口）** | lexer/（continuation.rs captain-exclusive） |
| 7 | 重定向/fd/dev 别名 | #89(G17)；niubash#118 #122 | S3+S9 | redir | spawn/redir |
| 8 | Windows 路径/argv 修辞 | #31 #59 #60、PR#103(#1/#5)；niubash#61 #62 #83 #88 | S10 | 环境绑定 | external_argument_path |
| 9 | 子壳隔离（同一问题两半分两次修） | niubash#70 #100 | S6 | 靠 niubash 回归 | compound_exec.rs、ast_exec.rs |
| 10 | 算术/错误消息/errexit | #67 #73 #74 #83 #88、PR#110 | S8+S11 | arith | arith、expr 错误模型 |
| 11 | trap/shopt/nullglob | #85 #91(G19)、niubash#121 | S9 | trap | exec/trap |

## 七、13 家族未覆盖的新问题域（issue 证据）

1. **CLI/调用选项**：bundled 短选项、`-c -l`（niubash#107、PR#112）——提了两次。
2. **位置参数暴露**：`$0/$1/$@`、`${arr[@]+"${arr[@]}"}` 解析（niubash#15 #73）。
3. **alias 语义**：脚本模式 expand_aliases 无条件开启、alias 覆盖函数（niubash#129、#22/#23）。
4. **交互/readline/PS1/completion**：niubash#53 #54 #75 #91 #117；bashdb 集成同域。
5. **后台任务/进程生命周期**：`&` 无法脱离、stdout 早关 panic（niubash#122 #125）——比 S9 更具体的"进程脱离/句柄释放"主题。
6. **性能/冷启动**：负 lookup 60–85ms、`-c` 550ms 固定开销（#71、niubash#79、PR#10–14）。
7. **多字节/编码边界**：中文路径 char_boundary panic、ACP 解码（niubash#84 #86 #88 #92）。
8. **环境注入/隔离**：THIS_SH 子进程 env scrub、temp env 不可见于嵌套（niubash#38）。

## 八、未关闭与映射缺口（需复核）

**open：** #62（gnu-baseline 归因账本）、#77（declare -A/-ai 回显——**已解决待关单**：assoc/array/quotearray 全零差；修复=assoc/declare 批次 27fa0076/4883ad0b/802f8382/1caa7246 等 + 复验 3dae1658，关单须补映射评论）、
#117（词级捷径白名单化母 issue）——**已解决（2026-09-22，2d7c973a）**：黑名单守卫级联删除（command_substitution.rs −280 行），替换为白名单谓词 command_substitution_body_is_trivial；有害输出 stub（硬编码 "129"/"4"）一并删除；continuation.rs 配套 7 行已 captain 审查。验证：comsub 矩阵 GNU 逐字节、comsub2 184→0、全量无回归。保留项：type -p 空 PATH ./ 回退 stub 单独跟踪。niubash#104（winget，特性请求）。

**issue→提交映射缺口（建议复验）：**
- **#66 嵌套花括号展开**：已关闭但找不到修复提交——最明确缺口，需复验是否真修。
- **#64 awk -F '\t'**：评论称 "fixed in working tree" 但无对应提交；确认回归测试
  `external_pipeline_preserves_quoted_awk_field_separator_argument` 在 master。
- **#98 (G26)**：关闭留言不给哈希，无法追溯。
- **niubash#118 只修 1/3**：ce89fa85 只覆盖 /dev/std* 别名；内部 cd 改道、ssh 无输出
  两项无 rubash 提交记录。
- **niubash#92 multibyte panic**：PR#104 是 follow-up，本体修复无独立提交。
- 反向缺口为零：所有声称修复的提交，issue 侧均已关闭。
