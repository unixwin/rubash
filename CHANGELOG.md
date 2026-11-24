# 更新日志

所有重要的项目更改都将记录在此文件中。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)。

## [Unreleased]

## [1.5.2] - 2026-11-24

### 修复

- **GNU 差分修复波（2026-11-22 – 11-24）**：窗口内本仓共关闭 48 单，其中 8 单（#2810、#2828、#3965、#4001、#4002、#4059、#4060、#4080）已列入 1.5.1 清单、不重复计入，本版新增 40 单（四仓同期共关闭 61 单、合并 PR 91 张；四仓本版新增合计 51 单），全部以 GNU Bash 5.3.20 为规范源、经真 oracle 逐形差分定谳后修复，每条均带以 issue 号命名的回归钉。按主题机械归类（每单只计一族）：fd/重定向 6、引号与词法 1、执行与子进程 1、declare/readonly/local 2、算术 3、参数展开 2、作业控制与 wait 1、case 1、补全 1、heredoc/herestring 5、数组/assoc 2、`[[ ]]` 2、procsub 6、trap 1、其他 6，合计 40（完整清单见下）。冻结头为 master `ce1eb288`（外部贡献者 jhermelink 的 #4171 合入点）。
- **推进波要点**：外部报告面大部全清——ShiomiHC 第二批八单（#4125、#4127–#4133）与 xuangeyouneihan 的 #4138、#4149 均已修复关单（同批仅余 #4126 待作者返工）。其中 #4149 把 #684 的控制台行规程推广到进程内模拟 `cat` 的三个 stdin 读者（共享纯裁决单元 `veof_verdict` 与 `ConsoleLineDiscipline` 状态机，`read` 的两处内联裁决同步迁入、无第二走法）；#4158 让进程内 `eval` 经普通重定向计划安装输入侧（绑定点在内建分发层，避开 `mapfile -C` 的 #3660 共享流绑定）；#4159 在两处算术求值点结算 DISCARD 类重定向展开失败的 latch；#4171（jhermelink）剥除进程内管道阶段操作数的 blank guard 泄漏，与简单命令路径的物化契约对齐。本波残差另立 #4166、#4170、#4172、#4175 跟踪。

### 完整清单

窗口（2026-11-22 – 11-24）内关闭、本版新增的 40 单按主题机械归类如下（每单只计一族，标题逐字取自 issue 原文）：

#### fd/重定向（6 单）

- #4099 重定向词内 :? 参数错误对普通命令被误判整脚本致命（GNU 只判命令失败续行）
- #4131 A builtin's `> file` stays open after the builtin returns, so a pipeline started right after cannot open the file with read-only sharing
- #4146 Failed expansion with 2>/dev/null leaks fd 2 to /dev/null for the rest of the script
- #4151 Redirecting, `touch`ing or `cp`ing to `/dev/tty`, `/dev/stdin`, `/dev/stdout` or `/dev/stderr` creates regular `CON`, `CONIN$`, `CONOUT$` files in the current directory
- #4158 eval's own input redirections are silently dropped (output side fine)
- #4159 A DISCARD-class redirect expansion failure silently swallows the NEXT command in the list

#### 引号与词法（1 单）

- #4132 [gnu-baseline] Sourced file: backquote body containing `<<` misroutes a later `exit` to the parent script (residual of #3962/#4018)

#### 执行与子进程（1 单）

- #4149 行内Ctrl-D把提交行切开：子进程收到EOF，后半段被shell执行，提示符也不换行

#### declare/readonly/local（2 单）

- #3970 [bug] declare 家族复合赋值不做 brace 展开：declare e=({1..3}) 只存 1 个元素（GNU 3）
- #4123 [gnu-baseline] declare/local compound assignment to readonly in function: missing function-named first diagnostic (follow-up to #2658)

#### 算术（3 单）

- #2826 [perf] 算术求值缺通用编译缓存：每求值新建 parser，现以三条形状 fastpath 代替
- #4090 [longtail G5] errexit: arithmetic expansion error must DISCARD the command and continue (assignment / echo / [[ ]] faces)
- #4130 Assignment: `$(( … $(cmd) … ))` is stored as text when the RHS is quoted or not a lone `$(( ))`

#### 参数展开（2 单）

- #4004 [gnu-baseline] double-quoted ${} span boundary: quoted } inside default ends span early, filename diverges from GNU
- #4068 [gnu-baseline] bad-substitution pre-scan discard skips word-internal comsub side effects

#### 作业控制与 wait（1 单）

- #4097 wait after STOP+CONT+KILL must report 147 silently (currently 137 plus a Killed notice)

#### case（1 单）

- #4031 [gnu-baseline] case 体在 $( 嵌套 placement 下被 case-pattern tracker 误读（#4026 钉文件自承残差）

#### 补全（1 单）

- #4101 bind -x widget 只 unset READLINE_LINE 时 READLINE_POINT 被钳制到 0（应保持原值）

#### heredoc/herestring（5 单）

- #4093 herestring to an external command: ${var:=word} in the herestring must not persist in the parent (child-context expansion)
- #4128 `"$( … )"`: a quoted `<<X` spanning lines appends 0x1c to the last word before `)`
- #4138 #1025 落下了heredoc
- #4140 Unterminated-heredoc EOF warning is deduplicated across distinct commands in one script
- #4141 Quoted comsub with piped heredoc leaks the delimiter line (with 0x1c) into output

#### 数组/assoc（2 单）

- #4098 数组元素赋值不应用重定向（入向静默 rc=0、出向不建文件）
- #4102 导入 GROUPS 的元素面仍走 live 组数组（${GROUPS[0]} / GROUPS[0]=e）

#### `[[ ]]`（2 单）

- #4125 `[[ =~ ]]`: `BASH_REMATCH` drops or rewrites the bytes 0x11, 0x17, 0x18, 0x1a, 0x1c, 0x1f
- #4133 [gnu-baseline] [[ =~ ]] under LC_ALL=C re-encodes subject bytes 0x80-0xff as UTF-8 (split from #4125)

#### procsub（6 单）

- #3094 [design] process substitution yields /dev/fd/N: enumerated cases of where that path resolves and where it does not
- #4092 [longtail G8] compound procsub diagnostic reports the compound's own line instead of the ambient frame line
- #4103 /dev/fd procsub 管道同秒内建时 -nt/-ot 视 mtime 有严格序（GNU 视其相等）
- #4127 Windows: inside `<(…)`, `>(…)` and `coproc`, variables whose names differ only in case become one variable
- #4129 `>(…)`: the consumer ignores the fd 1 / fd 2 redirections around the command — its output escapes `$( )`, `{ …; } > file`, `2> file` and `exec >`
- #4135 `<(…)`: the producer ignores the fd 0 / fd 2 redirections around the command — its stderr escapes `{ …; } 2> file`, `$( … 2>&1)`, `f 2> file` and `exec 2>`

#### trap（1 单）

- #4096 ERR trap: pipeline region adjudication in a subshell function reports the body line instead of the region (close-paren) line

#### 其他（6 单）

- #2812 [perf] 测量专用构建变体：feature 门控的分配计数，计时与分配两轴分离
- #4094 empty HOME: cd ~ must fail with 'null directory'; pushd ~ / pushd -n ~ empty-expansion faces
- #4095 type -a under-reports ./ matches for repeated empty PATH elements (visit/fold accounting missing)
- #4105 Windows 原生路径转换改写导出 POSIX 路径（issue149 基线红）
- #4150 `mkdir -p C:/x` (or `/c/x`) creates a stray `C:` directory in the current directory
- #4154 env-value conversion (#4064) collides with the #149 verbatim export contract — boundary ruling needed

## [1.5.1] - 2026-11-22

### 修复

- **深审修复波（1.5.1 向，2026-11-04）**：两轮深度审计（约 214 条经队长复核的发现）中可局部修复项全部落地——解析器诊断归因对齐 GNU（case/if/函数/算术-for/命令替换/EOF 各形态的报错令牌与行号，#2739–#2754、#2679）；heredoc 展开上下文统一（#2676、#2681）；解析器挂死与解析期代码执行两处高危关闭（#2682、#2686）；状态变更内建在重定向失败时不再执行（#2692）；环境导入不再误剥小写变量的导出标记（#2723）；compspec 锚定与补全分发面修正（#2724、#2726、#2727）；词法/展开/作用域多处 GNU 对齐（#2668–#2675、#2756–#2767、#2778）。每条修复均带以 issue 号命名的回归钉。结构性余项（单一展开管线、fdwalk 等）按 1.5.1 专项六件工程推进，残差均已逐条立单。
- **GNU 差分修复波（2026-11-04 – 11-22）**：2026-11-03 至 11-22 窗口内本仓共关闭 703 单（其中挂 1.5.1 里程碑 456 单；四仓同期合计关闭 926 单、合并 PR 1060 张），全部以 GNU Bash 5.3.20 为规范源、经真 oracle 逐形差分定谳后修复，每条均带以 issue 号命名的回归钉。按主题机械归类（每单只计一族）：fd/重定向 80、解析与诊断 68、引号与词法 28、执行与子进程 59、展开 10、declare/readonly/local 33、算术 28、参数展开 38、作业控制与 wait 40、case 23、补全 16、heredoc/herestring 23、数组/assoc 31、[[ ]] 18、procsub 19、nounset 13、glob 8、trap 19、coproc 18、其他 131，合计 703（完整清单见下）。冻结头为 master `eeb424da`（#4067 合入点）；已知轮换/环境性 census 红与 goldens #4038 切片非确定性均已逐项定性、不属本版回归（见发版卷宗冻结头测试总表）。

### 完整清单

窗口（2026-11-03 – 11-22）内关闭的 703 单按主题机械归类如下（每单只计一族，标题逐字取自 issue 原文）：

#### fd/重定向（80 单）

