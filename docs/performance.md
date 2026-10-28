# 性能对比账本（维护中）

> 本文档是 rubash 相对 GNU Bash 5.3 的性能事实唯一入口：只记录**已合并、
> 已门禁**的数字。归因细节与完整矩阵见各 issue 文档（`perf-issue-*.md`）；
> 测量脚本在 `~/bench_niu/`（`sweep_618_keys.sh`、`sweep_616_keys.sh`、
> `attrib_618.sh`）；回归电池在 `tests/`。
>
> 测量纪律（每次更新必须遵守）：同树 release 二进制、顺序轮换、
> min-of-N；本地 GNU 是 5.3.20（仓库 oracle 是 WSL 5.3.0），语义以
> `third_party/bash/` C 源码为准；主机从不满载（读数取 min 消除）。

## 数组元素写：O(n²) → O(1) 均摊（已落地 #618 #616）

`m[k$((i%K))]=$i` / `a[i%K]=$i`，N=20000 次写入，单次写成本（release）：

| keys | assoc master | assoc branch | indexed master | indexed branch | GNU |
| --- | --- | --- | --- | --- | --- |
| 1 | 44.75µs | 45.90µs | 14.80µs | 15.45µs | ~5µs |
| 100 | 87.45µs | 47.85µs | 87.75µs | 20.85µs | ~5µs |
| 1000 | 537.40µs | 48.90µs | 782.80µs | 20.45µs | ~5µs |
| 5000 | 2341.35µs | 52.40µs | 3571.40µs | 20.60µs | ~5µs |
| vs GNU | 445x | **9.9x** | 736x | **3.8x** | — |

- 机制：元素写 O(1) 追加一段存储段，预算触发规范渲染压缩（串长 ≤ 2 倍
  规范长度）；assoc 读零改动，indexed 读天然折叠。
- 写路径分解（K=5000）：重渲染 73–75%，parse-memo 的 Vec 深拷贝
  24–25%，字符串克隆 0.05% —— memo 化只去掉了扫描，修渲染不够。
- 附带修好 O(n²) tokenizer（每段 collect 整个剩余输入；10001 段真解析
  563ms→17.8ms 线性）。
- 残留（非本 issue scope）：固定 pipeline 开销（K=1 即存在，与 scalar
  同源）；K≤3 约 +1.5µs memo-miss 结构价（K≥5 转正）；4 处冷路径
  collect；legacy 非前缀串仍数段（无生产者）。

## 标量/展开/循环（归因完成，待修；issue 文档为准）

| issue | 现状 | 第一杠杆 |
| --- | --- | --- |
| #607 arith-for 2.37x | 关闭：#665+#671+#679 三刀全合（`:` 2.03x→1.77x；票据 dense ~5%；循环 −3.4%）；剩形状无关单命令地板（~2.8µs，另立 issue 才做） | — |
| #608 param 扩展 | 已合 #666：`${s^}` ~6x，`~~` ~10%，电池 33/33 | 关闭 |
| #609 scalar 赋值 | 关闭：#672+#678（net 1.06x GNU，电池 30/30） | — |
| #619 控制流 | 关闭：#661/#662/#663/#664/#681(a+b) 全合；#675 另合 | 残留见 #682 |

详见各归因报告与 `docs/perf-issue-609.md`；本表只在落地后更新数字，
归因阶段不写。

## 2026-10-28 性能轮：A 表回归通道 + shellbench 语料 v2（已合并、已门禁）

本节是 2026-10-28 性能轮的定版记录。数据单源：shellbench 仓
`results/PERF-ROUND-2026-10-28.md` 与本轮 REPORT（下文「REPORT §n」指该
报告章节）；本页是引擎性能数字的唯一正页，产品面只许摘要并链回这里。

### 口径（本节每个数字共用）

- release 构建，同机交错采样；采样门：load1 ≤ 2.0 且无其他 cargo/rustc。
- 被测：rubash master `231ffe1a`（release）、GNU bash 5.3.20（自编译
  oracle）、niubash master `c06b2a08`（release，内嵌 rubash `231ffe1a`）。
- 平台仅限本 Linux VM。跨平台计时推断禁止——本节数字不说明 Windows 量级。
- niu 阶段采样时环境负载偏高（load1 1.3–5.6），niu 绝对值是上界、比例偏
  悲观；引擎继承形状与 rubash 列贴合（REPORT §1）。

### A 表（30 例 harness，2026-10-25 原样未动——唯一跨轮回归通道）

user-median 比例（rubash/GNU，越小越快）：

| 用例 | 2026-10-25 | 2026-10-28 | 判定 |
| --- | --- | --- | --- |
| array_arith_read_8000（`$((a[i]))` 直读） | 10.98x | 2.99x | 形状差距关闭（PR #1615，见族账） |
| assoc_fill_100k | 3.91x | 5.03x | 回归 +29%——已由 F2 关闭（见族账） |
| array_fill_30000 | 3.83x | 4.99x | 回归 +31%——已由 F2 关闭 |
| array_fill_4000 | 3.68x | 4.75x | 回归 +28%——已由 F2 关闭 |
| floor_if_redir | 7.19x | ~9.4x | 标旗观察（绝对值小，见下注） |
| arith / func / assign / read_loop | 2.8–3.2x | 2.83–2.95x | 地板税未变 |
| pipe_200 | 0.05x | 0.045x | 领先保持 |
| comsub_loop | 0.11x | 0.107x | 领先保持 |
| heredoc_10mb | ≈4x | 4.05x | ≈ 基线 |
| big_scalar | ≈5x | 4.85x | ≈ 基线 |
| declare_p_dump | ≈4x | 4.10x | ≈ 基线 |
| array_literal_100k | ≈4x | 3.87x | ≈ 基线 |

