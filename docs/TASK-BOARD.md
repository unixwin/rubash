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

## 2026-09-28 深夜第二波（ecosweep 生态战役 + 8 车道清空）

- **11 单关闭**：rubash #284 #285 #286 #288 #292 #293 #296 #298 #299 + niubash #144 #145
- **8 车道全部回收**：i2re（历史展开）/m144re（输出流单一真路径，捕获-重放家族 grep 零命中）/perf4（configure-head -74%）/perf6（双走查 264→3ms，a[1]=$i -66%）/fieldsplit（#298/#299）/contperf（#292 park/resume+<<< 根源修，OMB 内层 867ms 进 GNU 区间 812-870）/carrier（#296 载体泄漏，83 套件 A/B 零变动）/ecosweep（OMB 83/83 主题全通+#296-#302 七单产出）
- ecosweep 生态复核：OMB 83 主题/36 插件/9 别名/58 补全 + liquidprompt/bash-sensible/complete-alias/mathiasbynens/direnv hook；bash-preexec 两旧单确认已修；CORPUS-COVERAGE +86 行
- CI 双绿连续 5 轮稳定（nextest 基建后无隐性挂死）
- **在途三车道（wt11）**：inter3（#297/#300 交互初始化）、lexmix（#302/#304 解析域）、almix（#301/#303 存储域）
- 队列：#295（arrayesc 深水）、#294（CI 挂观察）、niubash #146/#147/#148、contperf 同族后续（has_unclosed_quotes/compound 全扫）

## 2026-09-29 凌晨第三波（wt11 收官）

- **17 单关闭**（两日累计）：wt11 三车道全回收——almix（#301 别名反斜杠=DATA_BACKSLASH 载体化 / #303 '${@}' 字面量=!in_span 门）、inter3（#297 rcfile exit 三层根因 / #300 COLUMNS/LINES 双机制移植）、lexmix（#302 function 头分组器截断（纠正 extglob 误诊，真实 OMB svn completion source 干净）/ #304 <<< comsub 收集器残余）
- lexmix 遗留开单：heredoc 词后 # 尾（`echo x <<EOF>#comment`）
- CI 稳定绿（一次 savannah 500 外部故障 rerun 即愈）；用户环境部署引擎 5f59e621
- 剩余：#295（arrayesc 深水：$var 拼接引号泄漏/declare 非mut walker）、heredoc-# 新单、niubash #146/#147/#148 + 版本串 stale -dirty 小单、#294 观察、contperf 同族（has_unclosed_quotes/compound 全扫）

## 2026-09-29 全天战报（wt12-wt15 四波，48 单关闭）

**wt12**（八车道全清）：perf7（nvm -41/-32%+re-land Rc<str>）、perf8（quotes/compound park 化，feeder 重扫归零）、heredocfix（#305）、comsubfix（#289/#290）、deepvar（#295 载体族深水）、niufix（niubash #146/147/148+版本串）、ctrlcfix（#287 driver 自死锁+#294 后台重生 `&;` 腐蚀=CI 挂死真相）、modinit（#282 modernish 双根因）
**wt13**：perf9（configure -n 23s→3.8s）、perf10（source 双 tokenize -23%）、ecosweep2（bash-it 221/221、开 #311-316）、fixpack（#306-309）、sweepfix（#311-315 关+#316 部分）
**wt14**：ecosweep3（pbb 逐 snippet、真实 configure 执行、FFmpeg P0、开 #321-330）、pbbfix（十单全修：P0 后 FFmpeg config.h 784 行=GNU 基准；#321-328/330 关；#329 拍板 A+C→#331+上游 niubash#149）
**wt15**（2-3x 战役一轮）：parsearch（nvm -n 12.8x、parse -39%、分配 -40%）、execdeep（**VarTable att_* 位标志移植**：local3 13.4x、null 9.1x）、exphot（p15 -13%）
**基建**：#310 savannah→GitHub 镜像（CI 结构性免疫）、nextest 每测试超时、CI 双绿稳定