- #2395 [bug] exec 接管道 stdin 时物化残留临时文件且重定向丢失
- #2613 [fd] 管道 stage 子进程根本不交付 {var} 绑定（EBADF，W3 stage 方言）
- #2615 [fd] {var} 分配在模型空间而非 OS 空间，$v 数字与 GNU 分歧（文件结果一致）
- #2662 [upstream] read: `read -d` with a high-byte (>= 0x80) delimiter reads nothing
- #2688 [A2-01] Spawn fd aliasing: children inherit the same file at duplicate descriptor numbers
- #2689 [A2-02] command <external> as a pipeline stage drops the command's own redirections
- #2690 [A2-03] /dev/stdout reopen (O_TRUNC) semantics missing on the exec redirect face
- #2691 [A2-04] >&/dev/fd/N and >&/dev/stderr fail with ENOENT instead of duplicating the fd
- #2718 [B2-F06] Redirect-only commands expand redirect words before binding their (permanent) assignments
- #2748 [B6-R10] `function NAME` without `()` and with a redirection is run as an external command
- #2773 driver: set -v echoes rendered commands at execution time, not input lines as read [B4-F04]
- #2775 fd: varredir_close OFF ignored — command-scoped {var} fds always closed at command end [B4-F13]
- #2787 [B6-R13 face 4, decision D3] -c route reports status 2 for a read-time comsub failure; GNU reports 127
- #2793 /dev/tcp 进程内可用，但 socket fd≥3 不随外部子进程传递；/dev/udp 静默丢包
- #2803 [bug] exec with a command treats >&fd / >&- as output to a file named fd / -
- #2821 [bug] 带命令的 exec 只应用最低编号的输出重定向，其余输出重定向被静默丢弃
- #2848 [GAP-12 tail] Session face for the retained invocation's shopt flags (enter_interactive read)
- #2880 [bug] Background fifo reader can die before applying its redirects — parent's exec 6> fifo open deadlocks (run-test test1.sub dance)
- #2883 Interactive read -p prompt renders raw-byte marker pairs instead of the control bytes
- #2979 printf: remaining locale faces (byte_mode, \u gate, decimal_point) read the process env, not the variable-table guard
- #3046 exec in a pipeline/subshell ignores stdin redirect from a character device (reads EOF) and drops stderr redirects
- #3061 [bug] printf -v 'x[0]' element bind leaks raw compound storage encoding on scalar read-back
- #3104 raw-byte (U+E000) pairs leak at name-consumption boundaries: redirect targets, test/[ operands, source, cd (E000-domain counterpart of #3076)
- #3187 [bug] read -t with invalid timeout is diagnosed only after redirection succeeds; GNU validates the timeout first
- #3191 read -r -a 在单字节 locale 下字节域分裂丢空字段（#2819 残余）
- #3201 comsub 外部子进程 ambient fd 交付不稳定（多数运行整段缺失，与 #2688 别名异源）
- #3212 read -d 分隔符字节与后续数据字节构成合法 UTF-8 时在传输文本中不可见（joined 形态）
- #3213 varredir 物化 fd 在 shell 自身 /proc/$$/fd 视图不可见（GNU 可见）
- #3218 PATH/名字折叠丢逐字符来源：同名真 Latin-1 字与 byte char 混用取字节拼写，折叠串作键共享重定向旗标
- #3221 junit-formatter.bats 仍报 15 instead of 13：嵌套 tap13 行经 ambient fd 3 泄漏（#3149 残余）
- #3286 已删 cwd 下相对输出重定向至已存在 FIFO 即时 ENOENT（GNU 阻塞 rendezvous 后成功）
- #3291 read: C.UTF-8 file loop fd_table mb serve diverges on multibyte delimiter remainder
- #3293 read: byte-served path skips the -t timed judge (and invalidates the stream arrival table)
- #3294 read -n: illegal UTF-8 sequence measured as 1-byte units (GNU/glibc mbrtowc consumes the whole sequence)
- #3308 [bug][fd] 脚本模式 $(外部 | 外部)：管道首进程 stderr 绕过全部重定向直写控制台（-c 模式正常；GNU 无此行为）
- #3311 [bug] exec with a command opens fd>=3 redirect targets but never passes the fd to the child
- #3312 [bug] exec with a command routes device redirect targets through a /tmp sink file, silently swallowing write failures
- #3313 [bug] exec with a command replaces <&- with /dev/null instead of closing stdin
- #3325 timed 体内内层 compound 重定向失败不到达 ERR 裁决（time { if … < /bad; }）
- #3334 [bug] read -n/-N counts characters instead of bytes in C locale
- #3401 [bug] termsig death still delayed on comsub, blocking-read and simulated-exec waits (residual of #3284)
- #3402 [bug] pipeline stage stderr bypasses the fd table on the Windows concurrent path (Windows face of #3308)
- #3403 [bug] sequential redirect loop resolves 2>&1 against the live fd 1 target instead of the dup-time snapshot
- #3432 [bug] test -ef/-nt/-ot resolve /proc/self/fd/N and /dev/fd/N to the wrong file (off by one fd)
- #3539 [bug] $BASH_COMMAND of a top-level command renders redirections outside print_cmd.c print_redirection (>&2, 3>&2- dropped, &> split)
- #3560 [bug] Windows: a command's own N>&- does not revoke the ambient fd N handed to a rubash child
- #3640 [bug] Shell holds its startup cwd open on fd 3, so `exec 3>file` lands on fd 4
- #3644 Windows：command_substitution_values.rs 的 use std::io::Read 在 Windows 上未使用
- #3706 [bug] Group redirect `{ ...; } N<redir` does not restore fd N to its pre-group target
- #3726 fd: /dev/tcp|udp bind restores a retired entry (same hazard as #3706)
- #3727 fd: function-call stdin bindings restore retired fd 0 entries (same hazard as #3706)
- #3728 fd: read from /dev/tcp restores a retired fd 0 entry (same hazard as #3706)
- #3729 fd: stdio-marker reinstate restores a retired entry (same hazard as #3706)
- #3733 fd: redirect failure mid-list does not undo partial binds (GNU undo_partial_redirects)
- #3770 read on write-only fd 0 fails silently instead of `read: 0: read error: Bad file descriptor`
- #3785 [bug] read -n/-N counts characters instead of bytes in C locale on redirected / pipeline stdin (follow-up to #3334)
- #3792 [bug] cmd <&N on a write-only fd fails the redirect ("N: Bad file descriptor") instead of dup'ing the descriptor; emulated cat reads the file instead
- #3814 子 shell 的 8<&- 泄漏到父 shell：( … ) 8<&- 后父 shell fd 8 损坏
- #3815 嵌套组内层 fd 8 恢复落到野管道：{ { …; } 8</dev/null; …; } 8</dev/null
- #3819 [bug] failed pipe creation under fd exhaustion is silently ignored; the pipeline runs instead of failing ("pipe error")
- #3828 [bug] output redirects fail spuriously when umask strips owner-write from the created file ("f: Permission denied", rc 1, data lost)
- #3852 redirect failure does not set the exit status (GNU sets $? to 1)
- #3856 [bug] fd>=3 delivery to children dead on Windows since #3736: fork_respawn_transport_key strips __RUBASH_FD_* for every non-respawn birth
- #3892 exec: {var} 重定向绑定在同 list 后续条目失败时被回滚（GNU varassign 无 undo 记录，绑定存活）
- #3917 残差（#3640/#3891）：exec 3>file / exec 4>file 不落指定 fd，自带钉两格恒红
- #3938 持久 exec 重定向落点的完整性残差：dup 源不物化 / ≥10 被占用保持虚拟回退 / 快照恢复留 dup 描述符（#3917 相邻面）
- #3939 /dev/zero 的 <> 分裂模型：读写臂为两个文件描述，共享偏移与 GNU 分歧（P3）
- #3943 [bug] read -d with a multibyte delimiter stops on a raw byte equal to the delimiter's codepoint
- #3944 [bug] `read -d` with a single-byte delimiter >= 0x80 fails on file-backed input under a UTF-8 locale
- #3945 [bug] `read -t 0` returns success when no input is available
- #3985 subshell ( exec … >&3 ) 3>/dev/full: shell-side EACCES at fd binding instead of child's ENOSPC write failure
- #3991 [gnu-baseline] redirect undo saves are never materialized: dup from a save slot fails EBADF (1>&10 after exec 9>file)
- #3995 [gnu-baseline] cat - inside a file-redirected group poisons the next fd-0 read (silent empty, rc=0)
- #3997 [gnu-baseline] fd-table /dev/full binding never reaches external children (ambient fd>=3 delivery drops the slot)
- #3999 [gnu-baseline] read serve face has no undo-save resolution (0<&10 inside a read list still EBADF)
- #4000 [gnu-baseline] move (N>&M-) retires a handle a same-walk dup still aliases: flush writes to a closed descriptor
- #4024 [gnu-baseline] 同 list 从 walk save 槽 dup 回 stdio fd 得到死端点（静默丢输出），GNU 从真描述符 dup 正常
- #4027 [gnu-baseline] 同 walk 先重绑 fd N（生 save）后 move 关 N：GNU 恢复 save 绑定，rubash 的 persist 关闭胜出致后续 >&N EBADF
- #4044 [gnu-baseline] exec walk inside a nested context closing fd 1 also kills the parent shell's fd 1
- #4054 [test-infra] issue3997 fd-enumeration pins assume a clean ancestor fd set — harden against harness-held fds

#### 解析与诊断（68 单）

