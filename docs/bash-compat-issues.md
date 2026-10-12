# rubash 兼容性差距总览（issue #20-#26 汇总与维修计划）

> 历史汇总日期：2026-08-03（数据修正版：同步各 ISSUE 评论的最终数字）。数据来源：unixwin/rubash issue #20-#26 + 本地差分/上游测试。
> 核心结论：**7 个 issue 的差异高度重叠**——同一批根因族在不同套件（oil spec / mksh / ksh93 / bash 官方 / busybox）中反复命中。按根因族维修，而非按 issue 逐个修。
>
> 2026-08-12 追加：最新本地套件运行、DIFF 形态、实现归因、rubash/winuxcmd/winuxsh 架构边界见
> [`docs/issue-suite-diff-analysis.md`](issue-suite-diff-analysis.md)。
>
> 2026-08-14 续接基线：当前以 `docs/issue-suite-diff-analysis.md` 的
> `Continuation Checkpoint` 和 `target/issue-suites/results/` 最新 raw 结果为准。
> 2026-08-29 起兼容性状态曾以 `docs/COMPATIBILITY-STATUS.md` 为唯一权威来源；该文件已于 2026-10-10 删除，现以 rubash README「Compatibility at a Glance」台账及 issue #477 归档为权威来源，
> 2026-08-22 账本与归因文档已退役。`.right` 上游
> runner 的 `87/87` 仍是独立的期望文件基线，不代表真实输出已 83/83。当前第一
> 执行族为 GNU Bash `redir/vredir` 动态 fd、设备路径、fd 生命周期和
> ordered redirect；不得用旧的 `14/83`、`87/87` 数字覆盖新的证据。

## 2026-08-14 远程 Issue 与本地执行状态

通过 `gh issue list --repo unixwin/rubash --state all --limit 30` 核对远程
状态：兼容性批次 `#20` 到 `#26` 仍开放；`#28`（bashdb getopts_long）和
`#31`（外部参数路径转换）已关闭。远程标题中的历史差异数字仍是问题
背景，不覆盖本地最新 raw suite 结果。

本轮已完成 `#25` Bash 官方重定向族的一个根因切片：dynamic `exec` fd
move/close。Rust dynamic-fd focused tests 为 12/12，`part_080` 为
146/149；GNU Bash upstream `.right` runner 的 `run-redir` 与 `run-vredir`
均为 1/1、exit 0。该结果只证明当前 slice，不关闭任何远程 Issue；官方
`.tests` actual-output、BusyBox、Oil、mksh 和 ksh93 仍按原计划执行。

当前必须保留的 raw 路径：

- `target/bash-upstream-tests/logs/run-redir.log`
- `target/bash-upstream-tests/logs/run-vredir.log`
- `target/bash-upstream-tests/results.tsv`（最后一次 focused invocation）
- `target/issue-suites/results/native-bash-20260814-vredir/`

下一次续接从 ordered stderr/`<>` 三个 Rust 失败和 native
`vredir4/5/7/8` 的 primitive probe 开始；不要重新定位已经确认的
external materialization 根因。

## 一、7 个 issue 概览（最终数字，与 ISSUE 评论一致）

| issue | 套件 | 规模 | 核心领域 |
|-------|------|------|----------|
| #20 | 自研 probe | 20+ 项 | 命令替换状态污染 / heredoc / case / 参数替换 / 路径转换 / 重定向 / 调试钩子 |
| #21 | 自研 probe r2 | 11 项 | 花括号 / IFS 分词 / glob 反斜杠 / BASHPID / 进程替换 / **coproc 挂起** / wait / 特殊内建管道 |
| #22 | oil spec（首跑半程快照） | 219 项（当时快照） | alias 语义 / **语法宽松度** / 数组赋值族 / 算术错误码 / jobs-dirs |
| #23 | oil 全量 + mksh 全量 + ksh93 全量 | **684 / 436 / 46** | case / heredoc / eglob / expand / break 越界 / 错误码 / ksh 复合变量 |
| #24 | oil 684 + mksh 436（全量清单聚焦） | 高频领域 | **内置族整体**（umask/trap/kill/set/echo/cd/shopt）/ word-split 35 / var-op 48 / nameref / mksh 进制 |
| #25 | bash 官方 83 tests | 历史记录 14/83；当前账本 13/83、70 raw DIFF | 与 #20/#21/#22 同批根因族（`.right` 旧期望掩盖） |
| #26 | busybox ash_test | **143 项**（17/17 目录完成） | **heredoc_huge 挂起（P0 DoS）** / vars 29 / redir 14 / signals 11 / 解析 / psubst |

> 差距总量：**1,345+ 项**（31 手动 + 684 + 436 + 37 可靠 + 46 + 143），全部已归入上述 ISSUE。

## 二、根因族聚类（跨 issue 交叉验证）

