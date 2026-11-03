# 更新日志

所有重要的项目更改都将记录在此文件中。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)。

## [Unreleased]

### 修复

- **深审修复波（1.5.1 向，2026-11-04）**：两轮深度审计（约 214 条经队长复核的发现）中可局部修复项全部落地——解析器诊断归因对齐 GNU（case/if/函数/算术-for/命令替换/EOF 各形态的报错令牌与行号，#2739–#2754、#2679）；heredoc 展开上下文统一（#2676、#2681）；解析器挂死与解析期代码执行两处高危关闭（#2682、#2686）；状态变更内建在重定向失败时不再执行（#2692）；环境导入不再误剥小写变量的导出标记（#2723）；compspec 锚定与补全分发面修正（#2724、#2726、#2727）；词法/展开/作用域多处 GNU 对齐（#2668–#2675、#2756–#2767、#2778）。每条修复均带以 issue 号命名的回归钉。结构性余项（单一展开管线、fdwalk 等）按 1.5.1 专项六件工程推进，残差均已逐条立单。

## [1.4.3] - 2026-10-22

### 修复

- **命令词干大小写折叠**（#998/#1000）：System32 的盘上真名全大写（`PING.EXE`/`NETSTAT.EXE`/`TRACERT.EXE`/`HOSTNAME.EXE`），#799 的词干字节精确契约使小写 `ping`/`netstat`/`tracert` 全部 `command not found`（`PING` 大写反而能跑、`which`/`command -v` 两张脸不一致）。fold 臂改为 stem ASCII 大小写折叠（字节相等仍优先；无扩展名盘名保持字节精确；解析拼写恒报盘上真名）——与 CMD/PowerShell/msys 全生态一致；#799 的 harvest 误抬面（小写 `r` 抬 `R.exe`）按 Windows 生态惯例重新接受。tests/issue799 套件改写为新契约并新增 System32 大写盘名回归。
- **行编辑器 `${v^^}`/`${v:1:3}` 崩溃**（niubash#503，c143699c）：下标 walker 里不可达的 `LEX_WASDOL` `${` 臂被以裸 `{` 语义消费，交互输入含上述序列触发 6 GiB 分配失败（MSIX 与 NSIS 1.4.1 双复现）。删除该臂。
- **cli_tests 与宿主 Winuxsh 安装面隔离**（#996）：测试 spawn 不再被宿主安装的环境变量污染。

### 性能

- **splits_unquoted no-IFS 短路重落地**（#995/#1002）：#992 门因空展开折叠语义回退（无引号空展开把一个字段折叠为零、词消失），重落地版加入 `!expanded.is_empty()` 准入；splits_gate 4 万对抗输入 × 5 IFS 态差分网兜底，Linux 13/13 upstream 套件与基线逐字节一致。
- **assoc 交错访问固定成本**（#914）：`k[i]`/`k$i` 交错写读尾部的每操作固定开销削减；export/unset 交错循环二段身体削减（#984/#986/#920 后续）。
- **信号提示事件每 8 个命令边界采样**（#682）：信号 hint event 从每命令一次降频。
- **`[ $VAR -op N ]` 整数比较快展开**：测试面热路径。
- **裸 `**` 全局排序、mapfile 批量拆分/存储、数组元素 quote 单分配**等 #607-#609/#661-#680 系列切片全部落定（详见下方各条）。

### 新增

- **仓库 logo**（#988）：青色 rotor-r 引擎标识。

## [1.5.0] - 2026-11-03

### 新增

- **脚本级函数级性能剖析器**（#815）：`niu --profile <script> [--profile-output <base>]`——解释器确定性插桩（非采样），在函数入口/出口（`execute_cmd.c:5200 execute_function` 家族）、简单命令分发点（`execute_cmd.c:624`，试探帧由真实分发自分类：函数/外部派发取消、其余确认为内建——无第二准入表）、外部 spawn 面、`( )` 子壳与 `$()` 命令替换独立栈帧（subst.c:7143）挂 Drop 守卫；return/exit/展开失败等中止形态经栈展开正确归因。运行时为 thread_local 调用树（self=elapsed−children，唯一 collapsed 路径聚合 count/self/total），归因三分离（内建/外部 spawn/子壳，self-time 切分面总和=100%）。输出文本调用树（self/children/calls 排序）+ collapsed-stack（flamegraph/speedscope 可渲染）；报告只写文件（默认进程 temp 目录，绝不 stdout/stderr）——profile 开启时 stdout/stderr/rc 与关闭逐字节一致。关闭路径开销=每缝一次 relaxed AtomicBool load，#607-#609 热路探针 A/B（master 对比）零回退。招牌验收：83 套件 GNU 差分全集 profiler 开启重跑，失败集与基线双向一致（scripts/check-profile-differential.sh 脚本化）。

### 变更

- **wrapper 重spawn 的 argv 握手旗标按消费者在位门控**（#873 后续）：`--internal-bg-wrapper=<nonce>` 是 niu CLI 专属旗标，此前 wrapper 重spawn 无条件附加——嵌入式宿主（niubash 的 niu 启动器）与任何不经 `consume_background_wrapper_handshake` 的 CLI 会把它当未知选项拒收（`bash: --internal-bg-wrapper=<nonce>: invalid option` + usage），后台命令 spawn 即死（niubash master CI background_spawn_args 两红）。现旗标仅当本进程 CLI 在启动时调用过 consume（`background_wrapper_flag_consumer_present()`，rubash main.rs 每次启动必调）才附加；非消费者宿主得到 env-nonce-only 重spawn——env 键对非消费者惰性（不被读、消费者启动即清洗），单通道永不 arm（#866 契约不变）。rubash 本体行为逐 spawn 等价（消费者恒在位）；宿主采纳 consume 即恢复完整双通道握手与 #737 cascade。
- **Windows 无宿主 sudo 回落 PATH 执行**（#810 行为变化披露，#813 补记）：`sudo cmd` 在 Windows 且嵌入宿主**未注册 elevation handler** 时，曾一律由 host builtin 应答 "elevation handler is not configured"（rc 1），PATH 从不被咨询；自 #810 起该名字按外部命令走 PATH 解析——有 sudo.exe 的机器（Win11 System32）真实提升执行，没有的报 command-not-found rc 127，`builtin sudo` 无 handler 时报 `builtin: sudo: not a shell builtin`（rc 1）。introspection 契约不变：`command -v`/`type -a`/`enable -a`/`compgen -b` 依旧把 sudo 报告为外部命令（docs/builtins.md §3 隐藏 builtin 条款）。方向与 GNU 更近（GNU 无 sudo builtin）。#813 以两条 windows-only 测试钉死两臂执行路由（假 sudo PATH 实跑、`builtin sudo` rc 1）。

### 性能