- #2635 [bug] RLIMIT_NOFILE 软限内重定向目标 fd ≥ 限额时不报错（GNU: Bad file descriptor, rc 1）
- #2637 [bug] history -c 带多余操作数时报错且不清表（GNU 静默 rc 0 清空）
- #2639 [bug] times 无效选项报错独缺 line 1: 前缀（GNU: line 1: times: -x: invalid option）
- #2665 [upstream] Diagnostic fidelity family: line-number drift, message shapes, Rust-format leakage, help layout, missing identifier diagnostic, carrier-byte leakage (7 micro-items)
- #2680 [A1-F07] Parse/execute interleave: ;-separated commands before a later syntax error still execute
- #2714 [B1-11] Diagnostic prolog hardcodes `bash` instead of the argv[0]-derived name (owner decision: impersonation vs fidelity)
- #2725 [B5-01] Interactive stdin driver swallows the rest of the stream after compound-opener parse errors, then kills the session
- #2728 [B5-04] Exported-function (BASH_FUNC_*%%) import: wrong silence/diagnostic split and a laxer shape check
- #2734 [B5-17] Interactive-route diagnostics carry a `line N:` segment GNU omits (and a spurious `-c:` segment on -i -c)
- #2739 [B6-R1] Syntax error at a continuation position blames the previous command's line and drops the source echo
- #2741 [B6-R3] `if true then ...`: GNU blames the closing keyword `fi` (then is consumed as a condition argument); rubash blames `then`
- #2742 [B6-R4] `while do done` / `until do done`: GNU blames `do`, rubash blames `done`
- #2745 [B6-R7] `select` rejects a loop variable literally named `in`
- #2751 [B6-R13] Command-substitution inner syntax errors: missing `while looking for matching ')'` suffix, fragment echo, per-level escalation lines, `-c` status 2 vs GNU 127
- #2754 [B6-R17] EOF diagnostic arm reports first-command-line + 1, not the end-of-input line
- #2755 [B6-R18] Bad-interpreter exec diagnostic carries a `line 1:` segment GNU omits
- #2789 [read-time] comsub bodies validated at execution, not at read: skipped bodies never fail; parse-time body failures miss the -c 127 remap
- #2791 [D1 残面] 诊断前缀冷路径字面量未随 argv[0]：builtin 本地 fallback 的 "bash"/"rubash" 字面量
- #2796 [bug] cd 对软链父目录的 `..` 按物理路径解析，默认与 -L 下均偏离逻辑目录
- #2871 [bug] bind set silently accepts invalid bell-style/keymap values (no "could not set value" diagnostic)
- #2873 [bug] bind silently swallows malformed bind specs (no "no key sequence terminator" diagnostic)
- #2924 inputrc / bind -f parser still drops old-style binding lines (second consumer of the #2914 arm)
- #2947 [bug] POSIX-mode source diagnostic prints `.` instead of `source`
- #2966 shift: out-of-range diagnostic prints the parsed count instead of GNU's word (and invents one for the implicit count)
- #3008 printf %S 在 C locale 下应报错（宽字符转换失败），rubash 照常打印
- #3040 Stored U+E314 value used as a name renders the E400 pair: command-not-found diagnostic and redirect filename
- #3090 bind -f / history 文件面的相对文件名不锚定会话 cwd（进程 cwd 解析，平名复现）
- #3091 管道成员 spawn 失败的状态与诊断塌陷：PIPESTATUS 为 0、诊断丢 ./ 拼写（平名复现）
- #3193 [census] 产品字面 bash: 诊断前缀站点普查：逐点核 GNU 在不同 argv[0] 下的前缀真值
- #3209 函数体首命令带重定向时报错行指向命令自身而非函数定义行（行戳捕获面）
- #3222 命令未找到诊断的 stderr 写失败时退出码翻 1，GNU 保持 127（诊断收尾路径写错误传播进 exit_code）
- #3254 main.rs 启动诊断族字面程序名不随 argv[0] 派生（-o/-O/-c/长选项/debugger 全族）
- #3256 共享 diagnostic.rs / exec.rs 冷路径字面前缀收敛（待 #2968/#3214 合入后派）
- #3268 已删 cwd 下进程内路径串解析同族残差：comsub 内 cat、cd ..、壳侧输入重定向、模拟 cp 操作数面
- #3285 cd . 于已删 cwd：缺 getcwd 失败诊断且 PWD 拼写丢 /. 尾（#3268 b4 残差，resetpwd 路径）
- #3300 后台函数体重解析行戳取决父进程退出时序（`background_first` box 报 line 2、本地/GNU 报 line 1）
- #3328 重解析源回显行是 parser 重序列化文本而非原文行（__RUBASH_PARSE_SOURCE__ 跨度保真）
- #3333 [bug] simulated mkdir -p diagnostic names the whole operand instead of the first failing component
- #3335 [bug] cd . in a deleted cwd: pushd entry still fails rc 1; cd entry lacks getcwd diagnostic and PWD /. suffix
- #3340 [bug] bind --help 非交互下缺失 line editing 警告（help 门先行跳过了处理器前导诊断）
- #3343 [bug] unset 诊断路由谓词仍全表扫选项（操作数后 -f/-v 双置形分歧）
- #3355 空 argv[0] 调用时 $0 被种成 /、诊断印 /: …（GNU 印空名）
- #3388 未闭合的 [ 在解析期夭折整脚本（GNU 为运行时 missing ']' 错误、脚本继续）
- #3396 pushd 相对目标按 $PWD 解析（未设/空/陈旧时失败；GNU 按真实 cwd 解析）
- #3404 [bug] diagnostic re-print of a comsub inside a subscript keeps >&2 where GNU normalizes it to 1>&2
- #3407 [bug] alias-body diagnostics report invocation line + body offset; GNU reports the invocation line
- #3411 tests/issue3182_diagnostic_faces.rs:30 死字段使 master 全靶 -D warnings 红（#3373 钉善后）
- #3412 空 argv0 的 -c 形态下 alias/unalias 诊断头仍印 bash:（应随空 $0 印 :）
- #3415 非 shell shebang 的脚本直执行被当作 bash 执行：体报语法错 rc=2；shebang-only 文件静默 rc=0（#993 OMP 矩阵立项）
- #3420 [bug] Background child spawn chdirs by raw path string without consulting the cwd anchor (deleted-cwd diagnostic + rc 1 where GNU is silent rc 0)
- #3434 [bug] builtin diagnostics in a pipeline component print after later stdout
- #3453 [bug] diagnostics to a closed fd: printf pass-abort and pipeline-stage resurrection vs GNU
- #3574 cd entry in a deleted cwd does not emit GNU's chdir: diagnostic
- #3635 cp emulation: failed create diagnostics drop the `regular file` kind (GNU copy.c copy_internal)
- #3647 [parser] syntax error after `!` in reserved position does not echo the source line
- #3648 [parser] `! } { }` runs `}` as a command instead of a syntax error
- #3774 [fd] read 自身输出重定向 0>file 仍静默丢 EBADF 诊断（#3773 c4 残面）
- #3777 [fd] Windows 上 read < 目录在重定向打开层报 Permission denied，达不到 EISDIR 诊断（issue1495 钉 X299 双边既存红）
- #3808 parser: `>(...)` with a `{ ...; }` group body is rejected as `ambiguous redirect` (GNU accepts it)
- #3813 modernish builtin.t 007 解析死亡：别名 { f() { + $(…) 内奇数撇号注释致 rc=2
- #3818 [bug] input-redirect failures under fd exhaustion: wrong "N: Bad file descriptor" diagnostic, and "< file" on external commands silently dropped
- #3827 [bug] exec-failure diagnostic prints the absolute path for commands found via a relative PATH entry (GNU prints the PATH-joined form, e.g. "sub/mycmd: Permission denied")
- #3833 [parser] reserved words after pending-inversion `!` run as command words instead of syntax errors (siblings of #3648)
- #3845 regression: golden_issue1230 — pipeline-stage spawn diagnostic leaks to stderr inside comsub (suspect #3631)
- #3914 [bug] escaped newline inside a delimiter body is not counted in EOF diagnostic lines
- #3936 while/until 体错误归属同族残差：parse_loop_body 吞掉 latched 错误、回退报 EOF-from-while 形（#3184 缺陷 B 同族）
- #4028 [gnu-baseline] move 显式命名 fd 10（8>&10-）时 GNU 有 undo 期 Bad file descriptor 诊断，rubash 无
- #4047 redirect failures inside async subshell bodies are swallowed whole (rc=0, no diagnostic)

#### 引号与词法（28 单）

- #2684 [A1-F12] Single-quoted double-quote inside $() inside double quotes breaks the scanner
- #2706 [B1-04] Quoted expansion results lose IFS protection in mixed words ($a"$b" splits inside the quoted part)
- #2708 [B1-06] Tilde expansion is not vetoed by quotes inside the tilde prefix (~'' expands; GNU leaves it literal)
- #2747 [B6-R9] Function-definition errors blame `(` instead of the token after `()`
- #2781 arith-for single-`)` headers keep the matched-pair EOF form instead of blaming the inner token (F4/R11 residual)
- #2882 Escaped user U+E002 is eaten by blind QUOTED_NULL consumers (printf emits stray U+E400, echo emits nothing, concat drops the char)
- #2959 set -k (keyword): assignment words are re-judged after expansion — quoted words with '=' are stripped, bare assignments lost
- #2999 fix: comsub suffix folds backslash out of a preceding single-quoted segment
- #3005 __git_ps1 分离头描述值泄漏载体字节 U+0010 与引号（t9903 六例）
- #3017 fix: set listing of scalars with bare C0 bytes lacks $'...' quoting
- #3020 [bug] hash-leading command word dropped as comment after quoting or expansion
- #3029 [bug] non-ASCII command names printed raw in errors, GNU uses octal quoting
- #3031 [bug] syntax-error token naming differs from GNU (pipe vs paren faces)
- #3034 [bug] assignment y="($x)" leaks 0x10 + quotes into the stored value
- #3089 [bug] Redirect target joining a quoted space segment with a substitution reports "ambiguous redirect"
- #3103 unquoted splitting in command-word position (for list / set -- / args) loses data carrier bytes: \x1c eaten as glue, \x1f folded to '$'
- #3263 信任条件 (a) 理论误触：同词 glue 落他字时引号内 IFS 空白未受保护仍被判可信（#3089 返工面外观察）
- #3435 $(...) 跨度扫描对 backquote 组内未配平 " 盲（整文件误作未闭合引用）
- #3495 [bug] unquoted digit-leading compound LHS should be a GNU parse-time syntax error
- #3505 [bug] N>> append redirect (e.g. 3>>) rejected by the lexer
- #3557 [bug] 命令位加引号的 '(' / '{' 被当作子 shell / 组命令（GNU 为普通 WORD，command not found）
- #3582 [bug] unset 'a[~]' 诊断 token 应为 ~，rubash 报 /home/hatch（#3522 残差）
- #3639 [bug] Trailing unquoted `{ }` after `!` is folded into one word "{ }"
- #3663 [bug] Unquoted assignment x=a\\*e drops the backslash before * and ? (value becomes a*e), breaking the modernish DBLBRACKET guard
- #3664 [bug] Double quotes inside a backquote command substitution inside double quotes cause a syntax error
- #3838 [lexer] comsub alias pre-scan newlines untracked (second-order gap after #3678)
- #3859 [bug] $(...) body walkers: ANSI-C $'...' escaped quote hides ')' — bare comsub EOF error, quoted comsub wrong value
- #3904 unclosed-construct diagnostic fidelity: token-error wording (v=(a (echo) and closer char (v=(a[1 reports ')' for ']')

#### 执行与子进程（59 单）