**2-3x 剩余路径（全量化有主）**：dispatch 函数指针表（execute_cmd.c:624 轮）、嵌套体内联解析（~150ms）、船长 comsub park 二次方（configure 合流）、arith_dyn 维护条目模型
**待办**：#316 残留（引号头 glob 回归）、#317-320（fixpack 四单）、#331（自产路径 POSIX）、extglob toggle 残留态（execdeep 报，预存）、#322 continuation 补丁（船长审中）

## 2026-09-30 战报（wt18-wt19 两波，2x 战役主攻）

- **60 单关闭**（全累计）；wt18：perf17（configure park 83x→29x + dispatch 地板 8.8x）、misc17（#332+errors8 ok5-8）、sbxrc（沙箱历史真修 #134 + rc 覆盖矩阵 GNU 对齐）、up149（HOME 家族 fill-if-missing，归属反转=我们自己）
- **wt19**：qleak（release 全套件首份盘点表 + 指针执行：花括号 41x→16.8x）、perf19（configure 1247→420ms ~11x：heredoc 体不透明+case 分级闭包+token 重用；根因=顶层扫描器把体当活文本，525K 字符死组）
- 2x 五桶路线图落盘 PERF-BASELINE：达标 5 / 2-10x 3 / 环境绑定 3（MSYS 父 spawn）/ 解析绑定 5（嵌套体内联架构级）/ floor 6
- 新单：#335 nvm cd-print、( case x in esac ) 预存

## 2026-09-30 全天战报（wt20-wt21 两波，77 单关闭）

- **wt20**：gnusweep3（537 刁钻探针 96% 平价+孤儿 sub 24/26；开 #337-#356 二十单）、parse20（if 家族单遍内联+诚实再归因：configure 剩余=feeder 家族）、envfix3（pathmiss 4.5x/原生父 1.6x；:= 副作用 4→1=GNU；原生父 harness 双口径）、deepsweep（**17/20 一次关**，含 #354 载体泄漏 83 套件 A/B 零变动；#339-#351/#356 全 GNU 验证）
- **wt21**（2x 收官轮）：startup21（pre-main 6-10ms=Windows 主机地板铁证；rubash 侧→1.3ms；01/02 原生父 9.5ms=cmd 地板）、feeder21（电池融合+准入+削减：configure 386ms；join/re-lex 前提证伪）、resid21（#336/#353/#355 关+#335 partial+#352 归类；#353 读时 locale 语义、#355 fd 间接层；回归门禁拦下 cat-file /dev/fd 连带破绽并修）
- **2x 战役位置**：01/02 达 cmd 主机地板（2.4x 对 GNU-Linux）、5 套件 <2x、pathmiss 原生父 1.6x、configure 11x（token 架构级 quote-removal 是下一座山）、null 8.8x
- CI 双绿全程；unix shim+catfile 两起 CI/门禁红均当场修复

## 2026-10-01 战报（wt22-wt23 回收中）