- **字面单命令跳过词展开重建**（#607，PR #665）：每个简单命令在 `expand_command_words` 里付一次全量 `CommandNode` 重建（~1.34µs/次），裸 `:` 的展开恒为 identity。GNU 从不重建（`execute_cmd.c:4468` 跑存储的 SIMPLE_COM；`subst.c:13219` 无触发器文本无事可做）。白名单准入（单字面词、全部结构/重定向/赋值通道为空，`[`/`[[`/分发动词除外），共享 `preserves_word_metadata` 谓词（提取复用、永不复制），契约测试逐词锁定 fast==pipeline。`:` 循环 2.03x→1.77x GNU（~0.74µs/iter），`true` 1.86x→1.67x；`((1))` 路径按设计不动。门禁与 niubash 产品 A/B 零差异。
- **大小写改写去分配+首字符短路**（#608，PR #666）：`apply_parameter_case_mod` 曾对每个字符跑一次 glob matcher 并 `to_string()` 堆分配；First 算子（`^ , ~`）按 GNU `subst.c case_transform` 只测首字符，空 pattern 即 `?`。First 算子现只测/改首字符后原文追加（栈缓冲零分配），空 `~~` 走直接翻转，主循环改栈缓冲。`${s^}` 500ch x20000 ~1.42s→~0.24s（~6x），`${s~~}` ~10%。33 例 GNU 差分电池 33/33；门禁与产品 A/B 零差异。
- **标量赋值快 RHS 切片一**（#609，PR #672）：`t=$S` 循环 2.72x。`literal`/`$name`/`"$name"` 三形走白名单快路（字面按构造输出相等直接返回，dollar 拷贝一次借读 cell）；dynamics、`set -u`、非名、IFS 标记值走通用路，绑定仍经 `apply_shell_assignment`。100k `t=$S` 1096ms→927ms（1.18x）。30 例差分电池 30/30；门禁与产品 A/B 零差异。
- **标量绑定去双存储漫步切片二**（#609，PR #678）：`bind_marks_free_scalar`——一次 `attr_of` 探针放行无标记形，同值跳过双写，否则原地双写（镜像不变式成立）；只读照常诊断（`variables.c:3258`）。net assign 2.03µs→1.33µs（−35%），GNU 1.25µs。电池 30/30；门禁与产品 A/B 零差异。
- **算术循环机器**（#607 切片三，PR #679）：body 借指针重走（无逐 entry `Ast` 克隆/reader 包装），DEBUG 探针门控+体后重探，空表 reap 跳过。N=200k `:` 循环 −3.4%。6 契约测试；上游脚本逐字节一致；门禁与产品 A/B 零差异。
- **if 条件分发残留**（#661，PR #673）：空别名表时 raw 首词即展开首词，跳过逐命令 `expand_aliases`；单裸 `:`/`true` 条件跑无标记路径（影子函数/别名回退保 `set -e` 语义，附 4 回归测试）。50 命令 else 体 −26%；`ifcase` 55/55、`casearm` 29/30（仅预存 #17）；门禁零差异（niu 50/49=已知负载抖动，隔离绿）。
- **case 静态臂零分配**（#662，PR #669）：`expand_case_pattern` 逐臂全管线+~5 String 层。默认拒绝分类器（`~` 经 GNU 探针确认为展开，掉队）命中则 `Cow::Borrowed`。每臂斜率 ~2.5µs→~1µs；`casearm` 29/30（仅预存 #17）、`ifcase` 55/55；门禁与产品 A/B 零差异。
- **mapfile 批量拆分+存储**（#663，PR #670）：100k 行 243ms vs GNU 52ms。拆分一次解码+字节域切分（`-t` 字节域剥离），无 `-O`/`-C` 时批量直写 dense 存储，跳过中间 index map。243ms→174ms（4.7x→3.3x）。26 探针矩阵 25/26（1 预存 NUL 分歧，新旧一致）；门禁与产品 A/B 零差异。剩余存储侧是后片。
- **裸 `**` 全局排序**（#664，PR #674）：globstar 漫步深度优先，GNU `sh_sortglob` 全局重排。单 `**` 调用既有 `globsort_matches` 全局重排（C/POSIX 字节序 else live collation）。repro 与 GNU 逐字节一致；glob 电池 33/34→34/34（关 glob-12 观察）；门禁与产品 A/B 零差异。纯正确性片。
- **数组元素 quote 单分配快路**（#680 切片一，PR #686）：`quote_array_value` 逐元素 2 marker 替换+解码扫描+`ansic_shouldquote`+4 链式替换。纯可打印 ASCII（`sh_double_quote` 转义集之外）直接渲染 `"value"` 单分配。seq100k mapfile ~181ms→~99ms（~1.8x）。矩阵 25/26（预存 NUL 分歧）；门禁与产品 A/B 零差异。
- **mapfile 跳过整输入编解码往返**（#680 切片二，PR #688）：ASCII 无 carrier 字节输入解码/重编码皆恒等，直接切分（每段单分配）。seq1M −6%。4 契约测试（含 carrier 回退钉死 GNU 输出）；门禁与产品 A/B 零差异。
- **glob 排序一次 locale 解析**（#681 切片 b，PR #689）：逐比较 3 env 查询提到逐排序一次；C 下字节序、非 C 走原比较器（结果恒等）。`LC_ALL=C` 300 文件循环 −22%，默认 locale 不变。契约测试；双 locale 电池 34/34；门禁与产品 A/B 零差异。
- **`export`/`readonly` 补上越界负下标 abort**（#635 后续，PR #677）：`export -a a=( [-2]=q )` 曾打印 AFTER rc 0。`-a/-A` 委托内嵌 `declare_builtin`（`setattr.def:240-268`），`arrayfunc.c:804-809` any_failed + DISCARD（`subst.c:11169`）。复用 #658 原语：`oob_word` 上浮、操作数循环遇 OOB 即停、Executor 层裸诊断+exit 1+#637 abort。13 例新测试；18 例 GNU 探针矩阵 18/18；门禁与产品 A/B 零差异。
- **算术票据快路**（#607 切片二，PR #671）：identity 字母 `(( ))` 仍付全票据。双门（语法形只读+状态门：写日志空、标识符 unset/纯十进制）跳过 shopt/nounset/标记/上下文/同步。dense 探针 ~5%（~200ns/eval），headline 不变。6 契约测试；门禁与产品 A/B 零差异。
- **for-list 绑定补 dequote**（#675，PR #683）：`for p in "f*"` 绑了 `f`+CTLESC+`*`（长 3，全局 glob 死），只有双引号 for-list 路跳过解引用。绑定点统一走存储侧 dequote（`subst.c:4807`）。`casearm` 29/30→30/30，`ifcase` 55/55；门禁与产品 A/B 零差异。根因在 case 上游。
- **quoted 复合元素保留引用态**（#377，PR #730）：`declare -a d3=("$x")` 存了 glob 结果。NoMatch 臂推解引用重包形式，消费端 glob 解引用文本丢引用态。现引号内序列化 CTLESC 保护；`=` 臂解保护判下标。文件 10/12→11/12（剩预存）；glob 34/34；门禁与产品 A/B 零差异。
- **fd_redirects 并行竞态**（#667，PR #676）：6 测试并行红、串行绿。`external_fd_copy_bin_dir` 返回固定路径，多测试互删助手目录。现 pid+ThreadId+计数器唯一目录。并行 10/19→19/19（3x），串行 19/19。纯测试改动。
- **平台专属测试分流**（#144/#363/#555，PR #692/#693/#694）：`cmd /c` 流合并 10 测、`#!/usr/bin/env` 9 测、反斜杠字面量 11 行——在非对应平台上注定红。整文件/逐行 `cfg(windows)` + `$I363_PROBE`/PATH 解析，可移植行全留。纯测试改动。
- **测试进程 CWD 隔离**（#690，PR #691）：`cd` 内建 chdir 测试进程（语义正确），但 1586 测共用单进程 CWD，一个 `cd` 毒死后面全部相对 `target/` 用例（~990 失败摊满所有 part）。逐测 RAII `CwdGuard`（panic 安全），1589 处机械插入，零语义改动。`executor_tests` 串行 321/1229→1540/10（剩 10 全是预存环境/抖动）；门禁与产品 A/B 零差异。串行为限（CWD 进程全局）。
- **respawn 传输可写探针回退**（PR #752）：无 TMPDIR 且 `/tmp` 不可写（Android 有不可写的 `/tmp`）时临时文件建不出，回退内联 300KB `-c` 死于 E2BIG。存在性改可写性探针（pid+计数器），一次退到进程 temp dir、二次退 CWD。6000 函数后台任务恢复；无残留文件。
- **Windows-only tmpdir 测试分流**（#328/#331，PR #750）：`/var/tmp` 映射与 TMPDIR 自注入引擎臂都是 `cfg(windows)`，测试在外失败。`#[cfg(windows)]` 门。纯测试改动。
- **真 Unix stty**（#282，PR #732）：非 Windows 是空桩（恒 rc 0）。现 `isatty(0)` 探针（非终端全形态诊断+rc 1），终端下左到右执行（`size` 经 TIOCGWINSZ，raw 系经 termios），Windows 路不动。文件 2/4→4/4；门禁与产品 A/B 零差异。
- **交互 rcfile 只 source 一次**（#312，PR #712）：`--rcfile gate.sh -i` 跑两遍 gate.sh。`run_startup_files` 已 source 后各驱动又直调一次。用 `__RUBASH_SOURCED_RC` 去重（空 HOME/非交互扩展/login 原样保留），`#297` exit 语义不变。append 探针 2→1；文件 6/9→9/9；门禁与产品 A/B 零差异。
- **真 uid/euid 身份，去掉 root 模拟**（PR #698）：非 root 主机报 `UID`/`EUID` 0、`\$` 渲染 `#`。`uid_value` 回退硬编码 `"0"`（开发基线跑在 root 下的模拟）。现取真 `getuid()`/`geteuid()`（GNU `variables.c:984`，继承值保留，Windows 保持 0）。4 个 prompt 测试曾钉作者账号、`reader` 默认 prompt 钉 `#`，现按宿主校准。三方探针与 GNU 一致；门禁与产品 A/B 零差异。
- **关联数组元素写不再全量重渲染**（#618，PR #650）：`m[k]=v` 每次写曾做三次 O(keys) 操作（镜像字符串克隆、解析 memo 的 Vec 深拷贝、全量重渲染），填充数组是 O(n²)，K=5000 时单次写比 GNU 慢 433 倍。现改为 O(1) 追加一段 `[k]=v`，预算触发规范渲染压缩重复段（均摊 O(1)，串长不超 2 倍规范长度）；读路径零改动（追加永不过期）。K=5000 单次写快 45 倍，与 GNU 差距 445x→9.9x，key 数涨 5000 倍单次写仅漂 1.14x。附带修好一个 O(n²) tokenizer（每段 collect 整个剩余输入，10001 段真解析 563ms→17.8ms 线性）。50 例 GNU 差分电池 50/50；全量 failure-set 门禁与 niubash 产品 A/B 零差异。
- **索引数组覆盖写不再全量重渲染**（#616，PR #652）：`a[i]=v` 同理 O(n²)，K=5000 时 674x。覆盖/稀疏跳跃同样 O(1) 追加（indexed 读路径天然折叠重复，BTreeMap last-wins，无需改解析/渲染；每次追加同步 typed cell；`declare -p` 等原文打印在有重复时才折叠重渲染）。K=5000 快 177 倍，与 GNU 差距 674x→3.8x。附带修好 `declare -a a=([5]=x [2]=y)` 种下 dense cell 导致后续元素写复刻的正确性 bug（另见 #651）。50 例差分电池 49/50（唯一分歧即 #651）；门禁与产品 A/B 零差异。

### 修复

