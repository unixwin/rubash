---
name: bashrs-progress
description: BashRS Rust rewrite - comprehensive bash syntax support as of 2026-07-04
metadata:
  type: project
---

# BashRS - Rust 重构 Bash 项目进度

## 当前状态 - 2026-07-04 (Session 10) ✅

### 测试状态

| 测试类型 | 数量 | 状态 |
|----------|------|------|
| Rust 库单元测试 | 173 | ✅ 通过 |
| 执行器集成测试 | 1002 | ✅ 通过 |
| 词法分析器集成测试 | 39 | ✅ 通过 |
| 解析器集成测试 | 48 | ✅ 通过 |
| 参数展开测试 | 15 | ✅ 通过 |
| CLI 集成测试 | 30 | ✅ 通过 |
| **Rust 测试总计** | **1,307** | **✅ 全部通过** |

### 本轮新增功能 (2026-07-04)

1. **花括号展开交叉积** - `{a,b}{1,2}` 现在正确输出 `a1 a2 b1 b2`
2. **输出进程替换 `>()`** - 支持 `>(command)` 作为输出目标
3. **通用 FD 重定向** - `3>file`, `3>>file`, `3<file` 不再影响 stdout/stderr
4. **exec FD 管理** - `exec 3>file` 和 `exec 3>&-` 基本支持
5. **`<>` 读写重定向** - 解析器已支持 `read_write` 标志
6. **编译器警告修复** - 0 warnings
7. **30 个新测试** - 覆盖花括号展开、进程替换、FD 重定向、参数展开等

### 代码规模

| 模块 | 行数 | 说明 |
|------|------|------|
| src/executor/mod.rs | 17,671 | 核心执行器 |
| tests/executor_tests.rs | 735,852 | 执行器测试 |
| src/parser/mod.rs | 2,033 | 解析器 |
| src/lexer/mod.rs | ~800 | 词法分析器 |

## 已实现功能

### 词法分析器 (Lexer)
- ✅ 分词、引号（单引号、双引号、ANSI-C引号）
- ✅ 变量 ($VAR, ${VAR})
- ✅ 命令替换和反引号
- ✅ 花括号展开 {a,b,c} {1..10..2}
- ✅ 注释、转义字符、反斜杠续行
- ✅ Here-document (<<, <<-, quoted delimiter)
- ✅ Here-string (<<<)
- ✅ 进程替换
- ✅ 关键字识别
- ✅ 赋值语句识别
- ✅ 算术命令识别

### 解析器 (Parser)
- ✅ AST 生成与完整命令列表
- ✅ 管道、And/Or 列表、分号分隔
- ✅ 重定向 (>, <, >>, 2>, 2>>)
- ✅ for 循环、算术 for 循环
- ✅ case 语句 (含 extglob 模式)
- ✅ select 菜单循环
- ✅ 函数定义 (两种形式)
- ✅ 花括号组、子 shell、后台执行
- ✅ Here-document/Here-string 解析
- ✅ 算术命令和算术展开

### 执行器 - 内建命令 (全部已实现)
echo, printf, cd, pwd, export, declare, local, readonly, unset, set, shopt,
test, [[ ]], read, mapfile, source, eval, exec, exit, return, break, continue,
shift, getopts, trap, alias, unalias, hash, type, command, builtin, enable,
pushd, popd, dirs, jobs, wait, disown, fg, bg, suspend, kill, ulimit, umask,
times, history, fc, bind, caller, help, logout, dirname, basename, let

