# Rubash 任务板（永久闭环 harness）

> 本文件是唯一权威任务板。任何分发前先读本表；任何车道完成/回收后必须更新本表。
> 闭环规则：评测过的语料 → `docs/CORPUS-COVERAGE.md`；评测结果 → 各专用文档；发现分歧 → GitHub issue → 回到本表排队 → 分配车道 → 回收（cherry-pick → 四门禁 → 推送 → 关单）。性能轮换穿插。全部清零后：全量回归验证轮。永不停止。

## 在途车道（7 agent，2026-09-27 晚）

| 车道 | 分支 | 任务 | 产出文档 |
|---|---|---|---|
| param2 | wt3/param2 | #208 #209 #211 #212（参数/转义族） | — |
| ci | wt4/ci | 今日全部已验证矩阵 + 历史语料（nvm/bash_completion/git-completion/oh-my-bash/bashdb/rustup/ruby-build）+ true-baseline 快照 → 黄金文件回归 + ci.yml 接线 | tests/regression/ + .github/workflows |
| corpus2 | wt4/corpus2 | 先建 `docs/CORPUS-COVERAGE.md` 总表（从 issue/git/docs/target 工件挖掘），只跑清单缺席语料 | CORPUS-COVERAGE.md |
| ecoute | wt4/ecosuite | modernish 混合评测套件 + mvdan/sh 解析语料 + nvm bats + bats-core（查重后跑，差异开单） | CORPUS-COVERAGE.md §ecosuite |
| deep201 | wt4/deep201 | #201 comsub 行号深水区（evalstring.c line_number 生命周期移植） | — |
| perfsuite | wt4/perfsuite | 性能专用基准设施：benchmarks/ + run-perf-suite.sh + PERF-BASELINE.md，≥10x 开 [perf] 单 | docs/PERF-BASELINE.md |
| linuxrun | wt4/linuxrun | Linux 原生真构建真运行：冒烟矩阵、P0/P1 活体验证、[linux] 单 | docs/LINUX-RUN-STATUS.md |

## 排队（回收后依次分配）

1. 反引号延迟解析残留（perf 车道记录，待开单）
2. perf 残余：批式 tokenizer 每趟重词法化 → 增量读取器（独立深水子系统）
3. perfsuite/linuxrun/ecosuite 产出的全部新单
4. 全部清零后 → **性能轮**（PERF-BASELINE 最差比值逐个攻）→ **全量回归轮**（全套件 + cli_tests + lib + 双交叉，确认零回归）→ 仍清零则再挖新语料

## 今日战绩（2026-09-27）

关闭 21 单：#131 #176 #178 #191 #192 #194 #195 #196 #197 #198 #199 #200 #202 #203 #204 #205 #206 #207 #210 #213（+证据关单）；#201 半修在途。新开 #207-#213。提交 21+ 笔推送至 9bf2df9e。车道 28 条分发。

## 相关文档

- docs/COMPATIBILITY-STATUS.md — 83 套件权威账本（勿整轮重跑）
- docs/CORPUS-COVERAGE.md — 语料覆盖总表（corpus2 在建）
- docs/PERF-BASELINE.md — 性能基线（perfsuite 在建）
- docs/LINUX-RUN-STATUS.md — Linux 原生状态（linuxrun 在建）
- docs/cross-platform-gap-ledger.md — 旧跨平台缺口账本（历史参考，现状以本表为准）

