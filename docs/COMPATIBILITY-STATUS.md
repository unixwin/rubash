# Rubash ↔ GNU Bash 兼容性权威状态（单一事实来源）

> 最后核对日期：2026-08-29（fresh gen + check 后续会话）
> 核对方法：用 `./target/debug/rubash.exe` 直接跑 GNU 官方测试文件
> `third_party/bash/tests/<name>.tests`，对比 GNU bash 的真实输出。
> 基线约定（2026-08-29 起生效）：语义比对一律用 WSL GNU Bash 5.2.21
> （`wsl bash`）；`D:/Git/bin/bash.exe` 在引号/转义/花括号等区域的兼容性
> 低于 rubash，不得作为语义基准。
> 注：`scripts/run-83-tests.sh` 对比脚本本身已破损（`set -u` 下算术变量未初始化，
> 满屏 `系统找不到指定的路径`），不能用于判定，故本节全部为手动真实复现。
>
> 本文件是兼容性状态的**唯一权威来源**。其余 `docs/*.md` 中带日期的分析快照
> （如 `bash-test-update-20260829.md`、`rubash-compatibility-report.md` 曾宣称
> “92%、仅 1 个 bug”）已被真实复现证伪，相关文件已于 2026-08-29 删除，
> 不再作为判定依据。

## 一、总体结论

- 简单用例层面，数组、关联数组、算术、条件、nameref、mapfile、POSIX 命令替换
  (`$(...)`)、前缀花括号 (`foo{a,b}`)、转义逗号 (`\{a,b\}`) 等**均已可用**，
  与 bash 在隔离场景下一致。
- 但跑**完整 GNU 测试文件**时，仍有若干“早期终止/大行数缺口/解析错误”的实质缺口。
- 真实剩下的高优先级缺口（2026-09-06 完整复核）：
  heredoc、casemod、dstack、cprint、invocation、procsub，以及
  `posixexp2`、`cond`、`comsub-posix`、`braces`、`ifs-posix` 等。`mapfile` 已通过，
  不再列为未完成项目。
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

- procsub：GNU procsub 子进程退出后 /dev/fd 路径 -e 为假；rubash 用持久临时文件 -e 为真。需进程存活期语义（Windows 命名管道方案，高 blast-radius）。
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
GNU 侧导出 THIS_SH=/bin/bash 后 ${THIS_SH} 子调用真实执行，消除早退截断；固定单一二进制路径消除 $0/THIS_SH 路径污染。runner: true-baseline.sh；产物 true-baseline/。**此前多轮台账数字作废，以本轮为准。**

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
- **测量口径切换（团队标准）**：array.tests oracle 从 WSL /usr/bin/bash（5.2.21）改为 **/usr/local/bin/bash（5.3.0，2026-09-09 编译）**——run-ab-b.sh 已固化。5.3.0 口径当前漂移 **436 行**（5.2.21 口径 435，基本相同——差异非版本漂移，是真实 gap）。历史 461/444 为 5.2.21 口径
- **census quotearray 桶 #2 已修**（parameter_patterns.rs `assoc_subscript_key`）：原实现先跑 `expand_embedded_parameters` 再处理反斜杠转义，引号键内 `\$` 中的 `$` 照常打开命令替换（真执行了 echo uname）且键截断为 `x],b[\`。修法：展开**前**把 `\$` 换成 $ 标记（0x1F marker），展开器恢复为字面 $；未转义的 `$(cmd)` 下标照常展开（GNU expand_subscript_string subst.c:11063 语义）。P4/V1 与 GNU 5.3.0 逐字节一致；executor_tests failset 114 零新增；残余：**bare 键形态 `A[x\$(echo uname)]=v` 词法断词**（V2，skip_word_inner 下标内 `(` 判 metachar break），另批处理
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