注一：填充三例的 2026-10-28 列是修复前量测（master `231ffe1a` 含该
回归）；修复后绝对值见族账 F2，已回基线。
注二：floor_if_redir 绝对值很小（GNU user 0.0123 s / rubash 0.1162 s，
REPORT §6），窗口噪声是活的替代解释——列观察项，不作回归定性。

### 族账（本轮）

- **F1 长字符串模式操作——已修（PR #1909，已合并）**。机制：移除走查
  （`parameter_decode.rs`）与替换 finder（`parameter_replace.rs`）逐字符
  边界试一个候选切片、每次试都把 `case_pattern_matches` 的字节门控全量
  重扫，即每展开 O(n²)（REPORT §5 F1）。修法：门控按 (pattern, value)
  对只求值一次（新共享 `case_pattern_matches_prechecked`，门控命中的对
  保持逐候选原路）+ 锚定 finder 的无星号长度界剪枝。同口径正式复测
  （shellbench expand 类，门控窗口内）：

  | 用例 | master | F1 后 | GNU |
  | --- | --- | --- | --- |
  | scale_replace_anchor_head@l（`${v/#seg/SEG}`，5411 字符） | 498.68x | **1.00x** | 95.83 µs/op |
  | scale_replace_anchor_tail@l（`${v/%gz/GZ}`） | 206.68x | **0.41x** | 232 µs/op |
  | scale_suffix_rm_long@l（`${v%%.*}`） | 84.39x | **0.24x** | 574.52 µs/op |
  | scale_prefix_rm_long@l | 2.29x | 1.37x | — |
  | scale_replace_anchor_head@m | 22.46x | 2.03x | — |
  | scale_suffix_rm_long@m | 15.64x | 1.39x | — |
  | param_replace_prefix_l | 8.75x | 3.19x | — |
  | expand 类中位 | 3.980x | **2.925x** | — |

  门禁：差分探针逐字节一致（UTF-8 与 LC_ALL=C 双 locale）、diffcorpus
  306/306（失败集与 pristine 一致）、GitHub CI 全绿（含 niubash 配对
  构建、GNU goldens、跨目标）。

- **F2 元素写回归——已修（PR #1912，2026-10-28 合并，merge commit
  `5b71b664`）**。二分定位到 ded575d1（#1568，PR #1577）：其
  mixed-statement 臂跑在每条元素赋值之前，把整棵 `CommandNode` 深克隆
  并重建赋值记录——每写约 +4 µs（REPORT §5 F2）。修法：一次分类走查喂
  两个入场判定，元素-only 语句走原生臂。填充三例 user-median 绝对值
  （秒）：array_fill_4000 0.0551（基线 0.0552）、array_fill_30000
  0.4104（基线 0.4061）、assoc_fill_100k 1.3790（基线 1.373），对照组
  不变——回归全额回滚。门禁：赋值形状差分探针逐字节一致、diffcorpus
  306/306、`tests/issue1568_…` 13/13、CI 全绿。此三例已定性为 1.5.0
  发布门回归，在本轮内关闭（REPORT §9）。

- **地板税**：arith / func / assign / read_loop 维持 2.83–2.95x——这是
  简单解释器循环形状上每条命令的派发与求值固定成本带，本轮未动；
  跟踪在 #682（REPORT §3）。

- **`$((a[i]))` 形状关闭**：perf-arrays（PR #1615）落地后，其待验的
  严格窗口正式量测由本轮 A 表补上——10.98x → 2.99x，回到地板量级
  （REPORT §3）。

- **领先面**：pipe_200 0.045x、comsub_loop 0.107x——零 fork 进程内
  执行模型的结构性领先，本轮保持（REPORT §2）。

- **F3 关联数组元素读 / F4 解析吞吐 / F5 declare·getopts 每调用常数 /
  F6 procsub 读放大**：优化进行中。证据包在本轮 REPORT §5，数字待
  修复落地、门禁通过后再入本账——本节不定性、不预支。

### shellbench 语料 v2 与方法学声明

- 语料 v2 共 779 例（reps 5 / warmup 1，runner 按壳自动调迭代数至
  ~200 ms）。语料内计数：领先（≤1.0x）137 例、接近（1.0–1.5x）73 例
  （REPORT §4）。
- **v2 逐例比例与 2026-10-25 的 gap-v1 表没有共同分母**：v2 重写了
  生成器与夹具、runner 按壳自适应迭代数，旧新逐例比例不可比（证据见
  REPORT §4）。跨轮回归结论只许走上面的 A 表通道；v2 数字只在语料
  内部有效。语料与原始结果见 shellbench 仓
  （`github.com/unixwin/shellbench`）`results/`。

## 方法（防遗忘）

- failure-**set** diff，永不比 count；门禁串行 `--test-threads=1`；
  单 binary 双开 cargo 必串行（ETXTBSY 会废掉整轮）。
- 每个修复配 GNU 差分电池（master 基线先跑出 guard 分布）+ 回归测试
  文件 + 全量门禁 + niubash 产品 A/B（只切 `[patch]`）。
- 脏测量作废整轮：过期二进制、instrumented 二进制、中途改树。
- 终端交错：后台 agent 完成的通知会插进正在看的测量输出——关键读数
  一律落文件再读。