- **`mark_quoted_ifs_from_raw` 模拟器补上 carrier 字节识别，per-char IFS 标记不再被 carrier 词静默跳过**（#868）：`mark_quoted_ifs_from_raw`（`src/executor/command_prepare.rs`，#743 的实现核心）用平行状态机重读 raw 词、模拟 quote removal，仅在模拟结果与 cooked 词逐字符相等时才落 `\x1c` per-char 标记，失配即静默 bail 退回粗粒度整拆/整不拆——而模拟器把 cook 产出的每个 markers.rs carrier（`$'...'` 解码输出、`$"..."` 体、逃逸引号、逃逸 IFS 空白/模式字符）都当普通数据字节，carrier 词逐字符必失配，最需要 per-char 规则的词恰恰被静默降级。现按 lexer 权威 walk（`lexer/quotes.rs` cursor 系列）逐臂复刻 carrier 变换：逃逸符在引号内外分别 cook 成 DATA_DOLLAR/DATA_BACKTICK/DATA_SQUOTE/DATA_BACKSLASH/DATA_DQUOTE、逃逸 IFS 空白 cook 成 ANSI_C_IFS_GUARD 对（自护形，不另记位）、逃逸模式字符 cook 成 CTLESC 对；`'...'` 体内 `$`/`` ` ``/`"` 成 DATA_*；`"..."` 体内另加 `'`→DATA_SQUOTE、glob 集→CTLESC 对、非展开 `$`→DATA_DOLLAR（parse.y skip_double_quoted）；`$'...'` 直走权威 ANSI-C 管线（`decode_ansi_c_quoted`+`escape_decoded_ansi_c_quotes`）；`$"..."` 去掉 `$` 进双引号规则；非花括 `$name` 复刻 walk 的 pending_name 协议（下一 `$`/`'`/`"`/`\` 触发臂先发 PARAM_NAME_END_MARKER）。同轮修出同函数 apply 环的双重 `\x1c` 缺陷：`mark_literal_ifs_chars` 已护字符再叠一层 glue 时，splitter 把第一层 glue 花在第二层上、IFS 字符反被释放——`IFS=:; printf '<%s>' "a:b"$w`（含纯 ASCII `'a:b'$w` 家族）曾把带引号的 `:` 拆开；现在已护位不重标。失配 bail 安全网保留（真分歧形状仍粗回退，不产生错位标记）。WSL GNU 5.3.0(1) 双侧逐字节：45 形电池（carrier 在 IFS 值内/外、嵌套引号、多分隔符、纯 carrier 词、unset 展开）本层 42 形全对，3 处残差（解码引号×引号入 IFS、相邻引用态 `$w` 尾拆）经 base A/B 确认为其他层既有缺口、base 一致或更近，不属本票；回归 `tests/issue868_carrier_ifs_mark_regressions.rs`（7 例）+ `scripts/regression-catalog` case `issue-868`（base 分叉、修复侧 must-match）。#298/#795/#843/marker_leak_golden 等 9 套件家族 102 例全绿。
- **`${var@P}` prompt 变换通道不再泄漏命令替换状态**（#813）：`PS1='x$(false)y'; p=${PS1@P}; echo $?` 曾打印 1——`@P` 算子通道（`apply_parameter_transform_value` 的 Prompt 臂）内嵌 `$( )` 的收割状态经替换状态 cell 被赋值边界当作替换退出规则发布。GNU 律（parse.y:6684-6697）：`decode_prompt_string` 在 promptvars 遍历（`expand_prompt_string`，parse.y:6689）前后快照/恢复 `last_command_exit_value` 与 `last_command_subst_status`（subst.c string_transform 'P' 走同一 `decode_prompt_string`，`${var@P}` 同受覆盖）；#604 修的是交互渲染通道（`expand_prompt_string_mut` 快照恢复 rc/PIPESTATUS），本修复把同一律落到共享 prompt 遍历引擎（`expand_embedded_parameters_double_quoted` 快照/恢复替换状态 cell，含管道 `-i` 渲染通道）与 mut 渲染通道的 cell 残留。恢复取遍历前值（保真恢复而非清零）：`w=$(false)${PS1@P}` 的词自身替换状态照常发布（GNU rc=1）。WSL GNU 5.3.0 探针 `true; p=${PS1@P}`→rc 0、`false; q=${PS1@P}`→rc 0、混合 RHS→rc 1 逐一对齐；回归 `tests/issue813_prompt_transform_status_regressions.rs`（3 例）。
- **`set -T`（functrace）下 DEBUG trap 透传进 `$()` 命令替换**（#825）：`set -T; trap 'echo D:$BASH_COMMAND' DEBUG; x=$(true)` 曾打印 `got=`——`$(true)`/`$(false)`/`$( :)` 的 trivial 体状态快捷路（`expand_command_substitution_inner`，command_substitution.rs）排在 functrace+DEBUG 取消资格门**之上**，硬编码空输出短路从未进入替换子壳，GNU trap.c:1466-1474（`reset_or_restore_signal_handlers` 仅在 `function_trace_mode == 0` 时清除 DEBUG/RETURN）要求的继承触发完全丢失。GNU subst.c:7143 `command_substitute` 把每个体交给 `parse_and_execute`，execute_cmd.c:4648-4649 对 `true`/`false`/`:` 照发头行与 DEBUG。修复：两快捷路移到门之下（与 xtrace 取消资格同门同型），门命中即走真实 parser/executor。WSL GNU 5.3.0(1) stdin oracle 8 形态逐字节一致：`x=$(true)`→`got=D:true`、`x=$(false)`→`got=[D:false] rc=1`、`$( (true) )`→`got2=[D:true]`、`$(true; true)`→双触发捕获；functrace 关/DEBUG 无 `-T`/`-T` 无 trap 三个对照保持原语义（`got=`/`got= rc=0`）。已知预存外观分歧（本修复不改动）：顶层赋值的 `$BASH_COMMAND` 回显里 rubash 打印机重渲染体文本（`y=$(( true ))` vs GNU 原文 `y=$( ( true ))`）——触发集合与捕获内容逐字节一致。回归 `tests/issue825_comsub_debug_functrace_regressions.rs`（8 例）；审计 probe `debug-into-comsub-functrace-on` 转绿。
- **复合命令自身重定向失败诊断走 fd 表、单次发送**（#826）：`{ :; } 2>err2.txt > /nonexistent_dir/f` 曾把诊断打印**两次**到真实 stderr 且 err2.txt 根本没建——brace 包装层按字段序（`redirect_out` 先于 `redirect_err`）应用重定向，fd-1 open 在 fd-2 绑定前失败；错误被 ast_exec 分发的 IoError 臂吞掉后不 `continue`，同一命令经通用路**再分发**、再失败、再打印（面 1/2 双打印 + 捕获丢失）。`( { :; } > /nonexistent_dir/f ) 2>err.txt` 则"捕获 + 泄漏"并存：包装层 raw `eprintln!` 绕过子壳已建立的 fd 2 绑定（面 3）。GNU（redir.c:767-955 `do_redirection_internal` 按解析序左到右；redir.c:135 `redirection_error` 经当时生效的 fd 2 报告）：诊断一次、落在重定向目标里。修复：brace 四个 stdio 臂改**解析序**处理（位置取自 `command.redirects`），`2>`/`2>>` 文件目标记为**失败报告目的地**（不做常驻 fd_table[2] 绑定——常驻绑定会被 walker 下游的 `1>&2` dup 看见，而 GNU 的 dup 按其**自身**解析位次当时的 fd 2 解析，issue699 R1.1 `{ … } 1>&2 2>g` 因此回归）；臂失败时诊断经该目的地（fresh append 打开——截断由臂的 create 一次完成，append 保证消息落 EOF，GNU 以单一共享 file description 建模）或无既定目的地时经 `write_default_stderr`（fd_table fd-2 端点，子壳内即子壳绑定）**单次**发送；walker 的 fd-2 文件绑定改 append 模式（fd 1 不动），组级诊断经它落 EOF 而非覆盖文件头；ast_exec 的 brace IoError 臂改为 `finish_brace_redirect_failure` 单点裁定（状态 1、ERR trap 单发、errexit 提升、&&/|| RHS 照走 skip_and_or_rhs），取消再分发。三票面 + ERR 单发 + errexit 终止 + `2>>` 捕获 + `2>/dev/null` 静默（含子壳形）+ 嵌套 brace 捕获 + 体不执行 + `&&` 短路共 12 形态与 WSL GNU 5.3.0(1) 逐字节一致；审计 probes `compound-redirect-fail-status`/`brace-redirect-err-route`/`subshell-redirect-diag-route` 语义全部转绿（残余分歧仅诊断 prolog 的脚本名 `$0` 渲染：rubash `/d/...` vs WSL GNU `/mnt/d/...`，环境命名域差异，非本票机制）。回归 `tests/issue826_compound_redirect_diag_route_regressions.rs`（12 例）。另注（#831 PR 复核轮残留面处置）：ast_exec 复合 IoError+errexit 臂绕过区域收口（CR round 2/3 的 ast_exec.rs:1593 面）已随 #831 合入 master；本 PR 在新 master 上实测 `set -E; trap … ERR; f() ( x=1; { :; } > /nonexistent/f )` 两形态（无 errexit GNU 三发=体/区域收口/调用、errexit 形静默 rc=1）与 GNU 逐字节一致且本修复前后不变。
- **alias 同读守卫改单调读取生成身份，跨 read 单元不再误抑制**（#817）：执行级 alias 同行可见性守卫拿定义时的物理行号当身份（`__RUBASH_ALIAS_LINE_<name>` vs `__RUBASH_CURRENT_LINE`，`arithmetic_aliases.rs:867-882`），而嵌套读单元会重排行号——source 的文件从 1 重新计数、eval 串按 `caller_line+i-1` 续号（`trap_exec.rs` execute_eval_source）——定义行与外层后续物理行相撞时合法使用被误报 `command not found` rc=127，GNU 展开（规则是同**读**不是同行，parse.y 读时展开）。修复：定义改记 `(读取代, 行)` 二元组（`__RUBASH_ALIAS_GEN_<name>`，`ShellState::read_generation` + 单调高水位 `read_generation_ceiling`，兄弟单元永不共享身份），生成边界正是现有 `CURRENT_LINE` 保存/恢复接合点——source 在 `builtins/source/execution.rs` 的 evalfile.c:253-257 push_stream 处 push、run_unwind_frame（evalfile.c:395）处 pop，eval 在 `execute_eval_source` 外壳 push/pop；子 shell 按 fork 边界克隆当前代（同一读单元）。GNU 两差分复现（stdin+source 与 eval 形态：修复前 rubash `line 5: x: command not found`/`line 6: y: command not found` rc=127，GNU `AX`/`AY` rc=0）转绿；i368 审计 9 行必保语义表（同行抑制 stdin/source 内/eval 内三形、N/N+1 展开、source/eval 非撞行展开、兄弟 source 不共享生成、`-i` 管道多提交、`-i -c` expand_aliases 默认开、`$LINENO` 累计、`-c` 分组驱动撞行布局）逐行与 WSL GNU 5.3.0 一致零回退；`unalias`/`unalias -a` 同步清理生成标记。回归 `tests/issue817_alias_read_generation_regressions.rs`（9 例）。
- **`set -E` ERR-trap 嵌套上下文语义：折叠压制、扁平区收口 fire、return 归因**（#720，PR #831）：三面均以 WSL GNU 5.3.0(1)（stdin oracle）逐字节钉死。① 嵌套子壳过火：`( (false) )` 曾 fire 3 次，GNU 2 次——execute_cmd.c:1799-1806 把整体为单个子壳命令的子壳体标 `CMD_NO_FORK`，execute_cmd.c:662-663 原位执行（无 make_child/wait_for），中间帧从不观察失败、其 :769 `run_error_trap` 永不运行；现以 `subshell_fork_collapse` 旗标圈住该形体（node 形 + `f() ( )` 扁平形两路），聚合点与 `set -e` 早转臂在其下跳过，非折叠中间层（`( (false); false )`、`( { (false); } )`）照常 fire。② 扁平函数体欠火：`f() ( false ); f` 曾 fire 2 次，GNU 3 次——子壳完成态的 ERR 裁定属于 wait 帧（execute_cmd.c:756-778，无 `&&/||` 豁免；`$LINENO`=子壳 `)` 行 execute_cmd.c:696；`BASH_COMMAND`=区文本，:703-710 fork preamble 定格 `the_printed_command_except_trap`）；解析器现给 opener 盖 `end_line`=闭括号行，每条区域关闭路径（含 and-or/brace/pipeline/主派发 catch-all 的早退）跑 `close_flat_region_err_adjudication`，`return N` 在区边界吸收为普通完成态（GNU 子进程带 N 退出，父进程继续）。③ return-in-until 归因：`until false; do f; done`（`f(){ return 7; }`）曾 `ERR:1`/exit 1——派发门以循环条件的遗留状态 fire 并吞掉 `Return`；现门让道（return.def:57-76 `return_builtin` 经 get_exitstat 解析 N 后 sh_longjmp return_catch，仅非法参数臂报错），函数边界落 `exit_code=N` 后由调用点门裁决，GNU `ERR:7`/exit 7。④ 同轮修出 and-or 非末位元素的 ERR 压制缺口（execute_cmd.c:2943 首元素恒 CMD_IGNORE_RETURN、:1000 拒绝 fire）：`g() ( false && true ); g` 的 `false` 曾误 fire，GNU 只 fire 区域与调用两点。17 例回归（`tests/issue720_err_trap_nested_context_regressions.rs` 12 + CR 轮 5）与 GNU 逐字节一致；goldens 74/74、issue797 8/8、全量双向对比零回归。同车 #713 二轮：`docs/audit/context-inheritance.md`、probes 24→45、新票 #825/#826。
- **活动控制台 `read` 三面：`-t` 门饿死、deadline 行读被 conhost 预煮劈半、onechar 缺 ICRNL**（#806，PR 于 `fix/806-read-conpty`）：① `wait_readable`（`src/fd/windows_impl.rs`）只 peek 队头 1 条记录并要求 key-down——行编辑提交 Enter 的 key-RELEASE 卡在队头时，其后可消费的 key-down 永远不被看见，`read -t 2 -n1` 烧满 deadline 返 142 变量空（GNU select 同形 0.2s 取到键，WSL 5.3.0 oracle）。现 peek 16 条一批，批内任一"带字符的 key-down"（ReadFile 能吐出的）即 Ready，peek 不消费次序不动（niubash#280 的"空队列永不 Ready"保护原样保留）。② console 上的 deadline 读改走 one-char 路（`read.def:570-590` ttfd_onechar；read.def:659 每字节 check_read_timeout；GNU 的 zread 从不预煮 tty 行，read.def:736）：cooked 行纪律与 raw 队列是两个缓冲，首次 cooked `ReadFile(1)` 把整行吸进 conhost 的 pending read 只吐首字节，门再 peek 只见空队列→烧满 deadline 返 `a="h"`，余下 `ello\r` 孤儿漏进下一个 reader（行被劈两半）。one-char 下每个键都是门可见的 raw 记录，deadline 切割点与 GNU select 对齐、行原子。有意保留并已文档化的平台分歧（cr-816 NIT-1）：部分行超时（输入无 Enter）时 GNU 的行纪律扣住字节——记录为空，下一个 Enter 完成后被扣行交给**下一个读者**（oracle `read b` ⇒ `B=[hello]`）；rubash 的 one-char 路已把已消费键记进本记录（`t-partial-assign` 腿 `A=[hello]`），下一读者只见超时后的新输入（`B=[zzz]`）——字节两侧都不丢，只是归属不同，非 read.def:553 逐字等价，模型说明见 `tests/interactive/support/conpty_read_probe.py` 的 `leg_t_partial_assign` 注释。③ one-char 读补 CR→NL 翻译（`shtty.c:115` tt_setonechar 强制 `ICRNL`："make sure we get CR->NL on input"）——ConPTY 在任何模式下都把 VK_RETURN 合成 CR 记录，不翻译则 `read -n1`+Enter 收到 `\r` 计数据、`read -t` 行读到 Enter 也不终止；管道/文件读保留裸 CR（GNU 非 tty 从不碰）。执行级回归进 `tests/interactive`（pywinpty 真 ConPTY、marker 载荷断言、无控制台宿主时整体 skip）：修复前 `t-key-in-window`/`t-line-complete`/`line-atomic`/`t-partial-assign` 四腿在 74e60b01 全红，修复后全绿；read 家族（#275/276/277/279/280/282/700/701/714）与 `--lib` 690 全绿。边界备注：票内"无 -t 的 `read -n1` 挂死 / 普通 read 丢首字节"两面复现在 niubash 宿主（粘贴签名 lookahead 在提交 Enter 处吃走 typeahead 存进程内 pushback，命令拥有终端期间引擎 read 不可达）——宿主侧补丁（submit 时把 pushback 写回控制台队列）已验证三腿转绿，随票附提案。
- **tilde 与 cd/pushd/popd 共用同一 $HOME 基准：GNU 缺失/空值之分**（#754）：path model 家族——`HOME=` 置空后 `cd ~` 曾静默落进 USERPROFILE（rc 0），同脚本裸 `cd` 却报 "HOME not set"；`$(cd && pwd)` 快路对置空 HOME 走 tilde 回退输出 USERPROFILE；`pushd ~` 空展开是无声成功、`pushd -n ~` 把 cwd 规范化后入栈（GNU 推原词，栈为 `<cwd> ""`）、`popd ~` 空词误报 invalid argument。GNU 规范（`lib/tilde/tilde.c:344-356` tilde_expand_word）：`~`/`~/` 恒按展开时点的活 cell `sh_get_env_value("HOME")` 粘接（`glue_prefix_and_suffix` tilde.c:304-317，空值是真值——`HOME=; cd ~` 即 `cd ""` 报 "null directory"，cd.def:331-336），仅变量缺失才落到回退链（_WIN32 APPDATA tilde.c:349-350；passwd `sh_get_home_dir` variables.c:736，getpwuid 固定于 uid、不随 `$HOME` 变化；rubash Windows 人设仍映射 USERPROFILE）。裸 `cd` 只拒**缺失**：cd.def:316-322 的裁定是 `dirname == 0`（POSIX cd step 1），置空 HOME 流入 make_absolute 粘上 cwd，同目录 chdir 成功 rc 0 无诊断（`cd ""` 空操作数才走 cd.def:331-336 null directory——两条路不同）。pushd 的 Dir 操作数委托 cd_builtin（pushd.def:296-302），空展开继承 null directory（prolog 报 pushd）；`-n` 跳过 cd，pushd.def:301 `add_dirstack_element (savestring (list->word->word))` 无条件推**原词**（无规范化、无存在性探测）；popd 与 pushd 同文件：pushd.def:318 `popd_builtin()` 选项遍历遇空词 break（pushd.def:356-363），取默认栈顶弹出。修复：tilde 读取改按变量存在性判定（空值展开为 `""`/`/x`），裸 `cd` 置空 HOME 原地成功 rc 0、缺失仍 "HOME not set"，comsub 快路置空委托真实 cd/pwd、缺失保留快路裁定，pushd `-n` 推原词且免探测、popd 空词按 pushd.def:356-363 处理。另注：#754 报告的 `cd ~` 用启动时 HOME 一节，在仓库 v1.4.0（f69e7900）与 e785c3c8 源码构建均不复现，属已安装 niu 1.4.0 二进制滞后（移交 #827）。HOME 变更矩阵 20 例（cd/`cd ~`/`mkdir ~/x`+`cd ~/x` 配对/无操作数 cd/子 shell/`command cd`/pushd/赋值/重定向/`[[ ]]`/case/export/相对 HOME/连续变更/`cd --`/eval 间接/函数/PATH 式赋值）与空 HOME 电池、无操作数两形态裁定、comsub 两形态、`pushd -n` 栈内容均与 WSL GNU 5.3.0 一致；回归 `tests/issue754_tilde_home_baseline_regressions.rs`（12 例）。
- **PROMPT_COMMAND 执行后 `$_` 按 GNU 括回**（#797）：bash-preexec 类资产 source 后，DEBUG trap 里的 `preexec "$_"` 拿到的是 PC 内部最后的命令词（`bp_install`/`bp_precmd_invoke`）而非用户上一条命令的尾词。GNU `execute_variable_command` 在 PC 的 parse_and_execute 外括 `save_lastarg`/`bind_lastarg`（`parse.y:3017`、`3022-3023`；`save_parser_state` 不携带 lastarg，`7248/:7250` 只管 `$?`/PIPESTATUS）；exit 跳转路径跳过恢复，与 `$?` 恢复同一规则。`bind_lastarg(NULL)` 绑空串（`execute_cmd.c:4192-4193`）。piped `-i` 最小 bash-preexec 形与 wezterm 全资产 transcript 与 WSL GNU 5.3.0 逐字节一致；回归 `tests/issue797_bash_preexec_debug_trap_regressions.rs`。
- **source 的 DEBUG trap 作用域改表驱动**（#797）：source 内安装的 DEBUG trap 曾在本次 source 余下命令上全部哑火（`-c`/脚本/交互三路同病）。GNU `source_builtin` 只快照+摘除入口 trap（`source.def:209-217` `restore_default_signal`），文件内新装的是活表状态照常触发（`trap.c:750-756`），unwind-protect `trap_if_untrapped`（`trap.c:737-743`）仅在退出时 DEBUG 无 trap 才归还入口 trap，且排在 RETURN trap（`evalfile.c:395`）之后。旧的 duration flag 对"入口本无 trap + 文件内新装"过宽。ignore 边界（CR R1/R2/R3）：`trap '' DEBUG`（IGNORE，trap.c:900 同置 SIG_TRAPPED）与 `trap - DEBUG` 按 trap 表成员资格（`__RUBASH_TRAPS`）区分而非 action 串内容——入口 ignore 不快照不摘除（`TRAP_STRING` 对 ignored 返回 NULL，trap.h:57-59），文件内 ignore 令 `trap_if_untrapped` 不归还入口 action。嵌套 source、`trap -p` 可见性、functrace、ignore 三形态均与 WSL GNU 5.3.0 逐字节一致；回归同上文件。
- **#798 判定面钉死：source 中 `exit 1` 语义与 spawn 失败 `$?` 与 GNU 等价**：LibreSSL testenc.sh 沙箱形态下 `cat openssl.cnf >p` 先截断后失败 → `cmp` 比的是两个空文件 → `exit 1` 本就不可达，47-cipher 循环跑穿是正确行为（WSL GNU 5.3.0 同样整循环，stdout 100 行/stderr 201 行 rc 0 逐字节一致；同机 rubash 4.5s < msys bash 5.2 5.3s，CI HANG 是 2 核 runner 8 并发下 ~400 次 spawn 的体量问题）。真实失败形态（cmp 差）首个失败即整体 rc 1 退出、交互会话同终止。回归 `tests/issue798_testenc_exit_in_source_regressions.rs`（hermetic：cmp/rm 用函数替身，钉 spawn-fail 127、空重定向目标、循环+if 形的 exit 终止）。
- **算术下标补上 `set -u` 检查（写路径）**（#639，PR #654）：`set -u; a[k]=v` 曾静默存储 rc 0。GNU 所有下标经 `evalexp → expr_streval` 统一设卡（`expr.c:1190-1216`）；写路径求值器补上与兄弟函数相同的 prologue，五处下游诊断在 flag 下静默， abort 走 FORCE_EOF 通道（`-c` 下 127、脚本 1），另用独立 cell 区分 DISCARD 以免状态错位。25 例差分电池 10/25→25/25；回归文件 `tests/issue639_nounset_subscript.rs`。
- **复合赋值 `[@]`/`[*]` 报整词拒绝而非算术错**（#636，PR #656）：`a+=( [@]=q )` 曾报 `@: arithmetic syntax error`，GNU 在算术求值前整词拒绝（`arrayfunc.c:786-792`，`cannot assign to non-numeric index`）。两处 compound 臂加精确匹配 guard（不 abort，走 GNU break 语义：保留部分存储、eval 可包含）。15 例差分电池 4/15→14/15（剩 declare-abort 例归 #635）；回归文件 `tests/issue636_atstar_compound.rs`。
- **`eval` 包含赋值 DISCARD 跳转**（#637，PR #657）：`eval 'a[-2]=q'` 曾结束整个脚本。GNU 在 eval 边界捕获 DISCARD（`evalstring.c:365,422-442`），eval 返回失败、调用者继续。新增 bad-subscript class marker，eval 边界精确吸收（算术/nounset 照常逃逸）；eval 内按行继续，返回末行状态。20 例差分电池 9/20→20/20；回归文件 `tests/issue637_eval_discard.rs`。
- **declare/local 越界负下标 abort 而非静默丢弃**（#635，PR #658）：`declare -a a=( [-2]=q )` 曾打印 AFTER rc 0。builtin 解析器区分无头/越界，扫描中断并保留部分存储，报错词经 out-param 上浮到 Executor 层做 fatal 诊断+abort（复用 #637 原语实现 eval 包含）。20 例差分电池 6/20→20/20；回归文件 `tests/issue635_builtin_oob.rs`。`export`/`readonly` 的同类 abort  threading 记作后续。
- **函数定义 stdin 优先于调用点 stdin**（#655，PR #684）：#653 把优先级翻转了（`f() { cat; } < deffile; f < callfile` 打印 call）。GNU 调用点在外、定义在内（`execute_cmd.c:5205`），定义装了活 fd 0 就赢。`part_054` 14/14；门禁与产品 A/B 零差异。
- **复合赋值字段分裂传输保留空字段**（#298，PR #762）：`IFS=.; A=($1)`（`a.b..b`）存了 3 个元素（`b" "`），GNU 存 4 个。`compound_field_split_transport` 对空字段发合成 tag+空格，重分只在空白 IFS 下认；现空体等下个定位 gap，用真实分隔符按序分发，空白 run 仍坍缩一边。`a.b..b`/`a::b`/`:a:` 全对 GNU；298 文件 12/12；GitHub CI 全绿（本地 master 腿 diff 因设备击杀循环未竟，见 PR 备注）。
- **issue358 探针目录沙盒化**（#358，PR #766）：`mkdir -p /tmp` 在 Android 失败→`cd`失败→`touch zzq` 落 repo 根（测试靠自身污染通过）。temp_dir pid 唯一目录，用后删除。14/14，无残留。纯测试改动（#763/#765 同类）。
- **fd 偏移测试经 PATH 解析外部 true**（#655 后续，PR #687）：硬编码 `/usr/bin/true` 在 Android（无 /usr/bin）上 127。测试时经 PATH 解析（绝对路径绕过 builtin 仍是外部 spawn），缺席则跳过。双测 Android 绿。纯测试改动。
- **prompt 渲染不再破坏读者的 `$?`/PIPESTATUS**（#604）：交互驱动（niu 原生控制台通道，`expand_prompt_string_mut`）渲染 `PS1`/`PS2`/`PS0` 时，内嵌 `$(false)` 等命令替换的退出状态此前经赋值展开引擎的替换边界写入 `exit_code`，下一个命令的 `$?` 读到 1；GNU 的 prompt 渲染是读侧操作，从不动读者的状态（WSL GNU Bash 5.3.0 piped-`-i` 探针：`PS1='$(false)'` 渲染后 `echo $?` 仍打印渲染前的值）。修复：渲染通道快照并在收尾恢复 `$?` 与 PIPESTATUS——赋值 RHS 通道（`v=$(false)` → rc=1 的 POSIX 赋值退出规则）不受影响。附带 #604 的通道 parity 回归与相对性能上限测试（PS1 渲染路径 vs PROMPT_COMMAND 路径的中位耗时比值上限），并按 profile 数据澄清该 issue 的主刀：四主题 rr 成本为外部 spawn 数 × Windows spawn 地板（每次渲染 4-6 次 git spawn；裸 `std::process::Command` 对同一 git.exe 需 208.9ms，rubash 会话内 217.6ms），两条引擎通道执行同一替换体为 parity（同进程 dev 计 47/49/68ms，release 会话内逐次 git 209.7/217.6ms）。