- **wt22**：#361/#362/#363（direnv P0 家族：外部脚本 in-process 泄漏、复合体 comsub+重定向空返回）、quoterm22 游标基底（卫生门禁 char::From 两轮拦截后落地）
- **1.2.1 release 链已合**：rubash #366/#367（版本 bump+lock 同步，匹配 niubash git 依赖声明）；niubash v1.2.1（Winget portable #151）。合并后 release 构建验证通过，磁盘清理 18→2 工作树
- **wt23/bats364 已回收（2ad6c17c，CI 6/6 绿）**：#364 根因=展开后赋值值重扫 `<(` 物化 procsub（GNU subst.c:11358-11381：procsub 只在原始词文本游走时识别）——bats-preprocess CR-strip 循环执行了 bats.bats 内的 `<(normalize_variable_list)` 文本。修复=门条件收在展开前 RHS 字面 `<(`/`>(` 且非全引号。bats_pipe 155/155 TAP 字节同 GNU；bats.bats 70/146（余量为 perf→#375）
- **ecosweep4-B 274 红分诊落盘** docs/ecosweep4-B-reds-triage-2026-10-01.md：真回归 2（**#369** 引号元素 glob 泄漏 8c52cb7f、**#370** exec 复制 fd 外部命令落错流 ba69f0d9，均已 bisect 定住）；语义缺口 #371 `${assoc[*]@A}`、#372 case 模式关键字；过时断言伞 #373（72 红）；executor_tests 跟进 #374
- **#375 开**：bats DEBUG-trap 派发 ~0.4-1ms×2300 次/gather + 每触发 `$(cd;pwd)` 归一化（Windows 路径形态专属），2x 战役候补车道
- wt23/hist274 在途（2x floor 桶三项+f28+#368 关联+电池三机器评估）

## 2026-10-01 hist274 回收（b5b78aab，CI 6/6 绿）

- **ArithDynamicContext 惰性模型落地**：GNU variables.c:1844 INIT_DYNAMIC_VAR 读时物化的忠实移植，替掉每求值 9 项 HashMap 快照（2.15µs/次）；7 构造点机械切换；arithmetic_last_eval_input 改失败路径惰性写（GNU expr_string/lasttp 仅 evalerror 后有意义）；identity 路径 tflag 保值（全管道仍重查——comsub 可翻开关）。05 进程内 95.5→80.5ms（−15.7%，GNU 锚 12.5x→10.7-11.2x）；04 −3.4%。船长清掉一枚死探针定时器（t_tail，agent 门禁后遗留）
- **m1/m2/m3 矩阵 + true-baseline 六切片 base==lane 字节一致**；车道另记 harness 陷阱：**0 字节 rb.out = 运行作废须重跑**（此前 301/777/912 计数是空输出对 GNU 的伪计数）
- **for-arith memo 否决**（差分微探针：循环体真含循环变量，GNU 无 memo 付 0.6µs 我们 3.9µs——归因 per-command floor 家族）；errexit/xtrace 收敛只评估（发现 comsub 入口双编码陷阱角落，须先验 GNU execute_cmd.c:1669 再收敛）；电池三机器两阶段方案落 PERF-BASELINE（动 continuation.rs=船长专属，待拍板）
- **f28 根因落 #368**：预开 fd 3/4 在 `$()` 内不可见（nvm-exec 协议坍塌），非 xtrace 继承（GNU 从不自动导出 SHELLOPTS，shell.c:1971-1987）
- **#376 开（P0，船长抽查发现）**：词内两个 `$((...))` 中隔非字母材料（`:`/空格/`.`/`-`）误切分成一个合并表达式——`echo "$((1+1)):$((2+2))"` 直接报错。1.2.1 谱系预存（base==master 字节同败，非 2ad6c17c/b5b78aab 回归）；上游 arith.tests 无此形状=语料盲区；11 形态证据矩阵在单内。修复车道候选

## 2026-10-01 wt24 回收（d1cdb7b3/8c5fc861/085c6a74，CI 6/6 绿）