- #2565 [env] 导入的已导出 LINES 不进子进程 env，COLUMNS 进（组合/导入链路不同形）
- #2636 [bug] 子 shell 内的 ulimit 改动泄漏回父 shell（GNU 外层不变）
- #2642 [bug] #2514 残差：command -v/-V env 仍被仿真注册表遮挡（GNU: /usr/bin/env）
- #2652 [upstream] set -e: failure suppression in tested contexts (&&/||, !, conditions, eval) not honoured — shell exits at the first tested failure
- #2661 [upstream] exec: executing a directory reports "not found" / exit 127 instead of "cannot execute: Is a directory" / 126
- #2693 [A2-08] Pipeline-stage builtins are SIGPIPE-immune (PIPESTATUS 0 instead of 141)
- #2694 [A2-09] umask set in a pipeline stage leaks back into the parent shell
- #2696 [A2-13] Hash table stores absolutized path; type prints it instead of the as-found relative spelling
- #2699 [A3-R58A] Multi-argument builtin stdout/stderr interleaving lost in the shared buffered-output path
- #2717 [B2-F05] `command -v` reports non-executable slash-named files (no file_status qualification) and absolutizes them
- #2722 [B3-F05] BASH_SUBSHELL over-counts when a `( )` is a pipeline stage (stage bump + paren bump)
- #2723 [B5-05] Environment import silently de-exports every lowercase-first variable name (http_proxy & friends invisible to children)
- #2736 [B5-18] type/hash introspection does not reflect emulated dispatch (sleep reported as its PATH body, never hashed)
- #2753 [B6-R16] `--help` (CHECK_HELPOPT) missing from 36 of 55 builtins — no shared getopt help mechanism
- #2783 comsub body error echo shows the body fragment, not the enclosing line (F4/R13 residual)
- #2810 [api] Session 协作式取消：执行安全点轮询宿主取消令牌（不依赖信号）
- #2881 External boundary leaks the E400 escape pair into child env/argv (decode_raw_byte_markers_to_byte_chars lacks the LITERAL_CHAR_ESCAPE arm)
- #2884 Environment import is an unencoded entry point: inherited user values are interpreted as live carrier protocol
- #2901 [test] bats-core: re-run scenario chain + own suite on current master — adjudicate DIV-D (zero-exec on the engine binary)
- #2939 builtin: words after '--' re-checked as options instead of names
- #2942 enable -n : does not persist: the disabled-builtin carrier is colon-separated and ':' round-trips to empty
- #2949 BuiltinRow exposes pub fields of pub(crate) types (E0603 under -D warnings) — breaks niubash CI
- #2958 builtin_diag::valid_number rejects vertical-tab-padded numbers GNU accepts (trim_ascii lacks 0x0B)
- #2995 sleep Fastpath 仿真无 OS 子进程：ps 树杀链（bats 每测试超时）够不着在飞的 sleep
- #3054 [bug] respawn serialization drops first_word_end_line (residual of #3028)
- #3071 tempenv prefix assignment: self-referencing RHS is expanded twice into the child environment (PATH splice corrupts)
- #3076 E400 literal-char pairs leak at command-lookup / slash-execve-classification / cd / exec operand boundaries (same family as #3040)
- #3108 Session 快照把启动种子名误报 exported=true（from_environment 给种子表条目统一打 exported）
- #3147 Describe of a fresh PATH walk absolutizes the display instead of the as-written join (command -v / type)
- #3148 exec builtin face does not classify a bad interpreter (raw OS error, 127, line segment) where GNU reports sys_error bad interpreter, 126
- #3198 已删 cwd 下前台外部命令 spawn 全部 rc 127（spawn 按陈旧逻辑 cwd chdir）
- #3284 引擎等待面：前台命令执行期终止信号只在命令结束边界被消费，GNU 在等待中即信号死（niu #988 时序残差归宿）
- #3310 [bug] exec does not decrement SHLVL
- #3314 [bug] bad-interpreter exec exits 127 instead of 126 at top level
- #3315 [bug] plain external commands get basename argv[0] instead of the full path
- #3316 [bug] exec of a relative-path script leaves "/./" in $0
- #3317 [bug] exec with an empty command exits 126 instead of 127
- #3336 [bug] after pushd into a deleted cwd, external children run in the startup cwd instead of inheriting the deleted cwd
- #3339 [bug] command <builtin> --help 重派发面未过 help 门（GNU 出 help）
- #3347 [bug] respawn（脚本文件 spawn）在父壳伪造导出 $PWD 后从变量重派生逻辑 cwd——应随 respawn 运输集携带 logical_cwd 值
- #3348 [bug] $PWD 被伪造后，comsub 内 cd / pwd -L 仍按变量派生逻辑值（状态克隆未覆盖的派生点）
- #3358 [bug] Explicit 'hash name' records the absolutized path for relative PATH entries (follow-up of #2696)
- #3387 times 在子 shell/命令替换/管道段印宿主进程累计 rusage（#2932 残差：模拟 fork 不清零时钟）
- #3414 command 面 help 门未挂 prologue 钩子：bind 警告丢失、hash 拒绝被 help 抢先（#3381/#3385 双合后残面）
- #3507 [test] dead_code in tests/issue2066_pipeline_host_flag_regressions.rs under -D warnings on Windows
- #3512 [test] dead_code in tests/issue2304_numbered_devnull_spawn_regressions.rs under -D warnings
- #3513 [test] dead_code in tests/issue2272_fdvar_spawn_delivery_regressions.rs under -D warnings
- #3550 [bug] Pipeline member with a stale hashed path: wrong ENOENT name, status 0, PIPESTATUS collapses
- #3551 [bug] hash table ignores HASH_CHKDOT: ./NAME created after hashing via PATH=.:… is not preferred
- #3566 [bug] Windows prefix-concurrent pipeline drops the prefix's stdout when its last member wrote stderr
- #3585 [macOS] external commands fail: Mach-O binaries treated as ENOEXEC candidates (cannot execute binary file, exit 126)
- #3824 [bug] hash -p with an empty path stores "./": hash -t prints "./" and running the entry fails "./: Is a directory" (GNU stores "", run fails "No such file or directory")
- #3825 [bug] hash and type -a accept a non-executable file found via PATH (GNU: "not found", rc 1)
- #3865 [bug] issue2612 subshell-dup 4 cells red on master tip — introduced by #3849
- #3875 [bug] probe-then-bind double-open：#3828 的剩余四处（env 腿、异步 spawn、pipeline stage、plan-decline）
- #3905 comsub route: unclosed compound assignment inside $(...) reports EOF exit 2 / line 2, GNU reports 1 / line 1
- #3932 plain-spawn child inherits live SHLVL instead of exported level
- #3962 `exit` in a subshell stops the rest of the script when the sourced file has a multi-line string containing `<<`
- #3965 [bug] 外部命令 spawn 组合路径把 env 值中的 0xFF 字节宽化为 U+00FF（字节不保真）

#### 展开（10 单）

- #2629 [fd] stage 括号 applier 对带副作用的 {var} 目标词重复展开（GNU 只展开一次）
- #2651 [upstream] Expansion: POSIX mode rescans and re-expands the tilde result in an eval'd assignment (double expansion)
- #2752 [B6-R15] Substring expansion evaluates offset:length eagerly; GNU evaluates the length only if the offset is in range (and nothing for unset values)
- #2764 expansion: process-substitution pattern words are admitted but not executed at match time (split from #2687)
- #2819 [bug] C locale 下多字节 IFS 字符按整字符拆分（GNU 按字节拆分）
- #3255 script_driver 历史展开前缀与 invocation.rs API 种名仍落字面 "bash"
- #3394 cd 不展开 CDPATH 条目中的 ~（GNU 先按 $HOME 展开再搜索）
- #3650 [expansion] `read`-filled value compares unequal under `[`/`test` for control bytes 0x0c/0x11/0x13-0x1f
- #3651 [expansion] junit formatter XML leaves `[0m[K` remnant where GNU strips the CSI sequence
- #3826 [bug] commands are not found via a "~/..." entry in PATH: tilde is not expanded during PATH lookup (GNU expands it)

#### declare/readonly/local（33 单）

- #2658 [upstream] Attributes: readonly error inside a function is diagnosed twice and does not abort the function
- #2700 [A3-DYN] Filtered declare listings omit dynamic variables and show a phantom BASH_LINENO cell
- #2702 [A3-THISSH] Internal THIS_SH variable is visible in declare output (design question)
- #2720 [B3-F02] Local bindings of dynamic variables never shadow: every read face consults the dynamic provider by name, bypassing the cell
- #2770 variables: FUNCNAME has no listing view in declare/set output (split from #2700)
- #2944 [bug] set -a (allexport) ignored by local/readonly initialization
- #2964 [bug] set -a marks a flagless declare compound assignment inside a function (top-level quirk does not apply there)
- #2970 getopts: a no-argument option unbinds OPTARG (readonly mark lost); GNU binds it empty
- #2972 declare/typeset listing ignores attribute filter flags
- #2973 declare/typeset listing renders dynamic variables with values (BASHPID/RANDOM), extra SECONDS entry, GROUPS shape
- #2974 local -f/-F with no names prints function definitions; GNU prints nothing
- #2980 declare arr[0]=x: subscript operand over-marked -x by the allexport post-pass (both contexts)
- #3026 [bug] typeset listing shows BASH_ARGV0 and renders GROUPS=([0]="0")
- #3048 Residual diagnostic_prefix copies still pair frame line with top-level script name (printf/set/declare/setattr/kill/alias/help/shift/exit/shopt)
- #3049 bats.bats #71 (Don't hang on CTRL-C, issue #353) hangs forever: inner test registration hits "BATS_TEST_TAGS: readonly variable", latch never fires
- #3066 declare -a conversion of an existing scalar locale var skips the locale funnel (GNU warns again with the element value)
- #3186 [bug] readonly/export accept invalid identifiers silently: GNU diagnoses "not a valid identifier" (#2665 D6)
- #3338 [bug] declare/typeset/unset 在首操作数后继续扫描选项（GNU internal_getopt 首个非选项词即止）
- #3344 [bug] declare -f --help 不出帮助（-f 分流先于帮助门，GNU rc=2 出 help）
- #3345 [bug] declare -p 缺名诊断报告序与 GNU 不同（选项形名在先）
- #3350 declare 复合赋值元素边界被整 token 引号判定合并（首元素含引号时 n=1，GNU n=2）
- #3408 [bug] declare -p mixed diagnostic/output still reversed on a merged stream (follow-up to #3324)
- #3421 [bug] readonly-PWD bind family: after a failed readonly pushd bind and directory deletion, 'cd .' loses the getcwd diagnostic line
- #3487 [bug] bare `declare` omits the function definitions GNU prints
- #3529 [bug] local -p 两形态诊断仍与 GNU 有差（execute_local 预-pass 机制）
- #3533 [bug] export NAME=v 撞 readonly 时不设 export 属性（GNU setattr.def 赋值失败后仍 set_var_attribute）
- #3534 [bug] 导入的 bash 托管名（BASH/HOSTNAME/OPTIND/RANDOM/EUID…）丢 export 属性/被重建，GNU 逐名保留 att_exported
- #3609 [compat] local/declare shadowing a readonly outer-scope var spuriously reports readonly variable
- #3661 [bug] POSIXLY_CORRECT assigned at runtime does not switch export -p to POSIX form, breaking isset -x (modernish suite gate)
- #3782 [bug] scalar declare -i/local -i += does not wrap at int64 (GNU wraps)
- #3820 [bug] getopts treats ':' as a valid option letter and leaves OPTARG empty for a declared '?' option
- #3946 [bug] `declare -p` and `${x@Q}` print U+2028/U+2029 raw instead of GNU's `$'...'` octal form
- #4060 [gnu-baseline] assignment/unset/local on imported bash-managed scalar names rejected by static att_noassign table

#### 算术（28 单）

- #2641 [bug] let 对只读变量的报错绕过命令级 stderr 重定向（2>&1 / 2>file / 2>/dev/null 均失效）
- #2648 [upstream] Arithmetic expansion: parenthesised operand after `<`/`>` inside `$(( ))` is also executed as a command
- #2654 [upstream] Arithmetic: error inside an array subscript is silently swallowed (missing diagnostic)
- #2677 [A1-F02] $(( 1 + 2)x) is a fatal syntax error instead of GNU's recoverable parse fallback
- #2685 [A1-F15] Nested arithmetic expansion fails at depth 52 (GNU fine at 500)
- #2707 [A3-R1] Readonly-variable diagnostics from read / (( )) bypass the command's stderr redirection
- #2713 [B1-10] Substring/arithmetic diagnostics trim the raw expression text GNU echoes verbatim
- #2715 [B1-12] Command substitution inside `$(( ))` is mis-scanned when the body contains `;` or yields multiple lines
- #2749 [B6-R11] Arithmetic-for fallback manufactures the header error for failures after a complete header (post-header junk, EOF, unclosed header)
- #2780 eval 'for ((...' unclosed-header diagnostic names the wrong line (F4/R11 residual)
- #3022 [bug] ${foo/[/]-} misrouted to arithmetic parsing
- #3023 [bug] array subscript $"..." not evaluated as arithmetic
- #3024 [bug] unset ${foo:t5:h2:l} errors arithmetically instead of short-circuiting to empty
- #3059 [bug] arithmetic assignment to a locale variable never warns on setlocale failure (((LC_ALL = 5)) silent)
- #3078 Same-expression arithmetic diagnostics and locale warnings print in opposite order vs GNU (deferred diag flush vs bind-time warning)
- #3085 [bug] Array subscript $'...' leaks the $ into the arithmetic error token (GNU reports 'x y', rubash reports $'x y')
- #3086 [bug] $"..." inside $((...)) bodies and ${...} default words is not decoded as a locale string (shared walker has no $" arm)
- #3182 [bug] Diagnostic text leaks internal carrier bytes: command-not-found E400 pair, parameter-unset \x1c, arithmetic token \x01 (#2665 D7/D8, #2879 F5)
- #3184 [bug] Syntax-error message shapes diverge: unexpected-token choice, source-line echo, arithmetic token rendering (#2665 D2)
- #3204 并置复合命令的语法错误点名差：第二个 ((…)) 起头时 GNU 点名算术体首 token（2），rubash 点名 (
- #3246 算术 nameref 非法标识符报错不入求值序：序反/展开形缺失/marker 跨命令泄漏/状态被报告器劫持为 1
- #3287 致命快赋值后算术致命/展开错误标志跨命令泄漏：后续 (( )) 求值被跳过、诊断全静默
- #3295 算术文本内展开期诊断被入口标签误标：下标/嵌套展开应无标签
- #3383 混合赋值语句与 subshell-prefix 路径忽略 arithmetic_error（#3287 Owed 3 缺口，待探针坐实 GNU 期望）
- #3406 [bug] (( )) fd-0 redirect failure reports the command's first line; GNU reports its completion line
- #3456 [bug] pipeline-element prefix assignment ignores fatal arithmetic error (rubash#3383 face 2)
- #3541 [bug] ${a[$(...)]:?err}: an arithmetic error in the subscript still emits the :? report
- #3649 [expansion] arithmetic write side effect lost in list-subject substring expansion (`a[@]`/`@`)

