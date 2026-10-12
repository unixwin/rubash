# Windows 路径边界契约（Windows Path Contract）

> 状态：**权威现状**（boundary contract audit，2026-10-12，对 master
> `eb062957` 逐边界实测后成文）。本文把"每层边界谁负责转换、POSIX 形态
> 通到哪、什么必须 verbatim"从部落知识固化为契约，并配契约测试套件
> `tests/path_contract_boundary_*.rs`（三不变量 × 正/反用例，当前全绿；
> 红即契约被合并推翻）。配套阅读：
> [`platform-identity.md`](platform-identity.md)（per-child dialect 的设计
> 叙述）、[`issue329-path-form-policy-analysis.md`](issue329-path-form-policy-analysis.md)
> （四类转换源的归因分析）、AGENTS.md《WinuxCmd / Winuxsh Tooling Issues》。

## 0. 三不变量

| # | 不变量 | 内容 | 契约测试 |
|---|--------|------|----------|
| A | **native 核心** | rubash.exe 与 winuxcmd 工具链零 MSYS/cygwin 依赖（导入表纯 Win32/CRT）；spawn/parse 无模拟层 | `path_contract_boundary_native_core.rs` |
| B | **POSIX 形态可用** | shell 内（重定向/cd/pwd/$PWD/$(pwd)）吃 POSIX 且渲染 POSIX；winuxcmd 工具吃显式 POSIX 操作数；env 通道 PWD 形态与语义 | `path_contract_boundary_posix_forms.rs` |
| C | **verbatim 保真** | 单引号操作数字节保真（#511/#555 家族）；commit -m 消息字节；分支名/rev spec 零转换；GIT_SSH_COMMAND 等环境通道 | `path_contract_boundary_verbatim.rs` |

测试运行：`cargo test --test path_contract_boundary_`（Windows 实边界；
POSIX 平台自动 no-op）。B②/C② 两处依赖安装树（`WINUXCMD_HOME`）的单元
在工具链缺失时（CI）跳过并留提示，不失败。

## 1. 四层边界表

**总则（设计核心，未被近期合并推翻，实测重申）**：rubash 对标 GNU
`shell_execve → execve`（execute_cmd.c:6119-6127）——**shell 自己从不改写
argv 字节**；路径形态的取舍按"子进程自己有没有 POSIX 层"分配：

| 层 | 边界 | POSIX 形态（/c/...、/tmp、/d/...） | verbatim？ | 谁负责转换 | 契约锁 |
|----|------|-----------------------------------|-----------|-----------|--------|
| L1 | **shell 内语义层**（重定向目标 / cd / pwd / $PWD / $(pwd) / OLDPWD） | ✅ 操作数双形态全收（`/c/Users/...` 与 `C:/Users/...` 等价），**渲染恒 POSIX** `/<drive>/…`（#537/#556：`NIU_SHELL_PATH_STYLE` 等宿主变量对引擎已失效，仅宿主 prompt 渲染提示） | ✅（内建 wrapper 的操作数单次展开后 as-is，#511/#555 家族） | 引擎内部单一漏斗 `shell_path_to_windows`（含虚拟根 /tmp、/dev、/mnt/X、NTFS 冒号 U+F03A 编码）；渲染反向漏斗 `shell_pwd_display_path` | `posix_forms.rs` 前 3 测试 |
| L2 | **winuxcmd 工具参数层**（touch/ls/env/timeout/xargs/rm… 的操作数） | ✅ 显式 POSIX 操作数可用——**由每个工具自己的 POSIX 层解析**（rubash 不代转） | ✅ verbatim（POSIX-aware 子进程类，argv 原样） | 各工具自身 | `posix_forms.rs` 后 2 测试 |
| L3 | **winuxcmd 子进程查找层**（env/timeout/xargs/find -exec 的子命令查找） | ✅ 通过 PATH 解析子命令名；`-exec cat {} ;` 同 | ✅ 子命令与 `{}`/参数原样 | 各工具自身（PATH 查找） | `posix_forms.rs::winuxcmd_tools_accept_posix_operands` |
| L4 | **原生子进程 argv 层**（cmd.exe / 任意非 winuxcmd 树 .exe） | ❌ **不可用**——native 子进程收到的是统一翻译后的 Win32 形态（`D:\...`） | ✅ 字节保真（引号遵循 CommandLineToArgvW；通配符/MSYS 宿主检测才加引号）；**path 形操作数统一翻译，绝不半个 argv 翻译半个不翻** | rubash（`executor/path.rs`：`external_argument_path` + `translated_virtual_system_root_argument`；`__RUBASH_ARGV_DIALECT=legacy` 为回滚阀） | `native_core.rs::native_children_receive_win32_cwd_not_posix_persona` |