- **#376（P0）关**：三个整词 `$((` 快速路径的 strip_prefix+strip_suffix 准入把第一个 `$((` 配到最后一个 `))`。新 `whole_word_arithmetic_substitution_body` 真扫描器（括号深度 2 起算、引号/转义不透明、首个平衡 `))` 必须是词尾）替换三处准入（parameter_core/expand_word/command_prepare）；GNU 规范=parse.y:3877 parse_matched_pair 首个平衡 `))` 即断（:3988）、后续 `$((` 独立词事件（:5521）。11 形态+扩类边界双壳字节一致；14+3 回归测试
- **#369 关**：引号整词元素传输形态从 ARRAY_FIELD_SPLIT_MARKER 改纯引号存储词（store! 形态），W_QUOTED 存活到 glob 门。**bisect 纠正：真引入=306436d7 非 8c52cb7f**（原判定被陈旧二进制污染；#288 无涉）。20 例类矩阵字节一致
- **#370 关**：resolve_dup_source 补 fd_table 活绑定查询（镜像 fd1/2 ambient 播种）+ 管道兜底 drain 走 route_builtin_buffered_output 有序重定向状态。10 例 fd 矩阵字节一致；fd_redirects 两红转绿；cli_tests 19→17 预存失败（零新增）
- **跟进三单**：#377（未引号 `${a[@]}`/declare 元素不 glob——#369 反向预存缺口）、#378（外来子进程拿不到 exec 编号 fd——Windows spawn 平台限制）、#379（comsub 间 `$'\n'` 折叠为空格——预存）
- **winget**：437563 换代 444854（Unixwin.Niubash 1.2.1，zip+portable→inno，静默本地验证过）；旧单已留交接评注

## 2026-10-01 晚间：1.2.2 发布 + winget 换版 + #368 回收 + 管道调查大反转

- **niubash v1.2.2 已发**：rubash 98bc65ba 引擎（#364/#376/#369/#370 五族修复+ArithDynamicContext perf）。lock 手术式 bump（完整重解析会把 windows-sys 降级到 0.59/0.48 破坏特性门——已记入提交信息）；本机安装已升 1.2.2
- **winget 444854 换版 1.2.1→1.2.2**（manifest 删旧加新+标题/描述+换版评注）；#141/#155 已 ping v1.2.2
- **#368 关（3d311caa，CI 绿）**：Some(None) 写入臂走真 stdout 旁路（write_real_stdout_uncaptured）+ comsub 边界清 fd1 别名记录（subst.c:7320 新描述规则）；nvm juggle 双排布字节同 GNU
- **管道数据丢失大反转（#155/#141 已发更正）**：v1.2.2 **未**修复——判别矩阵补全后病灶收敛在 **niubash-runtime `-c` 路径 × WinuxCmd 子进程组合**（rubash.exe 双 profile 直跑全对、MSYS/cmd 管道全对、niu stdin 模式全对、环境变量排除、双构建同坏）。wt26/pipefix 车道深挖中（niubash 侧工作树）
- wt25/semix（#371+#372）在途

## 2026-10-01 深夜：wt25/semix 回收（27ffceb9/50cd08ce，CI 绿）

- **#371 关**：两根因——元素赋值清 DECLARED_UNSET_VARS（arrayfunc.c:815 真 cell）；分词按 chk_atstar（quoted `[*]` 一词、`[@]` 只按 IFS 切裸前缀，值体 CTLESC 存活）
- **#372 关**：解析哨兵无责，真凶=lexer close-char 扫描器无 pattern-region 状态；修复=command-位 `(` 未闭合判定用真解析器复核证伪（#117 走法2，范围有据收窄）；issue308 残留 2 红转绿
- **残留开单**：#380（comsub 执行层同族——lexer/skip.rs case-depth 机需 PST_CASEPAT 移植）、#381（空 case 严格性反向分歧——`in` 后换行未跳过）
- 在途：wt26/pipefix（niu×WinuxCmd 管道丢数据 P0）

## 2026-10-01 终局：wt26/pipefix 破案 + v1.2.3 发布

