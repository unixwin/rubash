# rubash 1.4.0 Release Notes（草稿 / DRAFT）

> 状态：draft。随 `release/1.4.0-prep` 评审更新，打 tag 前定稿。
> 版本：Cargo.toml `1.4.0`（crates.io 上一发布为 `1.2.0`）。
> 详细变更账本见 [CHANGELOG.md](../CHANGELOG.md) 的 `[1.4.0]` 小节。

---

## 中文

rubash 1.4.0 是 GNU Bash 5.3.0 兼容性的一次大版本推进：83 套件 true-baseline 总差异从 3427 行降至 2072 行（**−40%**），零差套件从 31 增至 **32**。本版本带来登录 shell、真 readline 交互 REPL、粘贴输入规范化、Android / OpenHarmony 交叉目标支持，以及 #437/#281/#242 三条性能线的实质推进。

### 亮点

- **登录 shell**（#487）：`-l`/`--login`（含 `-lc` 组合与 argv[0] 以 `-` 开头）走完整登录启动链——Unix 上依次 source `/etc/profile` → 首个存在的 `~/.bash_profile | ~/.bash_login | ~/.profile`；非登录交互 shell source `/etc/bash.bashrc`（存在时）+ `~/.bashrc`；`--noprofile`/`--norc` 分别抑制对应分支。Windows 无真实登录概念，`-l` 接受且行为不变。

- **真 readline REPL**（#419 readline 腿）：裸 `rubash.exe` 真控制台交互不再由 conhost 代管行编辑。新增 raw 控制台读行（`src/console_readline.rs`）：历史（↑/↓、C-p/C-n）、行内光标移动（C-a/C-e/←/→）与 C-r 反向搜索在真控制台下全部可用，与 cooked 桥接路径共享同一套编辑绑定；命令执行时恢复 cooked 行纪律，子进程不受影响。

- **粘贴语义**（#458）：桥接层送达的交互式输入行在进入引擎前规范化 C0 控制字节——终端粘贴携带的 NUL/控制字节不再以未定义方式进入词法。

- **Android / OpenHarmony 目标**：
  - aarch64/armv7 android 目标可针对 bionic 编译（#456）；
  - kill/trap 信号表在 android 上同样编译 Linux 编号的 1..31 槽位——此前 `kill -l 17` 在 android 上输出为空、`trap -l` 列出 bionic 不用的 BSD 名（#492）；
  - `ulimit` 针对 musl 与 OpenHarmony libc 编译修复（#449）；
  - 全仓 511 处平台 cfg 门审计（#492）。

- **性能三连（部分落地）**：
  - **#281 增量 reader 收官**（#486）：续行扫描 admission-gate、零拷贝参数体视图、逐行 batch-tokenizer 停靠、fold 完成趟 opener-state checkpoint——大文件增量解析不再整段重 lex（nvm 形状 270 KB 重扫归零）；
  - **#437 热点**：算术循环 per-command boundary O(1) 化；
  - **#242 执行器热路径（部分）**：named-event catch_flag 信号轮询、单遍 `${}` 算子索引 + simple-glob 直接匹配、walker `${}` 臂快路径、comsub-scan O(n²) 修复等；
  - 另有 **#375 第一刀**（#484）：`$(cd X && pwd)` 路径解析单句柄化，`$(cd && pwd)×5000` **−11%**、bats_debug_trap 形状 −8%，输出逐位等价。

- **`--version` 构建元数据**（#488）：GNU license 块之后追加 `build: <git-hash> (<profile>)`（build.rs 编译期捕获）；Windows 附加 persona 说明（原生 Windows 构建、无 MSYS runtime）。

- **新命令**：GNU `cp` 移植（-r/-n/-i/-v/-p/-u/-t、递归、缓冲 stderr）；`history -d start-end` 范围删除（GNU 5.3 特性）。

### Bug 修复清单

语义对齐（GNU 一致性）：

- 命令替换的退出状态在展开期内即刻成为 `$?`（#485）：后置 `$?`（后续词、同词后缀、同命令赋值 RHS）看得见替换状态，成功替换同样覆盖（PR #491/#494）。
- 管道中未加引号变量作命令字按 IFS 分词（#68）。
- 命令替换内的 `eval` 恢复完整重解析语义（#69）。
- `set -u` 下算术展开不再把已赋值变量判为 unbound（#67）；unbound 错误与普通参数展开对齐（直接上下文终止脚本，替换/管道段内只终止子上下文，GNU expr.c FORCE_EOF 语义）。
- 函数定义与花括号组的同行 `#` 尾注释不再解析失败（#118）。
- SIGPIPE：管道上游 broken pipe 不再中止解释器，`yes | head` 与 GNU 一致（#440，PR #455）。
- ERR trap 不再对词/赋值展开错误触发（PR #467）。
- 引号整词默认展开保持单一复合元素（PR #466）。