环境通道（env）横跨 L2/L4：**任何子进程类都 verbatim**——`PWD=/d/...`
原样可达 winuxcmd env 与 cmd.exe，引擎不在 spawn 时重渲染 env 值
（`posix_forms.rs` 两个 env_channel 测试）。

## 2. 每层细则与已知边缘

### L1 shell 内语义层

- 收：`/c/...`、`/c/`、`/c/.`、`/D`（大小写）、`/mnt/c/...`、`/tmp`、`/var/tmp`、`/dev/null`、native `C:/...`、`C:\...`。
- 渲染：`$PWD`、`$(pwd)`、`pwd -P`、cd 后的绑定值恒为 POSIX 形态且互等（`[[ $PWD == $(pwd) ]]` 恒真）。
- **已知边缘（violation，[rubash#566](https://github.com/unixwin/rubash/issues/566)）**：配置 shell root 后，**裸盘根** `cd /c`（无尾斜杠、无后续分量）被 `map_logical_path` 截走 → ENOENT。`/c/` 正常；`/d` 能用只是 winuxcmd 根下恰有 `d/` 目录的巧合。
- 内建 wrapper（cat/mkdir/touch/cp/rm/rmdir）单次展开后操作数 as-is：`rm '$FIX/r'` 报 GNU 同款错误、rc=1、绝不动 `$FIX/r`（#555 家族回归在 `verbatim.rs` 锁死）。

### L2 工具参数层

- touch/ls/env/timeout/xargs/rm/cat：显式 `/c/...` 操作数可用（verbatim 直通后由工具解析）。`/mnt/c/...` 工具侧同样可解析（WinuxCmd 1.1.5 实测），但引擎当前按 #444 对 `/mnt/X` 做**全类预翻译**（carve-out，见 [rubash#568](https://github.com/unixwin/rubash/issues/568) 的裁决待办）。
- **已知边缘（violation，[WinuxCmd#1170](https://github.com/unixwin/WinuxCmd/issues/1170)）**：`find` 的**起点操作数**不认 `/c/...`、`/d/...`（`D:/` 与相对起点正常；`-exec` 子命令查找不受影响）。契约测试里 find 单元因此用 `.` 相对起点，注释指向该 issue。
- **已知边缘（violation，[niu-git#7](https://github.com/unixwin/niu-git/issues/7)）**：`opt/niugit/git.exe` 启动器重建命令行时**丢弃 argv 中的 `"`**、破坏尾部 `\`（`commit -m` 消息字节损坏）。真 git（`mingw64/bin/git.exe`）与 rubash 侧均保真，损坏在 stub。git 的 `-C /d/...`、rev spec（`origin/feat/x`、`HEAD~1..HEAD`、`-- rev`）均零转换可用。

### L3 子进程查找层

env/timeout/xargs 的子命令、find 的 `-exec` 子命令：按 PATH 解析，POSIX 操作数随后原样传递。注意子命令查找读的是**进程 PATH**（Windows 分号表）；从外部驱动时请保证 usr/bin 在 PATH（见 §4）。

### L4 原生子进程 argv 层

- native 子进程（cmd.exe、System32 工具、任意第三方 exe）：path 形操作数统一翻译为 Win32；`cmd /c cd` 看到的是 `D:\...`。这是设计而非缺陷——native 工具没有 POSIX 层，翻译是 rubash 替它做的唯一一次。
- shell-wrapped 子进程（`#!` 脚本经 bash 重入）：与调用方同域，argv verbatim（rubash#259 的教训：混域翻译破坏子脚本路径算术）。
- MSYS 宿主检测（程序目录带 `msys-2.0.dll`/`cygwin1.dll`）只影响**加引号**（防其二次展开），不影响形态。

## 3. 不变量 A 实测结论（2026-10-12）

- `rubash.exe` 导入表：kernel32/ntdl/advapi32/bcryptprimitives/VCRUNTIME140/api-ms-win-crt-*——零第三方运行时。二进制里出现的 `msys-2.0.dll` 字符串是 L4 的**检测探针**（executor/path.rs），不是依赖。
- winuxcmd 工具链（winuxcmd/touch/ls/find/timeout/xargs/env/rm）：导入表纯 Win32（Go 形态：USER32/SHELL32/WS2_32/WINHTTP 等），零 MSYS/cygwin。字符串里的 `msys_runtime` 是 WPM 的告警文案。
- 洗空 PATH（只留 System32）后 rubash 内建面（echo/命令替换/uname 隐藏内建）照常工作——无对外部 POSIX 工具集的启动依赖。

## 4. 如何正确驱动 niu（外部/代理视角）

按"你在哪一层说话"选形态：

1. **rubash 会话内（脚本、-c、交互）**：一律用 POSIX 形态。`cd /d/repo`、`> /tmp/out`、`$PWD`、`find /d/repo ...`（find 起点除外，见 L2 边缘——用 `cd` + 相对起点或 `D:/`）。渲染回来的一定是 POSIX 形态，直接消费。
2. **从 rubash 里调 winuxcmd 工具**：POSIX 操作数放心给（L2 verbatim 直通）；子命令靠 PATH，别假设 `env` 会替你补 PATH。
3. **从 rubash 里调原生子进程（cmd、第三方 exe）**：给它 Win32 形态或让它自己处理——rubash 会把 path 形操作数统一翻译，但你拼接的**字符串内部**（引号里的路径、`/c` 开关）它不会碰。`cmd` 的 `/c`、`/D` 开关被识别为 native 开关不翻译（preserve_native_args）。
4. **必须用 `D:/` 形态的场景**：
   - 驱动 **winuxcmd find 的起点**（[WinuxCmd#1170](https://github.com/unixwin/WinuxCmd/issues/1170) 修复前）；
   - 在 **rubash 之外**（PowerShell/cmd/宿主直接 spawn）传给"POSIX 支持不可证"的工具——工具层各自的 POSIX 支持是**各自运行层的私事**（git 收 `/d/...` 是 git-for-windows 自己的账，不是契约）；唯一跨工具可靠的形态是 `D:/`；
   - 写进 Windows 侧配置（注册表、快捷方式、任务计划）的路径。
5. **环境变量值永远 verbatim**：跨边界传 `PWD`、`GIT_SSH_COMMAND`、`HOME` 时不要做形态"补偿"——两头看到的字节一致，谁消费谁解析。父层（winuxsh 宿主）的 export 转换属宿主账（issue329 (a)），引擎不背。

## 5. 代理驱动指南（供任务书引用）

> 本节写给以后所有跑在 winuxcmd/niubash 环境里的代理任务书：为什么任务
> 纪律反复要求"探针直打 rubash.exe / git 用 D:/ 形态 / 别用工具 shell
> 测量"。此前这些是试错得来的教训，现在是契约。

1. **探针直打 rubash.exe，不要经 shim 链。** `usr/bin/bash.exe` 是 shim → niu.exe（内嵌引擎 + 宿主变量注入）。审计引擎行为时 shim 链会叠上宿主的风格变量与旧引擎版本（本审计即因此发现 #566 的"旧好新坏"假象）。测引擎 = `target/{debug,release}/rubash.exe -c '...'`；测宿主产品才走 shim。
2. **引用规范用 D:/ 形态喂 git 任务书里的字面路径。** 修 [niu-git#7](https://github.com/unixwin/niu-git/issues/7) 之前，含 `"` 的参数（commit -m、config 值）经 `opt/niugit/git.exe` 会丢字节——消息里有引号时改用 `mingw64/bin/git.exe` 或把消息写进文件走 `-F`。rev spec、`-C /d/...`、分支名不受影响。
3. **写探针脚本用 LF 的 .sh 文件交给 rubash 跑，别拼多层引号的 -c。** 多层引用（宿主 shell → rubash -c → 子命令）是 verbatim 事故最高发的入口（#511/#555 家族全是这个形态的变体）；脚本文件让每层引用只处理一次。这也是 §4.5 "env verbatim" 的推论：任何一层自作聪明的补偿转换都会两头不讨好。
4. **不要在探针里假设 `/c` 可 cd**（#566 修复前用 `/c/` 或 native 形态），也不要用 `find /d/...` 起点当正例。
5. **别用 winuxsh 工具 shell 做测量基线**（AGENTS.md 既有纪律，此处重申其契约依据）：它的 glob/printf/awk/sed 管线自带的损坏会让差分结果指向不存在的 rubash 缺口。GNU 侧基线走 `scripts/true-baseline.sh`（WSL）。
6. **发现形态违规 → 开 issue，不要顺手修。** rubash 侧 → unixwin/rubash；工具层 → unixwin/WinuxCmd（标签用真实存在的 `compat-gap`/`bug` 等，勿造新标签）；git 启动器 → unixwin/niu-git。修是后续车道；本文与测试套件是判决依据。

## 6. 审计留痕

- 分支：`docs/boundary-contract-audit`（对 origin/master `eb062957`）。
- 契约测试：`tests/path_contract_boundary_{native_core,posix_forms,verbatim}.rs`，15 用例全绿。
- 违规台账：rubash#566（cd /c 裸盘根）、WinuxCmd#1170（find POSIX 起点）、niu-git#7（stub 双引号）、rubash#568（/mnt/c 契约矛盾：master --lib Windows 恒红，#444 漏改兄弟测试且前提被工具链演进消解）。
- 非边界类旁证（记录在案，未开票）：`rmdir` wrapper 对失败操作恒返回 0（`external_rmdir` `let _ =` + 硬编码 `exit_code = 0`，GNU 应报错 rc=1）；`mkdir` wrapper 缺父目录时自动补齐父链（GNU 裸 mkdir 应 ENOENT）。两者都是语义差异而非路径形态违规，留给语义车道。