### 变量展开 (Parameter Expansion) - 完整实现
- ✅ 基本展开、默认值、子串、模式删除
- ✅ 搜索替换、大小写转换
- ✅ 间接展开、前缀展开、长度
- ✅ 数组/关联数组操作
- ✅ 变量属性 (declare -i/-a/-A/-r/-x/-n/-l/-u)
- ✅ Nameref、复合赋值、算术赋值
- ✅ 变量转换 (@Q, @E, @P, @A, @a)
- ✅ 特殊参数 ($?, $!, $$, $0, $@, $*, $# 等)

### 控制流 - 完整实现
- ✅ if/elif/else/fi
- ✅ while/until/do/done (含 stdin 重定向)
- ✅ for/do/done (词列表和算术形式)
- ✅ case/esac (含 ;; / ;& / ;;&)
- ✅ select/do/done
- ✅ break N / continue N
- ✅ 花括号组、子 shell

### 算术 - 完整实现
- ✅ 全部运算符、赋值、比较、位运算
- ✅ 进制常数、三元条件
- ✅ RANDOM/SECONDS/BASHPID 动态变量

### 其他 - 完整实现
- ✅ 管道与 pipefail/PIPESTATUS
- ✅ 函数 (定义、局部变量、返回、调用栈)
- ✅ 别名展开 (含 parser-level alias)
- ✅ 花括号/波浪号/参数/命令/算术/路径名展开
- ✅ Extglob: ?(...), *(...), +(...), @(...), !(...)
- ✅ Shell 选项 (errexit, nounset, xtrace, pipefail, posix, noglob, noexec)
- ✅ 作业控制 (jobs, wait, disown, fg, bg)


### 本轮新增功能 (2026-07-04 第二轮)

1. **IFS 字段分割修复** - `splits_unquoted_expanded_word` 现在正确处理非空白 IFS 字符（如 `IFS=:`）
2. **IFS 环境隔离修复** - `Executor::new()` 始终重置 IFS 为默认值，防止测试间环境污染
3. **Extglob 路径名展开** - `pattern_contains_glob` 和 `pathname_expand_word` 现在识别 `@(`、`+(`、`!(` 等 extglob 模式
4. **`${!prefix@}` 引号区分** - 在双引号内，`${!prefix@}` 返回多个独立单词，`${!prefix*}` 返回 IFS 首字符连接的单词
5. **`read -t timeout`** - 解析并实现超时读取，支持 `timeout=0` 非阻塞读取
6. **`read -u fd`** - 解析 FD 参数
7. **`mapfile -u fd`** - 从指定 FD 读取输入
8. **ERR trap 完整实现** - 在命令失败时自动触发 ERR trap（不触发于 `||`/`&&`/`!` 上下文）
9. **RETURN trap 实现** - 在函数返回时自动触发 RETURN trap
10. **DEBUG trap 完整实现** - 在每个简单命令前触发 DEBUG trap，带递归保护
11. **`time` 命令改进** - 实际执行被计时的命令并测量执行时间，支持 `-p` POSIX 格式
12. **13 个新测试** - 覆盖 IFS、extglob、ERR trap、RETURN trap、DEBUG trap、time 等功能

## 运行

```
cargo test      # 运行 Rust 测试 (1,054个)
scripts/run-bash-upstream-tests.sh  # 运行 GNU Bash upstream runner
cargo run       # 启动 shell
```

### 本轮新增功能 (2026-07-04 第三轮)

1. **`${x^^[a-z]}` 大小写转换修复** — 修复了 `[a-z]` 中的 `-` 被误判为 `${var-default}` 操作符的 bug，将 case_mod 检查移到操作符分发之前
2. **`times` 命令实现实时计时** — 使用 `std::time::Instant` 追踪 shell 启动以来的实际挂钟时间，输出格式 `XmSS.CCs`
3. **`time` 命令格式修复** — 修复 `time` 输出格式为 `XmSS.CCs`（分钟+秒+厘秒），之前输出错误的 `0m107.00s`
4. **`shift_verbose` 支持** — 当 `shopt -s shift_verbose` 启用且 shift 超出 `$#` 时输出错误信息
5. **`compgen -W` 完整实现** — 支持 `-W wordlist`、`-A variable/function/alias/builtin/command/file/directory`、`-P prefix`、`-S suffix`、`-X filter` 选项
6. **`fc` 命令完整实现** — 支持 `fc -l`（列出历史）、`fc -s old=new`（替换重执行）、`fc -e editor`（编辑器模式，打印不支持）、历史记录基础设施
7. **37+ 个新测试** — 覆盖大小写转换范围模式、times 实时计时、compgen、fc 等
8. **编译器警告清理** — 0 warnings

### 本轮新增功能 (2026-07-04 第四轮)

1. **`history` 命令完整实现** — 支持列出历史（`history`、`history N`）、清除（`-c`）、删除条目（`-d offset`）、追加到文件（`-a`）、写入文件（`-w`）、从文件读取（`-r`）、打印不记录（`-p`）、替换记录（`-s`）
2. **`jobs` 命令实现** — 集成 `BackgroundChild` 作业表，支持列出后台作业、显示作业状态、`%N` 作业规格引用
3. **`wait` 命令实现** — 支持等待所有后台子进程（无参数）、按 PID 等待、按作业规格（`%N`）等待、`-n` 等待任意完成
4. **`umask` 真实系统调用** — Unix 平台调用 `libc::umask()`，Windows 保持内部状态；添加 9 个单元测试
5. **`disown` 命令实现** — 集成作业表，支持从后台作业列表移除
6. **后台作业跟踪基础设施** — `BackgroundChild` 结构体、原子 PID 分配器、作业生命周期管理
7. **67+ 个新测试** — 覆盖 history、jobs、wait、umask、disown 等功能

### 本轮新增功能 (2026-07-04 第五轮)

1. **`exec > file` 持久重定向** — `exec > file` 现在永久重定向后续所有命令的 stdout 到文件，支持 echo 和所有 builtin
2. **`complete` 命令完整实现** — 支持 `-F funcname`（注册完成函数）、`-A action`（动作补全）、`-W wordlist`（词列表）、`-r`（移除）、`-p`（打印所有注册的补全）
3. **`compopt` 命令实现** — 支持 `-o filenames` 选项修改补全规格、检查补全规格是否存在
4. **`compgen` 注册集成** — `compgen` 现在从 `CompletionRegistry` 查询已注册的补全规格
5. **`ulimit` 真实系统调用** — Unix 平台使用 `libc::getrlimit`/`setrlimit` 查询和设置资源限制
6. **`trap` 信号处理基础设施** — 添加 `PENDING_SIGNAL`、`SIGNAL_HANDLER_INSTALLED` 静态变量和 `pending_signal` 字段
7. **30+ 个新测试** — 覆盖 complete 注册、compopt、ulimit 等


### 本轮新增功能 (2026-07-04 第六轮 - 语法修复)

1. **And/Or 列表分号处理修复** — `false && echo a; echo b` 现在正确输出 `b`（之前 `echo b` 被错误跳过）。重写 `skip_and_or_rhs` 函数，逐个评估 `&&`/`||` 连接器而非跳过整条链。
2. **嵌套 `$()` 命令替换修复** — `$(echo $(echo nested))` 现在正确输出 `nested`（之前输出 ` nested)`）。修复 `split_shell_words` 函数跟踪 `$()` 深度，避免在嵌套命令替换内部分割空格。同时修复 `$(` 分支中 `i += 2` 后循环 `i += 1` 导致跳过字符的 bug。
3. **管道到花括号组修复** — `echo hello | { cat; }` 现在正确传递 stdin。修复 `execute_pipeline_stage` 对复合命令返回 `None`（而非空输出），让回退路径设置 `FUNCTION_STDIN`。同时修复 `cat` 命令在无参数/重定向时检查 `FUNCTION_STDIN`。
4. **子 shell 中 EXIT trap 修复** — `(trap 'echo trapped' EXIT)` 现在正确触发 EXIT trap。在 `subshell_end` 处理中添加 `run_exit_trap_for_status` 调用，在恢复父 shell 环境之前执行 EXIT trap。
5. **Case `;;&` 模式穿透** — 已确认正常工作（`case foo in fo*) echo 1 ;;& *oo) echo 2 ;; esac` 输出 `12`）。
6. **进程替换** — `diff <(echo a) <(echo b)` 已确认正常工作。
7. **Globstar** — `shopt -s globstar` 已确认正常工作。
8. **Coproc** — `coproc myproc { cat; }` 已确认正常工作。
9. **0 个编译器警告**


### 本轮新增功能 (2026-07-04 第七轮 - Extglob Negation 修复)

1. **Extglob `!()` 否定模式词法分析修复** — 修复 lexer 中 `!` 字符后跟 `(` 时被识别为 Keyword 而非 extglob 模式的 bug。`!(foo)` 现在被正确词法分析为单个 Word token。
2. **Extglob `!()` 模式匹配算法重写** — 重写 `match_extglob` 函数中 `'!'` 分支的实现。旧实现逐字符匹配（错误），新实现遍历所有可能的子串分割，检查消耗部分是否不匹配任何 alternative 且剩余模式匹配。
3. **0 个编译器警告**
