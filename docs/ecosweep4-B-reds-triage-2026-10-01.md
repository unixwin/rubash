# ecosweep4 B 节 274 红 — 分诊表（wt23/bats364 车道，2026-10-01）

零点报告原文（`target/issue-suites/results/ecosweep4/B-ZEROPOINT-REPORT.md`，基线 a522eb39,
2026-09-30）已不在磁盘上；本表在本工作树（基线 2e6b0e98 + #364 修复）上重新实测，
逐套件红数与零点记录一致（executor_tests 记 171 / 本树 172，同一零点提取口径差异，
逐名比对未见 #364 修复引起的翻转；procsub 家族套件全绿）。

实测命令：`cargo test --test <suite>`（RUST_TEST_THREADS=1 继承）。
GNU oracle：WSL GNU Bash 5.3.0（`/usr/local/bin/bash`），script-file 形式。
bisect 方法：临时 git worktree（`git worktree add <path> <commit>` + cargo build），
只跑最小复现探针；用后 `git worktree remove --force` 清理，未 stash。

## 汇总

| 套件 | 红 | 分类 | 证据 | 建议动作 |
|---|---|---|---|---|
| executor_tests | 172 | 混合：大部分过时断言/版本号策略错位；含待查族 | 采样见下 | 开分诊跟进单（见 issue 列表） |
| parser_tests | 55 | 白盒 AST 断言过时（载体架构）；内含 1 个真回归族（glob 泄漏） | `compound_assignments` 不再按旧 token 形状填充（`token_actions.rs:320` 门 `token.raw.ends_with("=(")` 与现行 lexer 形状脱节）；基础复合赋值语义探针全对 | 过时断言伞单 + glob 回归单 |
| parser_redirection_tests | 12 | 白盒 AST 断言过时 | here_string 保留 raw 引号（executor 后剥）；`{fd}` 从 words 移入 fd_var 字段；`read -u {fd}` GNU 同样拒绝、`<<<"alpha"`→alpha 语义探针与 GNU 一致 | 伞单 |
| lexer_quote_tests | 3 | 白盒载体断言过时 | token.value 合法携带 \x17（DATA_SINGLE_QUOTE）/\u{E010} 载体；语义由绿色套件背书 | 伞单 |
| lexer_tests | 3 | 同上 | 同上（\u{E010} PUA 载体） | 伞单 |
| parameter_transform_tests | 3 | 真语义 bug：`${assoc[*]@A}` 丢失赋值体 | GNU：`<declare -A assoc=([two]="beta" [one]="alpha" )>` 单词；rubash：拆成 `<declare> <-A> <assoc>` 且体全丢（探针文件 target/assoc-A.sh） | 开真回归/语义单（引入提交未定） |
| type_p_flag_regressions | 3 | 测试自身 harness 假设（cwd） | 测试脚本未 cd 到 fixture 目录，`PATH= type -p e` 搜的是 crate 根；手工探针（cd 后）rubash 与 GNU 均输出 `./e` rc=0（target/typep2.sh） | 改测试（加 cd） |
| parser_coproc_tests | 1 | 白盒载体断言过时 | words 里 \x11（CTLESC）前缀引号 glob 字 | 伞单 |
| utf8_mojibake_regressions | 1 | 过时断言（GNU 反证） | `set -x; arr=(中文) true` GNU 5.3.0 打印 `+ arr='(中文)'`（tempenv 复合赋值带引号）——rubash 现与 GNU 逐字一致；测试期望无引号形式 | 改断言为 GNU 实测 |
| issue308_regressions | 2 | 真兼容缺口：case 模式内复合关键字不惰性 | `case x in case) ...` rubash 报 `unexpected end of file from \`case'`；GNU parse.y 模式位置按词法读 | 开语义单 |
| issue339_340_procsub_admission | 1 | 过时断言（架构漂移） | `mid_word_procsub_splices_into_word` 期望 `.tmp` 路径；现行输出 `/dev/fd/63`（恰为 GNU 形态）——8f8caca9 引入 raw 路径后 fd-word 选择变化 | 断言改 /dev/fd 形态或确认架构意图 |
| cli_tests | 19 | 混合：12 bashdb_compat + invalid_cli_shopt 已知预存（8f8caca9 提交说明验证过 62c7e5fd 基线同样红）；fd_redirects×2 真回归；timeformat×2 过时；drive_path 过时；examples cwd 相关 | 见下 | 真回归单 + 伞单 |

## 真回归（已定引入提交）

### R1. 复合赋值中"整体带引号元素"发生路径名展开 — 引入 `8c52cb7f`（2026-09-28, rubash#288 "exactly one quote-removal pass"）