### 族 A：heredoc（P0 — 挂起/DoS，两个触发面）
- **触发面 1（上下文/状态）**：#20 §2.1 接收者上下文展开、#23/#25 heredoc 边界；本地已定位：heredoc body 词法收集在特定前置状态失败 + 后台 stdin 挂起（见 memory）
- **触发面 2（大输入/性能）**：#26 `heredoc_huge.tests`——**巨大 heredoc 输入** bash 秒完成、winuxsh 20s/30s 超时被杀（rc=124）、残留 .tmp——疑似缓冲/死循环/性能问题，与触发面 1 相互独立，需分别验证
- 修复点：heredoc.rs `<<` vs `<<<` 区分、body 收集状态复位、跨行收集、大输入缓冲路径

### 族 B：命令替换状态污染（最难，间歇性）
- #20 §1：深层函数链赋值捕获丢失 / cd PWD 拼接（已修 #12）/ printf 损坏 / 函数参数丢失
- 本地：#12 已修（PWD 拼接）；其余为执行后状态污染，需 debug 工具定位
- **已实测的机制之一：stdin 消费泄漏**——busybox 差分驱动中，测试脚本内的 `read` 会消费 while 循环的输入流（子进程继承 stdin fd），需显式 `< /dev/null` 才正常；推测与文档猜测的 FUNCTION_STDIN 环境变量泄漏同源，可作为定位线索
- 修复点：命令替换执行器状态隔离、FUNCTION_STDIN 等环境变量泄漏、子进程 stdin fd 隔离

### 族 C：IFS 分词 / word-split（高影响）
- #21 §1.2 空字段、§1.3 `$*` IFS（已修 #13）；#24 word-split 35 项大规模扩展
- 修复点：IFS 分词器边界（空字段/多字符 IFS/引号组合）

### 族 D：语法宽松度（bash 报错 rc=2、rubash 静默 rc=0）
- #22 §B、#23 eglob/bksl-nl、#24 echo typed args、#26 ash-parsing
- 修复点：解析器对 bash 拒绝的语法报错（数组 `a= (1 2)`、`[[ ) ]]`、extglob 错误等）

### 族 E：内置族整体异常（#24，覆盖约 120 项，单点修复收益大）
- umask（18 项，符号模式解析）、trap（37 项，ERR 语义/列表/-p/-l）、kill（15 项，-l/-s）、set/shopt（35 项）、echo、cd、jobs（17 项）
- 2026-08-xx 启动协议切片：Winuxsh 已将 `-n/-e/-u/-x/-o/-O/--posix/-s` 转交 Rubash 的共享 invocation parser；`-n` 与 `-e` 宿主回归通过。
- `pipefail` 已由 `src/executor/pipeline_exec.rs::pipeline_exit_status` 实现：2026-08-21 的脚本文件对照在 `-o pipefail` 下 GNU Bash 与 Rubash 均得到 `false | true` 的 `status=1`。仍需在下一次 cross-suite refresh 覆盖 `set +o pipefail`、CLI `-o pipefail` 与多段 pipeline。
- 修复点：各内置的完整选项/输出/错误码对齐（可对照 bash builtins/*.c + *.def）

### 族 F：数组赋值族（#22/#23，约 60 项）
- `+=`（`s+=(...)`）、稀疏负索引、`${@:off:len}`、declare 空格、空数组插值
- 修复点：数组赋值解析与插值边界

### 族 G：算术错误码（#22/#23/#24，约 25 项）
- bash 报错 rc=1（`'1'` 常量/浮点/负指数/nounset 算术）、rubash rc=0 静默
- 修复点：算术求值错误传播

### 族 H：alias 语义（#22/#23，约 15 项）
- 单引号 alias 也被展开（安全防御失效）、管道中 alias 失效（rc=127）、`unalias -a`、无参列出
- 修复点：alias 展开的引用检查 + 管道上下文

### 族 I：参数替换边界（#20 §2.5、#24 var-op 48、#26 psubst）
- `${v=}`/`${v:-}`/`${v:?}` 组合、slice 负偏移、patsub 反斜杠（#12 部分已修）
- 修复点：var-op 组合边界（对照 subst.c）

### 族 J：路径/glob（#21 §1.4 glob 反斜杠、#20 §4）
- glob 结果含反斜杠 → `${f##*/}` 失效（#12 已修 PWD/替换部分）
- 修复点：glob 结果路径分隔符归一化

### 族 K：coproc/进程替换/后台挂起（P0）
- #21 §2.1 coproc 挂起、进程替换嵌套输出丢失（§1.5）、#26 挂起家族
- 修复点：coproc/进程替换执行器（与族 A 的挂起模式同源）

### 族 L：调试钩子（#20 §6、#25 dbg-support）
- 已修部分：DEBUG/RETURN trap、PS4、BASH_COMMAND（对应 PR #3/#4 —— **需核实是否已合入**，git log 近期提交未见调试钩子修复）
- **未修**：FUNCNAME 缺 main（差分 case-10，rubash#20 §10.5）、BASH_VERSINFO、trap DEBUG/RETURN/EXIT 触发语义、#24 trap 37 项中的其余部分

### 排除项：ksh 特有语法（非兼容目标，勿修）
- ksh93 复合变量/多维数组（`${a[0][0]}`、`${p.len}`）：**bash 本身不支持**，rubash 无需兼容——ksh93 测试中这类用例的差异**不是缺口**，直接忽略
- **注意区分**：ksh93/mksh 测试里 bash 也支持的子集才是真差异，需逐个确认：
  - `n#base` 进制字面量（`$((16#ff))`，mksh 29 项中 bash 支持的部分）——bash 支持，rubash 需对齐
  - `$LINENO` 在 `[[ ]]` 内的展开（ksh93/mksh 互证）——bash 支持，需对齐
  - alias/case/heredoc/quoting 等 POSIX/bash 通用语义——bash 支持，是真差异（已归入对应族）

