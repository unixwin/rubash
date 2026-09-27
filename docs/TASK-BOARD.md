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