#### 参数展开（38 单）

- #2649 [upstream] Expansion: `${!*}` / `${!@}` rejected as "bad substitution" instead of performing indirect expansion
- #2650 [upstream] Expansion: nested `$'name'` (extquote) as a parameter name is rejected inside `${...#...}` patterns
- #2704 [B1-03] Tilde-prefixed words: the remainder of the word is never expanded (~/$a, ~/${a}, ~+/$a stay literal)
- #2711 [B1-08] `${#@...}` / `${#*...}` with an operator suffix is silently accepted instead of bad substitution
- #2712 [B1-09] `${!nr@A}` on a nameref renders the value; GNU renders the target NAME as the value (bug-compat decision)
- #2759 expansion: quoted-site IFS protection lost at unquoted ${arr[@]} junctions (residual of #2706)
- #2762 expansion: parameter-error pre-scan is not order-aware (${a:?} falsely fires after ${a:=ok})
- #2772 expansion: extquote OFF ignored inside double-quoted ${...} (lex-time verdict not threaded) [B4-F02]
- #2794 [Q6 残面] ${var:?} 展开错误不浮出 discard 面：分类契约与交互驱动均不可见
- #3019 [bug] bad substitution exits 0, GNU exits 1 (${} and 16 more shapes)
- #3027 [bug] ${var=word} assignment leaks into preceding ${var:+word} in the same word
- #3030 [bug] extra bad substitution error inside dollar-quoted string, GNU reports only command-not-found
- #3058 [bug] assignment RHS with a command substitution + ${v:+":${v}"} injects 0x1C before every space (carrier-byte leak, same family as #3034)
- #3063 ${?} substring expansion returns empty instead of slicing the $? value
- #3088 [bug] ${var:-word} alternate expansion results lose backslash protection (flat and nested forms)
- #3092 [bug] ${ cmd; } inside PS0/PS1 runs in a subshell while BASH_VERSION reports 5.3 (kills bash-preexec PS0 backend)
- #3116 [bug] alternate word: source \$ not recognised as escape in unquoted alternate (${x:-a\$b} yields [a\] not [a$b])
- #3117 [bug] alternate word: dq-span \q wrongly unescaped (${x:-"a\qb"} yields [aqb] not [a\qb])
- #3260 :? 诊断路径对双引号 span 漏载体恢复：${q:?...} 错误词面泄漏 IFS_GLUE(\x1c)
- #3296 子串偏移展开的诊断丢标签：${v:ref=1:2} 应带 v: 标签（GNU 有、我方无）
- #3433 [bug] bad substitution diagnostic leaks PUA marker U+E31B for assignment-like words with ${-...}
- #3441 bad substitution 诊断引片段而非整词（未加引号的 a=${-x} 形）
- #3499 [bug] bad substitution 多报诊断且不丢弃命令（DISCARD）
- #3500 [bug] compound 赋值元素词 bad substitution 诊断引片段而非元素词
- #3501 [bug] 模式词里的 bad substitution 应静默（GNU 不作参数展开诊断）
- #3502 [bug] ${v:?${-x}} 多报一行 v: （:? 与 bad substitution 竞发次序）
- #3542 [bug] PS4 with ${q:?err}: GNU reports and prints the prompt literally, rubash substitutes the message
- #3543 [bug] bad substitution diagnostic cites a comsub from raw text (>&2) where GNU prints the print_comsub form (1>&2)
- #3581 [bug] 带引号的 "${a[@]#...}" 单 "" 元素应诊断，rubash 静默（#3501 残差）
- #3784 [bug] ${!0} / ${!00} silently expand instead of erroring, and ${!#} is empty when $# is 0
- #3963 `${x//\\\\/@}` replaces each backslash, not each pair
- #3972 [bug] `${ }` 同行纯空白体报 bad substitution/EOF，GNU 5.3.20 输出空串
- #4003 [gnu-baseline] quoted non-blank IFS chars inside ${} defaults lose protection: false ambiguous redirect (condition-(b) sibling of #3263)
- #4017 [gnu-baseline] funsub 体以复合命令结尾无分隔符被拒（bad substitution），GNU 正常运行
- #4029 [gnu-baseline] bad substitution 诊断前引号定界符与反斜杠已丢失（#4025 钉 3 只断言 contains）
- #4035 ${#v} counts byte-marker (extended UTF-8) sequences by bytes instead of as one character — length-face residual of #3947
- #4062 [gnu-baseline] ${name:-default} treats import-preserved bash-managed scalars as unset
- #4080 [gnu-baseline] 未闭合 ${ 的诊断文本与 GNU 分歧：GNU 报 token 语法错误，rubash 报 unexpected EOF

#### 作业控制与 wait（40 单）

- #2340 [bug] kill + wait 后已终止作业条目的保留与 GNU 不一致（jobs 计数 0 vs 1）
- #2660 [upstream] Async: background job inside a redirected compound command keeps the outer stdout (compound redirections not inherited)
- #2698 [A2-16] Killed job still listed by jobs after its death notice was printed
- #2729 [B5-07] Shell-only SIGINT during a foreground wait is re-raised after the child exits, whatever the child did
- #2730 [B5-08] Emulated `sleep` has no real child for the wait machinery: group SIGINT cannot kill it; shell-only SIGTERM kills the shell instantly
- #2731 [B5-03] Interactive monitor mode defaults ON without a terminal (piped -i route)
- #2735 [B5-09] Signal-death notices are printed even when the shell dies by the same signal
- #2768 [bug] Paren subshell inherits the parent job table; GNU starts it empty (jobs/disown diverge)
- #2769 jobs: boundary death notices carry the just-finished command's line stamp (GNU stamps the next line)
- #2774 interactive: shell claims monitor mode without job control; missing init diagnostics [B4-F05]
- #2777 jobs: stopped (SIGSTOP) job reported as Running; checkjobs wording follows wrong state [B4 obs]
- #2834 [bug] ^Z echo is glued to the Stopped notice (missing newline)
- #2835 [bug] ^C at an empty prompt defers the next exit (shell stays alive)
- #2836 [bug] Interactive shell stops itself on ^Z instead of only the foreground job
- #2837 [bug] ^C aborting a partial line swallows the next line whole
- #2838 [bug] ^Z stopping a sleep inside a for-loop hangs the shell (no prompt)
- #2849 [bug] kill -KILL %1 + immediate exit is refused: the SIGKILL death is claimed one command cycle too late
- #2851 [bug] Subshell inside a loop stopped by ^Z does not abandon the parent loop (r70 slice-4 residual R1)
- #2859 [GAP queue] Session jobs-count face for the host BackgroundJobs prompt segment
- #2886 Bare disown fails on a ^Z-stopped current job (disown: current: no such job) while jobs lists it with the + marker
- #2896 [bug] wait re-wait semantics are inverted: jobspec vs pid after the job is consumed
- #2897 [bug] wait in a subshell reaps the parent shell's jobs
- #2898 [bug] wait with a pid above INT_MAX is treated as a pid instead of an invalid job spec
- #2925 [bug] kill -INT does not kill a non-monitor async job; GNU wait returns 130
- #2927 [bug] kill rejects pid operands with surrounding whitespace; GNU parses them (strtol)
- #2928 [bug] Killed job-death notice printed on async reap in -c mode; GNU prints it only at wait
- #2929 [bug] Killed job-death notice bypasses exec 2> redirect; GNU writes it to the redirected stderr
- #2931 [bug] jobs -x accepts other options before it and swallows -l after it; GNU errors / runs the command
- #2933 [bug] wait -p A -p 1bad validates the first -p, GNU validates the last
- #2935 [bug] wait/disown stopped-job warning pins expect `bash:` prefix; D1 argv[0] prolog renders the absolute binary path
- #2948 monitor flag m in $- does not track job-control availability (set without a tty, never set for library one-shots)
- #2983 bats file_setup_teardown #8: kill -SIGINT -- -$PID reports No such process for the bats subprocess group
- #3051 PTY byte-level ^D exits immediately with stopped jobs, bypassing the EOF deferral (pipe Ok(0) path only)
- #3321 controlling_terminal_available 只试 stderr tty：后台进程组下 ctty 误判
- #3637 [bug] Async-job signal-death notice lost after an intervening foreground wait, doubled after `jobs`
- #3638 [bug] `wait %%` / `wait %+` after the job table drains resolve a stale current-job pointer
- #3656 CI: 普查步在 issue2875 转红后永不可达（lib 步先失败即终止 job）
- #3662 [bug] Background jobs never run when a defined function contains a background job inside a command substitution ($( : & … ))
- #3802 Windows: jobs table renders signal-killed job as `Exit 137` instead of `Killed`
- #3803 Windows: immediate wait after a job's death prints no signal-death notice

#### case（23 单）

- #2678 [A1-F04] Tokens after case ;& / ;;& parsed as a new command instead of clause patterns
- #2679 [A1-F05] Case-header parse errors blame the wrong token and drop file:line source context
- #2687 [A1-F17] Unquoted < / > in case and [[ ]] patterns treated as literal characters instead of operators
- #2701 [B1-01] `:?`/`:=` semantics lost outside the main word pipeline (redirect words, arithmetic, case patterns, select lists, subscripts, nested fragments)
- #2740 [B6-R2] `case in esac` (missing subject) blames the previous line and drops the source echo
- #2743 [B6-R5] Case-clause error resync blames the resync token (esac) instead of the offending token
- #2744 [B6-R6] Case pattern list starting with `|` is accepted (acceptance bug)
- #2784 $(case x in) is intercepted by the comsub extractor's case-aware scan before the body parser runs (F4/R13 residual)
- #2830 [parser] 长 eval 串内 case 模式表反斜杠续行解析失败（modernish 初始化残废）
- #2916 bind -m keymap validation is case-sensitive; GNU accepts EMACS/vi etc. in any case
- #3000 fix: escaped reserved words + continuation in a case pattern list fail to parse (harden.mm shape)
- #3014 case 图案双引号段把 E400 转义对当裸 quoted-null 吃掉：带引用空字符的图案必 NOMATCH
- #3045 ${var=word} assignment is lost in case-subject / for-word-list expansion
- #3084 DQ-backslash case subject loses the backslash (Refs #3045)
- #3105 OS-argv positional params enter storage unencoded: bare carrier bytes stripped at word finalization (case/substring still see them)
- #3189 [bug] extglob quoted literal '[' never matches: case 'a[b' in @("a[b")) should match (#3050 family residual)
- #3235 case 子句命令的行戳来源与 GNU 分歧：应取 pattern 行（compoundcmd_lineno 族，#3209 残差）
- #3306 comsub unclosed-case residuals: space-separated a|/a;; bodies never arm the read-time extractor region (end-to-end rc 2, GNU 127); () empty-pattern forms stay disarmed at gate stage 4
- #3307 REPL completeness face still judges gate-released unclosed-case comsub units as unclosed (PS2 continuation instead of immediate syntax error)
- #3623 [bug] $(case $(cmd) in ...) nested single-command comsub in case subject truncates the outer substitution
- #3628 case-in-comsub: syntax-error diagnostics report the wrong unexpected token (GNU names the token)
- #3790 [bug] declare -l/-u apply full Unicode case mappings (İ→i+dot, ß→SS, ﬀ→FF) and convert in the C locale; GNU uses locale-aware simple mappings
- #3834 comsub 内含 case + 尾随文本（body 位）仍报 syntax error：GNU rc0 / master rc2