## 三、维修顺序建议（按根因族，非按 issue）

原则：**P0 DoS 优先 → 高影响确定性差异 → 单点收益大的内置族 → 深层机制**。
每族修完**立即跑全套回归**（差分 23 + 上游 87 + 该族聚焦用例），不攒到最后。

| 优先级 | 根因族 | 依据 | 预计工作量 |
|--------|--------|------|-----------|
| P0 | 族 A heredoc 挂起（#26 大输入 + #23/#25 边界） | DoS + 常用特性；本地已定位触发面 1 | 中（词法层，有定位结论） |
| P0 | 族 K coproc/进程替换挂起（#21 §2.1） | DoS | 中 |
| P1 | 族 D 语法宽松度（#22/#23/#24/#26） | 影响所有脚本解析正确性 | 中（解析器报错） |
| P1 | 族 C IFS 分词（#24 35 项） | 高影响、确定性 | 小-中（#13 已修核心） |
| P1 | 族 E 内置族（#24 约 120 项） | 单点修复、覆盖广 | 中（逐内置对齐） |
| P2 | 族 H alias（#22/#23） | 15 项、安全防御 | 小 |
| P2 | 族 F 数组（#22/#23 60 项） | 中等 | 中 |
| P2 | 族 G 算术错误码（25 项） | 中等 | 小 |
| P2 | 族 I 参数替换边界（#24 48 项） | 中等 | 中 |
| P2 | 族 J glob 路径（#21 §1.4） | 已部分修 | 小 |
| P3 | 族 B 状态污染（#20 §1） | 最难、间歇性 | 大（需 debug 工具） |
| P3 | 族 L 调试钩子残余（FUNCNAME/BASH_VERSINFO/trap 语义） | 已修核心，残余少 | 小 |

## 四、测试与验证策略

