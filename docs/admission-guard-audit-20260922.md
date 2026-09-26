# 词级准入守卫全仓审计（#117 未完成部分）

> **状态（2026-09-26 更新）**：C 类八桩（C1-C8）已全部清除并核销
> （2026-09-25，指纹 grep 归零；回归覆盖 `tests/c_stub_regressions.rs`）。
> B 类九族中 B4/B6/B8 已落地；**B1/B2/B3/B5/B7 仍是 TASKBOARD Q1 的活跃
> 工作计划，本文 B 节清单是其依据，保留为活文档。**

日期：2026-09-22。基线：master `a2e14bff`（任务书写的 bfa3ce43 已不是 HEAD，按现状审计）。
范围：`src/executor/`、`src/builtins/` 全部 `contains(` / `starts_with(` 命中共 **1123 行**
（原始清单：`target/issue-suites/results/admit-audit-lines.txt`，带上下文转储：
`target/issue-suites/results/admit-audit-dump.txt`）。
方法：逐条看上下文分类（A/B/C/D），参照范例 = commit 2d7c973a 删除的 comsub 黑名单级联
与白名单谓词 `command_substitution_body_is_trivial`。本审计只读，未改任何 src/。

## A. 分类统计

| 类 | 定义 | 约数 | 占比 |
| --- | --- | --- | --- |
| **A 合法** | 纯词法/字符分类、builtin 选项解析（`starts_with('-')`）、路径分隔符/Windows 盘符、内部 marker 前缀（`STORAGE_WORD_PREFIX`、`QUOTED_WORD_PREFIX`、`__RUBASH_*`）、信号/job 名（`%`）、测试断言 | ~1000 | 89% |
| **B 语义准入守卫**（#117 危险模式） | 用文本特征决定词/命令走哪条语义路径 | ~90 行（9 个语义族，见下表） | 8% |
| **C 硬编码输出桩** | 字面量指纹 + 固定输出/退出码 | ~23 行（8 处桩） | 2% |
| **D 诊断/日志** | `diagnostic_prefix()` 组装 `<script>: line N:` 等 | ~10 行 | <1% |

大块 A 类说明（不必逐条列入）：
- `read_builtin.rs` 107 处、`mapfile_builtin.rs` 16 处：`read -rsa/-ersa/-srep…` 组合选项的
  **排列枚举级联**（几百行手写排列）。语义上属 builtin 选项解析（合法），但实现是
  排列黑名单——与 read.def 的 `internal_getopt`（builtins/getopts.c 通用组合 flag 扫描）相比
  是结构性坏味道，建议作为独立重构项（非 #117 语义事故，不阻塞）。
- `declare.rs`/`declare_local.rs` ~80 处：属性字母/`-`/`+`/`=` 解析，A。
- `path.rs` 29 处：`/mnt/`、`/tmp`、MZ 头、UNC 路径，A。
- 其余多为 `starts_with('-')` 选项与 `contains('=')` 赋值词法判断。

## B. B/C 类完整清单

### C 类桩（最危险：伪造输出，无真实语义）

