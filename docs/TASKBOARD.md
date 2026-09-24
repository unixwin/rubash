# Rubash 任务板（Task Board）

> 唯一权威任务清单。每次派发/完成/验收后由协调者更新。
> 状态：DONE ✅ / IN-PROGRESS 🔵 / QUEUED ⏳ / BLOCKED ⛔
> 纪律：每任务完成必须带回归测试；禁止新增哨兵字节；PASS 只认 WSL GNU 5.3.0
> script-file 口径；完成后 master 复跑受影响基线切片。
> 最后更新：2026-09-25（82/83 台账 + Q5/Q6/Q7 核销 + nameref 时序归档）

## ✅ 已完成（2026-09-20 ~ 09-22 主冲刺）

| ID | 任务 | 证据 |
|---|---|---|
| T1 | 宿主层 A1/A2/B1 删除（niu 薄壳化） | origin/niu master 4873627 |
| T2 | markers.rs 注册表 + 解码边界闭合（M1-M5） | f8124f8b..5ada2463 |
| T3 | FdTable 真句柄落地（M1-M5 + 12 探针转正） | 7e56967d..a45e610c |
| T4 | 交互错误前缀 interactive 分支 + niu SHELL_NAME | 534bc2e8 / niu 1312cdd |
| T5-B1 | StdinBody enum 试点（B1 碰撞根除） | d6e1624a |
| T5-B2/B3 | PATSUB/守卫金标断言 | 430d3523 / 71628dbd |
| #117 | 白名单架构正解（−280 行黑名单，硬编码 "129"/"4" 桩删除） | 2d7c973a（已关单） |
| 审计 | 全仓准入守卫审计（1123 处分类，B 九族/C 八桩清单） | docs/admission-guard-audit-20260922.md |
| 归因 | harness 归因清扫（真账 ~480 / 噪声 ~195 分账） | docs/harness-attribution-20260922.md |
| #66/#64 | 映射缺口补录（7baee60f 等）+ niubash#118 复核（新立 #120） | GitHub |
| Q5 | histexp 修复（!!/!str 透传） | ✅ 09-25 台账切片归零（histexp 0 env=0） |
| Q6 | alias 修复（eval/alias 展开失效） | ✅ 09-25 台账切片归零（alias 0 env=0） |
| Q7 | redir/vredir fd→readonly 语义 | ✅ 09-25 台账切片归零（redir/vredir 0 env=0） |
| 严差 | nquote ANSI-C/quotearray carrier/coproc 脚本名前缀 | fa321959 / 06fc6124 / 71c933eb |
| 归档 | nameref `RO_PID`：coproc 收割时序竞争，GNU 隔离探针同样偶发——归时序残余，不修代码 | 82/83 台账（5780be63） |
| Q2 | **C 类八桩清除** | ✅ 09-25 核销：八桩指纹已全部消失（C1 kill -l 表/C3 cd/C2+C8 type coprocs/C4 coproc reprint/C5 aliasconv sed/C6 eval parse 错误路径/C7 histexp `\!`），守护套件 type/builtins/errors/dstack/comsub 全绿 |
| T5-B4/5/6 | ASSIGN_DATA/CTLESC/命名字符串三族金标断言 | ✅ 按治理文档 09-22 已落地（locale.rs decode_to_visible_text 生产断言 + marker_leak_golden.rs），核销 |
| Q1-B8 | THIS_SH 子串判定 → 路径等价 | 71ffce5c |
| Q1-B4 | BASH_COMMAND 文本探针 → GNU 无条件刷新模型（探针删除） | 9388e774 |
| Q1-B6 | `[[ ]]` RHS `raw.contains('\\')` → word_quotes 结构化（新增 QuoteKind::Backslash + 嵌套体跳过） | 7d65b573 |

## 🔵 进行中

| ID | 任务 | Owner | 备注 |
|---|---|---|---|
| T5-B4/5/6 | ASSIGN_DATA/CTLESC/命名字符串三族金标断言 | Devin | Batch 2 完成后顺延 |
| A-ShellState | ShellState 收尾（comsub 快照收编 + jobs 线程模型）| Devin | e2a70089 起步，milestone 持续落地（8e4e27c3 job table 已进）|
| A-niu基线 | niu 产品基线（RUB_OVERRIDE 已支持）| Devin | 14258c9a 前置已落地 |
| A-#117余项 | alias 解析流重构（parser input stream）| Devin | 6a1c4603 起 |
| A-S1-S5 | 引擎下沉清单（script_driver pub/EOF trap/cp src.//history rc/-c set -H）| Devin | 7294e563/e6422b1a 已落地部分 |
| X-1 | type -p/-P/-a 全因子修复 | ZCode agent | fix/type-p 已合入 master（46028796）|
| X-2 | issue 复核（#66/#64/niubash#118→#120）| ZCode agent | 已完成，#120 新立 |