- **管道丢数据真根因（引擎侧，LLDB 实证）**：`pipeline_exec.rs` 非末位管道成员的 100ms 墙钟硬杀窗**从调用时刻无条件武装**（本为 `yes|head` 滞留者设计），健康生产者被中途 TerminateProcess → 部分数据 + rc=0 + 逐次漂移。**此前"niu -c × WinuxCmd 格子"归因是计时噪声**：n=5M 下全形态（含 rubash.exe 单跑）都丢——两次假归因教训均已入测量纪律记忆（第 8/9 条）
- **修复 2c781657（CI 绿）**：杀窗仅在观测到直接下游退出后武装（+100ms 宽限），下游存活期无限期等待（GNU execute_cmd.c:2620 + jobs.c:3064 wait_for 无上限）；前向收割序保留。5M×5 全额、`yes|head` 金丝雀完好、lastpipe PASS
- **v1.2.3 已发布**（引擎 a6eb8451：管道修复+#368/#371/#372）；安装版实测 5M×5 全额；winget 444854 已换 1.2.3（manifest/标题/评注）；#155/#141 已 ping 终局评注
- **残留开单 #382**：硬杀滞留者退出码 1 vs GNU 141
- 今日总账：rubash 关 7 单（#364/#368/#369/#370/#371/#372/#376），开 8 张跟进单（#375/#377-#382）；niubash 发 1.2.2+1.2.3 两版；winget 437563→444854 换代换版

## 2026-10-01 深夜 II：winget 双层根因终破 + v1.2.4（静态 CRT）

- **437563 重跑校验的 VCRedist 提示=真凶**：niu.exe 动态导入 VCRUNTIME140.dll（MSVC Rust 默认），干净沙箱加载器直接 0xC0000135，代码未跑——#150 的丢树分析是第二层（同退出码两层堆叠）。已在 437563/444854/#150 三处留确认评注
- **v1.2.4 已发布**：release 构建 `-C target-feature=+crt-static`（工作流烧入），objdump 验证仅剩系统 DLL；安装版实测 5M×3 全额+版本 banner。release runner 断连（other side closed）致资产缺失→rerun --failed 补齐；下载侧遇截断必须 size 核对
- **winget 444854 第 4 次换版至 1.2.4**；管线在 PR 内换版后会卡死（新 head status 全空）——**close+reopen 重触发**有效（01-06 立即重跑）；当前 07-10 在跑
- 记忆更新：winget-listing 全量重写（双层根因+管线卡死处置+下载截断）

## 2026-10-01 军令状 + wt27-wt30 四车道齐发

- **Owner 三条硬指令入常设记忆**（rubash-hang-test-and-perf-1x）：①挂起类修复必须配钉死回归测试（短 timeout 保护）；②issue 全清不挂账；③性能全线 ≤2x 硬线、目标 1x
- **#141/#155 双关**（heptaspirit 1.2.4 验收：数据丢失全翻绿、读端字节=GNU、三段死锁 0.31s；231 双方三版本零复现按不可复现关）；#382 退出码残留补了他 的 PIPESTATUS 证据
- **#158 新 P0**（他回归中发现+船长复现）：末段 cat/nl/rev 且 stdout 直连 → `seq 20000 | cat` 确定性挂起=2c781657 耐心等待暴露的旧句柄残留。**wt27/hangfix 热修在途**（护栏=2c781657 全部成果不许翻）
- **wt28/bugs**（niubash）：#157 setup rc 坏文件（最优先）+ #153 裸 `/` 翻译 + #154 参数展开失败退出码 + #156 xargs 选项解析（含归属判定，WinuxCmd 只读）
- **wt29/rubugs**（rubash）：#377 未引号 fan-out glob + #379 `$'..'` 折叠 + #380 comsub casepat + #381 空 case 严格性
- **wt30/perf1x**：null 8.8x / configure 11x / errexit-xtrace 收敛（先验 GNU execute_cmd.c:1669）/ per-command 地板；架构级先设计后动；测量铁律含负载放大

## 2026-10-01 夜终：wt27/wt28/wt30 回收（wt29 在途）