| # | file:line | 准入条件（原文） | 伪造的语义 | GNU 锚点 | 替换方案 | 风险 |
| --- | --- | --- | --- | --- | --- | --- |
| C1 | `src/executor/expand_word.rs:279` | `word.contains("kill -l") && word.contains("128") && word.contains('+')` → 固定返回 `"HUP"` | `kill -l 128+1` 之类任意含 128 与 + 的词都打印 HUP（2d7c973a 删掉了 128→129，但这条 HUP 桩漏网） | `builtins/kill.def` `explain()`；`trap.c` `signames[]` | 删桩；`kill -l <expr>` 走真实求值 + 信号名表 | 高 |
| C2 | `src/executor/shift_echo_builtins.rs:143-155` | `__RUBASH_SCRIPT_NAME` ends_with `type4.sub` 且词含 `coprocs` → exit 0；ends_with `type5.sub` 且词含 `unset PATH` → exit 0 | `type -p/-t coproc 族` 与 `PATH 清空后 type` 的真实查找结果 | `type.def` `describe_command()`（2d7c973a 已注明 type -p 的 `./`-$PWD fallback 仍缺） | 实现 describe_command 语义（含 unset PATH 的 `./` fallback），删桩 | 高 |
| C3 | `src/executor/command_dispatch_primary.rs:61-68` | `script.contains("type3.sub")` → `cd` 直接 exit 0 | `cd` 真实行为（type3.sub 场景） | `cd.def` `cd_builtin()` | 删桩走 `execute_cd`，用 cd 测试套件收口 | 高 |
| C4 | `src/executor/type_functions.rs:131` | `body_text.contains("cat -u")` → 打印固定 coproc 重解析文本 | `coproc` 语法的 AST 重打文本 | `parse.y` print 族（`print_comsub` parse.y:4632 同族）；`coproc.def` | 实现 coproc 节点 reprint，删桩 | 中 |
| C5 | `src/executor/alias_helpers.rs:483` `is_aliasconv_line_pattern` | `pattern.contains("[a-zA-Z0-9_-]*") && pattern.contains('\t') && pattern.contains(r"\(.*\)")`，replacement 还必须是字面量 `mkalias \1 '\2'` | 假 `sed s///`：只对 aliasconv 测试里那一条 BRE 指纹生效 | `lib/glob/smatch`/BRE：GNU sed 是外部命令；rubash 内嵌 sed-replacement 应为通用 BRE 子集 | 接 WinuxCmd `sed` 或实现小 BRE 引擎；该行同时是 WinuxCmd 域候选 | 中 |
| C6 | `src/executor/trap_exec.rs:152-153, 173` | `source.contains("}>_[$($())]") \|\| source.contains("}>_[")` 且 `contains("{ echo")`；另一条 `source.contains(">_[${")` | eval 内函数定义+游离 `{` 的两条 parse.y 报错文本（含行号计算）整段硬编码，注释自认 "keep the check narrow to the two eval payloads" | `parse.y` 报错路径（`syntax error near unexpected token`、`unexpected EOF while looking for matching }`，`print_offending_line`） | 在真解析器里补函数定义体闭合/`${` 内 `}` 的错误路径，删指纹桩 | 高 |
| C7 | `src/executor/command_execute.rs:292-307` | `raw.contains("\\!") && raw.contains('"')`（且 words 恰为 `echo !`）→ 把 `!` 恢复为 `\!` | 双引号内 `\!` 的 history expansion 引号态；注释自认 "narrow to `\!` … the only … in histexp1" | `history.c` `history_expand()`；`subst.c` histexp 引号处理 | 让 history expansion 携带引号态（或按 PST_NOEXPAND 同族处理），删桩 | 中 |
| C8 | `src/executor/external_inner.rs:519` | `cmd.words.iter().any(\|word\| word.contains("coprocs"))` | C2 同族（coprocs 场景整体短路） | 同 C2 | 同 C2 | 中 |

### B 类语义准入守卫（按语义族分组）