### 构建

- **发行来源戳记补 dirty 工作树摘要**（#827 建议 3）：`--version` 的 `build:` 指纹此前只铸 `git rev-parse --short HEAD`——哈希诚实但不担保内容，v1.4.0 事故正是"banner 报某 commit、二进制实际编译了该 commit 加未提交改动"（#488 家族盲区：哈希可见、工作树内容不可验证）。现携带未提交改动的树铸 `<rev>-dirty-<8 hex>`，digest 为 `git diff HEAD --binary` 的 SHA-256 前 4 字节（暂存/未暂存改动皆入；未跟踪文件不置 dirty，同 `git describe --dirty`/`diff-index --quiet HEAD` 语义）：同内容同 digest，任何改动必改 banner，两个不同脏树不再可能印出同一戳。`RUBASH_BUILD_HASH` 显式钉值不能洗白脏树——钉值由 crate 侧 `option_env!` 直读（rustc dep-info 自带失效跟踪，改钉即重编），build.rs 把 git 派生戳发在独立变量 `RUBASH_GIT_STAMP`、脏半段单发 `RUBASH_GIT_DIRTY_SUFFIX`；钉值只赢修订半段，仓库在场时 `-dirty[-<digest>]` 仍追加。build.rs 不发任何 `cargo:rerun-if-*` 指令——任何一条（含 `rerun-if-env-changed`）都会让 cargo 退出默认整包扫描，增量重建即把陈旧戳烙进已变更的树（PR #939 评审复现的 #827 事故形：脏树增量构建仍印裸 HEAD）。`tests/build_rs_dirty.rs` 7 例 fixture（净树裸哈希、脏树 digest 稳定且随内容变、已暂存算脏、未跟踪不算脏、脏后缀恰为戳尾、源码级禁 rerun-if 守卫、还原恢复裸戳），与 niubash `tests/build_rs_dirty.rs` 同源。
- **命令替换 pipeline-filter 死链删除**（#710）：`command_substitution_pipeline_filter` 的 8 条 filter 臂（sort/sed/tr/head/grep/wc/tail/uniq）连同其唯一调用链——`command_substitution_pipeline_output`、`timed_command_substitution_inner` 的 `|` 臂、`command_substitution_pipeline_first_stage`、孤儿助手 `split_pipeline_words`/`echo_raw_output`——自 rubash#117 平凡性 gate 之后不可达：gate 字节白名单拒绝 `|`，能抵达 filter 的替换体不可能含管线；且该链还只住在 `time` 前缀快捷方式内，进一步收窄。运行时复核：入口探针在 producer×filter 电池（含反引号、嵌套 comsub、alias 注入 `|`、`cat`/generic 首段、`time` 前缀形）上零命中；阳性对照（gate 临时放行 `|`）全部探针命中并复现休眠特征（`$(…|wc)` 默认单列 `1`，而活跃路径是三列）。`$( … | … )` 的可观测行为本就由真执行器管线（pipeline_exec）与外部二进制产出，删除零行为差；同步移除孤儿 doc-sync 测试与 `docs/builtins.md` 的 `substitution-fastpath-list` 机器块（审计 §2.5 加 resolution 注记）。新增 `tests/issue710_comsub_pipeline_reachability.rs` 钉住路由（wc 默认三列、echo 引导管线不吞、八 filter 走真执行器、time 前缀管线、末段状态语义），期望值与 WSL GNU 5.3.0 逐字节一致；全量失败集与基线双向一致。