解析器/词法：

- 赋值前缀 + 子 shell 的 GNU 形态（#452 集群1，PR #453）。
- 复合体内 `]]` / `(( ))` 之后无终止符的 then/do 被接受（#460，PR #475）。
- 函数定义名含未引号 `$( )` 被接受（#462，PR #476）。
- `=~` 正则词片段永远是词数据，不再被误判为保留字（#461，PR #473）。
- 嵌套 keyword-form 函数体计入组 opener 追踪，不再提前 EOF（#465，PR #472）。
- 词附着的多行进程替换体跨行 join（#463，PR #483）。
- 词首数据 raw ESC 字节（如单引号首字符）不再被吞（niubash#200，PR #474）。

交互与会话：

- source 边界的语法错误不再终止会话/脚本（#451，PR #481）：交互报错后继续读下一条，非交互脚本继续执行。
- PS1 `\[`/`\]` 忽略标记在所有渲染路径剥离（#431，PR #480）：oh-my-bash / bash-it 主题不再泄漏标记字节；`${var@P}` 同步剥离。

兼容性家族归零 / 收敛（true-baseline 差异行数）：

- dbg-support 全族归零（635→0）：AND-列表双触发、source-scope trap 继承、非行首 `{` 词法、for 每迭代行号。
- rsh 全族归零（194→0）：`set +o restricted` 静默解除、BASH_CMDS 守卫、hash -p、受限 argv0。
- func 全族归零（58→0）：POSIX funcname 规则、AST printer、special-builtin 优先级。
- complete 全族归零（115→0）：多操作数 compspec 注册；另接受带连字符函数名（#427，PR #454）。
- invocation 全族归零（14→0）：BASH_ARGV0 导入、login argv0、长选项表、-o/-O 报错 prolog、--pretty-print。
- trap 归零（3→0）：ERR $LINENO、SIGCHLD 排队重放、后台剥离继承 trap 表、DEBUG 行号。
- history（190→127）：HISTIGNORE、fc -s 语义、comsub 内历史路由；`history -d` 范围删除见新增。
- globstar（182→101）：非相邻多 `**`、相邻 `**` 折叠尾斜杠。
- array/assoc（444+358→246+242）：复合赋值引号分组四层修复、元素赋值词边界、嵌套引号 patsub。
- 其他：unicode `\u`/`\U` 部分读取与位数要求、locale 子系统（setlocale 警告、MB_STRLEN）、comsub/pipeline（pathname-expand、管线 word list、assoc join 顺序）、assignment（全单引号 RHS、元素赋值不分词）、nameref 空值目标赋值、coproc 函数表快照（#441，PR #469）、Unix 进程替换 fd 传参（PR #468）。

### 升级 / 安装

```bash
# crates.io 发布后
cargo install rubash --version 1.4.0
```

二进制 release asset 由 CI 在 `v1.4.0` tag 上自动构建（见 release workflow）。

---

## English

rubash 1.4.0 is a major step in GNU Bash 5.3.0 compatibility: total true-baseline diff across the 83 suites drops from 3427 to 2072 lines (**−40%**), with zero-diff suites up from 31 to **32**. It ships a login shell, a real readline REPL, paste-input normalization, Android / OpenHarmony cross-target support, and substantive progress on the #437/#281/#242 performance fronts.

### Highlights