#### 补全（16 单）

- #2610 [complete] alias 跳后 -F/-C 绑定的 $1 仍是原敲词（GNU 传重建行首词）
- #2611 [compgen] 内建 -F 执行不绑定 pcomp_curcs，函数内 compopt 误报未在补全函数中
- #2719 [B2-F08] `complete -I` (initial-word) specs are stored in the regular compspec table (no itable); `complete -p` shows them where GNU errors
- #2724 [B5-10] Programmable completion resolves the compspec for the line's FIRST word, whatever segment the cursor is in
- #2726 [B5-12] COMP_WORDS handed to -F/-C completion hooks is truncated at the cursor and drops empty words
- #2727 [B5-13] compgen ignores the -o default and -o bashdefault fallbacks
- #2737 [B5-14] compgen/compspec -A user does not handle the `~` word shape (~ro matches nothing)
- #2738 [B5-11] API gap: `complete -E` / `complete -I` are engine-honored but unreachable through the host gate; no selective special-compspec query exists
- #2776 completion: compgen applies FIGNORE filtering; GNU compgen does not [B4 obs]
- #2846 [bug] Windows drive-letter words fail through complete_detailed under default COMP_WORDBREAKS (engine carve-out)
- #2856 [B5-11] complete -E / -I host gate unreachable — contract candidate Session::has_compspec_at
- #2900 [test] add real bash-completion distribution file to eco corpus + chain test triggering _comp_expand_glob eval (follow-up to #2876)
- #2904 [test] git test-lib: standing slice grows t9902-completion + t9903-bash-prompt; re-run slice on current master
- #2907 [test] git contrib: chain the real git-completion.bash (_git in a live completion attempt) + git-prompt.sh (__git_ps1 in a dirty repo)
- #3125 [bug] interactive source does not push a BASH_SOURCE frame (REPL-sourced file sees empty stack; blocks bash-completion suite fixture)
- #3951 [bug] `env` is registered as a shell builtin (compgen/enable/hash/builtin disagree with GNU)

#### heredoc/herestring（23 单）

- #2676 [A1-F01] Heredoc body expansion applies := before :- (operator order differs from argument expansion)
- #2681 [A1-F08] ${x:=...} in a heredoc feeding an external command persists in the parent shell
- #2682 [A1-F09] Heredoc on an if/while/until condition command hangs the parser
- #2686 [A1-F16] Heredoc delimiter word undergoes command substitution at parse time (executes commands)
- #2761 heredoc: bodies on raw-empty null commands are never expanded (empty-words dispatch bypass)
- #2763 executor: heredoc-body funsub exit latch lingers into the next command
- #2765 parser: heredoc body pairing can cross frame tiers (3+ heredocs on one line spanning a nested if header)
- #2930 [bug] `<<` inside $(( )) in a larger word is mis-lexed as a heredoc; rest of the line is swallowed
- #3003 解析回归：含「. 」token 的行之后 && 衔接 heredoc 解析崩坏（t1300 中止于 198/219）
- #3021 [bug] empty-word command with heredoc returns comsub status instead of 0
- #3183 [bug] Diagnostic line-number accounting: suite shapes from #2665 D1 unprobed on current master (heredoc warning prefixes, break/eval/redir line attribution)
- #3195 heredoc/herestring redirector at/above RLIMIT_NOFILE soft limit is silently bound (Refs #2635 residual)
- #3578 heredoc: 限下双 heredoc 同 list 绑定丢失第一条（exec 14<<A 15<<B，#3576 复核带出的存量缺陷）
- #3654 heredoc: exec 先 herestring 后编号 heredoc 时 heredoc 绑定丢失（exec <<<w 14<<A，#3578 残差）
- #3655 heredoc: 同 fd 先 heredoc 后普通文件重定向时顺序不生效（exec 14<<A 14>file，GNU 后者胜）
- #3725 fd: loop heredoc bind restores a retired entry (same hazard as #3706)
- #3823 funsub ${ ... } 体内 # 注释/heredoc 解析死亡，${x:-$(...)} 闭合 } 漏入输出（真 lexer 层，#3813 续作）
- #3864 [bug] #3855 parameter_errors.rs regresses 30+ pin cells across funsub/heredoc/compound faces (census 86→120)
- #3893 exec: {var}<< heredoc 的 fd 分配在 list 尾部发生，晚于同 list 后续条目的 undo-save 认领（GNU 按位分配）
- #3894 simple command: {var}<< heredoc body 被错绑到 fd 0（GNU 只绑分配 fd，fd0 不受影响）
- #3906 funnel face: apply_dynamic_fd_var_redirects applies {var} heredocs after the plain list, not at list position (GNU: at list position)
- #3964 `$(</dev/stdin)` and `cat /dev/stdin` read nothing from a pipe or here-string in some positions
- #3967 `<<<` inside `$( )` is read as a here-document within a top-level `{ }` group

#### 数组/assoc（31 单）

- #2595 [expand] 预扫描 :? 臂对下标内命令替换仍双重执行（#2558 残面）
- #2596 [expand] :? 嵌套进 :-/:+ 词内误致命（预扫描算子解析，字面下标+已 set 亦中招）
- #2631 [freeze-1.5.0] executor_tests hygiene: three wrong pins (BASH_ARGC/ARGV vs GNU 5.3.20) + parallel-fd full-run hang at mapfile -u3 pin
- #2634 [bug] extdebug 开→关后 BASH_ARGC 为空数组，GNU 为 ([0]="0")
- #2655 [upstream] PANIC: assignment through a nameref to an array element desyncs typed cell vs mirror store (variable_state.rs:1092/1106)
- #2656 [upstream] Arrays: `declare -p` prints elements in insertion order after delete + re-add (GNU: index order)
- #2657 [upstream] Arrays: `unset` of an associative element whose key is a quote/backslash character fails as "not a valid identifier"
- #2703 [A3-F2] Associative assignment with bracket-bearing keys stores the value with its quotes retained
- #2710 [A3-V1] test -v is blind to frame arrays FUNCNAME and BASH_SOURCE inside functions
- #2779 executor: pipeline brace-group staged input starves the next reader after mapfile -n (A2-11 residual)
- #2876 [bug] eval'd compound array assignment with an extglob element is a syntax error even with extglob on (bash-completion _comp_expand_glob shape)
- #2945 [bug] set -a wrongly marks arrays -x
- #2946 [bug] Associative array assignment evaluates the subscript twice
- #2963 [bug] set -a does not mark a scalar assignment over an existing indexed array
- #2971 export/readonly -aA: indexed attribute should win; assoc ghost remains
- #3065 locale funnel reads raw compound storage for array-bound locale vars (false warning + leaked 0x1D in message)
- #3097 locale funnel: element-assign and for-bind channels never fire for array locale vars; declare channel's funnel projection misses the array marks
- #3102 array mirror serialization panics/garbles multi-element assignments whose elements carry raw-byte pairs (\x1f proven)
- #3107 alias 定义后 declare -p BASH_ALIASES 在 debug 构建 panic：typed assoc cell 与镜像不一致（variable_state.rs:1106）
- #3192 单引号 / ANSI-C 引号段在数组汇合后缀位丢尾随空格（#2759 残余）
- #3203 scanner 保留字位镜像缺 ARITH_CMD/COND_END 臂：复合命令后下标词跨度在空格处断裂（a[1 2]=3 被读成 a[1 + 2]=3）
- #3225 bad array subscript 诊断在非字面/模式/子串展开形翻倍（4 次 vs GNU 2 次；#2527 同族残面）
- #3494 [bug] declare 'a[]=x' does not create the empty array GNU creates
- #3521 [bug] unset 经 nameref 指 assoc 数组 + 引号键仍报 not a valid identifier（#3493 残差）
- #3522 [bug] unset 索引数组反引号键静默 rc 0，GNU 为 rc 1（#3493 残差）
- #3573 Compound assignment to an undeclared name infers assoc from string subscripts; GNU builds an indexed array
- #3613 compound-declare 三重展开：错误 token 泄漏 PUA 载体且空数组被创建（GNU 未赋值）
- #3659 [bug] mapfile/read -a into declare -i/-l/-u array skips attribute conversion (GNU converts)
- #3660 [bug] mapfile -C callback reading mapfile's stdin sees EOF instead of the next unread line (GNU interleaves)
- #3950 [bug] `compgen -A arrayvar` / `-A variable` list declared-but-never-assigned arrays
- #3969 [bug] brace 反引号积粘连时 GNU 重扫报 bad substitution，rubash 字面保留/数组路径跨积配对执行

#### [[ ]]（18 单）

- #2646 [upstream] Parser: reserved word `then` not accepted directly after `(( ))` / `[[ ]]`
- #2683 [A1-F10] Same-line shopt -s extglob not seen by a later [[ ]] on that line
- #2750 [B6-R12] `[[ ]]` error resync: the follow-on `syntax error near` names `]]` instead of the following token
- #2782 conditional follow-on walk truncates a glued redirect item to its first byte (F4/R12 residual)
- #2785 a space before the closing paren extends a conditional's near-token by `)` inside comsub bodies (F4/R13 residual)
- #2827 [perf] [[ =~ ]] 每求值重编译 regex：compile_conditional_regex 无缓存
- #2987 bats timeout.bats: per-test timeout watchdog never fires; [[ ]] with '|' in generated -c code errors
- #3025 [bug] [[ -t a && -z b ]] returns 1, GNU returns 2
- #3100 [[ == ]] quoted RHS with extglob group never reaches the extglob matcher
- #3202 comsub 内条件式体错误被外层语法错误抢占：$([[ a == b c) ) 的诊断优先序与 GNU 不符
- #3227 cond 诊断: [[ a == #x ]] 的 syntax error near 令牌为 == 应为 ]]（# 注释语义分歧）
- #3231 [bug] `[[ == ]]` quoted extglob group with a parenthesis inside the quotes does not match (GNU matches)
- #3382 [[ ]] 操作数算术致命泄漏到下一命令（#3287 同族：分派缺 abort 检查）
- #3428 [[ ]] 反斜杠续行输入重定向失败报完成行，GNU 报起始行
- #3545 [[ =~ ]] regex operand: carrier bytes (0x0c, 0x14..0x1f) from $'...' or after | never match
- #3730 fd: [[ ]] stdio save restores retired fd 1/2 entries (same hazard as #3706)
- #3783 [bug] test -v / [[ -v ]] reject positional parameter names (all-digit operands)
- #3791 [bug] [[ =~ ]] rejects bracket expressions containing non-ASCII characters in the C locale ("invalid regular expression")

#### procsub（19 单）