## ⏳ 排队

| ID | 任务 | 依赖 | 验收 |
|---|---|---|---|
| Q1 | **B 类九族语义准入守卫替换**（审计文档 B1-B9）：B4/B6/B8 ✅ 已落地；剩余 B1（~40行白名单收敛，最高风险需全量台账）、B2（~30行 $@ 标志化）、B3（别名展开前移 lexer）、B5（procsub 解析阶段识别，23 站点）、B7（alias value 结构化引号态，低风险可控探针可缓）| T5 依赖已解除 | 每族 GNU 对拍 + 对应套件无回归 |
| Q2 | ~~C 类八桩清除~~ | ✅ 已核销（见上）| 守护场景持续通过 |
| Q3 | **niu 产品基线复跑**（niu.exe 47/2357 → 目标 ≥54/928 对齐）| 等 niu 分歧消除 agent 完成 | 双层台账对比表 |
| Q4 | **winux 痕迹改名迁移**（ADR 3.8：54 处/7 文件）| ✅ 完成（09-22，ccb3615a/f0bdaf45/473def11）：中性名优先（__RUBASH_PATH_STYLE/COREUTILS_PATH/SHELL_COREUTILS_DIR），旧名转兼容回退；WINUXSH_ROOT 导出标记 deprecated niu bridge；bash.rs 标注 AI invoker 入口；探针残留已清 |
| Q5 | ~~histexp 修复~~ | ✅ 已核销（见上）| 切片归零 |
| Q6 | ~~alias 修复~~ | ✅ 已核销（见上）| 切片归零 |
| Q7 | ~~redir/vredir fd→readonly 语义~~ | ✅ 已核销（见上）| 切片归零 |
| Q8 | 差分模糊测试放量（60 → 1000+ case）| 等原型验收后 | 分歧清单产出 |
| Q9 | pty 端到端测试（niu 侧 reedline/PS1）| 排队 | 交互探针集 |
| Q10 | 性能专项（#71 冷启动/-c 固定开销）| 兼容性达标后 | 基准对比 |
| Q11 | **/proc 最小仿真**（P1 引擎内合成：fd 别名 + open 咽喉；P2 参数经纪人物化，覆盖 bat 等第三方；计划见 docs/proc-vfs-plan.md）| P1 可立即（与 Q1/Q2 同域防冲突）；P2 依赖 P1 | P1: 字段格式单测 + read/mapfile/external_cat cli tests；P2: 外部 exe 以 /proc 参数读到内容（bat 探针）|
| Q12 | **Linux 交叉编译修复 + CI 跨 target 门禁**（fd 层双平台分发器 + unix POSIX 实现）| ✅ 完成（09-23，c144a0c9/d9486c1d）：fd/mod.rs 变 dispatcher（windows_impl 原样保留 + unix.rs POSIX 实现 HANDLE=RawFd），/proc hooks cfg 化（shell_options RawHandle/RawFd 分平台、init.rs HANDLE 别名解析、parser::assignment 公有化）；x86_64-linux-gnu / aarch64-apple-darwin / Windows 三 target `cargo check --tests` 全零错，`cargo test --lib` 424 过（仅 2 个既有 substitution_metadata sentinel）；CI 新增 cross-target-check 门禁（linux/darwin lib+tests check）|

## ⛔ 阻塞/等待

| ID | 事项 | 阻塞原因 |
|---|---|---|
| B-1 | niu 产品基线首轮复跑验证 | 等 Q3 的消除 agent 分支合并 |

## 📋 参考

- 治理文档：docs/pitfall-taxonomy-and-governance.md（事故分类/ADR/禁令）
- 归因报告：docs/harness-attribution-20260922.md
- 审计报告：docs/admission-guard-audit-20260922.md
- niu 基线：docs/niu-product-baseline-20260922.md
- niu 审计：docs/niubash-audit-20260920.md