## [1.4.2] - 2026-10-21

### 修复

- **env unbind 丢弃继承的进程 env 副本**（#917 rework，#943）：unbind 面残留清理。
- **read 对 fd-0 自 dup 走普通 fd-0 通道**（#975）。
- **`${...}` IFS-mark 复制臂使用 cook 自身的 dolbrace span**（#977）：防逐臂特判回归。
- Windows lane 补 `cargo test --lib`（#951），listing miss 以直查探针证明（#392 后续）。

### 变更

- **BASH_COMPAT 广告位跟随 vendored PATCHLEVEL 20**（#957/#895 链）。

### 性能

- expand 面：多字面量快路径镜像加固、walker 入口链去重、procsub 扫描白名单跳过；vartable P1a 脏写收口。

## [1.4.1] - 2026-10-19

### 修复

- **patsub 替换管道中的用户 PUA 码点 E400 转义**（#843）。
- **下标 ANSI-C 跨度的双引号态 + unset 操作数引号剥除**（#847）。
- **pushd/popd 继承 cd 的 readonly bindpwd 裁决并指名真实的越界词**（#844/#848）。
- **assoc 下标在 `((...))`/`let` 入口归一**（#828）。
- **复合命令自身的重定向失败诊断走 fd 表且只发一次**（#826）。
- `${var@P}` 命令替换状态括号、sudo 无 handler 路由钉死（#813 五项 CR 后续）。

### 变更

- crate 打包护栏：target 变体目录（如 target2-baseline，1.7GB 陈旧构建树）不得混入 cargo package（crates.io 上限 10MiB）。
- `(A;B;)` 杂散闭括号族全模式钉到 GNU 5.3.0（#845）。

## [1.4.0] - 2026-10-10

GNU Bash 5.3.0 兼容性大幅推进。83 套件 true-baseline 总差异从 3427 行降至 2072 行（−40%），零差套件从 31 增至 32。

### 新增