## 2026-09-27 晚更新
- corpus2 ✅ 回收（25c7b912：CORPUS-COVERAGE.md 总表 175+ issue 挖掘 + 钉死夹具）；新单 #214-#217 → **corpusfix 车道在途**（wt4/corpusfix，含 ble.sh 整文件解析修复）
- TASK-BOARD 建立（ab63f0e5）
- param2 ✅ 回收（54e8df34：#208/#209/#211/#212 全修关闭，今日 25 单）；待办：#211 残余边缘 = continuation.rs 船长 diff（funsub push term_ready:true @501 + 613 臂加 top.term_ready 门）
- ecosuite ✅ 回收（7cbb21ed：modernish/mvdan/nvm/bats 四套件首跑，#218-#224 七单开立）→ **ecofix 车道在途**（wt4/ecofix，修完重跑套件）
- linuxrun ✅ 回收（4d648499：Linux 原生可跑——uname/chmod/test -x 四修 + 17/24 套件字节一致 + nvm 加载干净）；16 单（#225-#240）→ **linuxfix 车道在途**（#226 不可杀优先）
- ci ✅ 回收（1f30c090：tests/regression 21 测试 + windows CI job + 性能哨兵）；**发现 master 回归**：brace_scan_cache 深度≥4 恢复洞 → **regfix 车道在途**（P0 修洞 + snapshot 甄别 + 5 项分歧开单）
- perfsuite ✅ 回收（7dfdaeb0：24 探针基准 + PERF-BASELINE.md，最差 nvm -n 852x/configure 超时，spawn 1.5x 平价）；3 单（#241-#243）→ **perffix 车道在途**（#243 不终止 P0 + 解析中止项 + 首次热路径攻坚）
- corpusfix ✅ 回收（e572930a：#214-#217 四单关闭，continuation.rs 船长 diff 审查通过合入；bash-preexec/tldr CLEAN）；ble.sh 新单 #244（词内 {} 分隔符）/#245（--noattach 超时）/#246（declare -i 复合赋值标量化）排队下一波

