# 更新日志

所有重要的项目更改都将记录在此文件中。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)。

## [Unreleased]

GNU Bash 5.3.0 兼容性大幅推进。83 套件 true-baseline 总差异从 3427 行降至 2072 行（−40%），零差套件从 31 增至 32。

### 修复

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
- **history 改善**（190→127）：`history -d start-end` 范围删除（GNU 5.3 特性）、HISTIGNORE harness 修复、fc -s 语义对齐、命令替换内 fc/session 历史路由。
- **globstar 改善**（182→101）：非相邻多个 `**` 重复发射修复、相邻 `**` 折叠后零深度目录尾斜杠。
- **array/assoc 改善**（444+358→246+242）：复合赋值引号分组四层修复（解析器 RAW 合并、declare 操作数收集器、Word 臂原子复合、执行器逐字守卫）、元素赋值词边界、嵌套引号 patsub 逐元素替换。
- **信号编号统一**：rubash 全表从 BSD/Cygwin 风格切换到 Linux 表（USR1=10、CHLD=17、RTMIN=34），与 GNU 5.3.0 WSL 契约一致；kill -l/trap -l 输出逐字节对齐。
- **unicode 修复**：printf 和 ANSI-C 引号中 `\u`/`\U` 部分读取保留、精确十六进制位数要求。
- **locale 子系统**：从 feat/locale-subsystem 合入，setlocale 警告、MB_STRLEN 长度。
- **外部命令**：GNU cp 移植（-r/-n/-i/-v/-p/-u/-t、递归、缓冲 stderr）、/bin/echo 和 /usr/bin/echo 路由到缓冲 builtin 通道。
- **comsub/pipeline**：pathname-expand 替换、管线 word list、assoc hash order 共享 join。
- **assignment**：全单引号 RHS 为字面数据、元素赋值词永不分词、转义引号内下标为数据。
- **nameref**：无值 nameref 接受有效目标赋值。

### 文档

- 重写 README.md 和 README.zh-CN.md：数据驱动的兼容性展示、近期修复表、架构概览。
- 更新 COMPATIBILITY-STATUS.md：2026-09-11 全量 true-baseline 重跑结果、总体结论刷新。
- 归档过时文档 7 个（gnu-bash-compatibility-implementation-plan、bash-implementation-inventory、bash-source-map、performance-debugging-process、typed-expansion-migration-checkpoint、source-layout、HANDOFF-20260829）。

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

- [GitHub Releases](https://github.com/unixwin/rubash/releases)
- [问题跟踪器](https://github.com/unixwin/rubash/issues)