| 族 | file:line（代表行；同族行随附） | 准入条件原文（代表） | 守护/伪造的语义 | GNU 锚点 | 替换方案 | 风险 |
| --- | --- | --- | --- | --- | --- | --- |
| **B1 展开字符文本探针（comsub/hoist 快路径准入）** | `assignment_expansion.rs:70-73, 370-372, 556, 597, 691, 728, 966`；`parameter_core.rs:129, 167, 288, 305, 309`；`expand_word.rs:89`；`embedded_mutations.rs:185, 262`；`command_execute.rs:332`；`command_prepare.rs:286, 805, 2021`；`arrays.rs:733, 837, 926`；`subscript_expansion.rs:106` | `!value.contains("${") && !value.contains("$'") && !value.contains("$(") && !value.contains('`')`；`word.contains("$(") \|\| word.contains('`')`；`word.contains("$((") \|\| word.contains("$[")` 等 | 决定赋值值/词是否走快速替换路径；漏一个字符类（`$"…"`、`$[[`、`$_`……）就复现 #59-#116 家族 bug。部分点（hoist_data_double_quotes:70）已是白名单式准入并有 GNU 注释，属合格样本；其余是散落的黑名单点 | `subst.c:11229 expand_word_internal()`；`subst.c:7143 command_substitute()`；`parse.y:4451 parse_comsub()` | 收敛到 2d7c973a 模式：统一白名单谓词（纯字面量/带完整引号结构才走捷径），或由 lexer 发出 typed flag，消灭逐点 contains | **最高**（行数最多、事故史最长） |
| **B2 `${…}` 备选词内 `$@`/`$*` 文本探针** | `command_prepare.rs:962, 1365-1368, 1401, 1460, 1525-1528, 1674-1677, 1770-1773, 2257`（约 30 行） | `alternate.contains("$@") \|\| alternate.contains("${@") \|\| contains("$*") \|\| contains("${*")` | 决定 `${x:-word}` 等备选词是否按位置参数逐词拆分/加引号；GNU 用展开返回的 `quoted_dollar_at` 标志，不做文本重扫 | `subst.c:11229 expand_word_internal()`（`quoted_dollar_at_p`、split 标志）；`subst.c:9777 parameter_brace_expand()` | 扩展结果携带 split/quote 标志（typed-carrier 同策略），删除 alternate 重扫 | 高 |
| **B3 别名重解析准入（文本特征决定是否重跑真解析器）** | `assignment_dispatch.rs:18-28`；`command_execute.rs:124`；`alias_reparse.rs:544`；同族 heredoc 探针 `embedded_mutations.rs:1342`、`command_substitution_pipelines.rs:224, 329` | 词表含 `; < > >> \| &` 或 `contains('=')/contains("$(")/contains('`')` 才 reparse；`source.contains("<<")` 才走 heredoc 重解析分支 | 别名展开后的语法重解析准入；黑名单漏类（如 `case`、`[[`、`(`）即静默丢语义。GNU 在 reader/yylex 循环内展开别名（parse.y reader），不存在"事后文本 reparse 准入" | `parse.y` reader 循环别名展开（`alias.c` `alias_expand`）；`subst.c:7143`→`parse_and_execute` | 把别名展开前移进 lexer/token 流（GNU 模型）；过渡期把准入反转为白名单（仅纯字面量词不 reparse） | 高 |
| **B4 BASH_COMMAND / `${!` 引用探测** | `command_text.rs:186-201` | `parameter.text.contains("BASH_COMMAND") \|\| text.contains("${!")`（对 words/word_metadata/assignments/redirects/heredoc 全扫） | 决定是否设置 BASH_COMMAND / 处理 `${!}` 间接；文本 contains 会把注释/字符串里的字面量误判 | `execute_cmd.c:624 execute_command_internal()`（BASH_COMMAND 赋值点）；`variables.c` | AST 判定：parameter 节点名/间接标志，不扫原始文本 | 中 |
| **B5 进程替换词形准入 `<(` / `>(`** | `pipeline_exec.rs:2166-2186`；`external_setup.rs:40, 53, 295-296, 587, 920`；`read_builtin.rs:54`；`command_prepare.rs:491-500` | `word.starts_with("<(") && word.ends_with(')')`；`raw.contains("<(")` | 参数位进程替换的识别（GNU 在解析/重定向阶段即识别，不做参数词事后形状判定） | `parse.y`（r_dir/redir 词）、`redir.c:298 redirection_expand()` | 解析阶段识别 procsub 为重定向/命令节点，删词形判定 | 中 |
| **B6 条件命令原始文本探针** | `conditional.rs:264-305, 623` | `m.raw.contains('\\')`；`!raw.contains('[')` | `[[ ]]` 内词是否含转义/是否模式；文本特征决定模式匹配语义 | `subst.c` 条件展开、`lib/glob` | 由 word_metadata 结构化携带转义态 | 低-中 |
| **B7 echo 别名反引义探针** | `src/builtins/echo.rs:103, 123` | `arg.contains("\\'")`；`arg.starts_with('$') && arg.contains(PROTECTED_BACKSLASH)` | 别名 value 经 echo 时的引号还原；受控探针但仍是字符黑名单 | `alias.c`（alias value 存储）；`print_def` 引号输出 | alias value 结构化存储引号态 | 低 |
| **B8 THIS_SH 子串判定** | `external_finish.rs:59, 100` | `command_name.contains("THIS_SH")` | 决定外层脚本 vs 同 shell 内嵌执行（fork 折叠）；子串匹配可被 `echo THIS_SHxx` 误触发；同函数内已有正确的路径比较 `expanded_is_this_shell` | `execute_cmd.c:624`（THIS_SH 由 `shell_execve` 路径比较设定） | 删除子串分支，仅用路径等价比较 | 低 |

## C. 分批替换计划

排序原则：危险度 × 行数影响；每批一个语义族，验收 = 对应套件 WSL GNU Bash 5.3.0 基线
（`scripts/true-baseline.sh`）无回归 + 新增聚焦回归。原始工件落
`target/issue-suites/results/true-baseline/<suite>/diff.txt`。