- #2234 链式输出 procsub > >(A) > >(B) 载荷不穿链（GNU 穿链，物化模型边界）
- #2526 [procsub] 被弃生产者在命令边界被等满全程（10.3s vs GNU 立即返回）（#1959 残面）
- #3053 [bug] first-element line stamp gaps: time prefix and procsub redirect target (residual of #3028)
- #3210 procsub body 内报错行差一行：body 文本行来源面（#1184 族相邻）
- #3292 tests: lib-only runs false-red the issue2875 procsub cell when no rubash bin is built
- #3299 复合命令 stdin procsub 体诊断行取 ambient 而非目标止行（`{…} < <(...)` / `while … < <(...)`）
- #3337 [bug] backquote comsub 下 >(…) procsub fd 解析与 GNU 不一致（#3132 不透明规则只覆盖 $-组）
- #3353 内建命令参数词的中段进程替换不拼接 /dev/fd（组体已生成执行，外部命令路线正常）
- #3354 procsub 跨度提取的括号匹配对 backquote 组盲（组体内未配平括号使整段漏认领）
- #3491 [procsub] numbered-fd output procsub 3> >(...) delivers nothing / Bad file descriptor
- #3506 [bug] `N>>` append redirect with a procsub target is a parse-time syntax error (GNU accepts it)
- #3731 fd: procsub-body parse restores a retired fd 0 entry (same hazard as #3706)
- #3732 fd: procsub spawn-plan bind restores a retired entry (same hazard as #3706)
- #3806 fd: deferred procsub body is dropped after an input rebind (GNU runs it at close)
- #3900 residual(#3701): issue551 c6 procsub EOF line still red after #3897 — EOF line arm does not cover process substitution
- #3941 procsub bodies leak inherited fd 3 (S1): simple-command body `>&3` succeeds instead of EBADF
- #3980 procsub body builtin writing to inherited read-only fd fails silently (no Bad file descriptor diagnostic) — #3941 residual
- #3994 [gnu-baseline] procsub redirect on a command-less assignment is bogusly diagnosed ENOENT (x=1 < <(sleep 1))
- #4007 `case` subject: `<(…)` / `>(…)` text in a variable's value is run as a process substitution

#### nounset（13 单）

- #3199 算术下标内 $ 展开绕过 nounset（(( b[$undef]++ )) 静默，GNU 报 unbound variable）
- #3200 算术元素值递归求值不触发 nounset（(( b[i]++ ))，b=(x y)，GNU 报 y: unbound variable）
- #3251 算术裸形 $ 展开绕过 nounset（(( $undef )) 静默，GNU 报 unbound variable）
- #3252 无 nounset 时算术下标诊断整体缺席（GNU 有 bad array subscript + not a valid identifier）
- #3429 set -u 下重定向目标未绑定：GNU 中止脚本，rubash 打印诊断后继续执行
- #3454 set -u unbound redirect target on if/while/for/case/[[]] continues instead of aborting
- #3496 let 运算元 $ 形被 nounset 误捕：GNU 报算术语法错误，rubash 报 unbound variable
- #3497 算术下标单引号内 $ 键被 nounset 误捕：GNU 报算术语法错误，rubash 报 unbound variable
- #3498 eval 时裸名 nounset 报错误名：GNU 报 i，rubash 报下标 $undef
- #3516 [bug] fd-2 redirect on a pipeline member forces sequential drain — unbounded producer hangs (yes | head -3 2>/dev/null)
- #3540 [bug] set -u: unbound array element with a comsub subscript names `echo` and never runs the comsub
- #3563 算术 nounset 同族残差：[[ ]] 操作数文本扫描、预扫描不见 $((k=1)) 绑定、nameref/declare -i/元素赋值诊断
- #4001 [gnu-baseline] nounset (set -u) in redirect words: residual cluster 535/3429/1915 pins still red after closures

#### glob（8 单）

- #2647 [upstream] Parser: unbalanced `[` inside an extglob group is a fatal parse error and silently truncates the script
- #2664 [upstream] Glob: extglob alternative may match `/` in pathname expansion; slash-in-bracket cases halt output entirely
- #2709 [B1-07] Glob: empty pathname components dropped next to `**` (**// arity collapse + result-name collapse)
- #3050 extglob group quoted literals never match: extglob_match_literal_with_case lacks the CTLESC escape arm (pure-ASCII repro)
- #3205 同行 shopt -s extglob 泄漏进同一读单元后段词法（&& 段 glob 生效 / 命令位 !( 误判），GNU 以单元开始态为准
- #3208 未引号未闭合 `[` 的 extglob 组在解析期误报 EOF：@(a[b) 应正常解析为 NOMATCH（lexer/skip.rs）
- #3224 extglob 反向泄漏：门开读入的模式节点被执行期 shopt -u 实时态臂拒绝，GNU 回退字面展开（#3205 残余）
- #3229 未闭合 extglob 组的解析诊断归属错误：报 ( 语法错而非 )-EOF

#### trap（19 单）

- #2630 [perf] trap disposition reconcile fast path dead under entry-ignored signals (~2x per-command tax); multi_session_isolation crawl is this, not a deadlock
- #2638 [bug] trap -p 遇无效信号规格时整批丢弃有效项（GNU 逐项处理）
- #2640 [bug] 子 shell 的 EXIT trap 触发后，后续 EXIT trap 一律不再触发
- #2659 [upstream] Traps: ERR trap does not fire on a redirection error of a compound command (set -e consequence also missing)
- #2692 [A2-07] State-mutating builtins execute despite failed redirections (export/readonly/declare/umask/alias/trap)
- #2716 [B2-F04] EXIT-trap string syntax error replaces the pending exit status; diagnostics lack the `exit trap:` context
- #2824 [bug] 脚本模式 history 保存漏记 trap 行本身（记录行集与 GNU 不一致）
- #2828 [perf] 运行时文本（trap/eval/source）缺内容键 parse 缓存：现仅 DEBUG trap 单槽，且键须含 alias 状态
- #2926 [bug] kill -l/-s reject trap pseudo-signals ERR/DEBUG/RETURN; GNU maps them to 66/65/67
- #2969 [bug] DEBUG trap 内 break/continue：for 触发点误报 only meaningful，且体命令被吞
- #2991 command substitution inside a DEBUG trap handler hangs under set -eET
- #3093 [bug] first DEBUG-trap firing precedes the history write - 633;E carries the previous command
- #3320 verbose 读点覆盖残差三面：eval 逐行采样 / trap 动作读边界 / 交互 fallback 路
- #3324 pipeline stage 输出跨流失序：trap -p 列表与诊断合流顺序与 GNU 不符
- #3327 DEBUG/RETURN/ERR trap 动作重解析同样未置源名上下文（debug/return/error trap）
- #3409 [bug] EXIT trap fires inside a pipeline element (type ls | cat prints the trap output twice)
- #3553 复合命令自身重定向失败的 ERR trap 内 $LINENO 与 GNU 不一致（line_number_for_err_trap 未移植）
- #3641 [bug] Trap action reparse silently accepts unclosed quotes (EXIT/ERR/DEBUG)
- #4002 [gnu-baseline] signal/trap deferral residual cluster: 1034/1035/1036/698/977 pins still red after closures

#### coproc（18 单）

- #2695 [A2-10] Coproc death is not noticed until wait (stale COPROC fds read as EOF)
- #2746 [B6-R8] Bare `coproc` falls through to external-command lookup instead of a syntax error
- #2802 [bug] Process substitution is passed literally in async/coproc commands instead of being expanded
- #2890 [bug] coproc with an empty compound body is accepted instead of a syntax error
- #2891 [bug] disown closes coproc pipe fds: reads/writes fail Bad file descriptor
- #2892 [bug] Second read past EOF on a coproc fd fails Bad file descriptor
- #2893 [bug] Coproc as a pipeline head prints a spurious Broken pipe diagnostic
- #2894 [bug] Coproc body does not inherit traps
- #2895 [bug] Coproc as a pipeline tail is not set up: COPROC stays unset
- #2918 coproc parse-error diagnostic echoes only the offending token; GNU echoes the full source line
- #3032 [bug] coproc invalid-identifier message shows unexpanded name, GNU shows (null)
- #3267 coproc rejects redirect-first body: `coproc >x name` is a syntax error
- #3301 coproc 折叠体诊断行取首叶起行而非止行（`coproc { foooo\\<NL>bar; }` GNU 2 / 我方 1）
- #3305 read coproc serve 未收敛到单位语义：分隔符首字节泄入记录、-n 单位首字节截断、续字节值分隔符逐字节误判
- #3633 coproc: a coproc childs exec >&- leaks the c2p write end, and COPROC stays bound after the child exits
- #3948 [bug] `read -u` on a coproc fd at EOF retires the coproc before any reap point
- #3949 [bug] `coproc cmd &` (trailing ampersand) does not establish COPROC/COPROC_PID
- #4008 `coproc … >file`: reading `${COPROC[0]}` never sees EOF

#### 其他（131 单）