## 配额中断（2026-09-27 20:15）
5 agent 撞 5h 配额墙（重置 22:52）。工作树状态已钉：regfix dirty=5（含 brace 洞修复中）、linuxfix dirty=11、ecofix dirty=16、deep201 dirty=5、perffix dirty=3。已约 22:55 定时复活：逐树续作协议唤醒（audit→build→续/弃）。
- regfix ✅ 回收（75b26599：brace 洞一行修复+可靠规则3、comsub2 快照之谜=死agent脏文件即缺失代码、21/21 零重录）→ 新单 #247-#251 排队
- deep201 ✅ 回收（2f611bcf：**#201 关闭**——深水半区 1500/1500 矩阵、comsub2 66→0）；残留：BASH_COMMAND canonical、xtrace PS4 LINENO 待开单
- perffix ✅ 回收（beb1eca6：#243 测量假象证据关单、#241 部分修复（}} 词规则+游离分隔符 35 形状矩阵）、剖析=重词法化排除/白名单 -5%）；剩 configure:5345+nst2≥3=skip_brace 族待发
- ecofix ✅ 回收（d3e8affa：#218-#224 七单，两处冲突=deep201签名+ecofix空体检查融合）；**红门禁已由 redgate 车道关闭**（wt5/redgate：nquote 快照红根因=d3e8affa escape_case_pattern_literal 把 raw-byte 标记对拆半 CTLESC 包裹 → `$'\v\f\a\b'` case 误报 bad；一并修复 shopt 多选项行扫描 `shopt -s nullglob extglob` 不开解析门 + scan_substitution_spans 只计 `$(` 不计普通 `(`（GNU parse.y:3952-3956 每个未引用 `(` 都嵌套）→ printf.tests 358-359 `@(hugo)`-in-comsub 拒绝；Linux comsub2=0 diff rc0、printf 359 行中止已修（残留 4 行+rc 差为窗口前旧账）；Windows regression 23/23、lib 481）；#252/#253 新单；t0286 continuation.rs 船长 diff 待审应用
- linuxfix ✅ 回收（5de3d1d2：14 单关闭，Linux 平价 17→19；#236 CTLESC glob 族、#238 环境绑定留单；线索：comsub2/printf Linux 侧回归系 9bf2df9e→4d648499 间引入——并入 nquote 红门禁同族调查）
- misc ✅ 回收（733d79c2：#247-#250 四单关闭+v=$(f) 双执行顺带修；新单 #254 builtin comsub 赋值位 xtrace 缺行排队）
- ble ✅ 回收（32d1458e：#244/#245/#246 三单关闭；ble.sh source 与 GNU 一致、-n 推进至 28570→新单 #255 排队）
## CI 从未绿过（用户发现 2026-09-28）——cifix 车道在途
三类红：macOS 2 个 uname 测试（linuxrun 改了 darwin 形态没跟上）、Windows golden_nounset_exit_matrix（本地绿 CI 红=最小 PATH 环境嫌疑）、nquote 快照（redgate 车道在修）。教训已记：回收流程加第五道门禁=gh run watch 远端 CI 结论。
- 三新车道：perf2（#241/#242 二攻：分配churn+完成命令边界检查点）、misc2（#254 builtin comsub xtrace/#255 ble 28570/#236 CTLESC glob/#238 环境甄别）、corpus3（git test-lib 框架/FFmpeg configure/OpenSSH configure/bash-it 测试套件——先查覆盖清单）。**开放板 9 单现已全部有主**（omb: #251-253；perf2: #241/242；misc2: #236/238/254/255）
- redgate ✅ 回收（d929ce5a：nquote 回归=标记对拆裂（对=单元修复）；comsub2 瞬态自愈；printf=harness 假零+2 真 bug（shopt 多选项/跨度全括号计数）——regression 23/23 恢复、Linux comsub2/printf 0 差异；教训：diff 对 NUL 输出报 Binary 被账本计 0，harness 需 --text 或字节级比较）
- audit 车道已发（wt5/audit）：GNU C 源码行级对照审计——6 大区（43 个 .def 内建选项表/redir.c 全形/subst.c 算子角表/parse.y 产生式/jobs-trap 显示/variables 属性），产出 docs/SOURCE-AUDIT.md + 差异逐个探针验证开单
- omb ✅（e22fa3af：#251/#252/#253 关闭，第 59-61 单）；audit ✅（9d81ae68：SOURCE-AUDIT.md 336 行为、11 新单 #261-#272——参数算子 74/74 字节一致、ulimit 是壳、<&7 静默错输属于数据损坏级）；corpus3 ✅（14692f46：FFmpeg configure 全清/OpenSSH exec 清/git harness 2/92 端到端、新单 #260/#262）；cifix ✅（3281de73+HOME 修复：CI 三红全治——本地 23/23 两连绿）
- perf2 ✅ 回收（70ee0dc3：cmdsub/spawn 1.0x 平价、readloop 106→38x、nvm 载入 -44%、-n -35%；#241/#242 留单——剩余=walker ${臂递归（需 SubXpassFrame memo 语义）+ 船长 continuation.rs 两处（has_unclosed_quotes 每个挂起 ${ 拷贝剩余输入 @762）待我应用）
- misc2 ✅（10361b52：#254/#255 修复+双执行顺带修，#236/#238 环境绑定证据关单——**ble.sh 全文件 -n rc=0 达成**；第 62-65 单）→ 三车道波 6 已发
- fdio ✅（b6cc735a：#260/#263/#264/#265/#266 五单关闭——while-read 5/5 迭代+性能地板、move-fd、<> O_CREAT、空目标分流诊断、<&7 数据损坏级；第 66-70 单）
- builtin2/shell2 撞配额墙（06:11 重置）→ 已直接复活（builtin2 dirty=21、shell2 dirty=17，续作协议）；若再次 1308 即等重置窗口
- builtin2 ✅（6d355a0e：#261-#272 八单关闭——ulimit 17 行内核表全重写+软硬不变量、#267 DISCARD、#268 time 格式、#269/#270 read/select、#271/#272 readonly、#262 退出码；死 agent 工作里的 eof 毒化回归被发现并修复；第 71-78 单）
## 残留清仓（2026-09-28）：车道报告中已记录未开单的 7 项已全部补开（#274-#280）+ 增量读取器深水伞 #281；真实开放板=7（在途/排队）+8（新开）=15 单全有主
- shell2 ✅（d1e598e0：#256/#257/#258/#259/#273 五单关闭——**modernish .t 166/166 满分**、bats filter.bats 全平价、#273 大小写零快照漂移；两处冲突=fdio move-form 准入+SCLOSEDFD 校验融合；第 79-83 单）；后续边界开单 #282/#283
| **今日总账：83 单关闭、93 笔提交、74 车道；开放板=#241/#242/#274-#283 共 12 单（性能轮 + 残留批）——闭环进入性能轮**
- 第 7 波已发：perf3（#241/#242/#281 性能轮——walker memo+边界检查点，目标 nvm -n 720x→<60x）、resid1（#274/#275/#276/#278 comsub/exec 族）、resid2（#277/#279/#280/#282/#283 套件/格式族）——**开放板 12 单全部分配完毕，零未分配**
- resid1 ✅（997d4a44：#274/#275/#276/#278 四单关闭——print_comsub 全端口+物理行号映射；第 84-87 单）；残余开单 #284（comsub 内空 case）/#285（近词法错误回显重构行）
- resid2 ✅（475c6dc0：#277/#279/#280/#283 四单关闭——printf 八项修复三平价、ble source 字节平价、#282 电池 PASS 攻至船长文件前）→ **船长接手：#282 的 continuation.rs has_unclosed_quotes 自旋（复现在 issue #282）**；WinuxCmd#1133 已开（hexdump 尾垫）
| 今日：91 单关闭；开放板=#241/#242/#251?/#282/#284/#285（+interactive 车道在途）——**船长下一动作=修自旋**