- **批次 1：C 类桩清除（8 处，~23 行）**
  范围：C1-C8。顺序：先实现真语义（kill -l 表、cd/type/coproc 路径、parse.y 两条错误路径、histexp 引号态），桩随实现删除；C5 aliasconv 假 sed 与 WinuxCmd 域联动（先提 issue 再定）。
  套件：`type`、`trap`、`kill`（kill-tests）、`histexp`、`alias/aliasconv`、`coproc` 相关。
  验收：基线 diff 不增；桩对应的 `.sub`/`.tests` 用例在删桩后仍绿；`grep -rn '__RUBASH_SCRIPT_NAME.*contains\|contains("type[0-9]' src/` 归零。
  风险：中——桩删早了会红套件，必须实现先行。

- **批次 2：B1 展开字符探针 → 白名单收敛（~40 行，9 个文件）**
  范围：把 assignment_expansion / parameter_core / expand_word / embedded_mutations / command_execute / command_prepare / arrays / subscript_expansion 的 `contains("$…")`/backtick 探针统一为 2d7c973a 式谓词（命名如 `word_is_literal_or_structured`），慢路径全部落到真展开器。
  套件：quote/quoting、comsub、dollar-quote（#109 族）、array、assign 全族 + 83 套件全量回归（carrier 域，必须全量）。
  验收：分类矩阵（赋值 RHS/参数位 × 字面量/变量/替换/算术 × 有无空格）与 WSL 5.3.0 逐字节一致（AGENTS.md 验证条）；全 83 套件账本无回归。
  风险：最高（carrier 字节域），按 AGENTS.md 需 83 套件全量账本。

- **批次 3：B2 `$@`/`$*` 标志化（~30 行，集中在 command_prepare.rs）**
  范围：展开结果携带 split/quoted 标志（typed-carrier 同策略），删除 alternate 文本重扫。
  套件：dstack、posparams、array、quoting/quote-splitting 族。
  验收：基线无回归 + `$@`/`$*` × 备选词/去引号 × 多位置参数差分矩阵逐字节一致。

- **批次 4：B3 别名重解析收敛（~15 行，4 个文件）**
  范围：别名展开前移 lexer；过渡期准入反转为白名单（含语法字符的词一律 reparse，纯字面量跳过——注意方向与现状相反，现状是"含语法字符才 reparse"，恰好是危险向）。
  套件：alias、appendix（alias 相关）、heredoc 族。
  验收：基线无回归；`a='echo hi; echo bye'` 类全类矩阵。

- **批次 5：B4+B5 解析阶段识别（~25 行）**
  范围：BASH_COMMAND/`${!` 改 AST 判定；procsub 在 parser/重定向阶段识别。
  套件：exec、bash-variables（BASH_COMMAND 用例）、procsub/process-substitution 族。
  验收：基线无回归。

- **批次 6：B6/B7/B8 收尾（~15 行）**
  套件：cond、alias、exec 族；验收同上。

独立重构项（非本审计阻塞项）：read_builtin/mapfile 组合选项排列级联 →
`internal_getopt`（builtins/getopts.c）式通用组合 flag 解析器，可删 ~300 行枚举。

## D. 预估总工作量

| 批次 | 规模 | 预估 |
| --- | --- | --- |
| 1（C 桩） | 8 桩 + 4 个真实现（kill 表/cd-type/coproc reprint/parse.y 错误路径/histexp 引号态） | 1.5-2 周 |
| 2（B1 白名单） | 9 文件 ~40 行 + 分类矩阵 + 83 全量 | 2-3 周（carrier 域，保守） |
| 3（B2 标志化） | ~30 行，依赖 typed-carrier 基建 | 1-1.5 周 |
| 4（B3 别名前移） | lexer 级改动 | 1-2 周 |
| 5（B4+B5） | parser 级改动 | 1 周 |
| 6（收尾） | ~15 行 | 0.5 周 |
| **合计** | | **约 7-10 人周** |

批次 2/3 若 typed-carrier 基建到位可合并，节省约 1 周。

## 审计注记

- 本次 grep 含少量误报（变量/函数名含 contains、`Range::contains`、`Vec::contains`、
  测试断言），已逐条剔除；`*_tests.rs` 与 `#[cfg(test)]` 断言一律记 A/D。
- `grep` 报 "No such file" 的 33 个文件（`src/executor/arraysstorage.rs` 等）在 HEAD 的
  工作树中不存在（应为历史/未检出路径），未计入 1123。
- B1 中 `hoist_data_double_quotes`（assignment_expansion.rs:55-73）是合格样本：白名单准入
  + 慢路径真扫描 + GNU 注释，改造其余 B1 点时以其为模板。
- 原始工件：`target/issue-suites/results/admit-audit-lines.txt`（1123 行清单）、
  `admit-audit-dump.txt`（带上下文转储）。