```sh
mkdir /tmp/gpdir && cd /tmp/gpdir && touch aa bb
x="*"
arr=("$x"); printf '[%s]' "${arr[@]}"
```
- GNU 5.3.0：`[*][literal]`（整体引号元素是数据）
- 8f897670（09-27）：`[*][literal]`（正确）
- 8c52cb7f（09-28）起至 HEAD：`[aa][bb][literal]`（引号元素被展开）

注：`pre"$x"`（部分引号）与 `'*.txt'`（字面单引号）均不受影响——泄漏点集中在
"整个元素 = 引号包裹的参数展开结果"。parser_tests 白盒家族（quote metadata 断言）
与该真回归同域，修复后需重审该家族剩余红。

### R2. `exec N>&M` 复制的 fd 对外部命令重定向失效 — 引入 `ba69f0d9`（fix(#144): single true path for merged output streams; capture-replay family deleted）

```sh
exec 3>&2; external_script >&3   # 期望落到 stderr
```
- GNU 5.3.0：stderr（探针 target/fdext-gnu.sh）
- c70d109f（含）之前：stderr（正确）
- ba69f0d9 起至 HEAD：stdout（错）
- 对偶用例 `exec 3>&1; err_script 2>&3` 同样反向错（落 stderr）。
- cli_tests::fd_redirects::c_external_command_uses_persistent_fd_copied_from_stderr /
  c_external_stderr_uses_persistent_fd_copied_from_stdout 即此两例。

## 真语义缺口（非回归或引入提交未定）

### G1. `${assoc[*]@A}` 丢失赋值体（parameter_transform_tests 3 红之一）
见上表。GNU 输出 `declare -A assoc=(...)` 一个词；rubash 只剩名字。
（注意：GNU 的关联键顺序是哈希序，测试里的 `[one]` 先序断言也需放宽。）

### G2. case 模式内的复合关键字必须惰性（issue308_regressions 2 红）
`every_compound_keyword_in_pattern_is_inert` / `harden_mm_reserved_word_pattern_continuation_parses`：
rubash 把模式位置的 `case` 等当作关键字开新复合命令，报
`syntax error: unexpected end of file from \`case' command`。GNU parse.y 模式
位置走词法路径。#308 主修复已合入，这两条是遗留未绿断言。

### G3.（#364 下游，非本表红）bats.bats 全量收割被性能卡住
#364 语义修复后 bats_pipe.bats 155/155 与 GNU TAP 逐字一致；bats.bats 300s 内
跑完 70/146 全 ok，估全量 ~10min（GNU <1s）。分解：DEBUG trap 派发+体内
`$(cd;pwd)`（bats 的 Windows 路径归一化，GNU/Linux 从不触发）+ 特殊数组
（BASH_LINENO/BASH_SOURCE）读取，每次触发 ~0.4-1ms（release），单文件 gather
~2300 次触发。属性能工作流，建议独立 perf 单。

## 环境绑定（不计 rubash 账）

- bats runlog 文件名含冒号（`2026-10-01 05:21:10 UTC.log`）：NTFS 拒绝冒号 →
  rubash 每测试一条 `line 190: Invalid argument`（bats_pipe 155 条、bats.bats 70 条）。
  GNU/DrvFs 会转义冒号建文件。Windows 文件名规则，非语义 bug；bats.bats 里另有
  `File name too long` 一例同类。分类 platform。

## 过时断言伞单清单（建议一次批量改）

1. parser_tests 55（assignment_tests 家族：compound_assignments 结构迁移）
2. parser_redirection_tests 12（here_string raw 保留、{fd} 字段迁移）
3. lexer_quote_tests 3 + lexer_tests 3 + parser_coproc_tests 1（C0/PUA 载体断言）
4. utf8_mojibake_regressions 1（GNU 反证：`arr='(中文)'` 才是 GNU 形态）
5. cli_tests timeformat×2（现行 `bash: line 1: TIMEFORMAT:` 前缀恰为 GNU 形态，
   测试还期望旧 `rubash:` 无行号形态）
6. cli_tests script_file_accepts_shell_style_drive_path 1（#224 路径域变化后期望未跟）
7. issue339_340 1（.tmp 期望 vs /dev/fd/63 现行/GNU 形态）
8. executor_tests 采样：part_013 BASH_ALIASES 键序（GNU 实测 foo,bar=现行为，测试期望反序）、
   part_008 版本串 5.2.37（现报 5.2.21(1)-release；目标 5.3.0——版本策略需船长定）
9. type_p_flag_regressions 3（harness cwd 假设；语义已 GNU 对齐）
10. cli_tests examples::gnu_dirstack（cwd 相关断言，需复核）
11. cli_tests getopts_long + 11 bashdb_compat + invalid_cli_shopt：预存（8f8caca9 记录），
    归 bashdb 家族单，不属本轮

## issue 清单（本次开出）

见 `gh issue list --repo unixwin/rubash --search "ecosweep4-B"`（本车道提交说明附编号）。