1. **回归基线**：差分测试（按对应 artifact 记录）+ 上游 `.right` runner（87/87 历史基线）+ Bash 实际输出账本（13/83）+ Rust 测试
2. **每族聚焦用例**：从 winuxsh probe/suites（oil/mksh/busybox）提取该族代表用例，固化到 tests/difftest/cases/ 作为回归
3. **大型脚本**：自建综合脚本（已发现 heredoc 挂起）+ 真实构建脚本（如 Git contrib、经典 configure 脚本），bash vs rubash 对比
4. **GNU Bash 源码映射**：`docs/bash-source-map.md` 有 .c/.def → .rs 映射；修每族时先读对应 C 源码（subst.c/execute_cmd.c/builtins/*.c）再改 .rs，避免盲改
5. **winuxsh 侧套件**：`winuxsh/scripts/probe/suites/` 下 5 个差分驱动（spec/mksh/ksh93/bash-tests/busybox）可随时全量回归，DIFF 数必须单调下降

## 五、GNU Bash 差距评估（当前，最终数字）

- 上游 run-* 套件（`.right` 对比）：87/87 —— **旧期望文件通过，不等于真实输出 83/83**
- bash 官方 83 tests 实际输出对比：13/83（2026-08-22 账本；70 项 raw DIFF，需按归因分类处理）
- 差分测试 26 case：22/26（真 bug 3 个：case-01/03/05 + 版本身份 1 个：case-10；新增 case-24 var-op / case-25 arith / case-26 alias 全 PASS，2026-08-03）
- oil spec：228 文件 684 项差异（#23/#24）——**系统性差距仍在词法/解析/执行边界**
- mksh：436 项；ksh93：46 项（**其中 ksh 特有语法部分为非目标**，bash 支持子集需逐个筛选）；busybox ash：143 项（含 vars/signals 新领域）
- **2026-08-03 修复进展**：族 D（语法宽松 a= (1 2)/[[ ) ]]）、族 C（$* IFS/$@ 多词/空字段）、族 F（数组 +=）、族 G（算术错误码传播）、族 H（alias 单引号/管道）、族 I（& 替换/嵌套 slice）、族 K §1.5（进程替换嵌套）已修；进制字面量/$LINENO-[[ ]] 验证通过；heredoc_huge（族 A 面 2）与 coproc fd 映射（族 K §2.1）为剩余 P0
- **验收量化门槛（引用 winuxsh#48）**：bash 官方 83/83、oil 684→0、mksh 436→0、ksh93 错误数→0、手动 31 项复测、cloc rubash/src ≥80%（当前 60,708 / bash 132,879 ≈ 45.7%）
- 结论：距完整 GNU Bash 支持仍有**中-大规模差距**，集中在：heredoc 机制（含大输入挂起）、coproc fd 映射、语法错误处理（解析器报错机制）、内置族完整语义（umask/trap/kill 等 120 项）、路径转换（/tmp 映射 winuxcmd 侧）（ksh 特有语法不在目标内）

## 六、bashdb 外部调试边界（2026-08）

bashdb 是 Rubash 的外部压力工具，不是 Rubash 的产品依赖。当前已验证的核心调试链路包括：`list`、`list foo`、`step`、`next`、`continue`、数字位置断点、`where`、`eval`、`info functions`、`info files`、nested `debug` 和 nested `shell --norc`。Windows source identity 已统一为 `/d/...`，`continue 4` 能稳定停在目标源码第 4 行。

已定位的边界：

- `shell --shell /usr/bin/bash --norc` 报 `--init-file: command not found`；native Bash 运行同一份 clean bashdb 也复现，因此归因于 bashdb 的 `OPTARG`/`OPTLARG` 组合，不作为 Rubash 修复目标。
- `shell --no-vars --no-fns` 已通过。
- `info variables` 无过滤参数、`info variables -a` 和 `info variables -A` 均已通过；根因是普通 `declare` 与显式属性过滤的输出模式混淆，现已分离并加入回归测试。
- sourced 文件错误现在保留 sourced 文件和真实行号，同时不再改变 `$0`；已加入 focused regression。
- 有界命令矩阵已覆盖 `help`、`break/clear`、`display`、`eval`、`set/show`、`info files`、`info functions` 和 `info variables`。`watch` 需要当前 frame 中已有变量；clean bashdb 没有顶层 `unwatch` 命令。`restart` 使用 Windows 相对 launcher 路径时仍有独立路径解析边界。

后续验收要求：每个边界命令必须同时记录 Rubash 输出、native Bash 输出、stderr、退出状态和超时情况；确认是 Rubash-owned 后才修改 Rust，确认是 bashdb-owned 则保留复现证据并在文档中明确排除。

## 七、平台环境形态伪影（2026-08 实证，勿当语义缺陷修）

实证（target/env-probe2.log，桥接 WSL GNU 5.2.21 对照）：`declare -p`（无参）导出行数 rubash=118 vs GNU=18；nameref 套件上下文里 GNU 只有 2 行 `declare -x`，rubash 121 行。

机制：GNU bash 本来就把所有继承环境变量导出——差异不在导出语义，而在**继承来源**。wsl.exe 只把 WSLENV 指定变量送进 Linux 侧（GNU 的 env 很小），但 WSL interop 启动 Windows 二进制（rubash.exe）时给的是 wsl 宿主进程的完整 Windows 环境，且 `env -i` 清不掉（interop 不用 WSL 侧环境启动 Windows 子进程）。rubash 的 init 再把继承变量全部标记导出（mark_initial_exported_vars），与 Git Bash on Windows 行为等价。

影响与结论：任何使 `declare -p` 退化为无参调用的测试（如 nameref 的 `${!ref}` 未设路径）会放大出 ~100 行环境差异；varenv 的部分超产同源。这是 harness-env 伪影类（与 stderr/stdout 交错同类），**不得**为此改 rubash 的导出语义；如需收敛只能由 harness 侧给 rubash 修剪 Windows 环境（当前无低成本方案）。相关：rubash 的 `/tmp` 映射到 Windows temp，与 WSL `/tmp` 不同，探针脚本写 `/tmp` 会在 WSL 侧 grep 不到——共享文件一律走 `/mnt/d/...` 挂载路径。

另注（THIS_SH 毒化，已修 362e01e5）：winuxsh 包装器把 `THIS_SH=niu.exe` 注入 Windows 环境，interop 传给 rubash.exe 后旧 `or_insert_with` 让继承值压过自检——所有用 `${THIS_SH}` 子脚本的家族曾在旧 shim 下产出幻影输出。修复后 `current_exe()` 覆盖继承值。
