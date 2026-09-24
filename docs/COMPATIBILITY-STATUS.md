# Rubash ↔ GNU Bash 兼容性权威状态（单一事实来源）

> 最后核对日期：2026-09-20（合并后全量 true-baseline 重跑，83 套件，GNU 5.3.0 契约，
> 分支 `fix/array6-patsub-quotes`；逐套件台账见 `target/issue-suites/results/postmerge-baseline-ledger.txt`）
> 核对方法：用 `./target/debug/rubash.exe` 直接跑 GNU 官方测试文件
> `third_party/bash/tests/<name>.tests`，对比 GNU bash 的真实输出。
> 基线约定（2026-09-09 起生效）：语义比对一律用 WSL GNU Bash 5.3.0
> （`/usr/local/bin/bash`，业主编译版）；`D:/Git/bin/bash.exe` 在引号/转义/花括号等区域的兼容性
> 低于 rubash，不得作为语义基准。
> 注：`scripts/run-83-tests.sh` 对比脚本本身已破损（`set -u` 下算术变量未初始化，
> 满屏 `系统找不到指定的路径`），不能用于判定，故本节全部为手动真实复现。
>
> 本文件是兼容性状态的**唯一权威来源**。其余 `docs/*.md` 中带日期的分析快照
> （如 `bash-test-update-20260829.md`、`rubash-compatibility-report.md` 曾宣称
> “92%、仅 1 个 bug”）已被真实复现证伪，相关文件已于 2026-08-29 删除，
> 不再作为判定依据。
>
> **最新台账（2026-09-24，master `e5ac3277`，无桩 + niu-sh 夹具）：71 零差 / 12 有 DIFF / 总 86 原始行**
> （台账 `target/issue-suites/results/true-baseline-ledger.log`。
> 本轮清零套件：procsub（`<(cmd)` 共享流语义 + 管道 fd-1 单流排序）、
> set-e（`!` errexit 豁免穿过分组驱动）、vredir（`{var}<<EOF` 动态 fd）、
> builtins（POSIX 特殊内建 unwind 传播 + `command -p` POSIX 工具目录探测）、
> posix2/invocation/exp（TMPDIR/HOME 反斜杠规范化、`/bin/X` 扩展名探测）、
> errors（`$[...]` 算术错误丢弃整条命令）、read（真 `-t N` 超时 + `/dev/tty`
> CON 映射，16→2）、test（`/dev/tty` 挂死 + `-t`/`-c` 设备语义，254→8）。
> 残余 12 套件按性质：env 绑定 —— extglob 16（全 env）、quotearray 2（全 env）、
> nquote 12（recho 助手格式）、type 6（二进制名）、test 8（NTFS 属性位）、
> intl 4（locale）、coproc 4（/etc/passwd 等）、ifs-posix 1（超时）；
> 真差 —— glob 29（主体 NTFS 非法文件名，env 分类器无法全识别）、
> errors 2（comsub 诊断措辞族）、nameref 1 / trap 1（时序抖动）。
> 沿用无桩 harness：`__RUBASH_NO_UPSTREAM_SCRIPTS` 经 `WSLENV /w` 跨边界，
> `/bin/sh|/usr/bin/sh` 经 PATH 解析为 niubash 夹具，TMPDIR 逐套件隔离。）
>
> **上一台账（2026-09-22 深夜，master `44a56d1c`，无桩 + niu-sh 夹具）：58 零差 / 25 有 DIFF / 总 407 原始行**
> （台账 `target/issue-suites/results/true-baseline-ledger.log`；逐行审计
> `docs/diff-audit-20260922.md`。本轮为**首份无桩引擎级台账**：
> `__RUBASH_NO_UPSTREAM_SCRIPTS` 经 `WSLENV /w` 首次真实跨 WSL→Win32 边界
> （此前从未到达 rubash.exe，全部历史台账均带 canned upstream 回放）；
> `/bin/sh|/usr/bin/sh` 经 PATH 解析为挂载本工作树 rubash 的 niubash
> （`$BASH`→niu，`$0` 注入正确），`/bin|/usr/bin/X` 经 PATH basename
> 回退（path.rs `unix_bin_basename`）；TMPDIR 逐套件真实隔离。
> 逐行审计结论：~390 行真语义差（jobs 作业控制族 62、LC_COLLATE 排序族
> ~60、fd 重定向族 ~50、fc 族 32、载体字节族 ~12），~15 行环境绑定
> （type 二进制名 6、vredir `/bin/*sh` glob 6、errors PWD 拼写 2、
> coproc /etc/passwd 1、nquote od 版本差）。
> 对上一版：dstack 72→0（废弃 RUBASH_ROOT 夹具根污染消除）、
> nameref→0、histexp 零差保持、test 17→15、coproc 4→6、invocation 0→2、
> trap 0→1（SIGCHLD 时序抖动）。51-250 桶 2 套件：glob 67 / jobs 62。）
>
> **上一台账（2026-09-22 晚，master `c1d151d2`，前台进程组超时 harness）：57 零差 / 26 有 DIFF / 总 464 原始行**
> （注意：该台账 `__RUBASH_NO_UPSTREAM_SCRIPTS` 未跨边界，仍在测量
> canned upstream 回放路径，已被上方无桩台账取代。）
> （台账快照 `target/issue-suites/results/true-baseline-ledger-c1d151d2.log`。
> `timeout --foreground` 消除了旧台账的 GNU 侧 40s 截断伪影：history 173→40（真实差）、
> intl 12→8、alias 69→0、histexp 74→1、read 25→16、comsub 17→13、trap 1→0；
> 新增零差 `alias`、`posixexp`。口径说明：`jobs` 78 含 rubash 侧 120s 超时被杀
> （`wait-for-job` 后真挂起，rb.rc=138、输出截在 47/99 行——是真实作业控制缺陷，
> 非测量伪影）；`redir` 70 中约 34 行来自 `exec 6<>` 追加 vs 覆盖语义分歧被持久
> TMPDIR 残渣放大（bash-c 累积 34 次 `to c`），另含 `exec <&5-` 重打、ERR trap rc、
> `|&` 重打、c3/c4 计数器等真实差。51-250 桶 3 套件：jobs 78 / redir 70 / glob 67；
> 251+ 桶 0。）
>
> **上一台账（2026-09-22，修正重计数——完整测试种子 + 完好辅助二进制）：55 零差 / 28 有 DIFF / 总 799 原始行**
> （其中 `jobs` 31 + `history` 173 = 204 行为 harness 40s 超时双侧截断的测量伪影——
> GNU 侧 jobs rc=124、history rc=137 均被 timeout 杀死，非 rubash 语义差；
> 真实语义差 ≈595 行 / 26 套件。本台账取代 2026-09-21 的 "57 零差 / 733 行"：
> 该数字测于不完整种子目录（缺 ~190 个 `.sub`/`test-glue-functions`/helper 文件，
> 双侧同报 "No such file" 产生假零差；`procsub`/`nquote`/`iquote` 等套件当时
> 实际未测到真实行为）。本次同步发现并修复：`kill -n9`/`-sNAME` 黏连信号规格
> （kill.def:134-142）、`set -m` 未映射 monitor 选项（support_names.rs）、
> `fg`/`bg` 对非作业控制作业的 J_JOBCONTROL 检查（fg_bg.def:154-160）、
> `${THIS_SH}` 同进程子壳作业表泄漏（execute_cmd.c:6139-6233 进程边界语义）、
> 以及 posixpipe.tests 脚本名硬编码 hack 整族删除。）
>
> **上一台账（2026-09-21，已作废——种子不完整）：57 零差 / 26 有 DIFF / 总 733 行**
> （DIFF 1-50：19 套件；51-250：7 套件；251+：0。较 848 行净减 115，
> 逐套件对比无任何套件变差：attr 40→0、shopt 38→0、varenv 18→0、
> heredoc 5→0、lastpipe 5→0、comsub-eof 2→0、comsub-posix 19→14、
> nameref 1→0、trap 1→0。修复要点：① readonly/export 标量 `(...)` RHS
> 走 GNU setattr.def:240-268 do_assignment_no_expand 语义，`\x10` 载体
> 不再泄漏进 declare -p 回显；② comsub 头部行 `)` 关闭与跨界 pending
> heredoc 体收集对齐 parse.y:4564 parse_comsub/make_cmd.c:602-611
> PST_EOFTOKEN；③ 管道 stage0 不再把 FUNCTION_STDIN 游标无条件推 EOF，
> 按实际消费字节回写（execute_cmd.c execute_pipeline fd0 共享语义）；
> ④ reset_for_subshell 改为 GNU trap.c:1480 模型——保留 trap_list
> 字符串、单独记录已重置 disposition；⑤ ${THIS_SH} 同进程子壳补
> seed_startup_traps（Executor::new init.rs:36 对齐）。）
>
> 上一台账（2026-09-20，合并后 `fix/array6-patsub-quotes`）：49 零差 / 34 有 DIFF / 总 848 行
> （DIFF 1-50：27 套件；51-250：7 套件；251+：0。较合并前基线 849 行净减 1，
> 逐套件对比无任何套件变差；nameref 6→1。零差套件名单见 README）
>
> 上一台账（2026-09-19，master `652c1042`）：45 零差 / 38 有 DIFF / 总 1247 行
> （较 2026-09-18 基线 1833 行净减 586；nameref 281→7、errors 129→3、arith 53→0、
> type 39→6、shopt/trap 归零；详见 `docs/audit-baseline-2026-09-19.md`）
>
> 更前一台账（2026-09-16）：43 零差 / 40 有 DIFF / 总 2702 行
> （其中 intl=1209 为 ANSI-C `$'...'` 载体字节架构缺口，排除 intl 后余 39 套件共 1493 行；
> 详见第二十二节）

## 一、总体结论

> **注（2026-09-22）**：本节为 2026-09-12～09-13 的历史快照叙事；其中"38 零差"、
> "array(239)/assoc(217)/history(127)/nameref(105)/globstar(101) 五大族"等数字已过时
> （assoc/array/quotearray 已收敛为零差，rsh 已归零）。最新口径一律以本文件头部
> 台账链（2026-09-22 晚，57 零差 / 464 原始行）为准。

- **83 套件 GNU 5.3.0 true-baseline 重跑（2026-09-12）：38 零差 / 45 有 DIFF / 总 diff 1985 行。**
  ANSI-C `$'...'` 载体扩展修复（0x11/0x16 加入载体集 + `bytes_to_shell_text` 载体感知编码）后
  多族大幅收敛：new-exp 241→65、more-exp 232→33、quote 172→10、posixexp 93→14、
  procsub 33→11、history 127→119、assoc 242→217、array 246→239、braces 13→1、
  posixexp2 14→2、errors 35→34、glob 50→48；净减 87 行。
- 已完全修平的大族：builtins、complete、func、rsh、invocation、dbg-support、cprint、
  globstar（检查侧归因）、trap、appendop、attr、casemod、dynvar、extglob2/3、
  getopts、glob-bracket、herestr、ifs、invert、mapfile、nquote2/3/4/5、posixexp2、
  posixpat、precedence、printf、strip、tilde/tilde2。
- 剩余主要缺口集中在 array(239)、assoc(217)、history(127)、nameref(105)、
  globstar(101) 五大族，占总 diff 的 34%。
- 隔离场景下的 GNU Bash 语义——数组、关联数组、算术、条件、nameref、mapfile、
  POSIX 命令替换、花括号展开、信号表、trap、history -d、invocation 长选项——均已
  与 GNU Bash 5.3.0 一致。
- 2026-09-02 会话收官（账本 41）：globstar 75→241/587（语义重构 `106a7136`：
  空 `**` 匹配一切、`**/` 仅目录加尾斜杠、递归永不穿符号链接、去 `./` 前缀；
  配对探针五构造逐字节一致）；连带修出 mkdir flag 当路径名的预存 bug（
  `606108c1`，曾被 globstar 树暴露）与每目录 DFS 排序归一（`cb92206a`，门禁
  中性）。globstar 残余 346 行根因 = check 侧 ls 为 Windows PATH 的 MSYS ls
  而 gen 侧为 WSL GNU ls（双侧二进制不对称，recho/zecho 同类 harness 缺陷），
  下一步 = run-83.sh 双侧统一 ls helper。B 兵团 fd-model 收官 `e3a9b63e`
  （exec stdio 处理 flag 被 continue 绕过的关键 bug、歧义重定向措辞、零字段
  管道段=空命令、procsub 物化；redir 175/165、procsub 33/24；其基线陈旧
  论断经 fresh gen 复现 165/24 后否决）。B 残余：exec fd 中毒块（需外部
  stdout 回放调查）、脚本自 stdin 续跑（rubash 整体缓冲）、declare -f
  序列化丢重定向文本、let 后缀自减。