## 2026-09-28 下午更新（交互/主题战役 + 车道验收）

**主线（用户环境连环修，均已提交推送）**：
- 启动挂死根因：宿主内置 `niubash-gitstatus` 后台线程死锁 → niubash#145 立案（LLDB 实锤+二分证据），本地 rc 已迁 oh-my-bash agnoster，**retire-theme 车道在途**（删宿主 git prompt+内置主题栈，wt9/retiretheme）
- `$-` 无 i：niubash REPL 不设 `__RUBASH_INTERACTIVE` → f93214b 修（enter_interactive）
- PROMPT_COMMAND 数组执行：引擎零实现 → **899ab036**（eval.c:305 全端口：数组逐元素/assoc 拒绝/标量一次 + run_repl/run_interactive_stdin 接线 + 4 单测）；宿主改调引擎 API
- bind 警告：无条件告警 → **b25e60f8**（交互静默/脚本保留，对齐 GNU no_line_editing）
- 数组 += 单引号整词转义：**847ab883** 修 storage 层；**初始复合赋值仍丢 → #288**（agnoster `[e[33m]` 噪声根因），**arrayesc 车道在途**（wt9/arrayesc）
- OMB 加载性能：-c 0.139s vs -C 2.58s，OMB source 1.44s 且组件裁剪无差异 = 引擎侧固定成本 → **ombperf 车道在途**（wt9/ombperf，owner 定性：我们自己慢，不改 OMB）
- CI 红根因=builtin2 的 ulimit Linux libc 类型炸 + 6 处 UTF-8 加宽 → **763e2e84** 修复；niubash CI 挂同一雷，rubash 绿后 rerun
- 新开单：rubash#288（数组转义）、niubash#145（启动挂死+删内置栈）、niubash#146（管道 -i 跳 rc）、niubash#147（真终端 cd 后 EOF+尾挂 `)`，ConPTY 域）

**车道验收（三验收 agent 并行在跑）**：
- interactive2（b2f3d0da）：❌ 退回——新 test target 7/8 红（`!echo` event not found），证据在 #286 评论，不 cherry-pick
- sourcefix（93147937+5953be2b）/ merge144（308ee940）/ resid3（bc6e6da7）：验收中（五门禁+GNU 对照）
- perf4：工作树脏半成品，不验收，待断点续作
- ConPTY 自动化已建：pywinpty 模板 `target/ombtest/pty_probe2.py`（真终端断言：banner/bind 警告/命令/cd EOF/PS1 噪声）