- **Login shell** (#487): `-l`/`--login` (including `-lc` and a leading `-` in argv[0]) runs the full login startup chain — on Unix, source `/etc/profile` → the first of `~/.bash_profile | ~/.bash_login | ~/.profile`; non-login interactive shells source `/etc/bash.bashrc` (when present) + `~/.bashrc`; `--noprofile`/`--norc` suppress the respective branches. Windows has no real login concept; `-l` is accepted and behavior is unchanged.

- **Real readline REPL** (#419 readline leg): the bare `rubash.exe` console no longer delegates line editing to conhost. A new raw-console line reader (`src/console_readline.rs`) brings history (↑/↓, C-p/C-n), in-line cursor motion (C-a/C-e/←/→) and C-r reverse search to real-console sessions, sharing one set of editing bindings with the cooked bridged path; cooked line discipline is restored before command execution so children are unaffected.

- **Paste semantics** (#458): interactive input lines delivered by the bridge are normalized for C0 control bytes before entering the engine — NUL/control bytes carried by terminal paste no longer reach the lexer undefined.

- **Android / OpenHarmony targets**:
  - aarch64/armv7 android targets compile against bionic (#456);
  - kill/trap signal tables compile the Linux-numbered 1..31 slots on android too — previously `kill -l 17` printed nothing and `trap -l` listed BSD names bionic does not use (#492);
  - `ulimit` compiles against musl and OpenHarmony libcs (#449);
  - full audit of all 511 platform cfg gates in src/ (#492).

- **Performance trio (partially landed)**:
  - **#281 incremental reader, closed** (#486): admission-gated continuation scans, zero-copy parameter-body views, per-line batch-tokenizer parking, and an opener-state checkpoint for fold-completion passes — incremental parsing of large files no longer re-lexes the whole buffer (the 270 KB rescans on the nvm shape are gone);
  - **#437 hotspots**: O(1) per-command boundary on arithmetic loops;
  - **#242 executor hot path (partial)**: named-event catch_flag signal polling, one-pass `${}` operator indexing + direct simple-glob matching, walker `${}`-arm fast paths, comsub-scan O(n²) fix, and more;
  - plus the **first cut of #375** (#484): single-handle `$(cd X && pwd)` path resolution — `$(cd && pwd)×5000` is **−11%**, the bats_debug_trap shape −8%, bit-identical output.

- **`--version` build metadata** (#488): appends `build: <git-hash> (<profile>)` after the GNU license block (captured by build.rs); on Windows, a persona note (native Windows build, no MSYS runtime).

- **New commands**: GNU `cp` port (-r/-n/-i/-v/-p/-u/-t, recursion, buffered stderr); `history -d start-end` range deletion (a GNU 5.3 feature).

### Bug fixes

Semantics alignment (GNU fidelity):

- Command-substitution exit status becomes `$?` immediately during expansion (#485): a later `$?` on the same command line (subsequent words, same-word suffixes, subsequent assignment RHS) observes it; successful substitutions overwrite too (PRs #491/#494).
- Unquoted variables as command words in pipelines undergo IFS splitting (#68).
- `eval` inside command substitution regains full re-parsing semantics (#69).
- Arithmetic expansion under `set -u` no longer flags assigned variables as unbound (#67); unbound errors align with ordinary parameter expansion (FORCE_EOF semantics: terminate the script in direct context, only the sub-context inside command substitutions and pipeline segments).
- Same-line `#` comments after function definitions and brace groups no longer fail to parse (#118).
- SIGPIPE: broken-pipe writes on pipeline upstream no longer abort the interpreter; `yes | head` matches GNU (#440, PR #455).
- The ERR trap no longer fires for word/assignment expansion errors (PR #467).
- Quoted whole-word default expansions stay one compound element (PR #466).

Parser / lexer:

- Assignment prefix + subshell GNU form (corpus cluster #452-1, PR #453).
- Terminator-less then/do after `]]` and `(( ))` inside compound bodies is accepted (#460, PR #475).
- Function-definition name words containing unquoted `$( )` are accepted (#462, PR #476).
- `=~` regex word fragments are word data, never reserved words (#461, PR #473).
- Nested keyword-form function bodies count as group openers; no premature EOF (#465, PR #472).
- Word-attached multi-line process substitutions join across lines (#463, PR #483).
- A word-initial data ESC byte (e.g. first char inside single quotes) is no longer dropped (niubash#200, PR #474).

Interaction and sessions:

- Syntax errors at the source boundary no longer kill the session/script (#451, PR #481): interactive shells keep reading the next line after the diagnostic; non-interactive scripts continue.
- PS1 `\[`/`\]` ignore-markers are stripped on every render path (#431, PR #480): oh-my-bash / bash-it themes no longer leak marker bytes; `${var@P}` strips too.

Compatibility families zeroed / reduced (true-baseline diff lines):

- dbg-support family zeroed (635→0), rsh family zeroed (194→0), func family zeroed (58→0), complete family zeroed (115→0, plus dashed `-F` function names, #427/PR #454), invocation family zeroed (14→0), trap zeroed (3→0).
- history improved (190→127), globstar improved (182→101), array/assoc improved (444+358→246+242).
- Also: unicode `\u`/`\U` partial reads and exact hex-digit counts, locale subsystem (setlocale warnings, MB_STRLEN), comsub/pipeline (pathname-expand, pipeline word lists, assoc join order), assignment (all-single-quote RHS, no word splitting on element assignments), nameref assignment through empty namerefs, coproc function-table snapshot for DEBUG traps (#441, PR #469), process-substitution fds passed to external command arguments on Unix (PR #468).

### Upgrade / Install

```bash
# once published to crates.io
cargo install rubash --version 1.4.0
```

Binary release assets are built automatically by CI on the `v1.4.0` tag (see the release workflow).