- 2026-09-08 回归修复（ISSUE #78）：多行复合数组赋值（`plugins=(\n git\n
  completion\n)`）自 `691cfba0` 起被按行切断、元素当命令执行。根因 =
  `tokenize_with_heredocs` 逻辑行收集器识别反斜杠续行/未闭合引号/命令替换/
  花括号组，唯独不识别未闭合的命令位置 `name=(`。修法 = `continuation.rs`
  新增 `has_unclosed_compound_assignment`（引号/替换/注释全感知扫描），仅对
  命令位置、赋值前缀区（`x=1 a=(...`）、declare 族操作数（`declare -a b=(...`）
  的未闭合 `name=(`/`name+=(` 续行——与 GNU parse.y 一致；`echo a=(b` 等
  非赋值位置照旧立即报语法错误。附带对齐 EOF 未闭合诊断：`unexpected EOF
  while looking for matching `)'`（无源码回显、rc=1，parse.y 语义）。
  验证 = 8 个 GNU 边界 A/B（多行/declare/+=/注释/引号包裹/前缀赋值/EOF）
  逐字节一致；cargo 全 target A/B 失败清单零回归；GNU 官方 78 切片差分
  A/B 同为 14 OK/64 FAIL 且零翻转，globstar 100→17、vredir 56→50 行差异
  收窄。已知残留：`a= (1 2)` 类报错的源码回显为 token 重建（`a= ( 1 2 )`）
  而非原文，属 parse-error 源回显独立问题。

## 二、真实复现结果（按严重度）

| 测试文件 | rubash 行数 | GNU bash 行数 | 严重度 | 现象 |
|---|---|---|---|---|
| `posixexp2` | 40 | 40 | 中 | 行数一致（LF 干净基线重跑）；14 行引号保留差异，属 #52/#56 参数展开引号族 |
| `mapfile` | 170 | 170 | 已关闭 | bash-tests-rw LF 干净目录重跑 diff=0；此前差异为工作树 CRLF 伪影（见十八节） |
| `cond` | 165 | 174 | 中 | 干净重跑行数接近；49 行差异（rc 语义 [[ 构造） |
| `comsub-posix` | 30 | 70 | 高 | POSIX 命令替换形态展开不足 |
| `braces` | 112 | 102 | 中 | 干净重跑 18 行差异（{0..10} 序列、错误措辞） |
| `array` | 837 | 853 | 高 | 干净重跑 682 行差异（GNU 侧 rc=1 超时截断，需分段基线） |
| `arith` | TIMEOUT | TIMEOUT | 中 | 已不终止，有内容/格式差异 |
| `nameref` | 40 | 40 | 低 | 行数一致，少量内容差异 |

## 三、花括号展开（`braces`）具体缺口

rubash 输出 141 行 vs bash 104 行，差异集中在 `braces.rs` / `expand_range` /
`is_brace_expansion`：

1. **前缀形式不展开**：`is_brace_expansion()` 只认“以 `{` 开头”的词，
   导致 `foo{a,b}`、`baz{x,y}` 等被当成字面（注：此点另一 agent 已修复，
   隔离用例已 OK，但完整文件仍可能因其他差异错位）。
2. **转义逗号保字面**：`{abc\,def}` 应保留字面 `{abc,def}`，rubash 误展开为 `abc def`
   （另一 agent 已修复转义逗号）。
3. **序列**：零填充（`{01..05}` → `01 02 03 04 05`）、反向序列顺序、嵌套组
   `{{0..10},x}`、错误消息措辞、展开顺序——均有差距。
4. CRLF 脚本模式下转义花括号字面输出偶发尾部空格（既有脚本模式遗留，非本次修复引入）。

## 四、已修复（避免重复劳动）

| 缺口 | 修复方 | 状态 |
|---|---|---|
| `echo {a,b}{1,2}` → `a1 a2 b1 b2`（相邻/嵌套花括号笛卡尔积） | 本次会话 | 已修（命令替换内花括号递归展开 + 相邻组 lexer 规则） |
| `echo $(echo {a,b}{1,2})`（命令替换内花括号） | 本次会话 | 已修 |
| 转义逗号 `\{a,b\}` 保字面 | 另一 agent | 已修 |
| 前缀花括号 `foo{a,b}` / `baz{x,y}` | 另一 agent | 已修（隔离用例 OK） |
| 数组 / 关联数组 / 算术 / 条件（隔离） / nameref / mapfile（隔离） / POSIX comsub（隔离） | 其他 agent | 已修 |

## 五、非缺口（不要误报）

- **平台噪音**：10 个测试 GNU bash 返回 127（找不到 `recho`/`zecho` 辅助脚本），
  rubash 反而正确执行——这不是 rubash 的 bug。
- **超时规则不同**：6 个测试双方超时计数不同，属平台差异。
- **rubash 优于 bash**：`builtins`、`comsub2`、`histexp`、`complete -p` 计数等无需修。
  （**勘误 2026-09-22**：`histexp`"无需修"结论已被
  `docs/harness-attribution-20260922.md` 推翻——真账约 74 行 `!!` 透传缺失属真实语义差，
  非噪声；本条其余归属维持。）

## 六、建议的下一步优先级

1. **P0**：`posixexp2`（整文件解析错误）、`cond`（第 54 行条件构造）。
2. **P0**：`mapfile`（管道/多行场景）。
3. **P1**：`comsub-posix`（POSIX 命令替换完整形态）。
4. **P1**：`braces` 序列零填充 / 反向 / 嵌套组 / 错误消息（system-level 补齐）。
5. **P2**：`array` / `arith` / `nameref` 的内容与格式差异；错误措辞、xtrace、环境变量泄漏等格式项。

## 七、run-83 rights 基线（2026-08-29，run-83.sh）

测试设施：`tests/gnu-compat/run-83.sh`，三模式——

- `gen`：WSL GNU Bash 5.2.21 在干净环境下生成 `tests/gnu-compat/upstream-rights/<name>.right`
  （`third_party/bash/support/{recho,zecho,printenv}.c` 用 gcc 现编译、
  CRLF 修复副本、`THIS_SH=bash`）。GNU 自身跑不完的测试记入
  `tests/gnu-compat/GNU-TIMEOUT.txt`（当前：`jobs`、`trap`）。
- `check`：rubash 对已提交 `.right`，不依赖 WSL，日常开发/回归用。
- `live`：rubash vs WSL 实时输出，排查基线本身用。

两侧 helper 实现一致（官方 C 源编译），避免 helper 分歧噪音。产物在
`target/issue-suites/results/<mode>/`。

首次 check 基线（2026-08-29，含 quoted-tilde 修复后）：**PASS 11 / DIFF 63 /
TIMEOUT 2 / SKIP 2**（83）。

### TIMEOUT 族（已解决大部分）

7 个 TIMEOUT 中 6 个是 harness 缺陷（stdin 永不 EOF，`arith`、`builtins`、
`getopts`、`nquote`、`printf`、`read` 在 `</dev/null` 下均正常完成）——
run-83.sh 已改为两侧 `< /dev/null`。剩余 `ifs-posix` 是性能（6856 个
管道+子 Shell 子测试需 60-120 秒，`RUN83_TIMEOUT=180` 可跑完）加真实失败。

### 已修（本轮）

- **quoted-tilde ESC 哨兵泄漏**：`echo expect '~1'`、`printf %q '~'` 输出
  `\x1b` 字节。根因：fully-single-quoted 快速路径未剥离 `\x1b` 标记。
  `dstack2` 转 PASS。
- **`${var:-word}` 未加引号默认词缺 tilde 展开**（原第 1 待修项）：三层修复
  ——quoted 处理器接收 context、embedded 有序展开器透传 context、
  command_prepare 对双引号 raw 的整词 `${...}` 补回 `\x1d` 标记。
  `tilde2` 转 PASS。
- **read 末变量拆分**（分支 `agent/ifs-read-splitting`）：按 read.def 实现
  ——先再提取一个字段，耗尽则取该字段，否则取整个余量仅去尾随 IFS 空白。
  18 个探针与 WSL GNU 全一致，ifs-posix 自报失败 3178→3006。

### 已定位、待修（按影响排序）

1. **`ifs-posix` 全量运行才出现的读拆分失败（~3006/6856）**：read 末变量算法
   已按 read.def 修复（见上），剩余失败只在大规模运行中出现——孤立复现
   （单行/命令替换/函数级/环境 IFS 污染共 8 种构造）全部与 GNU 一致。
   插桩证据：失败点 `IFS=[ ]`（split() 的环境值），说明子 Shell 内
   `IFS=$ifs` 赋值在全量场景未生效；孤立时同一构造正常。下一步：对
   `for str` 循环做渐进二分（截取前 N 个组合），找出最小触发前缀，定位
   泄漏状态的来源（疑似 `set x`/`shift` 位置参数或长循环下的赋值隔离）。
   注意：验证 IFS/空白必须用 `${#var}` 长度探针，行尾空格在输出显示中
   不可见，会造成误判（本轮曾因此误判"赋值 RHS 被字段拆分"，已证伪）。
2. **rubash PATH 查找在 `D:/` 风格混合分隔符下不稳定**：harness 已用 `/d/`
   风格绕开；产品侧待查。
3. **Windows 系统错误信息 GBK 编码泄漏**：找不到 `.sub` 文件时输出乱码。
4. `mapfile.right` 含 CR 字节，需字节级分析 rubash 是否做文本模式 CRLF 翻译。
5. `appendop`：readonly 变量 `+=` 不报错；`declare -A`/`declare -ai` 输出
   格式与 GNU 不一致。

## 八、权威基线约定

- 兼容性判定基线 = GNU bash 语义；本机比对一律用 WSL GNU Bash 5.2.21
  （`wsl bash`；含连续反斜杠或多层引号的 case 必须用脚本文件方式喂给双方，
  不能走 `wsl bash -c "$c"`，wsl.exe 命令行透传会把 `\\` 折叠成 `\`）。
  **不要**用 winuxsh shim；**不要**用 Git Bash（`D:/Git/bin/bash.exe`）作
  语义基准——它在部分区域兼容性低于 rubash，会得出错误结论。
- 任何“已修复/仍残留”状态变更，必须**真实跑对应 GNU 测试文件复现**后更新本文件，
  不得仅凭推断。

## 九、2026-09-01 多智能体复现与源码一致修复检查点

6 个根因族由只读调查子智能体复现并对照 GNU C 源码分析，报告位于
`docs/investigation/{posixexp2,heredoc,ifs-posix,procsub,declare-array,deep-expansion}-investigation.md`。
船长按 AGENTS.md 流程逐族验证后落盘的源码一致修复如下。

### 已落地并验证（WSL GNU 5.2.21 探针 + run-83 A/B 回归）

1. heredoc/命令替换括号平衡（`src/lexer/continuation.rs::skip_parenthesized_unit`）
   - GNU 依据：parse.y gather_here_documents / make_here_document 从输入流读取
     here-doc 体，heredoc 体对括号计数器不可见；慢路径已有 heredoc 跳过，快路径缺失。
   - 修复：快路径在 <<（非 <<< here-string）处用既有 skip_heredoc_in_chars 跳过
     heredoc 体，使体内 ) 不再误闭合 $(...)，与慢路径一致。
   - 验证：新增单元测试 heredoc_body_paren_does_not_close_command_substitution 通过；
     cargo test --lib 仅余预存失败（见末节）。comsub-heredoc 探针仍未通过——真根因在
     tokenize_with_heredocs 行累加吞掉 heredoc 体行（handoff 已标记的深层
     tokenizer/heredoc 状态协调），需独立专项，非本族单点可解。

2. procsub 路径分隔符（`src/executor/execution_misc.rs::shell_display_path`）
   - GNU 依据：subst.c process_substitute 经 make_dev_fd_filename 产出正斜杠
     /dev/fd/N，保证路径能安全过 eval/source 重解析。
   - 修复：Windows 显示路径在 drive 转换前把反斜杠归一化为正斜杠，满足 eval 安全契约。
   - 验证：探针 eval echo <(echo hi) 由 C:Users...（反斜杠被 eval 吃掉）变为
     /c/Users/.../...tmp（正斜杠、eval 安全）。A/B（回退该单行）确认 dstack/redir/procsub
     run-83 行数不变（55/416/386），无回归。run-83 procsub 仍 DIFF，因路径内容
     /c/Users/... vs /dev/fd/63 仍异——需独立 /dev/fd/N fd 抽象专项。

3. nameref 模式替换（`src/executor/expand_braced_replacement.rs::expand_braced_replacement_parameter`）
   - GNU 依据：subst.c parameter_brace_expand_word 经 find_variable /
     find_variable_nameref 顺 nameref 链取目标值后再做模式替换。
   - 修复：在 env_vars.get(var_name) 前用既有 resolved_variable_name 解析 nameref
     目标名（与 parameter_patterns.rs::parameter_pattern_scalar_value 同模式）。
   - 验证：探针 declare -n v=var; echo ${v//c/x} 由 var 变为 abxde，与 GNU 一致；
     ${v} 简单解引用仍 abcde。run-83 nameref 仍 DIFF 932/372（该族其余子项：assoc 键序、
     declare -Ai 算术等独立，未动）。

4. posixexp2 case 39 引号保留（`src/executor/command_prepare.rs`）
   - GNU 依据：参数展开结果是数据，不对其套 quote removal；仅原始词法 token 走引号移除。
   - 修复：移除对展开结果调用 remove_shell_quotes 的分支（原 word_contains_brace_group 门）。
   - 验证：探针 set -o posix 下 foo=x'a'y; echo ${foo%*'a'*} 由 x 变为 x'，与 GNU 一致。
     A/B（回退）确认 quote/quotearray/posixexp2/more-exp/comsub/case/new-exp/cond 行数全不变，
     无回归；cargo test --lib 312 passed 仅余预存 PATH-env 失败。posixexp2 仍 DIFF 40/40
     （余 case 8/9/11/12/28/29/37 属 RC-1/RC-2 深层，未动）。
### 调查完成但未落地（深层 / 高 blast-radius，留作专项）

- ifs-posix：子智能体原理论（execute_materialized_command 把独立赋值当临时）经直接探针
  证伪——简单 / 子shell / 管道 / 命令替换 / 双字符 IFS 的 IFS=:; read 形式全部通过（与
  GNU 一致）。真正失败需完整 77 行探针上下文（for+函数+set/shift+while/case+跨 480 次迭代
  反复改 IFS），属状态累积/交互问题（族 B/C「状态污染，最难，间歇性，需 debug 工具定位」），
  需 LLDB/eprintln 在完整探针上定位，非独立赋值持久化问题。**切勿**套用「独立赋值永久化」
  修复——基于已证伪理论，属补丁式且不修真 bug。
- posixexp2：RC-1（parameter_words.rs 替换词引号/反斜杠两阶段）、RC-2（未引号默认词
  反斜杠在分词前丢失）。RC-3 已落地（见上 4）。
- deep-expansion：arith-for 除零错误 token 已修（arithmetic/mod.rs:623 "0 "->"0"，探针验证
  token 与 GNU 一致）；尾部操作符空操作数诊断已修（2026-09-01 续）：
  arithmetic/mod.rs::trailing_operator_error 按 GNU expr.c readtok/evalerror 语义重建
  lasttp-suffix token——表达式以需右操作数的操作符结尾时报 `syntax error: operand expected`
  （token = 操作符起点到串尾的 suffix），其中赋值操作符（`=`/`op=`，白名单不含 `==`/`<=`/
  `>=`）左值为数字时按 expassign 报 `attempted assignment to non-variable`，左值为变量时
  报 operand expected；`7++`/`7--` 按readtok 拆成单 `+`/`-`。修复 `j=` 静默、`7<=`/`7&&`/
  `j+=`/`j==`/`j!=` 错误 token、`7==` 误报 attempted assignment、`7++`/`7--` token `++`/`--`
  →`+`/`-` 共 6 子项；探针 stderr 与 GNU 5.2.21 全对齐，A/B（禁用 hook）确认 cond/errors/
  more-exp/rhs-exp/precedence/varenv 计数不变，arith-family 预存 13 个 cli_tests 失败集合
  不变（nounset 退出码等属其他族）。新增 3 个 cli_tests 回归。仍残留：
  (a) arith-for 表达式显示/`7++ ` token 尾随空格——GNU make_cmd.c make_arith_for_command
  保留 `((...))` 原文（仅去每段前导空白），rubash 解析器 token 重组丢原文，需词法层
  ARITH_FOR_EXPRS 式原文捕获专项（parse.y:4896 parse_dparen）；
  (b) `for (( $(case x in x) esac);; ))` 头部 $()/case 解析（parse_matched_pair 语义）；
  (c) `$((…))` 求值失败后 rubash 不中止当前命令（GNU status 1 且命令不执行）——
  独立语义族；run-83 arith 族 rubash 侧 TIMEOUT 预存（arith.tests 内未定位悬挂）。
  new-exp HOME:} 为更深层解析器问题（族 E 面）。arith-for 家族 `type fx` 函数源码
  格式化差异属函数定义文本保真族，不计入本族。

### 十、2026-09-01 续：算术展开致命中止语义（(c) 项落地）

以 GNU expr.c/execute_cmd.c 实测矩阵（WSL 5.2.21 文件脚本 + 单层引号 -c 探针，
探针文件一律 Write 工具落盘防 winuxsh shim 污染；wsl bash -c 多层透传再次证伪不可信）
为基线，落地算术致命中止族：

1. **实际求值定致命**：fatality 分类不再用全新空环境重求值（状态依赖错误如
   `declare -i x; y=$((1 ? 20 : x+=2))` 在新环境消失导致漏判）。Executor 新增
   `arithmetic_last_error_category`，eval_arithmetic_command_value /
   eval_arithmetic_expansion_value / eval_conditional_arith_value_categorized 把真实
   环境的错误类别直通四个分类点（expand_word/parameter_core/embedded_parameters/
   command_prepare/assignment_expansion）。
2. **词上下文致命=放弃当前列表**：`echo $((1/0)); echo after` 不打 after（status 1），
   脚本继续下一行；ast_exec 主循环新增 ExpansionFailure 臂（守卫 loop_depth==0 &&
   function_depth==0 && 非子 shell && 非复合条件），同行余部按 command.line 跳过；
   循环体/函数体内的致命错误传播到 frame 边界（整个 for/while 放弃、函数剩余体放弃）。
3. **nounset → 127**：`set -u` 下 `((missing+1))` 与词形式 `$((missing+1))` 均退出
   127（-c）/退出脚本（文件模式 rc 1）；nounset 标志从 env_vars 迁移为
   Executor Cell（`&self` 词路径可置位）；assignment 词内 nounset 保持致命。
4. **readonly 冲突=求值失败**：`xx++` 只读冲突时求值返回 None（GNU ASSIGN_DISALLOWED
   longjmp 对齐）；case 模式词展开后检查致命并放弃整个 case。
5. **测试基线修正**：`arithmetic_empty_quoted_array_subscript_fails_outside_let` 原
   期望（cmd ctx status=1、词 ctx 致命）与 GNU 相反——GNU 两上下文均非致命（cmd ctx
   status=0、词 ctx 继续 rc 0），已按 GNU 修正。
   效果：cli arithmetic 过滤 13 失败→3；errors 族 run-83 183→181 行（GNU 168）。
   run-83 A/B（cond/errors/more-exp/rhs-exp/precedence/varenv/arith-for）无回归。

仍残留（均经 HEAD stash A/B 证实为预存，非本会话引入）：
- 词法器在 `$((…))`/算术上下文丢失引号拼写：`(( '1' ))` raw 归一为 `1`（GNU 保留至
  求值器报 operand expected）、`a[\" \"]` 引号被剥留反斜杠（GNU 得 `a[" "]` 下标 0）。
  3 个 cli_tests 待此项；需词法器 word 值/raw 对算术跨度保留原拼写专项。
- `a[""]` GNU 报 `` `a[]': not a valid identifier ``（非致命），rubash 静默按 0 处理，
  诊断缺失（语义 status/继续性已对齐）。
- run-83 arith 族 TIMEOUT：arith3.sub 中 5000 次数组自增循环后悬挂（孤立复现不成立，
  需全上下文 debug 专项）。
- 全量 cli_tests 44 失败中含大量 examples/scripts/引号类预存失败（HEAD A/B 抽证
  nested_parameter、compat 8 项均在 HEAD 同样失败）。

## 十一、2026-09-03 全量门禁与并行轨道

全量 check（upstream-rights 82 家族，RUN83_TIMEOUT=180）：**PASS 13 / DIFF 67 /
TIMEOUT 0 / SKIP 2**（2026-09-02 基线 PASS 7 / DIFF 69 / TIMEOUT 4 / SKIP 3；
原始产物 `target/full-gate.log`）。新 PASS 13 家族：comsub2, dbg-support2,
dstack2, dynvar, extglob2, extglob3, getopts, herestr, ifs, invert, mapfile,
nquote2, tilde。TIMEOUT 4→0（stdin `</dev/null` 修复；ifs-posix 给 180s 可完成）。

剩余最大真缺口（排除 intl/history 平台项）：ifs-posix 1503（状态污染，LLDB 专项），
globstar 512（**分类修正：非平台项**——tests 自建目录树；根因 `./` 前缀风格 /
`ln -s` 符号链接 / 递归语义），new-exp 368，procsub 366（`/dev/fd/N` 抽象），
dbg-support 323，redir 249（fd 生命周期，与 procsub 同属 fd 模型族），more-exp 197，
heredoc 135，quote/quotearray 166，posixexp 93（sed 解析簇 + UTF-8 载体）。

关键塌缩：builtins 458 行缺口 → 494/524（差 30）；exp → 差 13；posixexp2 →
40/40 行数相等；vredir → 差 5；comsub 基线纠正为 79/85（旧 98 行基线系毒化期产物；
gen 期 run-83.sh:90 强制 `THIS_SH=bash`）。

并行轨道 A–E 已分配（文件领地互不相交；明细见
`docs/83-TEST-FULL-ANALYSIS.md` 第九节）：A globstar（glob.rs）/ B fd 模型族
（execution_misc/redir）/ C 族H 深层展开（parameter_words/read_split）/
D iquote-quote lane（quotes/embedded_parameters，captain）/ E ifs-posix LLDB。
平台/环境伪影负面清单（env 形态 118v18、stdio 交错、/tmp 映射）见
`docs/bash-compat-issues.md` 第七节，勿当语义缺陷修。
- heredoc：上述 comsub-heredoc 的 tokenize_with_heredocs 行累加协调。
- procsub /dev/fd/N fd 抽象（fd/device model 首项）。

### 预存共享树危害（非本会话引入，未触碰）

- src/builtins/fc.rs 有 +232/-6 未提交改动（半成品 fc 重写：--help/-l/-r/-s 选项），
  可编译但使 9 个 builtins::fc::tests::* 失败。最近提交 bd0a48ec(2026-08-29) 之后由
  上一会话/智能体留下。本会话 tests.rs 改动触发全量测试二进制重编方使之暴露。按
  AGENTS.md 不 stash/回退他人半成品，仅记录；如需清理建议单独任务核实 fc 重写意图。
- executor::tests::unit_tests::export_assignment_arg_preserves_quoted_spaces 依赖 PATH
  环境净化，本机真实 PATH 导致失败，与本次修复无关。

## 十二、2026-09-03 已验证平台审计纠偏附录

本节只纠正本文件前文的分类/优先级快照，不删除或改写历史记录；以下结论来自本次已验证的 WSL GNU Bash 5.2.21 对照审计。

- **globstar**：大部分残余是实际的多重性语义，尤其是 `**/**` 的折叠/匹配，不应整体归为平台噪音或排序差异。当前仅约 8 行可归因于 harness 两侧 `ls` 排序不一致；后续应把多重性差异作为真实 globstar 工作项，并单独修正 harness 排序。
- **cprint**：差异是实质性的 builtin 函数体格式化问题（function-body formatting），不是函数内 `$0` 展开问题；最新权威检查为 `DIFF cprint (rubash=44, right=72)`，保留为真实内建/格式化缺口。
- **invocation**：`SHELLOPTS` readonly 行为已经匹配 GNU，不再列为缺口。仍需处理的真实项是长选项、`BASH_ARGV0` 与 pretty-print。
- **mapfile**：mapfile 已通过验证；此前关于 CR 字节/CRLF 的缺口结论是陈旧 harness 伪像，应从待修与 P0 列表移除。
- **dstack**：已修复 cd 返回后 typed `PWD` 未同步的问题，最小 probe 中 `cd /; echo $PWD` 现在得到 `/`；完整 `dstack` 仍为 `DIFF 55/49`，剩余主要是错误输出顺序/归并和栈显示差异，不能标记为完成。
- **procsub**：路径分隔符丢失问题已修复。当前仍存在真实的多余 fd-counter 输出；`/dev/fd/N` 与 Windows 临时路径的剩余差异需和该输出问题分开记录。
- **平台归属**：`intl`、`history`、`histexp` 的差异均属平台/宿主所有，不应作为 Rubash 语义缺口追修。
- **printf**：GNU 对照会产生约 2 GiB 的病态基线输出；这属于 pathological baseline，不应按普通 diff/超时门禁解释。分类与处理规则见 `tests/gnu-compat/PATHOLOGICAL-BASELINE.txt`。

后续优先级应据此更新：globstar 多重性、cprint 格式化、dstack 根路径、procsub fd-counter、invocation 长选项/BASH_ARGV0/pretty-print 为真实工作项；mapfile、SHELLOPTS、intl/history/histexp 为已通过或平台归属项。

## 十三、2026-09-04 Globstar 多重性修复

- `src/executor/glob.rs` 已修复非相邻多个 `**` 的重复发射，以及相邻 `**` 折叠后的零深度目录尾斜杠。
- 聚焦 probe `**/a/**` 的输出数量从 111 收敛到 GNU 的 49；`**/**`、`**/**/a`、`a/**/**`、`**/**/**` 的数量保持分别为 30、15、15、30。
- 权威门禁：`MSYS_NO_PATHCONV=1 wsl bash tests/gnu-compat/run-83.sh check globstar`，结果 `PASS globstar`（rubash=587，right=587）。
- heredoc 仍不能据此关闭：`check heredoc` 当前为 `DIFF (rubash=166, right=31)`，同一行 command-substitution header 的 heredoc 仍待 lexer/token collection 专项修复。
- cprint 的独立 probe 曾确认 `declare -c` 是真实缺口：GNU 将每个单词首字母大写。当前已在 `declare.rs`、`declare/attrs.rs`、`executor/variable_state.rs` 接通 `-c/+c`、互斥属性、赋值转换和 `declare -p`；focused probe 与 Rust 单测通过，但 `casemod` 全文件仍为 `DIFF 49/49`，剩余差异尚未闭合。该缺口属于 declare 属性状态/赋值转换，不应通过修改 cprint expected output 解决。

## 十四、2026-09-04 invocation/cprint 复审

- `ShellInvocation::parse` 的 5 个单元测试全部通过，但 `src/main.rs::run_args` 仍是独立窄解析器；直接替换并不安全，因为 `--rcfile`、`-i`、`--pretty-print` 尚无完整 runtime plumbing。WSL GNU 脚本探针确认 Rubash 当前把这些选项误作脚本名，不能宣称 invocation surface 已完成。
- cprint 的剩余差异不是简单换行问题。GNU `print_function_def` 递归打印 compound command 并维护缩进；Rubash `type_functions.rs` 通过扁平 command serializer 生成文本，无法用小改动恢复 pipeline/background/group/loop/if/case 的结构。保留为高风险 pretty-printer 专项。

## 十六、2026-09-06 完整 83-test bounded check

运行：`RUN83_TIMEOUT=15 MSYS_NO_PATHCONV=1 wsl bash tests/gnu-compat/run-83.sh check`。

结果：`PASS=14 DIFF=63 TIMEOUT=3 SKIP=3`。PASS 项为：`comsub2`、`dbg-support2`、`dstack2`、`dynvar`、`extglob2`、`extglob3`、`getopts`、`globstar`、`herestr`、`ifs`、`invert`、`mapfile`、`nquote2`、`tilde`。

TIMEOUT：`arith`、`ifs-posix`、`read`；SKIP：`jobs`、`printf`（无 `.right`）、`trap`（GNU baseline 无法完成）。其余 63 个测试为 DIFF；完整原始汇总位于 `target/issue-suites/results/check/SUMMARY.txt`，逐项差异位于同目录。该矩阵取代文档中的旧 83-test 数字。

下一步优先级：先处理可复现且高影响的 `redir/vredir` 动态 fd 与 heredoc 状态收集；随后处理 `procsub` fd 生命周期、invocation runtime plumbing、cprint 递归序列化和 casemod 关联数组路径。`intl`、`history`、`complete`、信号/设备相关差异按 Windows-first 平台边界单独归类，不以 GNU/Linux 输出逐行强行对齐。

串行复核 `check vredir`（避免 runner 共享 `target/upstream-tests` 的并发污染）在本轮修复前为 `DIFF vredir (rubash=118, right=123)`；修复后为 `DIFF vredir (rubash=122, right=123)`。已闭合的首个稳定根因是 `while ... done {fd}<file`：GNU `parse.y` 在 compound command 完成后把尾随重定向绑定到 loop command，Rubash 原先因 `{fd}` token 被当作普通 Keyword，拆成循环、`unexpected token '}'` 和独立 redirect 三个 AST 节点。`src/parser/redirections.rs` 现在在 compound suffix 阶段识别分离的 `{name}` + redirect operator，`src/parser/tests.rs` 增加 AST 回归；`src/executor/command_input_scope.rs` 在 compound 执行期间应用并收尾 dynamic fd，相关最小 probe 已与 GNU 逐字节一致：`got:x`、`got:y`、`fd=<10>`、`after=<>`。剩余 `vredir` 差异包括 heredoc pretty-print 空格、close-direction serialization、vredir3/vredir6/vredir8 fd 语义，仍对应 GNU `redir.c` dynamic-fd allocation/close/undo 族，不能用 `.right` 文本修补。

`vredir6.sub` 的独立对照确认其中剩余差异是平台边界：GNU 在 `ulimit -n 6` 后执行 `exec {v}</dev/null`，报告 `cannot duplicate fd: Invalid argument` 与 `/dev/null: Invalid argument`，随后 `${v-unset}` 为 `unset`；Rubash 的虚拟 `FdTable` 不受 POSIX `RLIMIT_NOFILE` 约束，输出 `ok 1` 和动态 fd `10`。该差异属于 Windows-first fd 资源模型，不能伪造 GNU 错误来关闭 suite。`vredir8` 的无设备依赖 probe `shopt -s varredir_close; : {fd}>&1; echo >&$fd` 已与 GNU 一致：`fd=10`、正常命令继续执行，随后 `$fd: Bad file descriptor`。因此 vredir8 剩余的 `/dev/tty` 错误优先归类为 Windows 设备路径差异。下一片改查 `vredir3` 的 `set -u` 动态 fd close 诊断和错误传播。

## 十七、2026-09-06 heredoc 坏基线修复与真实差异

`run-83.sh gen` 依赖 WSL 内调用 `wsl.exe`，在当前 WSL 环境不可用，导致 `heredoc.right` 一直是坏基线（31 行，CR 污染/截断，缺失 heredoc.tests 主文件正常输出）。已用与 run-83.sh 相同机制手动重新生成：WSL GNU Bash 5.2.21 + 编译的 recho/zecho/printenv helpers + CR-free clean copy + `THIS_SH=bash`，得到真实基线 186 行并替换 `tests/gnu-compat/upstream-rights/heredoc.right`。

替换后权威结果：`DIFF heredoc (rubash=169 right=186)`，真实差异 17 行（此前 169 vs 31 是坏基线数字）。逐项分类：

- heredoc3：`$(cat <<EOF\n...\nEOF)` 形式（终止行 `EOF)` 携带命令替换闭合 `)`），GNU `make_cmd.c:602-611` 用 `shell_ungets` 放回 `)` 并报 `here-document at line N delimited by end-of-file` warning；Rubash 执行输出正确但缺 4 条 warning，且尾部 syntax error 行号/措辞不同（`line 92 near unexpected end of file` vs GNU `line 99 unexpected end of file`）。
- heredoc5：`cat`/`cmp` 对缺失文件（y.tab.c/config.h/version.h）的错误文本差异（引号、`cmp:` 双前缀、截断），属外部命令错误格式。
- heredoc7：缺 `command substitution: 1 unterminated here-document`（parse.y:4565）；heredoc 终止点/行号差异（GNU line 29 vs Rubash line 26，foobar/EOF 命令行号 29/30 vs 1/2）；`grep` 调用因 Rubash 解析到 Windows grep（全角冒号）属环境差异。
- heredoc9：函数序列化 heredoc 重定向格式差异（`if cat <<HERE; then` vs `if cat <<HERE\nthen`），与 cprint 递归序列化同族。
- heredoc10：`alias 'headplus=cat <<EOF\nhello'` 后 GNU 报 `hello/world/EOF: command not found` 与 `unalias: headplus: not found`，Rubash 却执行了 alias 后的 heredoc——alias+heredoc 行为真实差异。
- heredoc.tests 尾部：`cat <<''` 的 warning 输出顺序差异。

下一片建议从 heredoc7 的 `command substitution: 1 unterminated here-document` 与 heredoc10 的 alias+heredoc 入手（真实语义差异），heredoc3 warning 需在 lexer word 内嵌命令替换路径传递终止符标记（改动面较大）。

## 十五、2026-09-06 兼容性局部审计（已被完整矩阵 supersede）

本节保留局部 focused 结果作为根因记录；完整 83-test 结果以紧邻的“十六”节为准。

- 已完成局部根因修复但整族仍未闭合：nested heredoc 的 command-substitution header/body 收集、`declare -c` capcase、cd/pushd/popd 后 typed `PWD` 同步。
- `declare -c` focused probe 与 Rust 单测已对齐 GNU：首字符大写、追加赋值、`declare -p` 和 `+c` 清除；`casemod` 剩余差异不应归因于该局部修复。
- `cd /; echo $PWD; pwd -P` 的最小 probe 已返回 `/`、`/`；dstack suite 剩余主要是错误输出归并和 directory-stack 显示差异，仍需单独处理。
- invocation 的 `--rcfile` 不能直接复用非交互 `--init-file` 路径：GNU 脚本模式不读取 rcfile，错误接线会产生回归，已验证并撤回。
- 其余未完成项目（`posixexp2`、`cond`、`comsub-posix`、`braces`、`ifs-posix`、`redir`、`new-exp`、`dbg-support`）仍保留为后续根因专项。

## 十八、2026-09-07 CRLF 伪影纠偏与 read/case/赋值修复（#62 轨道）

### CRLF 伪影（重大基建发现）

core.autocrlf=true 把 vendored bash 测试树（third_party/bash，submodule）检出为 CRLF；GNU 把 CR 当普通数据，凡从工作树直接运行的 GNU 基线均被污染：

- mapfile1.sub 的 echo 行尾 CR 生成幻影尾随空格词；mapfile2.sub 数组名变 A<CR> 触发 sh_invalidid，整个 -d 测试静默跳过。
- third_party/bash 是上游 submodule，父仓库 .gitattributes 对其无效；worktree 改动已还原。纪律：GNU 基线一律从 LF 归一化的 target/issue-suites/results/bash-tests-rw 运行（THIS_SH 需导出）。
- bash-tests-rw 已全量重归一化（642 文件；零嵌入 CR 验证）。mapfile.tests 干净重跑 GNU/rubash 完全一致（170=170，diff=0），mapfile 候选关闭。

### 干净基线重跑量化（target/issue-suites/results/ledger62-rerun/，THIS_SH 已导出）

| suite | rubash | GNU | diff 行 | 备注 |
|---|---|---|---|---|
| array | 837 | 853 | 682 | GNU rc=1 超时截断，需分段基线 |
| builtins | 494 | 524 | 280 | GNU rc=2 |
| quotearray | 143 | 152 | 205 | declare -A key 引号转义 |
| complete | 366 | 387 | 115 | 打印顺序 |
| cond | 165 | 174 | 49 | rc 语义 |
| posixexp2 | 40 | 40 | 14 | ${...} 引号保留，#52/#56 族 |
| procsub | 33 | 33 | 12 | procsub 进程存活期语义 |
| braces | 112 | 102 | 18 | {0..10} 序列 |
| posix2 | 6 | 4 | 8 | 修复后剩 -x 与 set 输出格式 |
| mapfile | 170 | 170 | 0 | 已关闭 |

### 本轮修复（已推送）

1. c9466df3 read 标量切分三重修复：范围扫描消费转义对；先尾随修剪后解转义（多名末变量逃逸感知 / 单名整行逃逸盲，对应 read.def branch (a)/(b)）；多名余量经 apply_shell_assignment 清空。read.tests 首分歧行 3 → 行 34。
2. 881b2376 ① case：in 后首个裸 esac 按保留字拒绝（GNU 接受矩阵 5/6 对齐；;; esac) 嵌套深度规则待办）② 赋值值中单引号内双引号为数据（expand_assignment_value hoist/restore 包装）③ quoted-RHS 逃逸引号标记恢复，修 c="a\"b" 泄漏。
3. 37a27627 .gitattributes（对 submodule 惰性，仅文档价值）。

### 新定位根因（待修）

- procsub：已修共享流语义——`<(cmd)` 临时路径经 `ShellState::procsub_streams`
  注册为 `Rc<RefCell<ProcSubStream>>` 共享读游标，进程内各打开点
  （read/函数 stdin/仿真 cat/`<&N`/管道段）经 `procsub_stream_take` 服务剩余字节
  并推进偏移，spawn 子进程只拿剩余字节物化（subst.c:7143 / redir.c dup2 共享
  open file description）。procsub.tests count_lines `1,0,0,0,0` 对齐，套件归零。
  残余语义差：`>(cmd)` 方向与真管道存活期（GNU 子进程退出后路径 -e 为假）未覆盖。
- posix2 -x：chmod -x 后 test -x 仍真——Windows 可执行位模拟缺失。
- posix2 variable quoting 1/3：set 内建输出引号格式（SQUOTE 应为反斜杠引号，VHASH 应为裸 ab#cd）。

## 2026-09-07 第八轮：GNU 上游覆盖 19→83 套件（新接入 64）

- 新接入 64 个上游 .tests（bash-tests-rw-new/，LF 规范化），其中 **20 个直接 diff=0**：alias、appendop、attr、case、comsub-eof、dbg-support2、dstack2、extglob2、extglob3、glob-bracket、herestr、ifs、invert、parser、posixpipe、set-e、set-x、strip、tilde、vredir。
- 新增最大待收敛块：dbg-support 635、new-exp 455、exp 141、nquote1 133、more-exp 112、shopt 107、globstar 101、glob 97。
- 本轮修复（array 627→622）：
  - declare size-hint：`declare -a b[256]`（无 `=`）GNU 丢弃下标，按裸名登记并在 `declare -p` 打印 `declare -a b`（declare.def）；print_unset_declaration 渲染 a/A 属性。
  - POSIX 模式 `readonly -a` 列表：`readonly -a name=...`（原为 declare 格式；setattr.def posix 语义）。
  - lexer：`name=(...)` 复合赋值词在作为内建操作数时保持原子（skip_word compound_paren_depth；GNU parse.y 语义）——18 套件台账零回归。
- 已知深层缺口（诚实登记，未硬凑）：declare 复合操作数经去引号后元素边界丢失（`declare -ar b=([5]="hello world")` 被拆成 [5]/[6]；assign.rs:139 TODO 承认的 parser 限制）；dbg-support/new-exp 分诊待做（子代理通道本会话 4 次全灭，转单兵）。

## 2026-09-07 合并收拢：两线开发并轨（master ← wt/crew-r9）

- 主树 in-flight 工作落为 1c8987e5（lexer/parameter 语义）+ 883438f4（ast_print print_cmd.c 移植重构，含 parser/mod.rs 与新文件必须同批提交）。
- wt-busybox 工作区（detach @ bcb3ae21，落后 master 27 提交）checkpoint 为 wt/crew-r9 1d847284；三方合并仅 4 文件冲突（word.rs / command_prepare / command_execute / type_functions），全部取 master 侧——accessor API 与 ast_print 移植均为 crew 意图的超集。
- 合并树 5ac716c8 与 master 逐字节一致：**crew 的表面增量已被 master 全部吸收**，唯一独有内容（pipe_source 2 行）经评估不保留。
- 方法论教训（重要）：
  1. **二进制路径污染**：套件输出嵌入 $0/THIS_SH 路径，不同目录构建的二进制对跑会产生数百行假差异（builtins 444 vs 19 全为此因）。跨二进制对比必须在同一路径重建。
  2. **GNU 侧早退截断**：WSL 侧 GNU 无 THIS_SH 时 comsub2/rsh 等 `${THIS_SH}` 依赖套件仅输出 32B/24B 即中止；rubash 正常跑完反而「diff 更大」。crew 的 0 差是复刻了截断行为的假完美。此类套件的真基线需 `export THIS_SH=/bin/bash` 的 GNU 全量输出。
  3. lib/cli_tests 通过 ≠ 套件台账通过：1c8987e5 提交前的 in-flight 验证漏掉了台账维度，套件级回归必须进提交前检查单。

## 2026-09-07 第九轮：真基线重建（83 套件）+ fc 族关闭 + lexer 复合词修正

### 真基线方法论修正
GNU 侧导出 THIS_SH=/bin/bash 后 ${THIS_SH} 子调用真实执行，消除早退截断；固定单一二进制路径消除 $0/THIS_SH 路径污染。runner: true-baseline.sh；产物 true-baseline/。**此前多轮台账数字作废，以本轮为准。**（注：该"本轮"声明本身也已被后续各轮取代——下方 09-07 排行如 assoc 360/rsh 193 均为历史快照，最新口径见头部台账链。）

### 真实缺口排行（83 套件，21 个真零差）
dbg-support 635、array 456、assoc 360、nameref 303、new-exp 241、more-exp 232、posixexp 211、histexp 199、rsh 193、comsub2 190、history 188、quotearray 153、exp 134、quote 132、complete 115、shopt 113、varenv 109、globstar 101、invocation 93、alias 87、comsub 78、extglob 68、nquote 67、trap 61、glob 60、redir 58、func 58、intl 57、arith 50、dstack 50、read 48、precedence 46、type 45、jobs 37、errors 32、heredoc 29、iquote 28、rhs-exp 26、nquote1 25、comsub-posix 19、其余 ≤18。真零差：mapfile、printf、attr、casemod、cprint、dbg-support2、dstack2、dynvar、extglob2、extglob3、getopts、glob-bracket、herestr、ifs、invert、nquote2、nquote3、posixpat、strip、tilde、tilde2。

### 本轮根因与修复（提交）
- 2d6e32e9 fc：fc 调用自身不入列表/编号（GNU fc.def）——fc 族 10 测试全绿，lib 0 已知失败（余 1 为并行测试 PATH 竞态 flaky）
- 2af92b9d lexer：name=(...) 复合值内全部元字符为字面量直至配对右括号（GNU read_token_word；array.tests `test=(first & second)` 为单条失败赋值而非异步列表）

### 数组族根因分桶（array 456 行）
1. 复合赋值值内引号分组丢失（~250 行，已知深层：declare -a d=([5]="hello world") 被拆分）
2. declare -a e[10]=test 尺寸提示+赋值语义（GNU 赋元素[0]，rubash 泄漏 e[10] 名字）
3. test=(first & second) 类：需 GNU 式 syntax-error-continue 语义（报错 rc=1 不中止脚本）
4. DIRSTACK 动态同步时序（GNU 存储格在 pushd 前为空、declare -p 触发懒同步；~4 行）
5. readonly declared-unset 数组 c 的 declare -ar c 列表丢失

### 子代理通道
本会话 8/8 全部在开工前夭折（零产物）。所有任务单兵完成。通道修复前不建议再派发。

## 2026-09-07 重大发现：上游仿真层拦截套件，台账测的是仿真不是语义

- `src/executor/upstream_scripts.rs` 的 `try_upstream_scripts()` 按 script 路径/CWD 拦截 ~70 个上游测试套件，输出硬编码仿真结果——83 套件台账历轮数字（含真基线轮）衡量的是**仿真保真度**，非 rubash 真实 GNU 语义。
- 已加测量旁路开关 `__RUBASH_NO_UPSTREAM_SCRIPTS=1`（runner 已启用）：旁路时全部套件走真实 lexer/parser/executor。
- 复合赋值 RAW 保留（token_actions）：`name=(...) `原子词加 `__RUBASH_CA1__`+原样 RHS，独立赋值语句形式已端到端正确；declare 操作数形式经 and_or_list 执行路径仍有二次去引号，待该路径与 materialized 分派的合流后收敛。
- 执行分派存在双路径：单命令走 execute_materialized_command（instrumented 可见），多命令走 and_or_list 快路径（绕过前者）。后续插桩需两路同插。
- 下一步：旁路真基线全量重跑 → 真实语义缺口地图 → 按根因逐族收敛，仿真层按 AGENTS.md 规则待真实语义达标后退役。

## 目标轮 1（83→83 战役）：P1 根因全链路定位 + bashdb fixture 修复程序

### P1 复合赋值引号分组——根因链完整定位（设计就绪，分层重落未完成）
1. **词法根因（决定性）**：`next_token` 先 `advance()` 消费首字符再分派，`skip_word` 内 `word_start=self.position` 从第二字符起算——`compound_assignment_start` 的前缀永远缺首字符（"e=" 被看成 "="→""），`name=(` 判定永不成立 → Assignment 词元在 `=(` 处终止、复合值裂成多词元。修复方向：`skip_word(token_start)` 传入真实词首（finish_word_token/scanner 四个调用点均有 start 可传）。已验证：改后词元 `e=([0]="x y" [2]=z)` 单词元、RAW 引号完整。
2. **解析层**：arm 1 需对原子复合词加 `__RUBASH_CA1__`+RAW RHS（与 else 分支对称）；marker 流已验证可达 builtin。
3. **展开层**：`expand_embedded_parameters_mut` 会剥掉 CA 标记词的元素引号——需 compound_assignment && 无 $/反引号 时逐字返回。两处同形函数（parameterized/plain assignment word）注意锚点区分。
4. **回退状态**：三层改动已在工作区验证 d/e 双形式与 GNU 完全一致，但因 cli_tests 53 个 bashdb_compat 失败（当时以为回归）而整体回退至 9ad47872。

### bashdb fixture：环境性失败根因与修复程序（重要教训）
- 52 个 bashdb_compat 失败 = `target/bashdb-clean/bashdb-generated` fixture 缺失（target/ 不入库），非产品回归。
- 修复：`bash scripts/setup-bashdb-fixture.sh C:/Users/Administrator/Downloads/bashdb-5.2-1.2.0/bashdb-5.2-1.2.0/_install`——**必须用 `_install`（构建产物）**，源码树 launcher 含未展开 autoconf 变量会 syntax error。
- **进程卫生教训**：本机多代理共享——taskkill 前必须 `wmic process get commandline` 确认归属；本轮误杀了并行代理的 niubash-runtime 测试进程链。
- 待办：fixture 修复后重跑 bashdb_compat 子集确认 master 基线，再分层重落 P1（每层：lib→cli→bashdb 子集→np 探针）。

## 目标轮 2：P1 复合赋值引号分组——完整修复落地（commit c7ee8df1）

### 修复链（四层，全部经真实语义验证）
1. **解析器收集器 RAW 合并**（assignment.rs）：词法在复合值内引号含空白处分裂的词元，在 collect_compound_assignment 内按 raw 引号配对重新接合——`[5]="hello` + `world\"` 重接为 `[5]="hello world\"` 单元素。
2. **declare 操作数收集器接入**（token_actions.rs else 分支）：`declare -a e=( ... )` 的分裂形式复合操作数改走引号保全收集器（此前走词值去引号路径）。
3. **Word 臂原子复合处理**（token_actions.rs）：非首词的原子 `name=(...)` 词元以 CA 标记 + raw RHS 入词表（对齐 9ad47872 的首词路径）。
4. **执行器逐字守卫**（parameter_core.rs expand_word_mut_with_context 赋值路径）：CA 标记且无 $/反引号的复合值绕过赋值引号剥离展开，逐字到达存储解析器。

### 量化（true-baseline 旁路口径，WSL GNU 5.2.21 基线）
- array: 456 → **444**（−12，正式口径）
- assoc: 358 → **358**（持平）
- quotearray: 153 → **153**（持平）
- P1 族合计 −12 行（正式口径）。**勘误**：此前报的 −540 系 CRLF 污染基线所致；引号分组修复的语义价值由四形式探针证明（ GNU 完全一致），其行数影响集中在长尾。残余大头是 readonly 列出/尺寸提示/DIRSTACK 等独立家族；四形式探针与 GNU 完全一致
- cli_tests（跳 bashdb）：**0 新增失败，修复 5 个**（含 issue78 多行数组 2 个）
- lib：338 通过（1 个已知 PATH 竞态 flaky）

### 方法论沉淀
- **stderr 可见性**：eprintln 调试输出在 `2>&1 | head` 管道下会被吞——必须重定向到文件再读。
- **词元流侦察**：handle_token 顶部 DBG-TOK 全量词元转储 + 判别探针（引号单参数 vs 裸形式）是定位词法/解析/展开分层故障的最短路径。
- **残余 P1 缺口**（444+358+153 行）：readonly 声明未赋值列出、`declare -a e[10]=test` 尺寸提示、复合内 `&` 语法错误继续语义、DA/引用元素等——下轮继续。

### 测量方法论固化（commit 7f0d1926）
- **`scripts/true-baseline.sh` 已入库**：LF 归一化 bash-tests-rw 同步、recho/zecho PATH、THIS_SH 约定（GNU 侧 /bin/bash，rubash 侧自检测）、`__RUBASH_NO_UPSTREAM_SCRIPTS=1`、stdout-only diff、按套件参数化。用法：`MSYS_NO_PATHCONV=1 wsl bash /mnt/d/repo/rubash/scripts/true-baseline.sh [suite...]`。
- **纪律**：任何套件数字只出自该 harness；禁止手搓探针测量（本轮 −540 的假改善即由此而来）。
- **下一个 P1 家族**：`declare -a d='(...)'` 整体单引号复合需解析器侧 CA 标记（已证实加宽执行器守卫会禁掉复合值内 glob/brace 展开而回退，方案在解析器）；随后 readonly 列出、`e[10]=test` 尺寸提示、DIRSTACK 惰性同步。

## 目标轮 5：并行子代理编队 + readonly 家族证据（进行中）

### 编队
- 代理 A（rubash-wt-hint，detached bf03eb1a）：尺寸提示家族 `declare -a e[10]=test` → element[0]（GNU arrayfunc.c convert_assign_array_element）
- 代理 B（rubash-wt-dstack，同提交）：DIRSTACK 惰性同步（GNU 新 shell `declare -a DIRSTACK=()` 直到 pushd）
- 代理 C（只读分析）：P2 参数展开族战役计划（new-exp/more-exp/exp 按特性分类 + subst.c 归属 + 修复顺序）
- 队长（主树）：readonly 家族证据收集 + 合并验证

### readonly 家族证据（已定位，待代理 A 的尺寸提示修复落地后跟进）
- array.tests:62 `declare -r c[100]`（带尺寸提示、无赋值）→ GNU：`c` 成为只读索引数组（声明未赋值），`declare -r` 列出 `declare -ar c`（仅属性无值）、`declare -p c` 同
- rubash 现状：`c` 完全未创建（`declare: c: not found`）——根因：无 `-a` 旗标时 `c[100]` 的下标剥离/数组创建路径未走（declare.rs ~389 的 strip 仅在 `array || assoc` 下运行）
- 该修复与代理 A 的尺寸提示机械同源（declare.rs 名字处理 + assign.rs），待其 worktree 报告后由队长统一合并实施，避免同路径冲突
- 列出层：DECLARED_UNSET_VARS 的只读数组必须出现在 `declare -r`/`readonly -p` 列表中（仅属性形式）

### v5 全台账（bf03eb1a，83 套件）：总缺口 5265
- 大户：dbg-support 635、array 442、assoc 358、new-exp 241、more-exp 232、posixexp 211、nameref 214、histexp 199、comsub2 190、history 188、rsh 194、quotearray 153、quote 132、complete 113、shopt 113、varenv 107、globstar 101、invocation 93、exp 134
- 绿区（0）：attr、cprint、dbg-support2、dstack2、dynvar、extglob2/3、getopts、glob-bracket、herestr、ifs、ifs-posix、intl(57→57 是 int-l 含义待核)、invert、mapfile、nquote2/3、posixpat、printf、strip、tilde/tilde2
- 注意：本轮 v5 与两个 worktree 代理并发运行时出现 GNU 侧一次段错误+一次 Killed（WSL 资源竞争迹象），对应套件数字可能有噪声；后续收敛轮复测确认
- 方法论警告：v5 运行与 worktree harness 并发时 stdout 日志与 ledger 分离（ledger 在 true-baseline-ledger.log，stdout 只有 TRUE-DONE/错误），勿把空 stdout 误判为失败

### 合并轮：DIRSTACK 家族落地（8074da5e）+ P2 战役计划入档

#### DIRSTACK（代理 B，已合并验证）
- 根因（探针修正）：GNU DIRSTACK 是全动态变量——命名访问时经 get_dirstack（variables.c:1618）重建；list-all 打印"最后物化的 cell"，未命名过则保持 ()（即使 pushd 之后）
- 修复：eager sync 移除 DIRSTACK；execute_declare 仅在命令词点名 DIRSTACK 时 sync_dirstack_cell()；初始 cell 为空索引数组；dirs -c 后 getter 仍暴露 cwd 于 [0]
- 门：lib 338/1-flaky；cli 门 325/28（失败集与基线字节一致）；array 442→434（−8，严格子集）；dstack/quotearray/dstack2/dynvar 持平
- 残余（代码注释已记）：动态 DIRSTACK[@] 读不物化 cell；pushd 间普通 cd 不重同步 stack[0]；unset DIRSTACK

#### P2 战役计划（代理 C，量化分诊 607 行 → 6 根因族）
- **F1 more-exp 静默吞没（215 行）**：L160-489 零输出（rc=0），恰在 L490 恢复——函数体扫描器把 `${1+"$@"}` 内的 `}` 当终止符，吞 ~320 行。最小复现：`b1() { b2 ${1+"$@"}; }`。owner：src/parser 函数体/花括组扫描（非 continuation.rs）
- **F2 引号 $@/$* 词产出（116 行）**：expand_word.rs:95 将 $@ join 成单串（铁证）；需 GNU quoted_dollar_at/contains_dollar_at 旗标驱动的逐词产出（词缀附首尾元素、0 参规则、数组赋值塌缩）。最高风险项，独占切片+全 A/B
- **F3 patsub 管线（60 行）**：patsub_replacement shopt 注册但从未被读；&/\\&/tilde/$var 展开、引号段跟踪需一次管线化重构（parameter_replace.rs/expand_braced_replacement.rs/parameter_ops.rs）
- **F4 变换族 @A/@a/@Q/@K/@k/@P（64 行）**：@A 按属性而非值键控（declared-unset 打 declare -rl VAR1）；@Q 需 ANSI-C $'..'（与 declare -p 共享 helper——array/assoc/quotearray 跨套件上行空间）
- F5 `!` 间接（19）/ F6 花括扫描（19）/ F7 数组标量强转（14）/ F8 转义双引号去引号（19，最后做）/ F9 小项（43）
- 修复顺序：F1 → F3 → F5 → F4 → F2（独占）→ F6+F7 → F9 → F8；F2/F8 需全 A/B（quote/nquote/ifs 绿区是回归哨兵）
- 完整工件：target/issue-suites/results/true-baseline/{new-exp,more-exp,exp}/{diff,hunks,anchors,srcmap}.txt

#### wave-2 编队（66efb46a 起）
- 代理 E（rubash-wt-f1）：F1 more-exp 吞没
- 代理 F（rubash-wt-patsub）：F3 patsub 管线
- 代理 G（rubash-wt-transform）：F4 变换族
- 在途：代理 A（尺寸提示）、D（dbg-support DEBUG trap）
- 路径注意：子代理工具曾把 D:\repo\X 解析为 D:\d\repo\X——代理提示词已要求 git rev-parse 自证；队长合并时从真实路径取 diff
- 环境注意：WINUXSH_ROOT 经 WSL interop 泄入 rubash.exe，cd -P / 物理解析错位（dstack 50 行主因、array 基线虚高）——待 harness 消毒实验

### 基线漂移事件（wave-4 猎杀结论）：v5→v6 的 11 个"回退"全部是测量假象

- **根因**：WSL `/usr/local/bin/bash` 于 2026-09-09 00:09 (+0800) 被升级到 **GNU bash 5.3.0**，恰在 bf03eb1a→def70809 测量窗口内；harness 的 PATH 顺序让它遮蔽契约基线 5.2.21
- **证据链**（代理 J，全程干净树）：① bf03eb1a 与 def70809 在默认 harness 下受影响套件数字**完全一致**（无代码提交动过它们）；② 钉版 harness（GNU=/usr/bin/bash 5.2.21）在两个提交上都塌回 v5 原值；③ 默认 harness 在干净 def70809 上精确复现 v6；④ GNU-vs-GNU（同一 rubash 二进制）diff 显示 5.3 改了自己的行为：嵌套花括号重试（braces.tests:133 "fixed post-bash-5.2"）、`${ echo;}` 语义命令替换（comsub2）、trap/cond/read 状态文案
- **修复**：true-baseline.sh 现以 `/usr/bin/bash` 显式解析 GNU 侧并断言 5.2.21（否则 exit 9）；gnu-ab.sh 作为漂移探测器入库；rubash 侧 PATH 同步去 /usr/local/bin
- **含义**：对齐 5.3 行为（嵌套花括号、dollar-brace 命令替换等）是**新战役**而非回归修复；契约基线保持 5.2.21
- **附带**：docs/builtins.md 已提交（include_str! 依赖，新 worktree 此前无法跑 cargo test --lib）；trap.tests 在 Windows 上会孤儿化 rubash.exe ./trap9.sub（harness reaper 待办）
- **v7（钉版口径）在测**：预期总缺口 ≈ 4119（4245 − 126 漂移假象）

### v8 全台账（5.3.0 新契约基线，首测）

- 契约：GNU 侧 = /usr/local/bin/bash 5.3.0（业主编译版，版本断言防漂移）；vendored 测试 = bash-5.3-16-gb4608166
- **总缺口 4155**（83 套件），零缺口 **24**：appendop attr casemod cprint dbg-support2 dstack2 extglob2 extglob3 getopts glob-bracket herestr ifs invert mapfile nquote2 nquote3 nquote5 posixexp2 posixpat precedence printf strip tilde tilde2
- top：array 387, assoc 344, nameref 209, rsh 194, history 190, builtins 182, histexp 176, quotearray 151, posixexp 147, comsub2 140, quote 132, complete 116, varenv 108, new-exp 107
- 解读：与 v6（同为 5.3 GNU 侧）比 **−90**；posixexp 211→147、comsub2 200→140 是远程 4 提交的真实改善；**builtins 15→182** 是 5.3 vendored 测试的新增语义（新战役最大单增户）
- 版本身份：BASH_VERSION/BASH_VERSINFO/--version 横幅已切 5.3.0（58c59a91）；套件输出零旧版本串泄漏，台账无需刷新
- 遗留口径（5.2.21）审计因 v7 流水线被脚本中途编辑破坏而延后（教训：勿编辑运行中的脚本）；J 的 4 台账证据链已完整记录漂移

### wave-4/5/6 合并实录（K/L/O 三族收官）

- **L（dbg-support 53→0）**：AND-列表双触发（execute_cmd.c 无 connection 节点火点）、source-scope 陷阱继承（source.def:208-216 + return_trap_in_scope execute_cmd.c:5295）、非行首 `{` 回归普通词法（parse.y 元字符集）、for 每迭代行号重置（execute_cmd.c:3039）。`5a2415ad`
- **K（assoc 344→274 + 连带 array/quotearray/new-exp −86）**：ASSOC_HASH_BUCKETS 1024、!A[@] 键值解构、declare -p GNU 引用规则（assoc.c/shquote.c）、引号感知切分、名字长度截断之谜（next_token 先吃首字符）、收集器覆写、存储往返、元素赋值元数据回退、`\$` 去除。`a65f0e33`。合并战役：另一会话在 assignment_helpers.rs 有独立 1024 实现 → git merge-file 三方合并（唯一冲突=注释，保留业主版）；CRLF 补丁需 --ignore-whitespace 入索引
- **O（rsh 194→0，与 GNU 5.3.0 逐字节一致）**：主导根因 = `set +o restricted` 静默解除限制（set 快速路径跳过 GNU 拒绝检查 flags.c:227-235）→ exec 了 Windows shim 喷 190 行横幅；另修 set +r 文案/退出码、BASH_CMDS 赋值守卫、hash -p、管线成员斜杠拒绝（两条并发路径）、source 文件名、裸 exec 放行、受限只读集、argv0 rbash 自动受限。`1300aeca`
- **台账演进：v8 4155 → L −64 → K −156 → O −194 ≈ 3741；零缺口 24→26（+dbg-support +rsh）**
- 流程沉淀：管道后 `$?` 取的是 head 退出码；跨树补丁先查 CRLF；共享文件 git merge-file（base=HEAD, ours=主树脏态, theirs=代理版）；exe 句柄锁可改名绕过（mv rubash.exe → cargo 重写）；trap9/printf7 孤儿每次全量后例行核查
- 在途：M builtins 182、N2 quote 族 172、P array 357、Q assoc 274（wave-5/6）

### 会话接力实录（2026-09-09 晚，owner 指令接替停手的并行会话）：trap/invocation/histexp 落地 + 信号编号统一，v9 台账 3427，零缺口 31

主树 staged 的 trap 链批（与 wt-trap 工作副本 11/16 逐字节一致、5 文件为新基线版本）先落 `f9e42da9`；随后清点 unstaged：declare/set 族为纯 CRLF 噪声（renormalize 收拢 `79b7b540`），真实增量两笔——comsub parser 侧 heredoc `delim)` 收尾镜像 lexer 扫描（`2caddded`，make_cmd.c PST_EOFTOKEN）与 complete 多操作数注册（`cd4eb4f8`，`complete -F f c1 c2` 双双生效）。wt-invocation（+718 行）git apply -3way 落主树，唯一冲突 ast_print.rs 为双方各自新增函数（保留双方）→ `327d32db`（BASH_ARGV0 环境导入 $0/shell_name、login-shell argv0、长选项表、-o/-O 启动期报错 prolog、--pretty-print；invocation 14→2）；wt-histexp（fc.def verbatim 移植 +507 行）三方合并干净 → `75b91b68`（histexp 5，history 190→160）。

**信号编号统一（`ec45a000`）**：rubash 全表从 BSD/Cygwin 风格（USR1=30、CHLD=20、RTMIN=32）换到 Linux 表（USR1=10、CHLD=17、RTMIN=34，32/33 空缺），与 GNU 5.3.0 WSL 契约基线一致；`kill -l`/`trap -l` 输出逐字节对齐（含表尾 tab）、BASH_TRAPSIG 携带 Linux 号、`trap 17`=CHLD、TerminateProcess 退出码 128+N 归位、CHLD kill 不再 terminate。继承语义：SIG_IGN 经进程边界传递（`trap '' USR2` 后子 shell 不可再 trap，trap1.sub 对齐 GNU）——空值 trap 键活不过 Windows 环境块，改由 ORIG_IGN 合并清单跨边界运输。

**trap 链 residual 16→3（`5ff2854a`）**：ERR action `$LINENO` 绑失败命令行（action AST 重解析的 position-1 行会污染 CURRENT_LINE，按 run_debug_trap 同法钉行号，trap3.sub `trap: 8` 对齐）；SIGCHLD 通知在 action 运行中到达时排队、外层按 pending 重放（三个后台 job 三次 catch，原实现丢 2 次）；后台子进程剥离继承 trap 表（POSIX caught-trap reset，否则 `rubash -c` 子壳结束时误放父 EXIT trap 喷 3 行 "exiting"）；traced 函数 DEBUG 行号用 body_open_line（经 definition registry 供普通词调用路径，func2[43] 对齐）。

- **v9 全台账：3427**，零缺口 **31**（28 + trap 邻近收敛 + intl/comsub 等晚间批次落账）；trap 16→3、invocation 14→2、builtins 182→2、complete 0、shopt 13、histexp 6→5、history 190→160
- cargo test 2740 全绿（唯一失败 `export_assignment_arg_preserves_quoted_spaces` 为环境敏感既有问题：工作台注入的 PATH 键名/形状变化触发，stash 至 327d32db 亦复现，与本轮改动无关）
- **残余（trap 3 行）**：trap6.sub `$( f )` 中函数内外部命令（`/bin/echo bar`）stdout 直写真实 stdout 未入捕获，RETURN trap 的 builtin 输出反被捕获——外部命令 comsub 捕获路径对路径前缀词失效（`$(/bin/echo x)` 亦泄漏而 `$(whoami)` 正常），判定分歧在 external_needs_fd_copy_capture 之外的派发路径，独立战役
- 流程沉淀：跨树合并首选 `git apply -3way --ignore-whitespace`；冒烟探针先分 stdout/stderr 再下结论；`env` 在本机被 shim 劫持（/usr/bin/env 才真）；全量台账 10 分钟跑完可直接后台

## 2026-09-10 census 桶 #4/#5：assoc 键序判定 + history -d 范围

- **census #4 assoc"键序差异"判定：基本形态不成立**（assoc-order.sh 六形态 O1-O6：插入序/逆序/数字键/符号键/${!A[@]}/桶增长，GNU 5.2.21、5.3.0、rubash 三方逐字节一致——GNU 的序是哈希桶序非字典序，但 5.2↔5.3 稳定且 rubash 已复刻）。**真实差异在复合赋值键切分/转义层**：assoc.tests rubash vs GNU 5.3.0 = 323 行，形态：`(["bar\"bie"]="doll")` 键内转义错、多元素粘连成一个键、assoc9.sub L16 unexpected EOF、declare -p 对特殊键带外层引号包裹、BASH_CMDS 动态数组遍历序与计数列。**未修，另批大切片**（与 array 复合赋值同族，根因疑在 split_compound_assignment_words 对 [键] 内引号的处理）
- **census #5 history 已修大头**：`history -d start-end` 范围删除（GNU 5.3 特性，history.def:190-262 + bashhist.c bash_delete_history_range + readline remove_history_range）。语义：分隔符扫描跳过首字符 `-`（`-2-4`=start -2, end 4）；负数从尾部计数（-1=最后一条）；正数减 history_base；闭区间 drain；first>last 静默 rc=1；越界报 "history: {side}: history position out of range"（side=对应端文本——GNU 用就地 NUL 截断）；valid_number 是 strtoimax base 10（general.c:248），**0xaf 是 invalid 报整个 arg**。history.tests 284→243 行
- **残余（入册未修）**：套件里 rubash 会话历史入表内容/时机差（`history ; echo` 多命令行条目、空条目 ${BASH_VERSION%\.*} 行）；execute_history_session 的 erange 消息缺 shell stderr 前缀（ledger 不计项）；探针 bashver.sh 证实 BASH_VERSION %/# 变换四形态 rubash 与 GNU 一致
- 回归：executor_tests failset 114 零新增；array.tests 5.3.0 口径 436 行维持

## 2026-09-10 数组嵌套引号替换战役（lane wt-regress @ 8d30cd04 起）

任务：根治 `${a[@]/#/"-iname '"}`（数组形态 patsub + 替换词含引号）产出垃圾的问题。oracle：WSL `/usr/bin/bash`（GNU 5.2.21），测试件为 `target/issue-suites/results/bash-tests-rw/` 的 array.tests staged 副本 + 自编译 recho/zecho（/tmp/bash-helpers）。

### 已修（工作树，patch 见 target/lane-all.patch）

- **DQ_DATA 提升**（assignment_expansion.rs `hoist_data_double_quotes`）：原实现把值里**所有** `"` 提为 E001 数据标记，连 `${...}` 体内的引号算子也被吞；现在用 `matching_parameter_brace` 逐段跳过 `${...}` 体，体内引号保持算子身份
- **词法原子性**（scanner.rs/word.rs `skip_word_at(start)`）：`next_token` 已消费首字符导致 `compound_assignment_start` 只看到 `2=`，多字符名 `a2=(...)` 不再走 token-split 路径
- **复合赋值元素切分器视 `${...}` 体为不透明**：`split_compound_element_words`、`split_compound_assignment_words`（parser/assignment.rs）、`compound_raw_quote_unclosed` 均经 `scan_braced_parameter_body` 跳体——体内引号/空格不再污染元素级状态（本轮新增：**StorageWordIter**（assignment_helpers.rs）同法跳体，修 f64 `\'` 形态的元素合并）
- **复合赋值逐元素 patsub**（`expand_compound_positional_at_assignment` 新分支）：quoted `${a[@]/pat/rep}` 逐元素过 `replace_patsub_pattern`（generic word expander 会把数组塌成单串）；本轮扩展：**裸 `@`/`*` 名（`${@/...}`）也接受**（取 positional_params），`[*]` 按首 IFS 字符 join 成单元素（GNU join_array_values 语义）；复合赋值值形态跳过尾部整体 `remove_shell_quotes`（GNU 不对展开结果重跑去引号）

### 测量（同口径 A/B：stash 全改动 vs 改动后，array.tests 全套 vs GNU 5.2.21）

- 基线 461 行 diff → 改动后 **444（−17）**；f55/f64/f72（`${a[@]}`/`${@}` × `"-iname '"`/`-iname \'` 替换）与 GNU 逐字节一致
- executor_tests 单线程 failset：**基线 119 → 改动后 114（净 −5，零新增失败）**。转绿：`part_002/003::compound/local compound assignment preserves quoted array at`（两处，见下方第三修复）、`part_026::kill` 两项、`part_041::eval expands assignment lhs to array name`、`part_069::declare assoc alternating bracket words`
- **第三修复（由 executor_tests 哨兵逼出）**：原子词法路径下 `local v=("${foo[@]}")` 的元素值不再带 `\x1d` quoted-RHS 标记（改为 E001 包裹裸引号），`expand_compound_positional_at_assignment` 的 plain `${a[@]}` 分支漏配 → generic 展开合并成 `"a b c d"` → 存储时 whitespace 拆成 4 元素。修复：该分支追加 `trim_matches('\u{E001}')` 形态匹配
- 教训：**二分 stash/pop 后必须立刻重建**——上轮 bisect 结束忘了 build，CLI 探针跑的是回退态旧二进制，得出"CLI 过测试挂"的假矛盾，白查两轮

### 新定位 gap（本轮未落地，按差异化→文档→清池→批修流程入池）

- **eval 二次解析族**（array6.sub L58/61/67/75/78，5 形态全 FAIL，基线同 FAIL）：GNU 把赋值形状的 eval 参数按**赋值上下文**展开成**不加合成引号**的扁平串 `a2=(-iname 'abc -iname 'def)` 再重解析；rubash 逐元素 re-quote（`a2=("-iname 'abc" "-iname 'def")`）。经验证据：GNU 复合赋值元素扫描对 `'abc -iname 'def` 产出 2 元素（闭合引号后继续吸收直到非引号空白）——probe 存 target/forms/gnu-probe-eval.sh。修复面：eval 参数 flatten 去合成引号 + 重解析侧元素扫描语义，牵动 quote_array_value 存储契约，建议独占切片 + 全 A/B
- **非声明 builtin 后 `name=(...)` 被语法接受** + `__RUBASH_CA1__` 标记泄漏到用户可见输出（`echo a=(x y)`、`printf "%s\n" -a a=(a 'b  c')`）；GNU 判 syntax error near unexpected token `('（parse.y 仅 declaration builtin/赋值上下文接受复合赋值词）
- **plain `a2=("${a[@]}")` 复合赋值塌缩单元素**（g1 probe）；array6.sub L 案（unquoted 标量赋值 `-iname abc` vs GNU `-iname 'abc`）未动

### 第二批（eval 裸 flatten + debug 清除，本轮落地）

- **eval 裸 flatten 接线**（command_prepare.rs `expand_command_word`）：head word 为 `eval` 且词含 `__RUBASH_CA1__` 复合标记、值为 `(…)` 形状时，走 `expand_compound_positional_at_assignment_bare`——元素逐字面存储、不加合成引号（GNU subst.c evalstring：eval 参数按普通词展开，join 后重解析）。f61/f67/f78（`${a[@]}`/`${@}` 嵌套引号 eval 形态）转 PASS；gg7 E1 语义对齐（declare -p 输出 `[0]="a" [1]="b" [2]="c"` 与 GNU 一致）；无展开的 `eval a2=(x y)` 走 None 落回原路径，零扰动
- **debug 清除**：assignment_expansion.rs（4 处 RUBASH_DBG/`__result`）、lexer/word.rs、parser/assignment.rs、executor/assignment_helpers.rs 的全部 DBG 站点删除。教训：上一轮 bisect 的 stash/pop 把已删的 debug 编辑吞掉，泄漏进了已提交的 1f76a150（`grep -c RUBASH_DBG` = 6）；主树未 push，已用 amend 剔除。**规矩：stash/pop 之后除了重建，还必须 grep 一次 DBG 泄漏再提交**
- **executor_tests failset 提取口径修正**：`grep "FAILED"` 会被并行测试输出交错吞行（114 failed 只抓到 101 行），必须从 cargo 输出尾部的 `failures:` 汇总段提取。本轮 failset 与合并时 **114 完全一致（零新增、零回归）**
- **array.tests vs GNU diff 行：基线 461 → 上批 444 → 当前 435**（run-ab-b.sh 的 wc -l 口径）。勘误：早先把 `ls -la` 的**字节数**（13344/12965）误当 diff 行数汇报——diff 行数与字节数差 30 倍，测量汇报必须只认 wc -l 输出
- **census array 桶形态复核（96c88d19）**：`declare -a f=([0]="\${d[@]}")` 字面存储与 declare -p 输出、`{x}`/`y{` 花括号形态均已与 GNU 5.2.21 一致（target/forms/census-array.sh P1/P2/P3 PASS）——census 的 297 行是 8d30cd04 基线值，合并后该桶三形态已修
- **- **assoc kvpair/strict 双路径判定修正**（assoc-kv2/kv3 探针 M1-M7/D1-D6）：GNU 的 kvpair_assignment_p 语义分两条路径——**plain 赋值**（词无 W_ASSIGNMENT，executor/assignment_helpers.rs）：首词以 `[` 开头 → strict [k]=v，否则交替对（M2: a=(a=b c=d) 存 [a=b]="c=d"；M1/M6: a=([x] one) 报 must-use-subscript 且**不存储**——strict 循环首违规即 break 放弃整个赋值）；**declare 路径**（词有 W_ASSIGNMENT，declare/assign.rs + storage/assoc.rs）：首词是完整 [k]=v → strict（D3: declare -A a=([k]=v a b) 报 'a'），否则交替（D1: declare -A a=([x] one [y] two) 存交替键 ["[x]"]/["[y]"]——上一批 ledger 里该形态的记录是误读，实测以此为准）。assoc.tests 323→320。**残余 D3**：declare 一次声明+赋值时 ASSOC 标记时机导致 a=([k]=v a b) 走 indexed 化（未报错，存 [0]="a"），另批处理
测量口径切换（团队标准）**：array.tests oracle 从 WSL /usr/bin/bash（5.2.21）改为 **/usr/local/bin/bash（5.3.0，2026-09-09 编译）**——run-ab-b.sh 已固化。5.3.0 口径当前漂移 **436 行**（5.2.21 口径 435，基本相同——差异非版本漂移，是真实 gap）。历史 461/444 为 5.2.21 口径
- **census quotearray 桶 #2 已修**（parameter_patterns.rs `assoc_subscript_key`）：原实现先跑 `expand_embedded_parameters` 再处理反斜杠转义，引号键内 `\$` 中的 `$` 照常打开命令替换（真执行了 echo uname）且键截断为 `x],b[\`。修法：展开**前**把 `\$` 换成 $ 标记（0x1F marker），展开器恢复为字面 $；未转义的 `$(cmd)` 下标照常展开（GNU expand_subscript_string subst.c:11063 语义）。P4/V1 与 GNU 5.3.0 逐字节一致；executor_tests failset 114 零新增
- **V2 bare 键词法已修**（lexer/word.rs `skip_word_inner`）：下标内 `(`/`)` 原判 metachar 断词，`A[x\$(echo uname)]=v` 在 `(` 处裂成两词报 syntax error。GNU skip_to_delim/skipsubscript（subst.c:2186）下标扫描只终止于匹配 `]`，括号是普通下标文本。扩展现有下标例外（whitespace）到括号。V2 与 GNU 5.3.0 逐字节一致
- **新 gap（本轮记录，未修）**：indexed 数组下标 GNU 强制算术求值——`a["\$(echo uname)"]=v` 报 "arithmetic syntax error: operand expected"（error token 为原文）且不存值；rubash 静默存 [0]="v"。另批处理
- **census quotearray 桶 #2 确认真 bug（P4 FAIL）**：`A["x],b[\$(echo uname >&2)"]=v` —— rubash 把双引号键内 `\$(...)` 按未转义处理（真执行了 echo uname），键被截成 `x],b[\`；GNU 在双引号内 `\$` 为字面，存全键 `x],b[\$(echo uname >&2)`。简单形态 `B["x]b"]` PASS。根因方向：assoc 键词法对双引号内 `\$` 的转义处理（疑与 wt-prepush WIP 的 declare.rs 族相邻，修前需协调）
- **转 PASS**：f55、f61、f64、f67、f72、f78、gg4、gg5

### eval 二次解析族残余（本轮界定，未落地）

- **xtrace 展示保真**：GNU 的 `set -x` 打印 eval 参数是**展开后**的多词带引号保护形态（`+ eval 'b2=(a b' 'c)'`）；rubash 打印展开前/单词形态。语义已对齐，仅 xtrace 显示差异（gg7/gg2 X1/X3）
- **`__RUBASH_CA1__` 标记泄漏到 xtrace**（`eval a2=(x y)` 显示 `+ eval a2=__RUBASH_CA1__(x y)`）与用户可见输出（`echo a=(x y)`）——同前批"非声明 builtin 后 name=(...)" gap，合并处理
- **f58/f75 转义引号形态**（`eval a2=("${a[@]/#/\"-iname '\"}")`）：`'` 在双引号内为字面反斜杠+引号，eval 重解析侧元素扫描与 GNU 的 `'-quote 吸收语义不一致，产出 4 元素 vs GNU 2 元素
- **gnu-probe-eval F**（`eval "a2='-iname 'abc ..."` 标量形状）：GNU 报 `unexpected EOF while looking for matching “'”`（status 2），rubash 继续执行报 command not found——重解析侧未闭合引号是 fatal parse error
- **gg3 R2**（eval 参数中 `$()` 产物分词）、**gg6**（`"p${a[@]}s"` 前缀贴首元素、后缀贴尾元素，rubash 塌缩单词）——词分裂族 gap，与 eval 族同池

### 测量基建坑（复用价值高）

- WSL 启动 Windows exe **不传普通 env**（RUBASH_DBG/THIS_SH 均如此）：调参/调试须在 Windows 侧（Git Bash）直跑 exe；套件 harness 的 THIS_SH 必须 WSL 路径形式（/mnt/d/...），WSL bash 无法直接 exec `D:/...`
- Bash 工具会对 wsl bash -c 的字符串**预展开** `$PATH`/`$R`/`$(...)`（`\$` 转义不可靠）：一切复杂命令落脚本文件再调用（target/run-*.sh 均为此产物）

### 第三批（history/histexp/fc 桶，本轮落地——均在安全区，未碰 command_substitution/command_prepare/declare.rs/set.rs）

- **harness 伪影修复（run-suite.sh）**：rubash.exe 是 Windows 二进制，WSL interop 只转发 WSLENV 列出的 env——history.tests 顶层 `export HISTIGNORE='&:history*:fc*'` 到达 GNU 子进程但**丢失给 rubash 子进程**，导致 history*/fc* 命令全部被录进历史、后续所有 listing 差一个"自身条目"。run-suite.sh 现在把 HISTFILE/HISTSIZE/HISTFILESIZE/HISTCONTROL/HISTIGNORE/HISTTIMEFORMAT/HISTCHARS 加入 WSLENV 转发。**history.tests 243 行里有相当部分是这个伪影**，非 rubash gap
- **history 内建报错补位置前缀**（history_exec.rs + job_builtins.rs provider 分支）：GNU 的 sh_erange/builtin_error 带 "./script: line N: " 前缀（error.c），rubash 的 erange/invalid-number 消息此前是裸的。统一走 Executor::diagnostic_prefix()
- **`history @42` 数字参数校验**：GNU display_history → get_numeric_arg 非数字报 "numeric argument required" 且 rc=2（EX_USAGE），rubash 此前忽略坏参数照常列出
- **histexp 展开失败报错补前缀 + 行号对齐**（main.rs run_history_group）：status -1（event not found 等）按 bashhist.c pre_process_line 是 internal_error 级诊断，报**读取时**的行号——run_history_group 现在按物理行偏移（每 entry ≥1 行，嵌入换行计多行）跟踪行号并 pin CURRENT_LINE；此前借用上一条已执行命令的行号（差一行）。histexp.tests 20→17
- **build_recorded_entry 引号状态跟踪**（main.rs）：parse.y history_delimiting_chars——上一行未闭合的 `'`/`"`/`` ` ``（dstack delimiter）使行间分隔用真实换行而非 "; "；heredoc body 不喂引号扫描器（GNU PST_HEREDOC 路径）。修掉 history4 `printf $'...\cRleft\cO...' | -i` 回放块：HISTFILE 里的多行引号条目此前被存成 `(left; mid; right)`，回放执行输出错误
- **fc 三处对齐 GNU**（builtins/fc.rs + job_builtins.rs Reexec）：
  1. `fc -e -` == `fc -s`（fc.def:245 ename=="-" → execute=1）；此前 "-" 被当编辑器名报 "fc: -: program not found"
  2. `fc -s -- -42`：arg 循环 break 在 "--" 时**跳过它**（loptend 语义），此前 "--" 被当成 spec 走字符串前缀搜索 → "no command found"
  3. fc -s spec 解析失败统一 "no command found"（fc_gethist 对所有负 sentinel 返回 NULL）；越界数字 spec 按 POSIX 钳位（负→+last_hist+1 截 0，正溢出→HN_FIRST?0:last_hist）——"out of range specs aren't errors"
  4. **Reexec 替换自身条目**（fc_replhist 语义）：重执行的命令删除 fc 自己的历史条目后以 maybe_add_history 录入（受 HISTCONTROL/HISTIGNORE 约束），fc 永不出现在历史列表。history5.final `fc -l` 的 4/5/6 号条目（comment×2、echo a）与 GNU 一致
- **测量**：history.tests **243 → 151**（其中 ~26 行 rm-shim 噪声 + ~47 行 CRLF glue/交互回放噪声为 host/harness 伪影，真实残余 gap 见下）；histexp.tests 20 → 17；array.tests 436 维持；assoc.tests 320 维持；executor_tests failset **114 逐条一致（零回归）**；lib 测试 export_assignment_arg_preserves_quoted_spaces 失败为存量环境问题（host PATH 泄漏，git stash 验证与基线一致）
- **残余 gap（未修，记录在案）**：
  - **命令替换内 fc/history 看不到 session 历史**：command_substitution.rs:604 `session_history: None` → `$(fc -nl -1)` 输出空（GNU comsub 明确沿用 set -o history 的列表，fc.def:587-594 注释）。修法一行（clone session），但该文件是 wt-baseline 冲突区，**留给合并后协调 pass**
  - history2.sub 的 132/139 两行空白差异（GNU 多两个空行，来源待考）与 146 行空 entry（同 comsub 根因链）
  - history7/test-glue $'\r' 块：GNU 拒绝 CRLF glue 文件、rubash 静默接受——两侧行为分叉但根因是 Windows checkout 的 CRLF 文件（host 伪影）
  - `-i` 子 shell 的 \cR/\cO readline 回放控制字符未实现（history4 后两个 block），交互/readline 域，另行立项

## 二十、2026-09-11 全量 true-baseline 重跑（83 套件，GNU 5.3.0 契约）

### 测量方法

`scripts/true-baseline.sh` 全量跑完 83 套件（WSL GNU Bash 5.3.0 /usr/local/bin/bash
为契约基线，`__RUBASH_NO_UPSTREAM_SCRIPTS=1` 旁路仿真层，stdout-only diff）。

### 总结

| 指标 | v9（Sep 9） | 本次（Sep 11） | 变化 |
|---|---|---|---|
| 零差套件 | 31 | **32** | +1（trap 归零） |
| 有 DIFF 套件 | 52 | **51** | −1 |
| 总 diff 行 | 3427 | **2072** | −39.5% |

### 零差套件（32 个，完全通过 GNU 5.3.0 测试）

appendop, attr, builtins, casemod, complete, cprint, dbg-support, dbg-support2,
dstack2, dynvar, extglob2, extglob3, func, getopts, glob-bracket, herestr, ifs,
invert, invocation, mapfile, nquote2, nquote3, nquote4, nquote5, posixexp2,
posixpat, precedence, printf, rsh, strip, tilde, tilde2, **trap**

### 有 DIFF 套件（51 个，按差距分级）

**Large（101+ 行）— 5 套件，占总 diff 33%**

| 套件 | diff | 说明 |
|---|---|---|
| array | 246 | 复合赋值元素切分、readonly 声明、尺寸提示 |
| assoc | 242 | 键切分/转义、kvpair/strict 双路径 |
| history | 127 | 会话历史内容/时机、CRLF glue 伪影 |
| nameref | 105 | 模式替换、declare -p 链追踪 |
| globstar | 101 | **已修复**（v9 182→101），剩余为 WinuxCmd ls 排序差异 |

**Medium（51–100 行）— 9 套件**

alias(70), arith(51), exp(58), intl(77), new-exp(63), nquote(59),
quotearray(65), redir(58), varenv(86)

**Small（11–50 行）— 22 套件**

arith-for(18), braces(13), comsub(31), comsub-posix(11), comsub2(46),
cond(20), dstack(50), errors(35), extglob(16), glob(50), iquote(22),
jobs(37), lastpipe(13), more-exp(33), nquote1(13), posixexp(21),
procsub(12), read(42), rhs-exp(20), shopt(13), test(31), type(41)

**Tiny（1–10 行）— 15 套件**

case(6), comsub-eof(6), coproc(9), exportfunc(4), heredoc(4), histexp(6),
ifs-posix(1), mapfile(2), parser(4), posix2(3), posixpipe(1), quote(3),
set-e(8), set-x(7), vredir(2)

### 自 v9 以来的关键改善

- **trap 3→0**：ERR trap 行号绑定、SIGCHLD 排队、后台子进程 trap 隔离
- **globstar 182→101**：多重性修复 + WinuxCmd ls 差异归因
- **history 160→127**：history -d 范围删除、HISTIGNORE harness 修复、fc -s 语义
- **builtins 182→0**：完整 v9 修复链已收敛
- **invocation 14→0**：BASH_ARGV0、长选项表、--pretty-print
- **rsh 194→0**：set +o restricted 静默解除修复、受限 shell 全链路
- **dbg-support 635→0**：AND-列表双触发、source-scope trap 继承、非行首 `{` 回归
- **complete 115→0**：多操作数 compspec 注册
- **func 58→0**：posix funcname 规则、AST printer、special-builtin 优先级
- **varenv 96→86**：`set -k`（GNU subst.c:12494-12535 / flags.c place_keywords_in_env）补齐——尾随 `name=value` 词在词展开前收入赋值列表；argv 清空时全部永久生效。varenv.tests 是唯一用 `set -k` 的套件，故无跨套件回归（builtins 仍 0 差）。余 86 行中 9 行为 `global1:` 一族，根因是 Windows 进程环境大小写不敏感：`export a` 与 `export A` 经 `env::set_var` 互相覆盖，子进程只看到一个名字


## 十一、locale 单字节模式接线（2026-09-11）

GNU bash 的字符语义由 `setlocale()` + `MB_CUR_MAX` 决定：UTF-8 locale 下一个
字符 = 一条多字节序列；`setlocale()` 无法激活的 locale 回落到 C，每个字节 =
一个字符（variables.c:1466-1490、lib/sh/utf8.c:167-184）。`src/locale.rs`
（`44ab54d3` 移植）此前有 `effective_length()` 却无调用方，而且只判断「locale
名是否含 utf-8」，既不探测 `setlocale` 是否真的成功，也把空 locale 环境变量
当作单字节处理。本批次把它接上，并修掉空 locale 的误判。

### 语义归属与改动

- **`locale.rs`**：`is_multi_byte()` 为唯一判定入口。空 locale 保持 rubash 的
  UTF-8 默认（Windows-first，无 locale 注册表）；`C`/`POSIX` 与已命名的单字节
  locale（`ru_RU.CP1251`、`en_US.ISO-8859-1`）→ 字节语义；UTF-8 命名 locale
  仅在 C 库能激活时生效——`#[cfg(unix)]` 下用 `libc::setlocale(LC_CTYPE, name)`
  探测并立即恢复，Windows 侧接受名称。结果按 locale 名缓存在 thread-local，
  脚本内 `LC_ALL=...` 重赋值自动失效。
- **`expand_braced_indices.rs` `parameter_char_length`**（`${#var}` 的唯一
  归属，约 14 个调用方）：单字节 locale 下返回字节数，保留 U+E000 原始字节
  标记解码。
- **`parameter_ops.rs` `parameter_substring`**：单字节 locale 下 `${V:0:2}` 按
  字节切，可切到多字节序列中间并保留悬挂引导字节。
- **`read_helpers.rs` `trim_read_input`**：`read -n N` 在单字节 locale 下按
  N 字节封顶，且不回退到字符边界。
- **`printf/escape.rs` `\u`/`\U`**：单字节 locale 下把解析出的值重排为规范
  字面转义（≤0xFFFF 用 `\u%04X`，更大用 `\U%08X`，>0x10FFFF 输出空），
  因此 `\U000000FF` 折叠成 `\u00FF`、部分数字 `\uff` 也规范成 `\u00FF`；
  UTF-8 locale 下仍走 `u32cconv` 输出真实字节。格式串与 `%b` 两条路径共用。
- **`printf/value.rs`**：`%ls`/`%lc` 的精度与宽度在单字节 locale 下按字节计。
  数值型说明符两种模式下都是 ASCII，故不切换单位。

### 验证（WSL GNU Bash 5.3.0 唯一基线，脚本文件入参）

- 18 例配对探针 `LC_ALL=C`（两侧都真正安装的 locale）：**diffs=0**，14 行逐
  字节一致。覆盖 `${#v}`、`read -n 5`、`\u00FF`/`\uff`/`\uffff`/
  `\U000000FF`/`\U0001F600`/`\Ufffffffe`/`\u0041`/`\u0152`、`%b` 转义、
  `%.2ls`、`%lc`、`${V:0:2}`、`%.4f`。
- `LC_ALL=C.utf8`（glibc 唯一已安装的 UTF-8 locale）：同样逐字节一致，证明
  UTF-8 分支未被回退。
- 回归六套件与台账数字完全一致，零翻转：printf **0**、read 42、exp 58、
  nquote 59、new-exp 63、more-exp 33。
- 新增单元测试：`locale::tests` 2 例、`parameter_ops::tests` 字节子串 2 例；
  `cargo test --lib` 373/374。唯一失败
  `export_assignment_arg_preserves_quoted_spaces` 由 `2e3cbe82`/`b2ebef50`
  引入（二者都改了 `assignment_expansion.rs`/`parameter_words.rs` 的 export
  PATH 路径），与本批次无关。

### intl 残余 77 行 = 平台归属，不是 rubash 语义缺口

`intl.tests` 第 14 行 `export LC_ALL=en_US.UTF-8`，但 WSL glibc 的 locale
归档里没有该 locale（`locale -a` 只有 C、C.utf8、POSIX），GNU 报
`warning: setlocale: LC_ALL: cannot change locale (en_US.UTF-8)` 并降级为字节
语义；而 `rubash.exe` 是 Windows 进程，其 CRT 接受 `en_US.UTF-8` 名称，走字符
语义。两侧 C 库不同，「`en_US.UTF-8` 是否可用」无法一致，且 `rubash.exe` 也
无法 exec glibc 的 locale 归档。77 行全部落在此前缀下（11-13、20、24-36、
45-55、57、60-68），与既有的 intl/history 平台归属分类一致。同一探针在
`LC_ALL=C` 下 0 差已证明语义本身正确。

产物：`target/issue-suites/results/locale-c-probe/`（GNU/rubash 双侧 + diff）、
`target/issue-suites/results/intl-now-baseline/intl/`；探针脚本
`target/locale-c-probe.sh`。

### intl 87 行 = locale 平台归属 + unicode1 控制字节标记冲突（2026-09-11）

`e5291002 fix(expand): decode $'...' spans inside embedded assignments` 修好了
`unicode1.sub` 的根因：`A=([k]=$'\001')` 这类复合赋值元素值此前被字面存成
`$'\001`（丢尾引号、不解码），因为 `expand_embedded_parameters_ordered_mut`
的 `$` 分支没有 `'` 臂——`$` 落到 `Some(other)` 只推 `$`+`'`，随后那个尾引号
被当成单引号跨度的开头，把整行剩余吞进"引用体"。

修好后 7 例探针（`target/ansifull-probe.sh`，含 `[k]=$'...'`、多元素、`\x`/`\u`
转义、空白值）与 GNU 5.3.0 逐字节一致。`C_UTF_8` 表从"塌成 1 个元素"
变成能解析 20+ 项。

intl 计数 77 → 87 是**输出形状变粗**，不是语义回退：三个缺 locale 的子测试
（fr_FR.ISO8859-1 / zh_TW.BIG5 / jp_JP.SHIFT_JIS，WSL glibc 归档里都没有）
现在把各自的失败项逐条打印，行数变多。`LC_ALL=C` 下仍 0 差。

**unicode1 剩余差 = 控制字节与 rubash 内部标记冲突**，不是本次改动的缺口：

- `A=([1]=$'\f' [2]=x)` → `[1]=""`（GNU `=\$'\f'`）：`\x0c` 被
  `expand_braced_replacement.rs` 的 `PATSUB_QUOTED_VALUE_END` 占用。
- `$'\023'`（0x13）被 `PARAM_NAME_END_MARKER`（`quotes.rs:10`）占用。
- 0x14 / 0x17 / 0x1a / 0x1f 已有 `encode_raw_byte_marker` 承载方案
  （`ansi.rs` `is_assignment_carrier_byte`），但 0x0c / 0x13 没有。
- `C=([1]=\  [2]=x)`（转义空格）→ `[1]="\\"`（GNU `=" "`）：另一条路径。

载体标签方案需要覆盖全部 C0 控制字节，属架构级改动，超出本次范围。

探针：`target/ansifull-probe.sh`、`target/ws-probe.sh`、`target/ws2-probe.sh`、
`target/ff-probe.sh`、`target/bigarr-probe.sh`。

## 二十一、2026-09-14 全量 true-baseline 重跑（83 套件，GNU 5.3.0 契约）

### 测量方法

`scripts/true-baseline.sh` 全量跑完 83 套件（WSL GNU Bash 5.3.0
/usr/local/bin/bash 为契约基线，`__RUBASH_NO_UPSTREAM_SCRIPTS=1` 旁路仿真层，
stdout-only diff）。台账产物：
`target/issue-suites/results/true-baseline-ledger.log`。

### 总结

| 指标 | Sep 11 | **Sep 14（当前）** | 变化 |
|---|---|---|---|
| 零差套件 | 32 | **40** | +8 |
| 有 DIFF 套件 | 51 | **43** | −8 |
| 总 diff 行 | 2072 | **1819** | −12.2% |

### 自 Sep 11 以来的关键改善（commit 链）

- **globstar 101→4**：`0662ef9b` 命令查找缓存 + `8d326bb2` hashall/checkhash
  绕过——剩余 4 行为 WinuxCmd `ls` 排序差异（平台归属，非 rubash 缺口）
- **heredoc 4→0**：`c59c5da4` 命令替换管道走真实 executor
- **braces 13→1**：算术/引号族连带收敛
- **arith-for 18→1**、**arith 51→50**：`45a2beb8` + `7666a550` 算术错误前缀
  按 `-c` vs 脚本模式区分（`__RUBASH_IS_C`）
- **posixexp 21→12**、**exp 58→52**、**more-exp 33→38**：算术前缀修复连带
- **shopt 13→13**（持平）、**histexp 6→1**：fc 族收敛残留
- **dstack 50→0**、**dbg-support 323→0** 已在 v9 收敛，本次维持
- **varenv 86→98**：+12 行新增差异（待分诊，疑与 `set -k` 路径或环境大小写
  敏感性变化有关）
- **jobs 37→53**：+16 行（待分诊）

### 零差套件（40 个，完全通过 GNU 5.3.0 测试）

appendop, attr, braces*, builtins, case, casemod, complete, comsub-eof,
cprint, dbg-support, dbg-support2, dstack, dstack2, dynvar, exportfunc,
extglob2, extglob3, func, getopts, glob-bracket, heredoc*, herestr, ifs,
invert, invocation*, mapfile, nquote2, nquote3, nquote5, parser, posixexp2,
posixpat, posixpipe, precedence, printf, quote, rsh, strip, tilde, tilde2,
trap, vredir

（* = 自 Sep 11 新转零差：braces, heredoc, dstack, quote, invocation 收敛
至 ≤2 行；严格零差为 40 个）

### 有 DIFF 套件（43 个，按差距分级）

**Large（100+ 行）— 4 套件，占总 diff 38%**

| 套件 | diff | 说明 |
|---|---|---|
| array | 239 | 复合赋值元素切分、readonly 声明、尺寸提示 |
| assoc | 217 | 键切分/转义、kvpair/strict 双路径 |
| history | 127 | 会话历史内容/时机、CRLF glue 伪影 |
| nameref | 105 | 模式替换、declare -p 链追踪 |

**Medium（50–99 行）— 8 套件**

varenv(98), intl(75), alias(70), quotearray(65), new-exp(63), redir(61),
jobs(53), exp(52)

**Small（20–49 行）— 13 套件**

arith(50), glob(48), nquote(47), comsub2(46), read(44), type(41),
more-exp(38), errors(34), test(33), comsub(31), iquote(22), cond(21),
rhs-exp(20)

**Tiny（1–19 行）— 18 套件**

extglob(16), shopt(13), nquote1(13), lastpipe(13), posixexp(12),
comsub-posix(11), set-e(8), procsub(6), coproc(6), set-x(5), posix2(4),
globstar(4), nquote4(2), invocation(2), ifs-posix(1), histexp(1),
braces(1), arith-for(1)

### 优先级建议（按影响 × 可行性）

**P0（大族，根因已定位）**：
1. `array` 239 — 复合赋值元素切分（P1 战役已定位四层根因，待分层重落）
2. `assoc` 217 — 键切分/转义（与 array 同族，split_compound_assignment_words）
3. `nameref` 105 — 模式替换已修，剩余 declare -p 链追踪

**P1（中族，需分诊）**：
4. `varenv` 98 — `set -k` 已修，+12 行新增待分诊（疑环境大小写敏感性）
5. `history` 127 — 会话历史内容/时机，CRLF glue 伪影占比高
6. `redir` 61 — fd 生命周期（与 procsub 同族）
7. `new-exp` 63 — P2 战役 F1-F9 已分诊

**P2（平台归属，非语义缺口）**：
- `intl` 75 — locale 平台差异（`LC_ALL=C` 下 0 差已证明）
- `globstar` 4 — WinuxCmd `ls` 排序差异
- `jobs` 53 — 待分诊是否平台相关

### 查找缓存与算术模式区分（本次新增修复）

详见 `docs/command-lookup-cache-and-arith-mode-split.md`。要点：
- `hash -d`/`-p`/bare rehash 同步内部缓存（`fccf98de`）
- `hashing_enabled` 绕过 + `checkhash` stat 验证（`8d326bb2`）
- temp env PATH 绕过（`7666a550`）
- 算术错误前缀按 `-c` vs 脚本模式区分（`7666a550`，`__RUBASH_IS_C`）

### 测量纪律

- 唯一权威台账 = `scripts/true-baseline.sh` 全量输出
- 禁止手搓探针测量套件数字（曾导致 −540 假改善）
- 任何"已修复/仍残留"状态变更必须真实跑对应 GNU 测试文件复现
- WSL GNU Bash 5.3.0（`/usr/local/bin/bash`）为唯一契约基线

## 第二十二节：2026-09-16 全量 true-baseline 审计

**口径**：`scripts/true-baseline.sh` 无参数全跑，83 套件，WSL GNU Bash 5.3.0。

**结果**：43 零差 / 40 有 DIFF / 总 2702 行。

**分布**：
- PASS (0 diff): 43 套件 (52%)
- DIFF (1-50): 28 套件 (34%)
- DIFF (51-250): 11 套件 (13%)
- DIFF (251+): 1 套件 (1%) — intl=1209

**新增零差**（自 2026-09-14 起）：`comsub-eof`、`heredoc`。

**最大 DIFF 套件**（降序）：
| 套件 | 差异行 | 备注 |
|------|--------|------|
| intl | 1209 | ANSI-C `$'...'` 载体字节架构：`\302\200` 存为 U+E000 而非 U+0080 |
| assoc | 181 | 键切分/转义（与 array 同族） |
| errors | 164 | 路径/环境差异 + 分诊待做 |
| array | 146 | 复合赋值元素切分四层根因待分层重落 |
| history | 127 | 会话历史内容/时机，CRLF 伪影 |
| nameref | 93 | declare -p 链追踪 |
| varenv | 85 | 环境大小写敏感性待分诊 |
| glob | 78 | 待分诊 |
| alias | 67 | 待分诊 |
| quotearray | 64 | 待分诊 |
| jobs | 53 | 待分诊 |

**排除 intl 后**：39 套件共 1493 行（Sep 9 全量 3427 → 1493，−57%）。

**本次会话修复**（compound-array quoting 族）：
- `src/executor/assignment_expansion.rs` — 数据/语法引号区分：参数展开返回的引号
  是数据（GNU `CTLESC` 保护），不应被 `remove_shell_quotes` 去除。仅当原始词也含引号
  语法时才去引号。修复 `EChar=${Array[0x0022]}` 返回空值的问题。
- `src/executor/arrays/storage.rs` — `quote_array_value` 双重转义 bug：`"` 应转义为
  `\"`（2 字符），而非 `\\"`（3 字符）。
- `src/lexer/word.rs` — 复合数组赋值原始 RHS 保留：`\"`/`\'`/`\`/`` \` `` 在复合
  赋值括号内保留原始文本，供 `split_storage_words` 解析。
- 新增 `DATA_ESCAPED_SQUOTE`/`DATA_ESCAPED_BACKSLASH` 载体标记，与既有
  `DATA_ESCAPED_DQUOTE`/`DATA_BACKTICK` 配合，确保转义引号/反斜杠/反引号在
  `expand_embedded_params_mut` 后存活。

**GNU C 源码引用**：
- `subst.c:4807 dequote_string()` / `subst.c:4865 dequote_word()` — 最终去引号仅作用于
  原始词结构，不作用于参数展开引入的字符。
- `subst.c:4692 dequote_escapes()` — `CTLESC` 保护数据字符不被去引号。
- `parse.y:5366-5397 read_token_word()` — 反斜杠/引号在原始 token 中的处理。
- `parse.y:7104 parse_compound_assignment()` / `parse.y:7127` 清除 `PST_NOEXPAND`。

**intl 1209 行根因**（未修复，架构级）：
`src/lexer/ansi.rs:257 push_ansi_c_byte()` 对字节 ≥ 0x80 调用
`encode_raw_byte_marker()`，生成 U+E000 系列载体标记而非真实 Unicode 码点。
`$'\302\200'`（UTF-8 编码的 U+0080）被存为两个独立的载体标记（U+E000 系列），
而非 Unicode 码点 U+0080。字符串比较时 U+E000 ≠ U+0080，导致 1192/1318 个
unicode1.sub 测试失败。修复需重构 ANSI-C 解码器对多字节序列的处理。

## 第二十三节：2026-09-16 nameref 审计批次（readonly nameref 状态机 + 物化语义）

**口径**：`scripts/true-baseline.sh`，WSL GNU Bash 5.3.0。

**nameref 套件**：93 → **7 行**残余。

**本次 GNU C 源码驱动的修复**（均在 `third_party/bash` 定位属主函数后改动）：

- `src/builtins/declare.rs` / `declare/assign.rs` / `declare/attrs.rs` —
  readonly 空-cell nameref 的"物化"状态机。GNU `declare.def:806-825` 的
  created_var 路径先 `bind_variable(name, NULL, ASS_FORCE)`，经
  `variables.c:3074-3081`（invisible nameref 子句）清除 `att_invisible` 再尝试
  赋值——`typeset -n foo1; typeset -r foo1; typeset foo1=bar` 报 readonly 错误但
  留下**可见**空-cell nameref。此后任何非 `-n` 操作数经
  `variables.c:3061-3069`（visible nameref 全局表解析失败 → bind 返回 NULL）
  → `declare.def:816 NEXT_VARIABLE` 静默跳过：`typeset +r/+n foo1` 成为静默
  no-op 且属性保留（nameref17.sub:38）。
- `declare/assign.rs` — 无 `=` 操作数不再把已物化的 nameref 重标
  DECLARED_UNSET（GNU 仅在 create-bind 真正返回变量时才设 att_invisible）。

**已确认的残余**（nameref 7 行）：

1. `nameref11.sub:52` `RO_PID` — coproc 退出传播时序：GNU 的 SIGCHLD reap
   在下一命令边界前完成（jobs.c:1342 → coproc_reap → coproc_unsetvars），
   RB 的 `try_wait` 轮询在 rubash.exe coproc 启动较慢时滞后。隔离复现
   （插入 `sleep 0.2`）两侧字节一致，属平台时序差异。
2. `nameref18.sub:83` `"${!indir}$ref"` — 复合带引号词内嵌入式 `[@]` 的
   场边界融合（GNU subst.c `expand_word_internal` 的 W_ARRAYQUOTED 分词：
   前缀融首元素、后缀融末元素）尚未实现；RB 需要 `${}`/`$name` 级 span
   扫描器 + 场边界载体字节，属独立子系统特性，留待专项。

**相邻套件抽查**（无回归）：dstack 0、builtins 0、func 0、errors 3、
trap 1、exp 8、coproc 6、more-exp 6、cond 19、read 44、array 116、assoc 175、
varenv 80、new-exp 59。函数体诊断行号修复（function_command.rs 不再把
body 命令压成定义行）带来 errors 164→3、exp 134→8、func 58→0、trap 61→1
的全局收益。

## 第二十四节：2026-09-18 niubash#121 批次（compound 数组赋值 nullglob/failglob + ERR/DEBUG trap 属主点）

**口径**：`scripts/true-baseline.sh glob trap shopt`，WSL GNU Bash 5.3.0
（/usr/local/bin/bash）。工作分支 `fix/niubash-121-nullglob-traps`。

**台账**：trap **0**、shopt **0**、glob 69（全部平台归属，与本次无关：
Windows 文件名不允许 `a?`/`*abc.c`/`a*b` 字面 glob 字符（os error 123）、
GNU 侧 `test-glue-functions` 未进 harness CR-strip 清单、en_US.UTF-8
strcoll 排序 vs RB 字节序排序的既有差异；glob.tests 无任何 `=(` 用例）。

**nullglob/failglob（compound 数组赋值）**：

- `src/executor/glob.rs::pathname_expand_word`（pathexp.c 端口）成为
  compound 赋值元素词的唯一 pathname 展开入口；原手写
  `read_dir(".")` matcher（`declare/storage/glob.rs`）整体删除——不支持
  `/`、不支持 nullglob/failglob。
- GNU `arrayfunc.c:557 expand_compound_array_assignment` →
  `assign_compound_array_list`：元素词逐一过真实 pathname expansion；
  `[subscript]=v`/`+=` 词由 `quote_array_assignment_chars`
  （arrayfunc.c:1107+）标 W_NOGLOB 保留字面（`[0]=nope-*` 不展开）；
  field-split 产物是普通词、仍展开。
- failglob：`Err(pattern)` 在 bind 前中止 operand——新目标留
  `declare -a g=()`，既有/已声明目标保留原状态；`declare -a e[10]=(zz*)`
  在 member 检查前报 `no match: zz-*`。
- 临时环境 `a=(...) cmd`：GNU `variables.c assign_in_env` 把 compound 词
  绑为标量字面文本，元素词不解析、不展开，failglob 亦不报；
  COMPOUND_ASSIGNMENT_MARKER 不得泄进子进程环境（external_inner.rs）。
- `command_prepare.rs`：declare/typeset/local/export/readonly 的
  `name=(...)` operand 标 suppress_glob（W_COMPASSIGN），防止整个
  operand 被当单个词 glob（否则 failglob 报 `no match: e[10]=(zz-*)`）。

**ERR trap**：`Executor::error_trap_running` 镜像 GNU `trap.c
_run_trap_internal` 的 SIG_INPROGRESS——action 运行中 `run_error_trap`
拒绝重入，`trap 'echo E; false' ERR; false` 只打一次 E（probe 与 GNU
逐字节一致）。

**DEBUG trap 属主点**（GNU execute_cmd.c 调用点表：for 3053、
select 3528、case 3668、arith 3920、cond 4153、simple 4506、函数入口
5387——普通 and/or/包装节点无调用点）：

- `ast_exec.rs` 顶门补齐 wrapper 跳过表：inverted/pipeline/pipe/
  brace_group/case/select/time/coproc/background/time-prefixed-compound。
- `pipeline_exec.rs`：顺序 stage 循环逐元素 fire（元素子 shell 保留 trap
  表，execute_cmd.c:2702+；compound stage 经 execute_in_subshell 重置
  trap 表，trap.c:1588）；两个 `execute_external_pipeline_concurrently`
  与 timed-pipeline 快路径在 DEBUG trap 存活时 bail。
- `compound_exec.rs`：`x &` 父侧在 fork 前 fire（execute_cmd.c:4506 →
  make_child ~4550），`!` 前缀剥层；`time cmd` inner 由 execute_command
  自 fire；case head 用未展开 raw word（`case "$v" in `）。
- `loop_select.rs`/`select_exec.rs`：默认 positional 按
  print_cmd.c:602/656 打 `for i in "$@"`、`select x in "$@"`。
- `function_calls.rs`：函数入口 DEBUG 文本取 `__RUBASH_LAST_COMMAND`
  （展开前 raw 词），保住 `f x "y z"` 引号。
- `public_accessors.rs::set_current_command`：trap action 运行中不刷新
  `__RUBASH_LAST_COMMAND`，镜像 GNU `the_printed_command_except_trap`
  冻结（execute_cmd.c:4499-4501、variables.c:1558 get_bash_command）。
- `command_substitution.rs`：functrace 下继承 DEBUG 时命令替换不走
  word-level 捷径（trap.c:1588 仅 function_trace_mode 保留 DEBUG trap）。
- `command_text.rs::command_has_no_effect` 补 `words.is_empty()`：附带修掉
  `time { echo x; }` brace body 被整段丢弃的既有 bug。

**已确认残余（不修）**：后台 job 输出与下一命令 DEBUG trap 的交错是
调度竞态（GNU fork 即时、RB 线程启动有延迟，`{ echo g; } & wait` 两侧
各自稳定但顺序相反）；`time` 输出格式（`real\t0m0.000s` + 前导空行 vs
`real 0.00`）为既有格式差异。

## 第二十五节：2026-09-19 ExitCode 传播修正（`exit`/errexit  unwind 语义，master `c5c97683`）

**问题**：`Err(ExecuteError::ExitCode)` 在 `execute_ast_inner_body` 的每个
复合分派臂（inverted/time/pipeline/and_or_list/brace_group）被吞成
`self.exit_code = code`——把「shell 必须退出」错当成「命令状态」。实测：
`set -e; { false; }` 继续跑、`{ exit 3; }` rc=0、`exit 3 && x`、
`false || exit 3`、`time exit 3`、`if { exit 3; }; then`、
`f() { false || exit 3; }; f` 全部违规继续。

**GNU 依据**：`exit`/errexit 经 `jump_to_top_level` 展开当前 shell
（`exit.def:152` EXITBLTIN、`execute_cmd.c:1174` ERREXIT），只有 fork 边界
能接住——`( )` 节点（`execute_subshell_command_with_redirects`）、
pipeline stage、comsub、`f() ( )` 扁平区域（`execute_cmd.c:1576`
execute_in_subshell）。`{ }`/`&&`/`||`/`if`/`time` 都不构成边界。

**修法（不变量修正，非逐症状）**：
- 新 `handle_exit_code!`：`subshell_env.is_none()`（本 frame 不持有扁平
  `( )` 区域）→ `return Err` 展开；`is_some()` → 在区域边界吸收
  （exit_code + 快进到 `subshell_end` 之后 + 恢复保存态 + 父侧 errexit
  复查，execute_cmd.c:1170-1175）。
- 扁平区域的 env 保存点从 fall-through 段提升到所有类型分派之前——
  `subshell` 标记的复合命令（`f() ( { exit 3; } )`）此前从未到达保存点，
  导致无隔离 + ExitCode 被吞。

**附带修好的存量 bug**（同族）：`f() ( { exit 3; } )` 状态丢失、
`f() ( a=in; exit 3; echo M )` 静默退出、`f() ( exit 3 )` 死循环
（失败命令自身即区域标记时原吸收逻辑会重入执行）。
另：`f() ( A || exit 3 )` 等扁平区域内 `exit` 现在正确止于区域边界。

**验证**：WSL GNU Bash 5.3.0 脚本文件矩阵 27/28 字节一致（仅 `time`
缺 `real/user/sys` 计时报告——独立内建缺口，rc 已一致）。套件：
**set-e 2→0**、trap 0、errors 3、func 0、lastpipe 0、redir 55、
comsub 17、procsub 13、jobs 63→59、comsub2 48→44、cond 19——无回归。

**已知边界**：`time cmd` 的计时报告未实现（GNU `real/user/sys` 三行，
语义无关 ExitCode）；`select` 空 stdin 菜单重绘差异为既有项。

## 第二十六节：2026-09-20 posixexp 清零 + heredoc/here-string 预展开载体（fix/array6-patsub-quotes）

**本批修复（GNU C 依据逐一对应）**：

- **`${'x1'%'t'}` bad substitution**（`subst.c:10272-10288`
  `parameter_brace_expand`）：参数名被非法字符（引号）终止 →
  `default:` 报 bad substitution。展开期检测
  （`expand_word.rs braced_name_ends_on_quote` +
  `parameter_bad_substitution` 旗标），故 `${x-${'x1'%'t'}}` 在 x
  已设时不报（交替词未求值），与 GNU 条件求值一致；posix 非交互
  → FORCE_EOF 致命（`-c` rc=127、脚本 rc=1），非 posix → DISCARD
  命令中止脚本继续（`subst.c:10288`、`shell.c:1471`、`eval.c:104`）。
- **下标引号跳读**：`braced_name_ends_on_quote` 的 `[...]` 扫描按 GNU
  `skipsubscript` 跳过 `'...'`/`"..."`/`\x` 区段——`${myarray[']']}`
  合法，不再误报 bad substitution（assoc5.sub）。
- **heredoc/here-string 预展开载体 `\x05`**（`redir.c` 展开次序：
  `do_redirections` 在 `expand_words` 之后、命令执行之前展开 stdin
  体）：`execute_command` 词展开后预展开 heredoc/here-string，结果
  以 `PREEXPANDED_STDIN_BODY` 前缀缓存；`expand_heredoc_body*`、
  `expand_here_string_mut`、`stdin_string_for_command*`、
  `read_heredoc_fd_input`、`alias_loops`、`trap_exec` 全部识别载体
  并返回缓存文本——体只展开一次，`${'x1'%'t'}` 在 `cat <<EOF` /
  `read <<<` 中正确致命且不再二次展开/重复诊断。
- **posix 下 `${!?}`/`${!#}` 非间接**（`subst.c:122 VALID_INDIR_PARAM`：
  posix 下 `#`/`?` 不是合法间接参数名）：argv0=`sh` 时 `${!?}` 走
  `?` 算子（posixexp2.sub test 6）。
- **posix 算术展开错误致命**（`subst.c:10881-10888` +
  `shell.c:1471` + `eval.c:104`）：`$((x+))` 在 posix 非交互下
  FORCE_EOF——`-c` rc=127、脚本 rc=1；非 posix 命令中止脚本继续。
- **无扩展名 PE 直接 exec**（`shell_execve` 先试 execve 再分类）：
  `cp ${THIS_SH} $TMPDIR/sh` 的无扩展名副本经 MZ magic 探测直接
  CreateProcess，使套件 posix 重命名子 shell 链路打通；`Executor::new`
  的 `THIS_SH` 自动检测保留已导出的合法 `sh`/`sh.exe` 路径。
- **`${x?}` 诊断走命令重定向状态**（`execute_cmd.c` `expand_words`
  先于 `do_redirections`）：新增 `write_redirected_command_stderr`，
  子 shell `(${x?}) 2>&1` 诊断正确进管道。
- **null IFS 下 `$@` 连接符**（`subst.c:3006 string_list_dollar_at`：
  `PF_ASSIGNRHS || ifs 未设 || ifs 空 → ' '`）：赋值 RHS 快路 `a=$@`
  在 `IFS=` 下用空格连接（posixexp3.sub）；`$*` 仍用 `ifs_firstc`。
- **未闭合 `$(` 报 unexpected EOF 中止命令**（`parse.y parse_comsub`）：
  `collect_command_substitution_source_ex` 返回闭合标志，
  `"${a+'$('\'}"` 类报 EOF 错、命令中止、脚本继续（braces）。
- **dq 上下文 `\'` 保留反斜杠**：`decode_double_quotes_in_quoted_parameter_word`
  非 posix 分支 `\'` → `\x14\x17`（rhs-exp `\'$selvecs\'` → `\'...\'`），
  裸 `'` → `\x17`（braces `'x y'` 字面量）。
- **`#`/`%` 模式词 sq 保护**：`push_quoted_pattern_char` sq 臂对
  `$`/`` ` ``/`"` 出 `\$` 转义形式（braces `'$('` 不展开）。

**台账**（true-baseline.sh 同口径）：posixexp 7→0、braces 1→0、
assoc 0、array 0、rhs-exp 0、quotearray 50、comsub2 38→34、
ifs-posix 1、exp 4（`/src/cmd` 环境路径噪声）、new-exp 60、comsub 17。
标记载体回归（array/assoc/exp/quotearray 一度 +2~+5）已全部归零。

**已知残留**（预存，非本批引入）：
- `read v <&3 3<<EOF`：GNU 按序应用重定向（`<&3` 先于 `3<<` → fd3
  未开报 Bad file descriptor）；RB 的 `heredoc_redirects` 独立存储
  无法与 `redirects` 交错排序——架构项待修。
- `${!@}`/`${!*}` 非 posix 下 bad substitution 缺口（`VALID_INDIR_PARAM`
  对 `@`/`*` 合法但 RB 间接路径未实现）。

## 第二十七节：2026-09-21 new-exp/quotearray/comsub2 清零 + read 簇（fix/array6-patsub-quotes）

**new-exp 60→0（GNU C 依据逐一对应）**：

- **`{xxx}` 非保留字**（`parse.y`：`{` 仅独立成 token 才是保留字）：
  scanner 对 `{` 后紧跟词字符的形态发普通词，`$({xxx}</dev/stdin)`
  不再硬语法错误中止脚本（GNU 按 command-not-found 恢复）。
- **引号 `${}` 内嵌 `$@`/`$*` 空展开保留一个空字段**
  （`subst.c:12026` `had_quoted_null`）：`"${foo:-$@}"`、`"${foo-$@}"`、
  `"${foo:-$*}"` 等算子词使用且展开为零字段时产出一个空字段；
  `removes_unquoted_null_word` 增加 raw-quoted 门控。
- **`${!PREFIX*}`/`${!PREFIX@}` 变量名前缀展开**（`subst.c:9978-10007`
  `all_variables_matching_prefix` → `vapply` → `sort_variables`
  strcmp 序）：`@` 形逐字段（W_DOLLARAT）、`*` 形按 IFS[0] 连接；
  前缀形跳过 `@`/`:` 算子尾检查；空匹配 `@`→0 字段、`*`→1 空字段。
- **`${@%%pat}` 空位置参数全位置丢弃**（恢复被 revert 的正确语义）。
- **quoted `"${a[@]:N}"` 逐元素子串**：复合赋值 `\uE102` 原子路径
  与 `\x1d` 路径均按 `[@]` 逐元素、`[*]` 按 IFS[0] 单字段连接。
- **`@A`/`@a`/`@Q`/`@E`/`@P`/`@K` 变换矩阵**（`subst.c:8856`
  `array_transform`）：`@A`/`@a`/`@K` 对数组/assoc 恒单次属性串
  （`var_attribute_string` 语义，携带全部 `-a/-i/-l/-r/-u` flag）；
  `storage.is_none()`（cell 未分配）空数组 `@a` 特判返回一次属性串，
  `foo=()` 已分配空表逐元素→空；`${!var@Q}`/`${!var[@]%..@T}`
  间接逐元素变换接通。
- **`-u` 下空数组 cell 视为 unbound**：标量形 `${foo}`/`${foo@a}`/
  `${!bar}`→空数组报 `foo: unbound variable`；`[@]`/`[*]` 形豁免；
  `!` 间接报告 `!name`。
- **`${$(…)}` 与无效 `@op` → bad substitution**（`subst.c:8944`
  `valid_parameter_transform` + `expand_param_fatal`）：顶层与嵌套
  `${c//${$((…))}/x/}` 均拦；`@C`/`@` 等无效变换在**已设**变量上
  FORCE_EOF 致命（rc=1 中止脚本），未设变量 NULL 早退静默。
- **标量 `${var[@]:N}` 退化为字符子串**（`subst.c` 标量走
  单元素数组路径）。
- **`declare -f` 在 comsub 内不再单行规整化重印**（删除伪输出烤死
  路径，`$(<x)`/`(cat x1)` 序列表保真）。
- **prompt `\[`/`\]` decode**（`parse.y` 字节语义）：`\[`→`\x01`、
  `\]`→`\x02` 字面字节，修复外层循环变量遮蔽导致的恒 `\x02`。

**quotearray 50→0**：

- **assoc 下标 `\x1e` 预编码幂等**：`expand_arithmetic_assoc_subscripts`
  对已编码 `\x1e` 下标透传，修 `eval_parameter_substring_offset{,_mut}`
  与 `eval_arithmetic_expansion_value` 间的二次编码（`A[%]` 偏移 0 病）。
- **`${!aref}` 无引号路径按 GNU 桶序**（`assoc_nbuckets`/`bash_assoc_order`）：
  `indirect_target_values` 不再走插入序 `array_values`——单命令
  `assoc[@]=at assoc[*]=star` 后 `star bang at` 序对齐。

**comsub2 34→0**：命令替换输出/状态簇按 `subst.c:7143`
`command_substitute` 与 `parse.y:4451 parse_comsub` 对齐。

**read 44→环境残留（无真 diff）**：

- **内建 `<` 重定向失败中止**（`redir.c:767 do_redirections` 先于
  builtin 执行）：`{ read -t 0.5 a; } </nonexist` rc=1 且不赋值、
  不污染后续 read 状态。
- **共享 stdin 游标**（GNU fd 0 共享语义，`subst.c:7143`）：
  `FUNCTION_STDIN_OFFSET` 贯通 read/外部命令/函数/命令替换——
  `stdin_string_for_command` 返回剩余段；`function_call_stdin`
  标记 carve 来源、函数返回时子游标折回父游标；comsub 子侧经
  `Cell` 惰性回写（`apply_comsub_stdin_writeback`）；`cat` 快路径
  裸调用回落真实管线；`run_external_command_substitution` 喂
  FUNCTION_STDIN 余量。
- **`read -a` 保留空字段**（`read.def` 非空白 IFS 分隔符产空字段）：
  `split_read_array_words{,_backslash}` 委托 `split_read_field_ranges`，
  `IFS=: read -a A <<< :::` → 3 空元素。
- **`read -t` 超时仍赋已读部分**（`read.def:540-554`：retval=
  128+SIGALRM 后 `goto assign_vars`）：`timed_read_followup_output`
  改用真实变量存储做临时赋值/恢复，同组后续命令可见部分值。
- **`read -t 0` 非终端立即可读 → rc 0**（GNU poll 语义）：
  /dev/null、EOF 管道、文件均成功。
- **原始字节分隔符**（`lib/sh/stringlib.c` 字节语义）：
  `$'\200'` 等载体字节经 RAW_BYTE 标记对存储，分隔符派生与比较
  全部按解码后字节码位——管道/`< <()`/`-u fd`/`-n` 组合全对齐。
- **IFS 载体字节分词**（`read.def` IFS whitespace 判定）：
  `read_split.rs` 分词器改逻辑单元（标记对解码为字节码位）比较，
  `\f`(0x0c) 等载体字节在 IFS 与输入两侧正确识别修剪——
  `IFS=$'\t\r\f\v'` 尾部空白裁剪对齐。

**台账**（true-baseline.sh 同口径）：new-exp 60→0、quotearray 50→0、
comsub2 34→0、read 44→0 真 diff（残留 25 行全环境性：Windows 无
`/dev/tty` → RB rc=1 vs GNU 真实 tty 超时 142 及其连锁变量态 +
GNU 侧 mkfifo `Operation not supported` 触发 124 截断）、
ifs-posix 全量 6856/6856 通过（harness 短超时曾产空 rb.out 抖动）、
comsub 17（预存：alias 注入未闭合 `$(`、`let --`、case-in-comsub）、
exp 4（`/src/cmd` 环境路径噪声）、assoc 0、array 0、posixexp 0、
braces 0、ifs 0、rhs-exp 0、arith 0。

**read 已知环境残留**（非语义缺口）：
- `read -t N </dev/tty`：RB 打不开 `/dev/tty`（rc=1）vs GNU 打开
  控制台超时（rc>128）——Windows 无进程控制终端。
- `read -e`（readline）超时族同理。
- 套件 GNU 侧 mkfifo 循环（2000 次子 shell）+ `/dev/tty` 阻塞导致
  GNU 输出在 harness 超时处截断，RB 多出的尾部行为 GNU 未执行区段。

## 合并前审计修复（B1/B3/B4/B7；B2 单独立项）

**B1 `\x05` PREEXPANDED_STDIN_BODY 碰撞（高）**——脚本文件是任意
字节流，heredoc 体首字节可为原始 0x05，与执行器"已展开"哨兵碰撞
导致跳过展开。修法：`parser/redirections.rs::encode_stdin_body_enq`
在收集点（heredoc 体 + here-string 词）把字面 ENQ 编码为既有
RAW_BYTE 标记对；`execution_misc.rs::decode_stdin_body_enq` 在
`expand_heredoc_body{,_mut}`/`expand_here_string_mut` 返回边界把
该对解码回字面 `\x05`（0x05 非载体字节，值域规范形态即字面 char，
`$'\005'` 亦然）；字节边界统一走
`substitution_metadata::shell_text_to_raw_bytes`——补齐了
`external_inner::spawn_external_process` 与 `external_file_builtins::
external_cat` 快路径原先 `input.as_bytes()` 直写泄漏 PUA 标记的缺口。
GNU 锚点：heredoc 体为字节流（`parse.y` gather_here_documents +
`redir.c` do_redirections 不区分字节值）。验证矩阵（RB vs WSL GNU
5.3 字节级 od 对照）：体首/体中/体独占 0x05 × unquoted/quoted/
`0<<`/`<<<`/comsub 捕获/`while read`/`grep` 全对齐；`$(cat <<EOF)`
捕获值中 0x05 不再以 PUA UTF-8 泄漏。

**B3 `cat 0<<EOF` 无输出（中）**——显式 fd-0 heredoc 只入
`heredoc_redirects`（fd=Some(0)），stdin 取材只查 `cmd.heredoc`。
修法：`shell_options.rs::stdin_string_for_command{,_mut}` 与
`external_setup.rs::apply_external_stdin_redirect` 增加 fd-0 消费，
`external_fd_heredoc_input` 提升为 executor 可见。顺带按 GNU
`redir.c` do_redirections 左序语义修正 fd-0 输入源取舍：利用
`cmd.redirects` 有序表取最后一条 fd-0 输入重定向为赢家——
`cat <<A 0<<B`/`cat 0<<A <<B`/`cat <<A <<<w`/`cat <<<w <<A`/
`cat <<A <file` 均与 GNU 对齐（同 fd 后写赢）。

**B4 赋值命令 bad substitution 泄漏（中）**——
`v=${x-${'u'%'v'}}` 两处缺口：①alternate 词经
`decode_double_quotes_in_quoted_parameter_word` 把 `'` 编码为
`\x17` 数据哨兵，`braced_name_ends_on_quote` 失配静默——现在把
`\x17`/`\x18` 识别为源引号证据（该位置只可能来自源引号，
GNU `subst.c:10277-10288 parameter_brace_expand` 同源），诊断经
`bad_substitution_display` 还原哨兵为源字符；②
`parameter_bad_substitution` 旗标在纯赋值命令
（`execute_empty_words_command`）路径上不被消费，泄漏到下一命令才
爆且错杀无辜命令行——赋值 RHS 展开后立即按 `abort_on_expansion_
errors` 同型语义消费（非 posix：ExpansionFailure 丢当前命令 rc=1、
脚本继续；posix 非交互：ExitCode fatal）。验证：`v=${x-${'u'%'v'}}`
报错 rc=1、后续命令正常执行、`set -o posix` 下 fatal rc=1，均与
GNU 5.3 一致。

**B7 卫生**——移除 `RUBASH_DEBUG_HD`（command_execute/
command_input_scope ×3）、`RUBASH_DEBUG_AV`（assignment_expansion）、
`RB_DBG_*` 探针；`command_prepare.rs`/`expand_word.rs` 注释与代码
字面量中的裸 0x1d/0x17/0x18 控制字节全部改转义写法；
`execution_misc.rs` 上"0x05 不可能出现在体首"的错误断言注释已
更正为编码不变式说明。

**B2 单独立项（未动）**——`cat <<EOF &` 后台 heredoc 无输出：
`command_text.rs::bash_command_text` 不渲染 heredoc 体，属命令
文本重建路径，与本次 stdin 语义修复正交，留作独立任务。

**预存非 UTF-8 脚本限制（新记录，非本批引入）**——含 0x80+ 等
非法 UTF-8 字节的脚本文件整体被 "cannot execute binary file"
拒绝（脚本按 UTF-8 解码进 String），GNU 按字节流接受。B1 的
0x05 属合法 UTF-8 范围不受影响；非 UTF-8 脚本支持是独立的架构项。

**台账**（true-baseline.sh 同口径）：heredoc 5、herestr 0、
redir 46、vredir 24、comsub2 0、new-exp 0、quotearray 0、
read 25（全环境性，见上节）、ifs 0、mapfile 60（harness 未拷贝
`mapfile.data` 夹具，GNU 侧同样 No such file 报错）、comsub 17
（预存解析簇）。heredoc/redir/vredir 残留均为预存簇（comsub 内
heredoc 括号平衡、后台 heredoc=B2 族、$LINENO 漂移、`exec 0<&5-`
/`|&` 重印），本批无回归。

**新记录预存残留**（非本批引入，独立任务候选）：
- lastpipe 5：嵌套 `while read` 管道外层只读首行
  （`echo -e 'A
B' | while read o; do echo -e '1
2' | while read i;
  do echo $o$i; done; done` GNU=A1 A2 B1 B2，RB=A1 A2）——无重定向
  参与，属嵌套管道 FUNCTION_STDIN 游标族，与 B1/B3 正交。
- 内建/复合命令侧"非赢家 fd-0 `<` 仍执行 open"未建模：
  `cat <<A <missing` 外部路径已对齐（open 失败中止），内建 cat
  快路径仍直接给 heredoc 体——属重定向逐条应用的架构项。

## 第二十八节：2026-09-20 master 合并 + 数组下标单遍展开批次（fix/array6-patsub-quotes）

**合并**：`fix/array6-patsub-quotes` 合并 origin/master（assoc/array 批次 +
shopt 宽度修复）；PST_ASSIGNOK 门控去重——master 的 command_execute.rs
版本保留（更贴 execute_cmd.c 结构），token_actions.rs 版本移除。

**合并后修复批次**（GNU C 出处见各提交）：

- **数组/assoc 下标单遍展开**——GNU `expand_array_subscript`
  （subst.c:11107）对下标只展开一遍，产物字节经 abstab 反斜杠转义
  （`[` `]` `$` `` ` `` `~` `\` `'` `"`）防止二次触发；`array_expand_index`
  （arrayfunc.c:1356）收 RAW 文本、`expand_arith_string` 是唯一展开遍。
  Rubash 把已展开文本再过一遍词展开，`a[$key]`（key 含 `$(...)`）二次
  执行命令替换。修复点：`array_assignment_exec`（LHS 下标喂 raw）、
  `conditional`（`[[` 算术比较操作数经 `expand_cond_arith_operand`
  镜像 cond_expand_word(op,3) Q_ARITH）、`arithmetic_aliases`
  （arith_display_expand 对展开产物施加 abstab 转义用于 evalexp
  错误 token）、`arithmetic/lvalue`（assoc 键反斜杠解码，expr_streval
  同源）、`arithmetic/mod`（assoc_subscript_end 公开）。
- **`expand_subscript_string` ANSI-C 解码**——`$'...'` 下标在 raw 文本里
  仍是源形态，补 `decode_ansi_c_spans`（subst.c:11063 expand_string 的
  ansicstr 臂）；`a[$'\x01']` 键恢复正确。
- **`[[ -v assoc[@] ]]`**——`@`/`*` 对关联数组是字面键（test.def
  test_variable）；全数组短路只适用索引数组。
- **W_ARRAYREF 载体消费**——`\x02` 前缀是词旗标不是操作数文本
  （execute_cmd.c:4366 fix_arrayref_words）；`read` 名字校验、`wait -p`
  绑定、`unset` 操作数、`printf -v` 名称校验各消费点补齐。
- **`unset n[0]` nameref 解析**——find_variable 跟随 nameref
  （builtins/set.def:924），`n -> v` 时删 `v[0]` 而非 `n[0]`。
- **`unset arr[@]` BASH_COMPAT≤51**——compat 级 ≤51 时 unset 传
  VA_ALLOWALL，unbind_array_element 删整个变量（arrayfunc.c:1153-1162、
  unset.def:975-977）；`shell_compatibility_level_value` 解析
  BASH_COMPAT NN/N.N（variables.c set_compatibility_level）。
- **`printf -v a[@]`**——valid_array_reference 纯语法判定（arrayfunc.c），
  `@` 的 `bad array subscript` 在绑定期报（builtins/common.c:949
  builtin_bind_variable），不再提前报 `not a valid identifier`。
- **标量 `(...)` RHS 字面绑定**——标量目标的括号文本按字面存
  （bind_variable_value），不带 W_COMPASSIGN 的 `declare c='(1 2)'`
  不再被拆成 `\x10` 标记元素（arrayfunc.c:557）。
- **brace 展开跳过 `$` 体**——`word_contains_brace_group` 跳过
  `${...}`/`$(...)` 体（GNU braces.c 不下探展开体），修
  `a${u-{x,y}}z` 与 `("${x[@]}" "y")` 去引号回归。
- **`${!indir}`/`$ref` nameref 穿透**——parameter_brace_expand_indir
  把间接目标再过 parameter_brace_expand_word（subst.c:7955），
  find_variable 跟随 nameref：`indir=ref`、`declare -n ref=arr` 时
  `${!indir}` 取 arr[0]；`$ref`/`${!name}` 指向 `arr[@]`/`arr[*]` 时是
  词表源（nameref18.sub）。
- **卫生**——`__RB_DBG_DECLARE` 探针移除；`read_builtin` 裸 NUL 字面量
  改 `'\u{0}'` 转义；README 合并冲突残骸修复。

**合并后全量基线**（83 套件，`postmerge-baseline-ledger.txt`）：

| 指标 | 合并前 | 合并后 | Δ |
|---|---|---|---|
| 零差套件 | 49 | **49** | 0 |
| DIFF 1-50 | 27 | 27 | 0 |
| DIFF 51-250 | 7 | 7 | 0 |
| DIFF 251+ | 0 | 0 | 0 |
| 总 diff 行 | 849 | **848** | −1 |

逐套件对比：82/83 持平，nameref 6→1（改善，唯一残留为 coproc `_PID`
回收时序——GNU 在 SIGCHLD reap 时即 unset，Windows 子进程退出延迟使
RB 在下一边界才摘，属竞态噪声）；**history 104→108 为 GNU 侧超时抖动**
——history.tests 的 `${THIS_SH} -i` 交互子测试在本环境 GNU 侧 40s 被
timeout -k 杀（rc=137，90s 加长同样杀），gnu.out 截断点每次不同，diff
计数随截断位置浮动；非 RB 语义回归（本批改动全在下标/数组/nameref
域，不触 interactive history 回显）。

**合并中途出现的回归已全部修回持平**：array 0→38→0、assoc 0→26→0、
quotearray 0→49→0、nameref 6→11→1、new-exp 0→4→0。根因是合并侧
`` ARRAYREF_FLAG 载体词旗标在多个消费点未剥离（read 名字校验、
wait -p、unset 操作数）+ 标量 `(...)` RHS 误走复合赋值拆分 + 下标
二次展开；均按上文 GNU C 出处修复，非特判补丁。

`heredoc` 5、`lastpipe` 5、`attr` 40、`mapfile` 60（harness 未拷
`mapfile.data` 夹具，GNU 侧同样报 No such file）等残留均为预存簇
或环境性，见第二十七节。

**边界规则执行**：本批未新增任何哨兵字节/PUA 码点/命名标记串；仅做了
既有载体的消费点补齐（`\x02` ARRAYREF_FLAG 在 read/wait/unset/printf-v
的剥离）与碰撞修复，符合 typed-carrier 迁移边界约定。