- **登录 shell 启动链**（#487，PR #489）：argv[0] 首字符 `-` 或 `-l`/`--login`（含 `-lc` 组合形态）置 login 标志后，Unix（`not(windows)` 门，覆盖 Linux 与 macOS）上登录 shell 依次 source /etc/profile → 首个存在的 ~/.bash_profile | ~/.bash_login | ~/.profile；非登录交互 shell 依次 source /etc/bash.bashrc（存在时）+ ~/.bashrc（--rcfile/--init-file 只替换个人 rc）；启动文件中 `exit` 终止 shell 且后续不执行（与 --rcfile 一致）。`--noprofile`/`--norc` 分别抑制对应分支。Windows 无真实登录概念：-l/--login 接受且行为与现状完全一致（不读任何链文件）。链顺序/fallback 以注入式存在性检查做单元测试；Linux/macOS 端到端由 CI 对应 job 验证。
- **真 readline REPL**（#419 readline 腿，PR #482）：裸 `rubash.exe` 真控制台交互补齐 readline 行编辑——此前控制台分支 stdin 留在 cooked 行输入模式，编辑由 conhost 代管，shell 历史（↑/↓、C-p/C-n）、行内光标移动（C-a/C-e/←/→）与 C-r 反向搜索全部失效。新增 `RawConsole`（`src/console_readline.rs`）：提示符与接受行之间切 raw 模式，按键逐事件翻译成引擎编辑分派器已有的字符流，编辑行每键重绘（raw 模式下终端不再回显）；命令执行前恢复 cooked 模式，子进程仍见正常行纪律控制台。编辑分派器从读循环抽出为 `apply_edit_char`，cooked 字节流路径（#434 桥接语义不变）与 raw 控制台路径共享同一套绑定；PS1/PS2（含 `--rcfile`）渲染共用。补全（Tab）与 i-search 的 raw 态渲染为后续分层。
- **`--version` 构建元数据**（#488，PR #490）：GNU license 块（保持逐字节不变）之后追加 `build: <git-hash> (<profile>)`——build.rs 编译期捕获 `git rev-parse --short HEAD`（git 不可用时回退 unknown，发布打包可 `RUBASH_BUILD_HASH` 钉值），profile 为 debug/release；Windows 附加一行 persona 说明：原生 Windows 构建、无 MSYS runtime，`*-pc-msys` MACHTYPE 不应读作 MSYS2/Cygwin bash 移植。
- **GNU `cp` 移植**：-r/-n/-i/-v/-p/-u/-t、递归、缓冲 stderr；`/bin/echo` 与 `/usr/bin/echo` 路由到缓冲 builtin 通道。
- **`history -d start-end` 范围删除**（GNU 5.3 特性）。
- **`stty` builtin** (PR #567): console mode queries and mutations without leaving the shell (`stty size`, `stty raw -echo`, ...), complementing the `read` fd-semantics work below.
- **`bash --help` / `sh --help` fork to the GNU usage contract** (#585, PR #596): the help screen follows the invocation name and prints GNU's usage contract — option table and exit status — instead of a product-generic help screen.

### 修复

- **命令替换的退出状态在展开期内即刻成为 `$?`**（#485，PR #491/#494）：GNU 在父壳回收替换子进程时就把其状态写入 last_command_exit_value，同一命令行内后置的 `$?`（后续词、同词后缀片段、同一简单命令的后续赋值 RHS）都看得见，成功的替换同样覆盖（`echo "$(cd nope && pwd)" $?` 报 1、`false; echo "$(true) $?"` 报 0、`v="$(exit 3)$?"` 得 v=3 且 rc=3），命令自身执行后照常覆盖。此前 rubash 只把状态记进 `last_command_substitution_status` 单元、仅在空词/赋值收尾时提升，词展开路径上丢失该时序。修复：AST/function 替换与 `&self` 包装器三个回落点的父侧回收处直接写 `exit_code`（fork 模型保存/恢复中的 `saved_exit_code` 不再回退状态），整词引号 `"$( )"` 快路径发布同一状态；反引号嵌词从遗留 `&self` 展开器改道可变嵌入 walker（与 `$()` 词共用生产路径与同一批提升点）。`local` 屏蔽赋值状态、`set -e` 下替换赋值失败中止（rc=1）而词内替换失败不中止（echo 自身 rc=0）均保持 GNU 一致。
- **函数定义与花括号组的同行 `#` 尾注释不再解析失败**（#118）：`f() { # note`、`f() { echo x; } # note`、`{ echo x; } # note`、`function f { … } # note`、TAB/空注释等形态此前被 lexer 把注释切进花括号组 token（`{ # note`），parser 因 `function_command.rs` 的 `value.trim() == "{"` 判定失败而回落成 `syntax error near unexpected token `('`。`skip_brace` 现在汇报配对 `}` 是否命中以及组内顶层注释起点，scanner 只把注释之前的文本作为 token 并把位置回退到注释处；`brace_close_can_end_compact_group` 也把 `}` 之后的行内 `#` 视为组结束。另修正 `skip_brace` 的 `comment_start` 初值，`f() {#note` 仍按 GNU 报语法错误而非静默接受。
- `set -u` 下算术展开不再把已赋值变量判为 unbound（#67）：nounset 扫描器去掉了无法识别错误 token 时的合成 "syntax error in expression" 回退，并跳过赋值左值（`for ((i=0; i<n; i++))` 的 init/update 不再报 `i` unbound）。
- `set -u` 下算术展开的 unbound 错误与普通参数展开对齐：直接上下文中终止脚本，命令替换与管道段内只终止该子上下文（GNU expr.c expr_streval 的 FORCE_EOF 语义），并消除了随后把展开文本当命令执行的 `command not found` 级联。
- 管道中未加引号变量作命令字现在按 IFS 分词（#68）：`v="echo hi there"; $v | cat` 以 `echo` 为命令名、`hi there` 为参数执行，管道任意段、子 shell 与进程替换内一致。
- 命令替换内的 `eval` 恢复完整重解析语义（#69）：`x=$(eval "echo hi")` 得到 `hi`；移除把 `eval ` 前缀剥掉后当普通命令执行的捷径，`$(eval "$cmd" | sort)` 等真实脚本形态正常工作。
- **dbg-support 全族归零**（635→0）：AND-列表双触发（execute_cmd.c 无 connection 节点火点）、source-scope trap 继承（source.def:208-216）、非行首 `{` 回归普通词法、for 每迭代行号重置。
- **rsh 全族归零**（194→0）：`set +o restricted` 静默解除修复（set 快速路径跳过 GNU 拒绝检查）、set +r 文案/退出码、BASH_CMDS 赋值守卫、hash -p、管线成员斜杠拒绝、受限只读集、argv0 rbash 自动受限。
- **invocation 全族归零**（14→0）：BASH_ARGV0 环境导入 $0/shell_name、login-shell argv0、长选项表、-o/-O 启动期报错 prolog、--pretty-print。
- **trap 归零**（3→0）：ERR action $LINENO 绑失败命令行、SIGCHLD 通知排队重放、后台子进程剥离继承 trap 表、traced 函数 DEBUG 行号用 body_open_line。
- **func 全族归零**（58→0）：POSIX funcname 规则、AST printer 移植、special-builtin 优先级。
- **complete 全族归零**（115→0）：多操作数 compspec 注册（`complete -F f c1 c2` 双双生效）。
- **history 改善**（190→127）：HISTIGNORE harness 修复、fc -s 语义对齐、命令替换内 fc/session 历史路由（`history -d start-end` 范围删除见新增）。
- **globstar 改善**（182→101）：非相邻多个 `**` 重复发射修复、相邻 `**` 折叠后零深度目录尾斜杠。
- **array/assoc 改善**（444+358→246+242）：复合赋值引号分组四层修复（解析器 RAW 合并、declare 操作数收集器、Word 臂原子复合、执行器逐字守卫）、元素赋值词边界、嵌套引号 patsub 逐元素替换。
- **unicode 修复**：printf 和 ANSI-C 引号中 `\u`/`\U` 部分读取保留、精确十六进制位数要求。
- **locale 子系统**：从 feat/locale-subsystem 合入，setlocale 警告、MB_STRLEN 长度。
- **comsub/pipeline**：pathname-expand 替换、管线 word list、assoc hash order 共享 join。
- **assignment**：全单引号 RHS 为字面数据、元素赋值词永不分词、转义引号内下标为数据。
- **nameref**：无值 nameref 接受有效目标赋值。
- **SIGPIPE 语义**：管道上游写端遭遇 broken pipe 不再中止解释器，`yes | head` 等流式管线行为与 GNU 一致（#455）。
- **输入规范化（粘贴语义）**：桥接层送达的交互式输入行在进入引擎前规范化 C0 控制字节（#458）——终端粘贴携带的 NUL/控制字节不再以未定义方式进入词法。
- **source 边界的语法错误不再终止会话/脚本**（#451，PR #481）：sourced 文件的语法错误按普通失败中止剩余文本（GNU evalstring.c 语义）——交互会话报错后继续读下一条，非交互脚本继续执行后续命令；此前中止被误分类为 reader exit jump，交互 shell 在诊断后立即结束（ConPTY 下表现为 source 后首条命令永不执行），非交互脚本以 rc 2 中止。
- **PS1 `\[`/`\]` 忽略标记在所有渲染路径剥离**（#431，PR #480）：oh-my-bash inretio / bash-it 主题形状下不再把标记渲染为字面文本或泄漏 raw `\x01`/`\x02` 字节；`${var@P}` 提示符变换同步剥离；真实 ESC 颜色序列与光标定位几何不受影响。
- **赋值前缀 + 子 shell 的 GNU 形态**（审计语料 #452 集群1，PR #453）。
- **`complete -F` 接受带连字符的函数名**（#427，PR #454）。
- **复合体内 `]]` / `(( ))` 之后无终止符的 then/do 被接受**（#460，PR #475）。
- **函数定义名含未引号 `$( )` 被接受**（#462，PR #476）。
- **`=~` 正则词片段永远是词数据，不再被误判为保留字**（#461，PR #473）。
- **嵌套 keyword-form 函数体计入组 opener 追踪，不再提前 EOF**（#465，PR #472）。
- **词附着的多行进程替换体跨行 join**（#463，PR #483）。
- **词首数据 raw ESC 字节（如单引号首字符）不再被吞**（niubash#200，PR #474）。
- **ERR trap 不再对词/赋值展开错误触发**（PR #467）。
- **Unix 上进程替换 fd 作为外部命令参数传递**（PR #468）。
- **coproc 子进程继承父函数表快照，DEBUG trap 函数可解析**（#441，PR #469）。
- **引号整词默认展开保持单一复合元素**（PR #466）。
- **Pipeline members now honor shell functions** — `date(){ ...; }; date | cat` runs the function, `echo(){ printf ...; }; echo hi | cat` shadows the builtin: two dispatch sites classified a pipeline stage before any function lookup, where GNU resolves functions before the PATH search in every stage (#587, PR #594).
- **A `PATH=` assignment prefix owns command resolution for its own command** — `PATH=/opt/bin:$PATH git status` resolves `git` through the new PATH within that same simple command, including the pipeline and concurrent fast-path stages (#588, PR #594).
- **`cat`/`mkdir`/`touch`/`cp`/`rm`/`rmdir` stop re-expanding already-expanded argv** (P0 pair) — `v='*.txt'; cat $v` silently read the wrong file and the mkdir/touch/cp/rm/rmdir family created or removed wrong paths; the builtins no longer re-run expansion over words the shell already expanded (#511, PR #552; #555, PR #558).
- **`$PWD`/`pwd` render the POSIX persona form unconditionally** — the native-drive-style hatch is retired; it starved nvm's find_up loop, and native drive spelling now survives only at the Win32 spawn boundary per the path contract (#537, PR #556).
- **GNU semantics waves** (each batch issue-locked and WSL-verified against GNU 5.3.0):
  - Wave 1 (PR #561): trap `exit` unwind, pipeline redirect rejection, nameref unbound gating, brace-expansion shape validation, redirect `set -u`, arithmetic latch (#524 #525 #529 #532 #535 #536).
  - Wave 2a (PR #560): SIGQUIT ignored in non-interactive shells, `-s` argument scan order, readonly attribute direction, `declare -na` structure wins, ERR single-fire guard.
  - Wave 2c (PR #573): RANDOM reseed on `declare -i`, heredoc-max fatal ordered before the unclosed-substitution scan, export/array/dynamic-variable/encoding semantics (#533 #534 #538-#543).
  - Wave 3 (PR #569): xtrace quote fidelity, redirect message rendering, `caller` frame limits, ERR trap function-entry, extdebug DEBUG-skip sites, `kill` signal names (#517-#522, #550).
  - Wave 4a (PR #576): redirect ambiguity gate, backtick diagnostic window, command-position paren, compgen tilde expansion (#514-#516, #523).
  - Wave 4b (PR #578): `$[ ]` EOF scan, `declare -aA`, procsub fd window, trap listing/DEBUG freeze, `bash -o` listing, jobs/alias parameter semantics (#543-#547, #549).
- **`read` console/fd semantics** — empty-line exit status, fd 0 file-loop ownership, carrier-byte domain normalization, dup-stdin console reads, fractional `-t` deadlines, and the dup-fd ladder behave like GNU (#275-#282, PR #567).
- **`-s` stdin-script reads on demand through the canonical library driver** — the parser consumes stdin one command at a time like GNU's non-seekable stream, so a `$(exec cat)`/`read`/top-level `exec cat` consumer sees exactly the bytes after the last submitted command and the unconsumed tail never executes as code (the `curl | bash -s` install-script hazard). The driver moved into `script_driver::run_stdin_script`, so embedding hosts get the same contract instead of slurping (#586, PR #593). Related: an exec'd child keeps the shell's stdin channel in every context (#575, PR #583).
- **CLI entry-shape semantics** — POSIX special-builtin/assignment fatals map per entry shape (`-c` exits 127 like GNU `run_one_command`, scripts/stdin keep 1, subshell-contained stays 1); the shim forwards the bash/rbash invocation faces so `$0` reports the invoked name (and `sh`/`ash` keep the POSIX-mode contract) (#589 #590 #591, PR #598; #504).
- **`eval` EOF-shape probe is comment-aware** — restores the modernish init battery (the #318 `unclosed_comsub_eof_shape` regression) (#282, PR #572).
- **Multiline `[[ ]]` with folded `(( ))` continuations parse like GNU**, the `bind -v`/inputrc engine surface is honored, and the #433 rc regression ledger is pinned (PR #571).
- **Arithmetic-for sections evaluate the verbatim `(( ))` capture** and carrier bytes never reach eval-error echoes (#548, PR #584).
- **Compound redirection failures reach the ERR trap again** (the body-exemption gate was type-blind, PR #557), and the redirect expansion-error verdict propagates on the function fast path (PR #579).
- **P1 semantic batch** — errexit body adjudication, stdin parse-error status, printf write-failure status, dirstack `EX_USAGE` (PR #553).
- **POSIX-mode coverage audit completed** — 42/75 audited GNU posix-mode behaviors implemented: fatal assignment families, redirect no-split, `time` fd-2 routing, trap usage exits (PR #559).
- **Diagnostic fidelity** — the C-family message formats match GNU: stdin line offsets, dolbrace scanning, PST_REGEXP two-stage EOF, outermost unclosed-block line naming, write-error prefix, and the `[[ =~` two-part diagnostic (#551, PR #563); a top-level unclosed procsub continues to the next line and `[ (` is reported as a diagnostic token (#564 #565, PR #577).
- **Windows path contract** — POSIX drive-form operand pre-translation now covers every non-shell-wrapped child class with context/position/existence invariance: `/d/...` operands work the same inside loops, subshells, functions, pipelines and command substitution, for to-be-created targets and flag values, while data operands (grep/sed/awk patterns, `basename`/`dirname`) stay verbatim (#592, PR #600); `/mnt/c` is retired from the verbatim loop per the #568 adjudication (PR #574); bare drive-root POSIX forms are accepted (#566, PR #577).
- 回归钉子（无用户可见行为变化）：sourced `exit N` 终止语义（PR #457）、函数定义花括号行 token 语义（PR #459）、#424 重定向目标拒绝与 procsub 输入侧（PR #464）。

### 性能

性能三连（#437/#281/#242）与 bats-core 热点（#375）推进：

- **#281 增量 reader 收官**（PR #486，closes #281）：续行扫描 admission-gate + 物理行二分戳、零拷贝参数体视图、逐行 batch-tokenizer 停靠（GNU read_token 流式模型），以及 fold 完成趟的 opener-state checkpoint——fold 趟只需支付组自身文本，不再从字节 0 重新 lex 整个累积缓冲（nvm 形状 4281 次全量趟中 19 次大重扫的 270 KB 归零）。
- **#437 热点**：算术循环 per-command boundary O(1) 化（数组追加与算术循环 micro-benchmark 报告的执行热点）。
- **#242 执行器热路径（部分）**：named-event catch_flag 信号轮询、单遍 `${}` 算子索引 + 直接 simple-glob 匹配 + 算术快照容量、walker `${}` 臂 inline fragment 解析与模式快路径、allocation-free shopt 咨询、字面词快路径、comsub-scan O(n²) 修复等。
- **#375 第一刀（部分）**：`$(cd X && pwd)` 命令替换路径解析单句柄化——一次 CreateFileW 同时取目录属性 + 物理路径（PR #484）；`$(cd && pwd)×5000` 878→779 ms（−11%），bats_debug_trap 形状 −8%，输出逐位等价。
- **perf-suite**：专用基准 harness 与首个基线（24 probes，rubash vs WSL GNU 5.3.0）。
- **Lexer construction wins**（rubash#241，PR #554）: fold-before-push moves, gated leading_ws capture, and inline TokStr strings — parse throughput up under alias-correct ground truth.
- **Executor dispatch floor**（rubash#375，PR #562）: shape-only cd/pwd command-substitution admission and a DEBUG-trap parse-once cache retire the handle-table scale myth.
- **Hot-path construction-only wins**（rubash#242，PR #501）: test argv zero-copy, gated fd marks, single word materialization, copy-on-write IFS strip, cached debug switches.

### 平台

- **Android 目标**：aarch64/armv7 android 目标可针对 bionic 编译（#456，`sa_restorer` 平台门修正）；kill/trap 信号表在 android 上同样编译 Linux 编号的 1..31 槽位（bionic 与 Linux 内核编号逐值一致）——此前 `kill -l 17` 在 android 上输出为空、`trap -l` 列出 bionic 不用的 BSD 名；RT 字面量块保持 glibc 契约的 Linux 专属（PR #492）。
- **平台门全量审计**（PR #492）：src/ 下 511 处平台 cfg 盘点分类；确认无「漏 macOS 腿」同类问题，真 Linux 专属门（RT 34..64 字面量、`sa_restorer` 等）保留。
- **OpenHarmony / musl**：`ulimit` 针对 musl 与 OpenHarmony libc 编译修复（PR #449）。
- **信号编号统一**：rubash 全表从 BSD/Cygwin 风格切换到 Linux 表（USR1=10、CHLD=17、RTMIN=34），与 GNU 5.3.0 WSL 契约一致；kill -l/trap -l 输出逐字节对齐。
- Residual Windows assumptions are gated outside `cfg(windows)` so non-Windows builds stay honest (PR #499).

### 构建

- **crates.io 打包瘦身**：发布包不再携带 `tests/` 与 `third_party/`（参考源码与测试语料，仅本地开发/CI 需要），包内容 2053 个文件 / 15.6 MB → 400 个文件 / 8.9 MB，回到 crates.io 10 MiB 上限之内；`exclude` 同步移除失效的 `!third_party/bash/tests/*.right` 反规则。功能与构建不受影响（`cargo publish --dry-run` 验证通过）。
- Internal: twelve ghost module skeletons that never entered the compile tree are removed (PR #597); stale regression snapshots are re-baselined with an airtight snapshot env and a CI golden-refresh artifact (PR #496).

### 文档

- 重写 README.md 和 README.zh-CN.md：数据驱动的兼容性展示、近期修复表、架构概览；其后一次事实保持的风格微调（PR #495）。
- 更新 COMPATIBILITY-STATUS.md：2026-09-11 全量 true-baseline 重跑结果、总体结论刷新。
- 归档过时文档 7 个（gnu-bash-compatibility-implementation-plan、bash-implementation-inventory、bash-source-map、performance-debugging-process、typed-expansion-migration-checkpoint、source-layout、HANDOFF-20260829）。
- README restructured again, evidence-first, with the cross-platform identity (EN+zh) (PR #500); the three boundary invariants — native core, POSIX forms, verbatim fidelity — are codified as contract tests plus a windows-path-contract document and proxy-driver guide (PR #570).
- Platform gap documented: foreign children cannot receive exec-established numbered fds (#378, PR #580).
- Baseline numbers (PERF/CORPUS/COMPAT/LINUX) extracted from archived status ledgers (#477, PR #581); the debugger M0 design (DAP-over-stdio) is published (#497, PR #582); dangling COMPATIBILITY-STATUS references are repointed to the archived ledger (PR #502); the GNU suite ledger closes at 83/83 with the nameref residual proven to be GNU's own coproc reap race (PR #506).

## [0.3.0] - 2026-08-22

本版本将项目版本、兼容性证据和近期 Bash 语义修复同步到当前源码状态。

### 修复

- 修正关联数组算术下标的引号处理；带引号的 `$()` 和算术下标不再被错误地预展开。
- 改进数组元素下标诊断，对空、`*` 和负下标报告与 Bash 一致的错误，同时保留参数展开的非致命行为。
- 修正参数展开扫描器对单引号内 `}` 的处理，并拒绝 Bash 不支持的空格包围命令替换形式。
- 修正算术条件错误 token 的渲染，以及 `enable -d` 无效 builtin 的使用状态和退出码。
- 保留进程替换、coproc、printf 溢出和 extglob 解析等近期修复的回归覆盖。

### 文档与验证

- 发布当前兼容性归因账本，区分真实 Bash 输出差异、宿主/fixture 问题和历史 runner 期望文件。
- 更新 Windows-native 路径、bashdb 调试边界及 GNU Bash 源码语义映射说明。
- 明确 `cargo` 包版本为 `0.3.0`；上游 `run-*` 的 `87/87` 仅表示旧 `.right` 期望通过，不代表真实输出已经 83/83 对齐。

### 构建

- CI 在推送 `vN.N.0` 发布 tag 后，会等待测试通过并自动创建 GitHub Release。
- Release workflow 支持在 Actions 页面手动输入发布 tag 触发，并会校验 tag、checkout 提交和 Cargo 版本一致。

### 修复

- alias 展开为 `if ...; then`、`while ...; do`、`for ...; do` 等复合语句前缀时，会继续拼接到匹配的 `fi`/`done` 后重解析执行，补齐更接近 Bash 的 parser-level alias 行为。
- 赋值 RHS 紧邻 `<(...)` 进程替换时会作为赋值值解析并物化为可读路径，补齐 `p=<(cmd); cat "$p"` 这类 Bash 语法框架。
- 赋值 RHS 中的 `>(...)` 输出进程替换会登记为可写路径，并在后续重定向写入该路径后执行替换命令。
- `[[ -e <(...) ]]`、`[[ -p >(...) ]]` 等条件文件测试会识别进程替换操作数的 pipe/fd 形状，补齐条件命令中的进程替换文件测试框架。
- `for`/`select` 词列表中的 `<(...)`/`>(...)` 会先物化为可读/可写路径，再作为循环变量值使用。
- `case` 匹配词中的 `<(...)`/`>(...)` 会先物化为路径，再参与 pattern 匹配。
- 同一 word 中混合 `$((...))`/`$[...]` 算术展开和参数展开时，按 Bash 的从左到右顺序保留自增等副作用。
- `test -v` 与 `[[ -v ... ]]` 会识别 `RANDOM`、`SECONDS`、`BASH_COMMAND` 等动态 Bash 参数的已设置状态。
- Windows/Git Bash 环境下 `test -O`、`test -G` 与 `[[ -O/-G path ]]` 对当前用户创建的现有路径返回成功，补齐所有权条件 unary 的跨平台框架。
- `case` pattern 中被引号保护的 `*`、`?`、`[` 等 glob 元字符会按字面量匹配，变量在双引号 pattern 中展开后也保持字面匹配语义。
- 关联数组 key 含 `]` 时会在内部存储和 `declare -p` 输出中强制加引号，避免 `declare -A a=([x] one)` 这类交替 key/value 语法把字面 key `"[x]"` 误解析为下标 `x`。
- alias 引入复合语句前缀时，匹配 `fi`/`done`/`esac` 会按嵌套复合语句深度处理，避免内层 `if` 或循环提前截断外层 alias 语句。
- alias 引入 `time` 前缀并计时复合命令时，同样按嵌套深度收集被计时的 `if`/循环主体，避免内层结束词提前截断 `time` 语句。
- alias 引入 `case` 时，case clause body 边界会跳过内层 `case ... esac`，避免内层 `;;`/`esac` 提前截断外层 clause。
- alias 值提供 `function name` 前缀、函数体由后续 `if` 或循环复合命令提供时，会继续收集到匹配结束词后重解析为函数定义。
- 输出方向进程替换作为普通参数传给管道外部命令阶段或函数调用时，会物化为可写路径，并在阶段/函数结束后执行替换命令。
- 管道中的复合命令和函数阶段改为子 shell 风格执行，避免 `cd`、函数定义和算术赋值等状态泄漏到外层 shell。
- `builtin` 命令统一走完整内建分发表，避免无重定向调用时漏掉 `let`、`read`、`mapfile`、job control 等已接入内建命令。
- `shopt -s lastpipe` 开启后，管道最后一段可在当前 shell 执行，使 `read` 和 `while read` 等 final stage 状态更新保留到外层。
- `coproc` 默认管道的父子端接线修正，并将 coproc 子进程纳入 `wait` 可识别的作业表，避免 brace body 在 Windows 上出现 stdio 访问错误。
- 反引号命令替换外部的转义反引号会保留为字面量，反引号命令替换内部的转义反引号分隔符可用于嵌套命令替换。
- 命令替换内部的简单 word splitting 会将嵌套 `$()` 保持为同一个 word，避免 `$(echo $(echo nested))` 被内部空格拆碎。
- `printf -v` 支持将格式化结果写入 indexed array 和 associative array 的元素目标。
- `printf -v arr[subscript]` 的 indexed array 目标会按 Bash 算术规则解析 subscript，并支持已有数组的负下标。
- `compgen -W` 支持从 wordlist 生成匹配候选，并接入 `-P`/`-S` 前后缀和无匹配返回状态。
- `compgen -V varname` 支持将生成的候选写入 indexed array 变量而不是输出到 stdout。
- `compgen -A builtin` 和 `compgen -A keyword` 会生成内建命令/保留字候选，并支持 prefix 过滤和 `-P`/`-S` 包装。
- `compgen -b` 和 `compgen -k` 会分别按 Bash 的短 action 形式生成内建命令与保留字候选。
- `compgen -A signal` 会复用当前 `trap`/`kill` 支持的信号名称表生成候选。
- `compgen -A shopt` 和 `compgen -A setopt` 会复用当前支持的 `shopt` / `set -o` 名称表生成候选。
- `compgen -G` 会展开 glob pattern 生成路径候选，并支持 `-P`/`-S` 包装和无匹配状态。
- `compgen -X` 会过滤已生成候选，支持 `!pattern` 反向过滤，并保留 Bash 的候选生成状态语义。
- `compgen -d` 和 `compgen -A directory` 会从文件系统生成目录候选，并支持 prefix 过滤、`-P`/`-S` 包装和 `-X` 过滤。
- `compgen -f` 和 `compgen -A file` 会从文件系统生成文件名候选，包含普通文件和目录，并支持现有候选过滤与包装流程。
- `compgen -A helptopic` 会复用 `help` 主题表生成帮助主题候选。
- `compgen -A enabled` 会生成当前支持且未被禁用的启用内建命令候选。
- `compgen -A disabled` 会基于 `enable -n` 状态生成禁用内建命令候选。
- `compgen -j` 和 `compgen -A job` 会基于当前后台 job 表生成 job 命令候选。
- `compgen -A running` 会基于当前后台 job 表生成运行中 job 命令候选。
- `compgen -A stopped` 接入停止状态 job 候选框架，当前无 stopped job 状态时保持空成功。
- `compgen -A hostname` 会基于当前 shell 的 `HOSTNAME`/`COMPUTERNAME` 生成主机名候选。
- `compgen -A binding` 会基于 GNU Readline 默认函数名表生成 binding 候选。
- `compgen -u` 和 `compgen -A user` 会基于当前 shell 的 `USER`/`LOGNAME`/`USERNAME` 生成用户候选。
- `compgen -g` 和 `compgen -A group` 会基于当前 shell 的 `GROUP`/`GROUPNAME`/`USERDOMAIN` 生成用户组候选。
- `compgen -s` 和 `compgen -A service` 会基于当前 shell 的 `SERVICE`/`SERVICENAME` 服务名状态生成服务候选。
- `compgen -v` 和 `compgen -A variable` 会基于当前 shell 变量表生成变量名候选。
- `compgen -a` 和 `compgen -A alias` 会基于当前 shell alias 表生成别名候选。
- `compgen -A function` 会基于当前 shell 函数表生成函数名候选。
- `compgen -c` 和 `compgen -A command` 会合并内建命令、保留字、alias、函数和 `PATH` 文件生成命令名候选。
- `compgen -A arrayvar` 会基于当前 shell 的 indexed/associative array 标记生成数组变量候选。
- `compgen -e` 和 `compgen -A export` 会基于当前 shell 的 exported 变量标记生成导出变量候选。
- `compgen -A readonly` 会基于当前 shell 的 readonly 变量标记生成只读变量候选。

### 测试

- 增加 alias 值内含复合语句控制词和分号的 `if`、`while`、`for` 回归覆盖。
- 增加赋值 RHS 中独立和嵌入式输入进程替换的回归覆盖。
- 增加赋值 RHS 中独立和嵌入式输出进程替换的回归覆盖。
- 增加 `[[ ... ]]` 条件文件测试中输入/输出进程替换操作数的回归覆盖。
- 增加 `for`/`select` 词列表中输入/输出进程替换的回归覆盖。
- 增加 `case` 匹配词中输入进程替换的回归覆盖。
- 增加同一 word 中算术展开与参数展开交错时序的回归覆盖。
- 增加 `test -v` 与 `[[ -v ... ]]` 检测动态 Bash 参数的回归覆盖。
- 将 `test -O/-G` 与 `[[ -O/-G ... ]]` 的现有路径覆盖对齐 Windows/Git Bash 行为。
- 增加 `case` 引号 pattern 与双引号变量 pattern 的字面匹配回归覆盖。
- 增加关联数组交替 key/value compound assignment 和普通元素赋值中 bracket key 的回归覆盖。
- 增加 alias 引入的外层 `if`/`for` 内嵌套 `if`/`while` 的结束词匹配回归覆盖。
- 增加 alias 引入 `time` 前缀后计时嵌套 `if` 和嵌套循环的回归覆盖。
- 增加 alias 引入 `case` 后 clause body 内嵌套 `case` 的回归覆盖。
- 增加 alias 引入 `function name` 前缀后接 `if` 和 `for` 函数体的回归覆盖。
- 增加 `tee >(cat ...)` 管道阶段和函数参数中的输出进程替换回归覆盖。
- 增加管道 brace group 与函数阶段的工作目录隔离覆盖，并将函数定义、算术命令 pipeline 阶段预期对齐 Bash。
- 增加 `lastpipe` final `read`、final `while read`、final 赋值和 `PIPESTATUS` 状态覆盖。
- 增加 `coproc NAME { ...; }` 默认 stdout pipe 与 `wait $NAME_PID` 的回归覆盖。
- 增加转义字面反引号和嵌套反引号命令替换的回归覆盖。
- 增加嵌套 `$()` 命令替换保持完整 word 的回归覆盖。
- 增加 `printf -v arr[index]` 和 `printf -v assoc[key]` 的回归覆盖。
- 增加 `printf -v` indexed array 算术下标和负下标目标的回归覆盖。
- 增加 `compgen -W` prefix 过滤、`-P`/`-S` 输出和无匹配状态的回归覆盖。
- 增加 `compgen -V varname` 数组写入和 stdout 抑制的回归覆盖。
- 增加 `compgen -A builtin` 和 `compgen -A keyword` 候选输出、过滤和无匹配状态的回归覆盖。
- 增加 `compgen -b` 和 `compgen -k` 短 action 候选输出与无匹配状态的回归覆盖。
- 增加 `compgen -A signal` 信号候选输出与无匹配状态的回归覆盖。
- 增加 `compgen -A shopt` 和 `compgen -A setopt` 候选输出与无匹配状态的回归覆盖。
- 增加 `compgen -G` glob 候选输出、`-P`/`-S` 包装和无匹配状态的回归覆盖。
- 增加 `compgen -X` 普通过滤、反向过滤和全过滤状态的回归覆盖。
- 增加 `compgen -d` 和 `compgen -A directory` 的目录候选、过滤与包装回归覆盖。
- 增加 `compgen -f` 和 `compgen -A file` 的文件名候选、过滤与包装回归覆盖。
- 增加 `compgen -A helptopic` 候选输出与无匹配状态的回归覆盖。
- 增加 `compgen -A enabled` 候选输出与无匹配状态的回归覆盖。
- 增加 `compgen -A disabled` 禁用内建命令候选和 `compgen -A enabled` 排除禁用项的回归覆盖。
- 增加 `compgen -j` 和 `compgen -A job` 后台 job 候选、过滤与包装回归覆盖。
- 增加 `compgen -A running` 运行中 job 候选、过滤与包装回归覆盖。
- 增加 `compgen -A stopped` 无停止状态 job 时空输出成功的回归覆盖。
- 增加 `compgen -A hostname` 主机名候选、过滤与包装回归覆盖。
- 增加 `compgen -A binding` readline 函数候选、过滤与包装回归覆盖。
- 增加 `compgen -u` 和 `compgen -A user` 用户候选、过滤与包装回归覆盖。
- 增加 `compgen -g` 和 `compgen -A group` 用户组候选、过滤与包装回归覆盖。
- 增加 `compgen -s` 和 `compgen -A service` 服务候选、过滤与包装回归覆盖。
- 增加 `compgen -v` 和 `compgen -A variable` 的变量候选、过滤与包装回归覆盖。
- 增加 `compgen -a` 和 `compgen -A alias` 的别名候选、过滤与包装回归覆盖。
- 增加 `compgen -A function` 的函数名候选、过滤与包装回归覆盖。
- 增加 `compgen -c` 和 `compgen -A command` 的命令候选、过滤与包装回归覆盖。
- 增加 `compgen -A arrayvar` 的数组变量候选、过滤与包装回归覆盖。
- 增加 `compgen -e` 和 `compgen -A export` 的导出变量候选、过滤与包装回归覆盖。
- 增加 `compgen -A readonly` 的只读变量候选、过滤与包装回归覆盖。

## [0.2.0] - 2026-07-17

### 新增

- 补齐复合数组赋值中的未引用命令替换拆词行为，并在拆出的字段上继续执行
  pathname expansion。
- 补齐 `arr=($var)` 这类未引用参数复合数组赋值字段的 pathname expansion。
- 复合数组赋值普通元素支持 brace expansion，并遵守 `set +B` 对 brace expansion
  的关闭状态。

### 修复

- quoted parameter expansion 后不再误触发 pathname expansion，行为更接近 Bash。

- Linux/Unix 上执行无 shebang 外部脚本时，遇到 exec format error 会通过 shell
  回退执行，行为更接近 Bash。
- Unix shell 回退不再完全依赖被脚本或测试改写过的 `PATH`，会兜底查找标准
  `/bin/sh`、`/usr/bin/sh`、`/bin/bash` 和 `/usr/bin/bash`。

### 测试

- 统一测试中临时外部命令的写入逻辑，在 Unix 上自动设置可执行权限，修复
  Linux CI 中禁用 builtin 后走外部命令相关用例返回 `126` 的问题。
- GNU Bash upstream runner 本地基线更新为 `87/87` 通过。
- 将 `run-minimal` 纳入默认 upstream runner 集合。
- 收敛 upstream bridge 的重复输出逻辑，降低后续维护成本。

### 文档

- 更新 README 中的当前实现状态、builtins 覆盖、测试规模和代码结构说明。
- 更新 README 中的功能进度，说明当前实现已超过早期骨架阶段，重点转向 Bash
  兼容细节和上游测试覆盖。
- 更新 README 和 GNU Bash upstream 测试文档中的测试进度、运行命令和更新时间。

## [0.1.1] - 2024-06-11

### 增强执行器

#### 已完成

##### 执行器
- 新增内建命令: `env`, `set`, `unset`, `test`, `[`
- 添加 `redirect_err_append` 字段支持 `2>>` 重定向
- 使用 match 语句简化代码，替代函数指针

##### 测试
- 扩展执行器测试: 4 → 18 个测试
- 新增环境变量测试
- 新增命令链接测试
- 新增内建命令测试

## [0.1.0] - 2024-06-11

### 首次发布

这是一个重要的里程碑，完成了 Shell 的核心功能。

#### 已完成

##### 词法分析器 (Lexer)
- 基础分词 (单词、符号、关键字)
- 运算符识别 (`|`, `&`, `;`, `<`, `>`)
- 引号处理 (`'`, `"`)
- 变量识别 (`$VAR`, `${VAR}`)
- 命令替换 (`` `cmd` ``, `$(cmd)`)
- 花括号展开 (`{1..5}`, `{a,b,c}`)
- 注释处理 (`#`)
- 转义字符处理 (`\`)

##### 解析器 (Parser)
- AST 生成
- 简单命令解析
- 管道解析
- 分号分隔命令
- 赋值语句解析
- 重定向解析

##### 执行器 (Executor)
- 内建命令: `exit`, `echo`, `pwd`, `cd`, `export`, `true`, `false`
- 外部命令执行
- I/O 重定向支持
- 退出码处理

#### 测试

- 词法分析器测试: 33 个
- 解析器测试: 13 个
- 执行器测试: 4 个
- 单元测试: 8 个
- **总计: 58 个测试**

#### 重构

- 使用 Rust 2021 新特性 (`matches!`, `let...else`)
- 代码优化: 减少约 48% 行数
- 添加 `#[inline]` 优化

#### 文档

- README.md
- CONTRIBUTING.md
- CODE_OF_CONDUCT.md
- LICENSE (GPL-3.0)

### 待完成

- [ ] 变量展开 (`$VAR`, `${VAR}`)
- [ ] 控制流 (`if`, `while`, `for`, `case`)
- [ ] 管道实现 (真正的进程间通信)
- [ ] 函数定义
- [ ] 作业控制
- [ ] 命令历史
- [ ] 更多内建命令 (`read`, `printf`)

### 已知问题

- 管道尚未实现真正的进程间通信
- 变量展开尚未实现
- 不支持控制流语句

---

## 版本命名规则

- 主版本: 不兼容的 API 更改
- 次版本: 向后兼容的新功能
- 修订版本: 向后兼容的 bug 修复

## 链接

- [unixwin.dev](https://unixwin.dev) — 产品门户（文档与下载统一入口）
- [GitHub Releases](https://github.com/unixwin/rubash/releases)
- [问题跟踪器](https://github.com/unixwin/rubash/issues)