## 2026-09-28 晚终版（交互战役收口 + CI 判定结构清晰化）

**已回收（master 3795ccc4）**：
- readhang（#293 前身 Windows 域）：b6cc735a 回归根因（fd0 绑定缺 FUNCTION_STDIN 镜像→read 回退阻塞读进程 stdin，GNU read.def 对照）+ 根治 7/26 旧伤（initial_pipeline_input 命令名硬编码表 slurp 整删）；33 例矩阵 GNU 字节一致；niubash host_contract 42/42
- arrayesc（#288 已关）：四处双重 quote removal 修复，三臂探针+agnoster 门与 GNU 字节一致；两深水遗留单列（$var 拼接引号泄漏/declare 非mut walker）待开单
- ombperf（性能纪律样板）：assoc O(n²)→内容寻址 memo（assoc.c/hashlib.c 对照）+ indexed memo + GNU 基线锚点文档；OMB 内层 1093→787ms（<GNU-on-NTFS 830ms）；niubash 宿主层 560ms 差距归 retire-theme 后再测；lexer 提交因 sourcefix 重写冲突退回车道新基线重做
- sourcefix（误 revert 后恢复）：分组驱动+park 两提交；nvm 1.7s；nquote 未重录恢复

**CI 结构判定**：rubash CI 红=且仅=bash-upstream-progress 的 TIMEOUT_FAIL>0（普通 FAIL 是 progress 记录不阻塞，STRICT=0）。当前挂死=execscript/jobs/minimal 三件（Linux）→ **#293 linuxhang 车道在途**，修完即全绿。

**在途**：retire-theme（#145，5 提交交付+收尾）、linuxhang（#293）、continuation perf（#292 船长已批准 A/B 计划）、perf5=lexer join 优化重做（sourcefix 新基线）
**新开单**：rubash#292（continuation perf）、rubash#293（Linux 三挂死）、niubash#148（--norc 顺序）、niubash#145/#146/#147 前述
**退回待重交**：merge144（#144 评论：删 legacy 回退+三类输入覆盖）、interactive2（#286 评论：S1 测试红）
**用户环境**：niu.exe @ 引擎 3795ccc4 待 retire-theme 收口后统一构建部署；agnoster 渲染干净（#288 修复已部署）

## 🏁 2026-09-28 深夜里程碑：两仓 CI 首次双绿

- **rubash CI 全绿**（run 36426368984，6/6 job success，master 40fc2faf）：红了几个月的 bash-upstream-progress 由 linuxhang 转绿（#293：execscript ELF-source NUL 剥离移植 + jobs=预算地板）；ubuntu Rust tests 的 30 分钟隐性挂死由 nextest 每测试 90s 硬超时终结（40fc2faf）——挂死未在 nextest 进程隔离下复现（cargo test harness 的句柄交互形态嫌疑），#294 保留观察一个周期，再现即被点名。
- **niubash CI 绿**（run 36420424868）：strict-warnings 三平台全 target 清零（ca93c2a→a8d4c3a，教训：模拟必须 `cargo test --no-run`/`--all-targets`）+ hosted runner ConPTY 跳过（c28c21b，桌面/自托管可强制）+ savannah submodule 网络抖动 rerun。
- 今日累计：rubash #284 #285 #288 #293 关闭（#294 新开观察中）；niubash #145 关闭（内置栈净删 1.24 万行）；7 车道回收（readhang/arrayesc/ombperf/retire-theme/linuxhang/perf5/resid3+sourcefix）；3 车道退回（merge144/interactive2 待重交）。
- 用户环境终态：niu.exe @ 引擎 6ea4b037 + retire-theme（48b17e0 后构建）；agnoster 干净渲染；`niu -C` 2.58s→1.53s（OMB 内层 1093→972ms，GNU 锚 781ms）。