- **#158 关（e4d84c8b，CI 绿）**：末段捕获管道排空提前到正向等待前（GNU subst.c:7428/7437/7441 并发序）+ 顺序路径 stdin 写线程投喂；**钉死回归 5/5（预修 4/5 红）**；2c781657 护栏全保
- **#157 关（niubash b4fc878）**：setup rc 模板 `\}` 逃逸闭括号→对齐 example 的 `${USERPROFILE//\//}`；单测+e2e（真实 setup 产物过 `niu -n`）
- **#153 关（引擎 bd83a0fb + niu dbd3e3c）**：裸 `/` 是操作数字符永不翻译（execute_cmd.c:6126 argv 原样）；16 形态双壳字节一致
- **#154 重大更正**：车道 GNU 矩阵是 source 模式探针伪影——直测真值 `-c` 模式子壳/函数/循环全是 **127**，原实现本就正确；推荐修复已试装→实测变差→回退。真分歧仅剩 comsub 角落（GNU=0 且吞诊断）——已重定域开在单内
- **perf1x 回收（7002db37+74a06435，CI 绿 a92540a4）**：comsub 入口 errexit 对齐（修 4 活分歧：`$(f)` 内函数体 errexit 复活等）+ **删除活标记单一源化**（修 `set -e; set +o errexit; false` 被杀、xtrace 残留两族；读点 2 查→1 查）。诚实结论：~5% 估计低于噪声地板，Cell 缓存**不必再做**（已写 PERF-BASELINE 防浪费）；下一杠杆=`__RUBASH_CURRENT_LINE` Cell 移植
- **#156**：归 WinuxCmd，上游单 WinuxCmd#1139 已建（bug+compat-gap）；niubash 侧钉死
- 侧发现待开单：posix 翻转 inherit_errexit 粘滞（GNU `set +o posix` 不复位）

## 2026-10-02 wt29/rubugs 回收（a8eb9e93/c52a7211/dc054fca/600cf42c + 船长 0641d4e7，CI 绿）

- **#377 关**：marker 传输不变量三站点收口（未引号 fan-out 逐字段 glob；declare 引号产物保 CTLESC）
- **#379 关**：ANSI-C 解码空白=词数据（ANSI_C_IFS_GUARD U+E401，markers.rs 注册表内新成员——String 词域非 Vec<u8> 载体域，已标注 owner 复核）；33 套件台账不动
- **#381 关**：空 case 跳过 in 后换行+游离定界符报错（双向矩阵）
- **#380 部分**：车道 executor/parser 侧 + 船长 continuation.rs/skip.rs/quotes.rs 全机器 PST_CASEPAT 穿线（含 close_char 残差态新字段、corrected lookahead 喂 ';'）；**裸关键词 comsub 形态转绿**；括号列表形态残留已仪器化定界（pattern-paren 的 at_push 记账在 esac-) 处弹错栈——单内有完整下一步建议）
- **#154 教训存档**：车道 GNU 矩阵系 source 模式探针伪影；船长直测翻案（-c 子壳/函数真值 127=原实现对）
- 期间调试仪器全部清除、诊断字符串逐字节核对还原

## 2026-10-02 夜：pattern_paren 落地 + #383 开单 + wt31-wt34 四车道齐发

- **4928cfa6**：模式列表 `(` 独立 delimiter 种类（弹栈绕过 case_depth 守卫）；调查链更新到 #380（失败扫描中 pattern-paren 未被推栈、全量 oracle 调用返回 None=拒绝来自聚合器另一谓词——下一刀起点）
- **#383 开**：posix 往返 inherit_errexit 粘滞（wt30 侧发现）
- **wt31/casepat**：#380 残留攻坚（聚合器逐谓词打点定位真凶；continuation.rs 修复以 diff 交船长）
- **wt32/optfix**：#383 + #382（退出码 141）+ niubash#154 comsub 角落
- **wt33/testmaint**：#373+#374 过时断言三分法清理（禁为绿灯改产品）
- **wt34/perf2**：CURRENT_LINE Cell 化 + null 8.8x 地板 + parse/feeder 桶（军令状 ≤2x 冲 1x）
- 监控：winget 444854 全绿等版主；1.2.5 攒包中（等本波回收）