- #2606 [bind] 注册表以书写字符串为键，同一键的 \C-u 与 $'\025' 拼写并存（GNU 以字节序列 canonical 化）
- #2653 [upstream] Alias: alias whose value contains a newline is registered but never expanded
- #2705 [A3-T1] printf %(...)T ignores Olson TZ names (renders as UTC)
- #2721 [B3-F03] FUNCNEST overflow does not abort the enclosing command list (no DISCARD-class unwind)
- #2732 [B5-02] Interactive BASH_SOURCE[0] for REPL-defined functions is the shell name, not `main`
- #2733 [B5-16] histexpand (`H` in $-) missing on the `-i -c` and `-i script` interactive routes
- #2771 variables: POSIX-mode in-function kept-tempenv conversion unwired (split from #2720)
- #2797 [bug] pwd -L 照印 $PWD 变量原文，不做逻辑目录规范化
- #2798 [bug] pwd -P 在 cwd 已删时不报 getcwd 错，静默印陈旧目录且 rc 0
- #2799 [bug] 子壳启动丢弃继承的逻辑 PWD，重置为物理路径
- #2820 [bug] pushd 对非法 -<字母> 首参与目录参数报 too many arguments（GNU：invalid number + usage，rc 2）
- #2833 [GAP-13] bind/readline 装配面契约化 + 账本记账修正波（十轮复审产出）
- #2847 [bug] Ledger tripwire does not validate §6 item paths against actual exports
- #2853 [GAP-13 follow-up] bind_registry types never demoted to internal (Q-E remainder)
- #2860 [GAP queue] Retire the stray-close arm onto Session::run (GAP-2 parked arm)
- #2872 [bug] bind -s/-S macro listing prints control characters raw instead of canonical escapes
- #2874 [bug] bind -X listing prints the -x command unescaped; -x command text over-interprets \n and \t
- #2875 [test-harness] Input process-substitution child resolves its exe without the test-binary fallback
- #2879 [audit] Systematic sweep of the C0/PUA carrier protocol (markers.rs) — the recurring ESC/control-byte incident family
- #2885 Marker protocol enforcement gates: per-channel leak-golden coverage, blind-consumer lint, registry completeness (sweep #2879 follow-up)
- #2903 [test] oh-my-bash + bash-it: re-run the host (niu) chains now that the phase-1 interactive gate is closed (niu#806 family)
- #2906 [test] mvdan/sh: pin the 559-snippet extractor and add an execution differential (not -n only)
- #2908 [test] ruby-build: execute the corpus copy offline (--definitions + fake-definition install) as a golden
- #2910 [docs] eco manual-run list: one recorded home for the MANUAL faces (fzf live, ble.sh rendering, oh-my-niu gallery, nvm slow/iojs)
- #2914 bind: old-style keyname specs (Control-t:/Meta-x:) are silently dropped
- #2915 bind 'set: foo' is silently swallowed; GNU truncates at ':' and reports the variable name
- #2920 consumer_tests::shell_spelling_to_native drops the drive separator, failing a Windows test
- #2923 bind prefix-meta (ISKMAP) has no registry kind: sub-map never materializes in listings
- #2932 [bug] times prints stub 0m0.000s instead of real process times
- #2936 umask: bare '-' rejected, post-'--' words ignored, '--help' misparsed
- #2937 dirs/pushd/popd: option loop stops at the first index word; rotation and '-n' mishandled
- #2938 enable: '-a'/'-n' filter precedence wrong — '-an' prints nothing instead of the full listing
- #2940 type: '-type'/'--type' (and '-path'/'-all') not rewritten by the prescan
- #2943 [bug] type -a reports the cwd match once per empty PATH element; GNU collapses adjacent empty elements
- #2955 shopt: --help in option position does not print the long help (reports -- as invalid option)
- #2956 shift/return/break: numeric validity must be checked before the too-many-arguments (no_args) check
- #2957 set: apply loop lacks the '?' arm — set -o? reports an invalid option and returns 1 instead of usage + 0
- #2960 set -t (onecmd): script reader never honors just_one_command — execution continues past the setting command
- #2961 set: apply-loop 'i' (forced_interactive) has no state slot — set -ovi leaves $- without i
- #2965 [bug] printf %d with a multibyte char constant uses the code point in the C locale; GNU uses the first byte
- #2975 bare unalias exits 1; GNU exits 2
- #2981 bats file_setup_teardown #5/#6: nested-run failure trace lines land at different indices
- #2982 bats file_setup_teardown #7: bats_debug_trap/bats_error_trap command not found in the .src run context
- #2984 bats junit-formatter: four standalone failures (XML escaping, FD3 teardown_suite marking, setup error paths)
- #2985 bats load.bats tail #24/#26 fail standalone (failure-wording face)
- #2988 bats warnings.bats: BW01/BW02 stack-trace line lands off lines[4] in reentrant_run capture
- #3004 __git_complete_index_file 对含 \x1c–\x1f 字节的文件名返回空候选（t9902 #165）
- #3009 printf -v 写 LC_ALL 不发 setlocale 失败警告（赋值通道有），funnel 旁路写残余面
- #3013 契约补齐：BindKindView::Keymap 公开面 + 两侧账本行（#2993 后续）
- #3016 fix: U+E316 (PROMPT_ESCAPE_GUARD) typed-data folds to backslash in assignment concatenation
- #3018 [test] E6: debugger/JSON (DAP) channel has no marker-leak golden cell
- #3028 [bug] error line number after backslash-continuation is off by one
- #3035 [bug] error inside a sourced-file function reports the top-level script filename
- #3038 expand_prompt renders the E400 escape pair for a stored U+E314 value instead of the raw char
- #3060 [bug] for-loop over a locale variable warns twice on setlocale failure (GNU: once)
- #3074 DAP/JSON channel renders variable values undecoded (storage carrier encoding leaks into variables/evaluate)
- #3083 Debugger snapshot values for BASH_VERSINFO / DIRSTACK carry a literal \x1d text prefix (capture-time dynamic-declaration RHS extraction)
- #3106 emulated chmod treats a post-mode '--' as a file operand
- #3111 [tests] stderr assertions hardcode "bash:" prefix, red on the rubash binary path (issue639 9 cells + issue2754 1 cell)
- #3149 bats junit-formatter: nested fixture-suite tap13 lines leak into outer TAP stream in full-file runs (Executed 15 instead of 13, rc=1)
- #3185 [bug] help listing layout: two-column fill is not column-major at COLUMNS=80 (#2665 D5)
- #3194 bind -X 列序与 GNU 分歧：rubash 按注册序、GNU 按键序字节序
- #3196 pushd 不查 CDPATH（cd 同形正常）
- #3197 pushd // 被归一化成 /（dirs 顶与 PWD 均错）
- #3207 入口编码契约缺口：SessionBuilder env 种子与 VariablesView::set 裸 set_env 落存储（#3105 同机制残余）
- #3211 动态关联变量整表复合赋值被对账丢弃（BASH_ALIASES=([x]=…) 无属主落点门）
- #3219 ulimit 硬限角：父 shell 软/硬限读回 777/777，GNU 为 4096/4096（rlimit 虚拟化口径待定）
- #3230 [perf] bats gather 阶段在现 master 下病态缓慢（疑 #3177 后回归）
- #3275 删后原地重建目录的幽灵穿越：静止会话经复活路径读到新目录内容（GNU 仍坐死 inode 报 ENOENT）
- #3276 进程内模拟 mkdir/rm/touch 在已删 cwd 下同红（#3268 同族普查）
- #3289 issue977 pin delivery settle too tight: 500ms window flakes under box load
- #3318 [bug] $BASH is the resolved binary path instead of the invocation path
- #3322 set_args_may_set_onecmd 的 'o' 前截断漏检（set -ot）
- #3329 [bug] history -s drops empty / whitespace-only entries
- #3330 [bug] unix process_console_size stub: help width ignores window size when COLUMNS unset
- #3349 [bug] 已删 cwd 下 pwd -P 失败后，默认 pwd / pwd -L 仍印陈旧逻辑串——GNU 经 resetpwd 毒化后同面转红
- #3357 part_024 times 两格断言写死交互裸名前缀（应按名源链派生：bash: line 1:）
- #3359 [bug] Deleted file hashed via a relative PATH entry is not re-validated on lookup
- #3372 #3218 残差：追加漏斗与同胞消费者仍走折叠串键（同串碰撞未除）+ type 显示面
- #3391 fc -s 两面分歧：替换空结果 quirk 与脚本模式追加非替换（PR #3390 Owed）
- #3392 history 条目在 HISTTIMEFORMAT 设定前创建仍被盖墙钟戳（GNU 为空戳、显示 ??、-a/-w 不写 # 行）
- #3393 history -r 无条件重组多行条目（GNU 仅当 HISTTIMEFORMAT 在读之前已设才重组）
- #3395 cd -P 经相对 CDPATH 条目命中的回显被绝对化（GNU 回显条目+参数原样相对路径；#1329 follow-up）
- #3397 part_024 三格 dirs 期望缺陷：期望 dirs 跟随 PWD= 赋值（GNU 四形实测均不跟随；仅测试面）
- #3405 [bug] at EOF inside an unclosed compound, GNU reports the innermost open construct's error; rubash reports the outer EOF error
- #3410 [bug] set -- -t still routes -t as an option after -- (over-acceptance)
- #3413 空值 BASH_ARGV0 import 被当作未设置（回落 argv[0] 种值，GNU 尊重空值）
- #3416 [bug] Simulated cp stat classification: intermediate non-directory reported as ENOENT instead of ENOTDIR
- #3417 [test] issue2702 pin file: dead 'stderr' field is a second full-target -D warnings baseline point
- #3425 [bug] Simulated chmod stat classification: intermediate non-directory reported as ENOENT instead of ENOTDIR
- #3430 CI 不跑新回归钉 stem：Rust tests 只跑精选 nextest 表
- #3448 chmod apply body ignores dead cwd (node stat classifies, apply still uses string paths)
- #3457 [bug] missing/unreadable --init-file/--rcfile is diagnosed instead of silently skipped
- #3488 [ecosuite] bats timeout.bats #2/#3: timed-out test's process tree/pipe not reaped, outer run takes full 10s
- #3489 [ecosuite] junit-formatter: ANSI CSI sequences not stripped before XML escaping
- #3503 [test] #[cfg(unix)] helpers in unix-only pin stems break full --tests under -D warnings on Windows
- #3514 [test] dead_code in tests/issue3413_bash_argv0_empty_import.rs under -D warnings
- #3515 [test] unused-mut in tests/issue2071_test_empty_path_regressions.rs under -D warnings
- #3517 [bug] $SHELLOPTS 是静态物化快照：set -o/命令行/环境导入都不更新它（派生视图缺失）
- #3527 [test] dead_code in tests/issue2883_read_prompt_carrier_regressions.rs under -D warnings on Windows
- #3535 history: comment-char arming is sticky in GNU — reads after unset HISTTIMEFORMAT are still armed
- #3546 pattern bracket expression: carrier bytes (0x0c, 0x14..0x1f) inside [...] never match
- #3568 [bug] common::issue2197_wait_bounded_* 两 pin 在 Windows 环境失效（X299 双边 0/2，Linux 9/9 全绿）
- #3570 [bug][unix] Startup PWD repair rewrites literal backslashes in the physical cwd
- #3571 [ledger] api-ledger.md §5 wording 'by declaration patterns only' is stale after #3556 (paired rubash/niubash change)
- #3572 [ledger] check-api-ledger.py: #[path] only recognized on the immediately preceding line
- #3575 POSIX same_file arm missing: pwd -L prints stale path in a deleted cwd under set -o posix
- #3619 [bug] $'\u/\U decode consults the requested locale name, not the surviving locale after a failed setlocale
- #3625 截断 Mach-O（< 28 字节头）在 macOS 上被承认为原生、execve 失败路径未实测
- #3645 fc -s =x 空模式替换被当成命令名（GNU 视为替换）
- #3772 tests/issue1794: cfg(unix)-gated helpers are dead code under RUSTFLAGS="-D warnings" on Windows
- #3776 [test-hygiene] issue523 钉文件在 cfg(windows) 下 use std::io::Write 未使用，整树 -D warnings 检查在 Windows 上硬错（#3772 同类）
- #3779 [test-hygiene] 整树 -D warnings 普查：16 个钉文件共 31 处死代码/未用 import（#3772/#3776 同类，Windows 腿）
- #3786 [bug] printf %f-family rounds exact-half decimals down (1.2345 -> 1.234, GNU 1.235)
- #3787 [bug] echo -e drops \u/\U escapes for non-scalar code points under UTF-8 locales (GNU encodes them as-is)
- #3788 [bug] cd/pushd into a directory without search permission reports success although the directory change never happens
- #3789 [test-hygiene] issue3255 钉文件在 cfg(unix) mod interactive 内 use super::* 未使用，整树 -D warnings 检查硬错（#3772/#3776 同类）
- #3798 net bind `<>` opens two TCP connections where GNU opens one
- #3800 [test-hygiene] issue3255 interactive cell asserts literal bash: prefix but rubash: contains it as a substring
- #3816 字面控制字节常量损坏：32 个 CC 常量中 10 个被替换为元字符
- #3839 [ci] UTF-8 boundary hygiene gate red on master since #3630 (strmatch.rs 5 unreviewed widenings)
- #3851 regression: issue1868_empty_output_builtin_still_pairs_fifo_open hangs and panics on master
- #3866 [bug] #3861 leaves issue3782 negative-side wrap pin red on master (fail-first cell never went green)
- #3899 regression(#3897): unclosed compound assignment at EOF exits 2, GNU exits 1 (compat_issue78)
- #3909 [bug] extdebug enabled at startup (-O/--debug) does not latch the BASH_ARGC bottom frame
- #3918 残差：issue3185 茎恒红——pwd 表项 [-LPW] 扩展后期望格未重生成
- #3921 残差：零长匹配切半载体 pair；字面 U+E000 按一单元走边界（#3546 余项）
- #3924 pwd usage 行断言互斥：issue705/issue3348 断言 [-LP]，issue2753/2791 断言 [-LPW]
- #3942 [bug] printf %q emits non-printable multibyte chars raw instead of $'\ooo' escapes in UTF-8 locales
- #3947 [bug] `$'\U...'` with a code point above U+10FFFF is left literal instead of being encoded
- #4059 [gnu-baseline] printf %T -2 start time shifted by imported SECONDS offset

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
