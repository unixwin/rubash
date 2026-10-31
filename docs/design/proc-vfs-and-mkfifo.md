# /proc 最小仿真 + mkfifo rendezvous 统一设计（#1917 / #1900 / #1903 / #1905 四条收编）
> **入仓状态（2026-10-31，rubash#1917 收口片）**：本设计 2026-10-28 以工作区
> 草稿形态先行（四条收编：#1917/#1900/#1903/#1905），A 期与 B 期已按本设计
> 施工并合并入 master —— A 期：PR #1922（feat/vfs-1917-proc-a）及 #1900
> exe/cwd、#1918 PPid 真值、#1935/#1936 读写与谓词面诸片；B 期：#1903
> rendezvous 诸片（标记桥 `src/winuxcmd_fifo.rs`、引擎 mkfifo 改口径、
> create→connect 竞态收口）。C 期归 #1905（外部子进程物化，独立线）与
> #1917-P2 余项（他 pid fd/* 按 §3.5 C2 裁定不做、只做 self）。
> §1 现状盘点与 §3 施工图的时态以 2026-10-28 基线为准，是设计的历史
> 记录，不按今日代码回改；与今日代码的出入以本头注与 §9 为准。
>
> **§9 落地后记（2026-10-31 补）**：
> - §3.4 的 procsub 协同猜想已证实并已被利用：#1959 的 procsub argv-word
>   流式投递正是经 #1903 的 fifo 标记 rendezvous 端点工厂完成的（端点
>   工厂协同点成立）；procsub 载体本体仍为临时文件，未切换，与 §3.4
>   「只留接口余量」一致。GNU 侧的对应关系：`execute_cmd.c:832
>   delete_procsubs()` 的注释自述其职责为 “closes fds or unlinks
>   fifos” —— GNU 的 procsub 清理同时覆盖 fd 与 fifo 两种载体，正是
>   「FIFO 与 procsub 共享底层设施」在源码里的原文证据（vendored
>   bash 5.3 树 `execute_cmd.c` 中 `delete_procsubs` 一带）。
> - §4.4 待裁点①（/proc/version 内容）已裁：维持 Linux 形串（主会话
>   2026-10-28 裁定，A-1 钉按此断言）。②③ 仍属 C 期开工前事项。
> - §4.2 性能预算已以结构钉落地：proc_vfs 内的 ToolHelp 快照计数器
>   （self 面 == 0、根枚举 == 1），不用墙钟断言，与本节立约形态一致。
>


- 性质：设计先行文档，**不含产品代码**。四条 issue 一并收编成一个分期方案：#1917（诉求总纲）、#1900（exe/cwd）、#1903（FIFO rendezvous）、#1905（外部子进程物化）。
- 基线坐标（2026-10-28 开工时 `gh api` 核实）：
  - rubash master = **779b0ccba56d0e21a5b50ac7ee127631deea0625**（PR #1909 merge，2026-10-28T04:10:47Z）
  - WinuxCmd main = **797254c51bfc9d1b46c1a81882f6f31188f5a972**
  - 四条 issue 状态经 `gh api repos/unixwin/rubash/issues/<n>` 逐条直读核实为 open（防 phantom inventory 规矩）。
- 前置资产：`~/workspace/vfs-audit/REPORT.md`（下称「审计」）§2.6/§2.7/§2.8/§2.10、§4.6、§5 D7/D10/D12；仓内 `docs/proc-vfs-plan.md`（2026-09-23 立项，下称「Q11 计划」）；S1+S2 已合并（PR #1906）落地 `src/special_paths.rs` 注册表 + `open_classified_input` 统一打开阶梯。
- 平台边界（审计 §4.2 已裁定，本设计不重开）：模拟层 **Windows-only**（`#[cfg(windows)]` 真实现 + 非 Windows 桩答 None）；Unix 侧真 procfs 直通，引擎绝不拦截。

---

## 1. 现状盘点（逐点，代码坐标 @ 上述基线）

### 1.1 /proc 内容面：比 #1917 诉求描述的走得远，但保真度有硬伤

`src/proc_vfs.rs`（589 行）已有真实实现，入口 = 注册表 `special_paths::synthetic_proc_content`（special_paths.rs:125-127）→ `proc_vfs::proc_file_content`（proc_vfs.rs:20-48）。打开面经 S2 的 `open_classified_input`（shell_options.rs:217-262）以「预填管道转 File」服务重定向/内建读取；`synthetic_proc_content` 的其余消费者 = command_input_scope.rs、command_substitution.rs、external_file_builtins.rs、loop_select.rs、read_io.rs、fd/windows_impl.rs（code search 核实）。

| 条目 | 今天状态 | 坐标 / 说明 |
|---|---|---|
| `/proc/version` | ✅ 已有 | proc_vfs.rs `version()`：固定串 `Linux version 6.1-rubash-proc (rubash minimal /proc emulation) (windows) #1 SMP`。形态仿 Linux、尾巴诚实标注——内容取舍见 §4.4 待裁点 |
| `/proc/meminfo` `/proc/cpuinfo` `/proc/stat` `/proc/loadavg` `/proc/uptime` | ✅ 已有 | GlobalMemoryStatusEx / CPUID / GetSystemTimes / GetTickCount64 合成，字段集对齐 Linux grep 习惯。**即 #1917 的 P2 里 meminfo/cpuinfo 两项已提前交付**，本设计将其从 P2 划掉、只留验收钉 |
| `/proc/self`、`/proc/<pid>` 目录存活探测 | ✅ 已有 | `proc_pid_file` tail=="" 分支：pid 不存在答 None（目录随进程消失，procfs 语义对） |
| `/proc/<pid>/cmdline`、`/proc/self/cmdline` | ⚠️ 近似 | ToolHelp 快照只有映像名 → cmdline 恒为「映像名 + NUL」，**不是真 argv**。LLM 语料里 `tr '\0' ' ' < /proc/self/cmdline` 类脚本会拿到假数据（形状对、内容假——比报错更隐蔽） |
| `/proc/<pid>/status` | ⚠️ 近似且一处造假 | 字段 Name/State/Tgid/Pid/PPid/Threads 已有；但 **PPid 是编的**：`pid_status`（proc_vfs.rs）对 self 答 0、对其他 pid 答 shell 自身 pid。ToolHelp 的 `PROCESSENTRY32W.th32ParentProcessID` 就在快照里、现被丢弃 |
| `/proc/self/exe`、`/proc/<pid>/exe`、`/proc/self/cwd`、`/proc/<pid>/cwd` | ❌ 未做（= #1900） | proc_pid_file 对这些 tail 一律 None → 落飞地字面路径 ENOENT。且形态与现有条目不同：它们是 **symlink 型条目**（readlink 语义），不是内容文件——现有 `proc_file_content -> Vec<u8>` 的接口形状装不下，这是 #1900 的真正施工点 |
| `/proc/<pid>/fd/*` | ❌ 未做 | proc_vfs 对 `self/fd/*` 明确答 None 并让注册表 FdAlias 类接管（`/proc/self/fd/N` 已通）；**他 pid 的 fd 目录**与 fd 条目的 symlink 呈现（`pipe:[…]`/`socket:[…]`/实路径）全无 |
| `/proc` 根目录枚举（`ls /proc`、glob `/proc/*`） | ❌ 未接 | `list_pids()`（proc_vfs.rs 末尾，ToolHelp 升序枚举）**全仓零消费者**（code search 核实仅定义处命中）。枚举面（readdir 语义）不在内容接口的射程内，是独立缺口：`ls /proc` 今天走 WinuxCmd ls 读真 fs → 飞地无此目录 |
| test/[[]] 对 /proc 的 stat 面 | ❌ 不问 proc_vfs（= 已立 #1901，S4 线） | 审计 D8：`cat /proc/stat` 可用而 `test -e /proc/stat` 为假，仓内自相矛盾。本设计与 S4 的衔接见 §5 |

### 1.2 FIFO 面：生态两半各做了一半，断点在 shell

| 环节 | 今天状态 | 坐标 |
|---|---|---|
| WinuxCmd `mkfifo` | ✅ 落盘标记文件 | `src/commands/mkfifo.cpp:158` 调 `native_path::create_winux_fifo_w`；标记格式契约在 `src/utils/native_path.cppm:1064-1142`：文件首字节 `!<fifo>`（`kFifoMarker`）+ `FILE_ATTRIBUTE_SYSTEM`；管道端点名 `\\.\pipe\winuxcmd-fifo-<hash>`，hash 由** canonical 路径**派生（native_path.cppm:1101-1128），使独立进程 rendezvous 到同名端点 |
| WinuxCmd 工具打开标记 | ✅ 桥到命名管道 | native_path 层识别标记后走命名管道、阻塞式 POSIX open 语义（mkfifo.cpp:179-181 的自述）；非 WinuxCmd 工具只见一个小标记文件 |
| rubash 引擎内 `mkfifo` | ⚠️ 诚实拒绝 | `external_file_builtins.rs:2155-2218`（`external_mkfifo`）：Windows 上逐操作数报 `mkfifo: cannot create fifo 'NAME': Operation not supported` rc 1（#1725 收窄后的口径）；非 Windows 让给真二进制 |
| shell 重定向打开 FIFO | ❌ 恒假 | `redirect_path_is_fifo`（shell_options.rs:2197-2213）非 unix 恒 `false`（注释「Windows has no filesystem FIFOs」）→ 打开 winx 标记 = 普通文件语义、无 rendezvous（= #1903 本体）。连带：`probe_input_redirect_gate` 等 #1820/#1868 的 FIFO carve-out 在 Windows 全部走不到 |
| procsub 载体 | 临时文件（非 FIFO） | 审计 §2.9：`external_setup.rs` procsub 流物化走临时载体（#1691 收口）。GNU 在 w32/无 /dev/fd 环境以 FIFO 为 procsub 机制之一（#1917 引 execute_cmd.c:832 `delete_procsubs()`）——本设计 §3.4 只留接口余量，不把 procsub 载体切换并入本期 |

### 1.3 外部子进程面（#1905）

- 引擎内读 /proc 已通（§1.1）；**外部命令的操作数** `/proc/*` 今天原样下传 → 子进程侧 os error 3 类失败。Q11 计划的「挂接点 C」（path.rs 谓词加 /proc 分支 → 翻译瞬间物化到 `<NIU_SHELL_ROOT>\proc\`）从未落地，审计 §2.8/§4.6 D12 同结论。
- WinuxCmd 工具侧：`native_path.cppm` 对 `/proc/` 只有 fd 拼写识别（:820-821，`/proc/self/fd/N`、MSYS 展开的 `/proc/<pid>/fd/N` → fd 号，#1056），**无 /proc 内容面**。即自家工具今天也读不到 `/proc/stat`——命名空间协作问题是双向的（§4.1）。

---

## 2. 四条收编总表（归属期次与验收形态）

期次定义：**A 期** = /proc 保真与补全（引擎内）；**B 期** = FIFO rendezvous（引擎 × WinuxCmd 契约）；**C 期** = 外部可见性（P2 物化与跨仓数据源）。A、B 可并行；C 依赖 A 的内容集冻结。

| issue | 诉求 | 归属 | 验收形态（一句话） |
|---|---|---|---|
| #1917-P1a | `/proc/version` | A0（已交付，补钉） | 内容钉：字段形态断言（见 §6 钉 A-1） |
| #1917-P1b | `/proc/self`、`/proc/<pid>` 的 cmdline/status | A1 保真修复 | PPid = ToolHelp 真父 pid；cmdline = PEB 真 argv（无权读时回退映像名且形状不变） |
| #1900 | exe / cwd（self + pid） | A2 | readlink 型条目进注册表新类；`cat` 跟随链接读目标、`test -e` 真、readlink 呈现对（引擎内面）；外部 readlink 面归 C 期 |
| #1917-P1c | mkfifo → named pipe + read/cat/重定向打通 | B 期（与 #1903 同体） | `mkfifo f; echo hi > f & cat f` 在 niu 内往返成立（带超时护栏钉） |
| #1903 | 同上（shell 侧识别） | B 期 | 同上；外加 `test -p` 对标记答真 |
| #1917-P2a | `/proc/<pid>/fd/*` | C2（可选档） | 引擎内 `ls /proc/self/fd` 等价呈现 + 条目 readlink 文本；不承诺外部面 |
| #1917-P2b | meminfo / cpuinfo | **已交付**（§1.1） | 仅补与本设计同批的回归钉引用，不再施工 |
| #1917-P2c | winuxcmd ps 数据源打通 | C3 | 字段语义对表（设计级契约，不共享代码）；ps 保持原生实现 |
| #1905 | /proc 对外部子进程物化 | C1 | 真实外部 exe 以 `/proc/stat` 为操作数读出内容（Q11 计划挂接点 C 原案） |
| （附带） | `/proc` 根枚举 | A3 | `ls /proc`（引擎 glob + WinuxCmd ls 经 C1 物化目录）与补全（S5）共用 `list_pids` + 顶层名表 |

---

## 3. 分期施工图（文件级）

### 3.0 共同地基：注册表扩类（A 期第一片，B/C 都踩它）

`src/special_paths.rs` 的 `SpecialPathClass` 现有五类（NetRedirect / NullDevice / FdAlias / SyntheticProc / Filesystem）。新增一类：

- `SyntheticLink` —— readlink 型合成条目（exe/cwd/fd 条目）。注册表新增查询 `synthetic_link_target(target) -> Option<String>`，与 `synthetic_proc_content` 并列；`classify_special_path` 的规范序里排在 FdAlias 之后、SyntheticProc 之前（拼写不重叠，序仅为定义完备）。
- proc_vfs 侧新增 `#[cfg(windows)] pub fn proc_link_target(path) -> Option<String>`（非 Windows 桩答 None，与现纪律同形）。
- **名单双判据自检**（主会话规矩）：/proc 条目名集 = 内核定义的文件名集，宇宙**闭**；漏项失败方向 = 落回真 fs → ENOENT **响亮失败**。与审计 §3.4 对 proc_vfs 名单的「健康」评估同形。

### 3.1 A1：cmdline / status 保真（proc_vfs.rs 内）

- `snapshot_identity` 改为同时带出 `th32ParentProcessID`；`pid_status` 的 PPid 用真值。self 的 PPid 同理（现答 0 是造假，改为 shell 进程的真父 pid——即宿主 niu/终端的 pid，这正是进程树消费者要的）。
- 真 cmdline：新增 `pid_cmdline_from_peb(pid)`——`OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ)` → `NtQueryInformationProcess(ProcessBasicInformation)` 取 PEB → 读 `RTL_USER_PROCESS_PARAMETERS.CommandLine`（同 WinuxCmd `ps.cpp:147-157` 已在用的同一手法，跨仓手法对齐、代码各自实现）。失败（权限/位数不符/进程已退出）→ 回退现行映像名形状。self 走快路：`GetCommandLineW` 直接取（见 §4.2 性能）。
- status 字段集只做最小增补：PPid 真值化 + `State` 保持现映射；不扩 VmRSS 等（无数据源支撑的字段宁缺勿造——meminfo 的 0 值字段是「字段名在、值 0」的既定形状，status 不跟这个例）。
- 验收要点：cmdline 内容 = 真 argv 的 NUL 分隔串；`ps` 形消费者（`awk -F: '/^PPid/'`）拿到真父子关系。

### 3.2 A2：exe / cwd（#1900 本体）

- `proc_link_target` 实现：
  - `/proc/self/exe` → 引擎自身映像路径（`GetModuleFileNameW(NULL)`，无快照）；呈现拼写 = shell 命名空间的 POSIX 形（经现有路径反向映射，与 $PWD 呈现口径同源）。
  - `/proc/<pid>/exe` → `OpenProcess` + `QueryFullProcessImageNameW`；失败答 None（条目缺失形状，与 procfs 权限不足时条目不可读的实际观感有别——Linux 是条目在、readlink EACCES；本层取「答 None = 不存在」是**已知的形状妥协**，记入 §7 风险，不在本期造 EACCES 呈现面）。
  - `/proc/self/cwd` → 会话 cwd（session 状态的 POSIX 拼写；注意 G-02 后 cwd 是 Session 状态、不是进程全局——必须问 session，不得读 `std::env::current_dir`。这是本条与多 Session 线的硬接点）。
  - `/proc/<pid>/cwd`（他进程）→ PEB `CurrentDirectory.DosPath` 读取（与 A1 同一条 PEB 通道）；仅同位数可读，失败答 None。**优先级低于 self**：A2 内部分两档，self 两条必达，`<pid>/cwd` 可降为 A2-可选（验收时分开报）。
- 消费者接线（引擎内）：
  - 打开阶梯 `open_classified_input` 加 SyntheticLink 臂：`cat /proc/self/exe` = 跟随链接打开目标文件本身（GNU 语义：open 跟随 symlink）。cwd 同理（打开目录的目标由重定向机器按目录处理）。
  - `readlink` 呈现：引擎内的 readlink 消费面（external_file_builtins 的文件操作数读取路径与 test/stat 面）问 `synthetic_link_target`。注意 shell 的 `readlink` 命令本体在 WinuxCmd 侧——引擎内只能保证重定向/cat/test 面；**外部 readlink/ls -l 的呈现归 C 期跨仓面**（§3.5），A 期验收不得把外部 readlink 写进通过条件。
  - test/stat 面与 S4（#1901/#1902）的合流方式见 §5。
- 目录呈现：`/proc/self/` 的条目集（ls 形枚举）= {cmdline, status, exe, cwd, fd/}——枚举数据源在 proc_vfs 内以条目名表形式与内容/链接生成器同表定义（防第十套名单：名表、内容生成、链接生成三者同文件同表）。

### 3.3 A3：/proc 根枚举接线

- `list_pids()` 接消费者：引擎 glob 展开与内部目录读取面在 `/proc` 上枚举 = 顶层合成名表 ∪ `self` ∪ `list_pids()` 结果。WinuxCmd `ls /proc` 的面归 C1 物化目录（物化时把 pid 目录一并落盘为空目录 + 文件快照；pid 集合以物化时刻为准，快照语义明示）。
- 本片只交引擎内枚举 + 为 S5 补全预留同一数据源（§5），不单独验收 ls。

### 3.4 B 期：FIFO rendezvous（#1903 + #1917-P1c）

施工顺序 **B2 → B3 → B1**（先有会合，才许造标记；逆序会造出「mkfifo 成功但语义是假的」的窗口）：

- **B2 命名管道端点（fd_table + shell_options）**：
  - `redirect_path_is_fifo` 的 Windows 臂：标记探测 = 读文件首 `kFifoMarker.len()` 字节比对 + `FILE_ATTRIBUTE_SYSTEM`（与 WinuxCmd `is_winux_fifo_w` 同契约，native_path.cppm:1074-1095）。探测只读文件头、不 open 管道——保持 #1820 钉住的「探测不消耗 rendezvous」纪律。
  - 打开臂（`open_classified_input` / 输出侧对应点）：目标是 winux-fifo 标记时，按 canonical 路径派生端点名（与 native_path.cppm:1101-1128 同算法：路径规范化后 hash——**算法对表是本期的跨仓契约点，须逐字节同结果**，在 WinuxCmd 侧以其既有实现为准、rubash 侧复刻并以互操作钉验收）。
    - 读打开（server 角色）：`CreateNamedPipeW`（BYTE 模式、单实例先到先得）+ `ConnectNamedPipe` 等对端；写打开（client 角色）：`WaitNamedPipeW` + `CreateFileW`。角色分配按「先开者建 server」——与 WinuxCmd 工具打开标记时的既有角色规则对表（其阻塞 open 语义已定，shell 侧服从同一规则，不得另立）。
    - **阻塞与中断（本期最高风险点）**：POSIX FIFO open 是可被信号打断的阻塞；`ConnectNamedPipe` 同步调用会把 shell 线程钉死、Ctrl+C（#1662 机器）打不断。设计规定：管道建立一律 `FILE_FLAG_OVERLAPPED`，等待 = `WaitForMultipleObjects([overlapped event, 会话中断事件])`，中断到达即 `CancelIoEx` 并以 EINTR 形错误退出重定向（GNU 被信号打断的 open 形状）。验收钉含「阻塞 open 中 Ctrl+C 可退」一条（PTY 面，与 #1662 同门）。
    - 端点落 fd_table：以 OS File 句柄形态进入（命名管道句柄可包装为 File），不新增 FdRead/WriteEndpoint 变体——除非实现期发现 File 语义不合（如 seek 探测），届时按最小变体新增并记设计偏差。
  - shell 内部读者（cat/read/重定向）与 WinuxCmd 工具读者混开同一 fifo：因端点名与角色规则同契约，天然互通——验收钉含一条跨侧混开。
- **B3 test -p 与谓词面**：`test -p <marker>` 答真、[[]] 同（经 S4 的 fd 事实通道问「是否 winux-fifo 标记」，不答常量——#1902 的教训）。
- **B1 引擎 mkfifo 改口径**：`external_mkfifo`（external_file_builtins.rs:2155-2218）Windows 臂由诚实拒绝改为创建标记（复刻 `create_winux_fifo_w` 契约：写 `!<fifo>` + 置 SYSTEM 属性，原子性与错误文案按 GNU coreutils mkfifo 形状）。**行为变化明示**：#1725 的「诚实拒绝」是当年无 rendezvous 时的正确收窄；B1 是在 B2 落地后将其 supersede，PR 描述必须写明这一层关系。`rm` 对标记的删除已是普通文件删除、天然成立；`ls -l` 的 `p` 类型呈现归 WinuxCmd 侧（其 stat 已认标记与否，实施期对表核实，不在本设计承诺）。
- procsub 接口余量（不施工）：fd_table 端点工厂若按 B2 形状长出「按名 rendezvous 端点」，未来 procsub 切换 FIFO 载体（GNU w32 形态）可复用之；#1915（procsub + set -u 误展开）是展开面缺陷、**与载体无关**，本设计不声称对其有修复作用，仅回应 #1917 的协同猜想：协同点在端点工厂，不在 #1915 的根因。

### 3.5 C 期：外部可见性（#1905 + #1917-P2）

- **C1 物化（挂接点 C 原案落地）**：spawn 边界上，外部命令操作数命中 SyntheticProc 类（内容型条目）时，翻译瞬间把当前快照原子写（tmp+rename）到 `<NIU_SHELL_ROOT>\proc\<name>`，子进程收到真实 Win32 路径。纪律沿 Q11 计划 §3/§4.1：根不可写 → 静默降级（引擎内面照常）；与用户真实 `<root>\proc` 冲突 → 降级 + 一次告警。self 条目物化的是**spawn 时刻**的快照（pid 是 shell 的 pid——对子进程而言 `/proc/self` 在 Linux 指子进程自己！这是物化路线的**根本语义差**：引擎内 `/proc/self` = shell 自己，与 GNU/Linux 下「谁读指谁」不同。Q11 计划当年已接受此差（其 §0 验收场景全是 shell 视角）。本设计明示：此差为既定妥协、写进验收注记；agent 语料里 `cat /proc/self/cmdline` 在 niu 下读到的是宿主 shell 的命令行——dsh-ops 评估（#633）若对此有异议，属待裁点 §4.4-②，不得默默翻案）。
  - SyntheticLink 条目**不物化为链接**（Windows symlink 需特权/开发者模式，与「绝不要求管理员」红线冲突）——外部 `readlink /proc/self/exe` 的正解是 C3 的 WinuxCmd 侧桥（工具问数据面），C1 只覆盖内容型。
- **C2 `/proc/<pid>/fd/*`（可选档）**：引擎内呈现：条目集 = 目标进程 fd 表——**只有 self 有引擎 fd 表**；他 pid 的 fd 集合 Windows 无低成本枚举面（NtQuerySystemInformation handle 表成本与权限都高）。裁定式设计：C2 只做 self：`/proc/self/fd/N` 已由 FdAlias 通（读），补的是**枚举与 readlink 文本**（N → 其端点描述：文件路径 / `pipe:[rubash-<n>]` 合成文本）。他 pid fd/* 明确不做、答 None（响亮缺失），理由记此。
- **C3 跨仓数据源对表（ps + 工具读 /proc）**：
  - ps：WinuxCmd `ps.cpp` 已有原生实现（ToolHelp + NtQueryInformationProcess + GetProcessTimes/MemoryInfo），**不改其数据源**；「打通」的落地形态 = **字段语义契约表**：本设计附表（见 §6.3）规定 /proc/<pid>/status 字段与 ps 列的同名同义关系，后续两仓各自改动不得单方改义。rubash 侧 A1 的 PEB 手法与 ps.cpp 同源即是此契约的第一条履行。
  - WinuxCmd 工具读 /proc 操作数：经 C1 物化路径接收（它们在 #620 类别里是 POSIX-aware 拿原文拼写的——C1 的物化翻译必须对 WinuxCmd 类别同样生效，即 /proc 分支是类别谓词的**例外臂**：这是与 #620 口径的一处显式接口，实施 PR 须引用本设计此节，不算私自改口径）。
  - WinuxCmd `readlink`/`ls -l` 对 exe/cwd 的呈现：工具侧识别 `/proc/.../exe|cwd` 拼写并问一个**数据面**——候选通道（文件快照清单 vs 现问现答的命名管道查询）属待裁点 §4.4-③，C 期开工前定。

---

## 4. 开放问题答复（#1917 三问 + 两待裁）

### 4.1 虚拟 /proc 与 winuxcmd 工具的命名空间协作

答复（三层分工，即 §3 的结构本身）：
1. **引擎内消费者**（重定向、内建、引擎内工具族）问注册表——单一事实源，不经文件系统。
2. **WinuxCmd 工具**走两条既有生态契约：内容型走 C1 物化（spawn 瞬间快照文件，与 dev_fd_operands 的物化机制同族）；类型型（FIFO 标记、未来的 exe/cwd 呈现）走「拼写识别 + 桥」——与 FIFO 标记同一手法（标记/拼写是跨进程契约，数据在带外）。
3. **第三方原生程序**只得 C1 物化（内容型）；链接型与活数据对它们不可见——这是「模拟层搬进产品」形态的结构性边界（审计 D12 已定性），不追求与 MSYS 运行时级透明等价。
命名空间一致性纪律：任何新条目先进行注册表（引擎）与跨仓契约表（WinuxCmd）双登记，才许有消费者——防第三套 /proc 知识在 WinuxCmd 的 native_path 里野生生长（其现有 /proc fd 拼写识别 :820-821 是历史存量，C3 时收编进对表清单）。

### 4.2 /proc/self 高频调用性能预算

问题成立且现实现有实际浪费：`proc_pid_file` 对**每一次** /proc/<pid>/* 打开都调 `pid_identity` = 一次全系统 ToolHelp 快照（O(进程数)），agent 循环 `for p in /proc/[0-9]*` 是 O(N²) 次快照行走。设计预算（以**调用次数**立约，不用墙钟数字——跨平台计时纪律）：
- `/proc/self/*` 任何条目：**零 ToolHelp 快照**。self 的 exe/cmdline/status/cwd 全部走自进程直查 API（GetModuleFileNameW / GetCommandLineW / session cwd / 自 pid 常量）；status 的 PPid 走一次性缓存的自进程 ToolHelp 单查或 `NtQueryInformationProcess(self)`。
- 单个命令周期内对他 pid 的枚举类操作（`list_pids`、glob、批量 status）：ToolHelp 快照 **≤ 1 次**，以「命令周期标脏」复用（#592 S2 已落地的 per-pass 计算 + 标脏模式，同形状复用，不新造缓存机制）。
- 内容生成（meminfo/cpuinfo 等）：每次打开新鲜快照是 procfs 语义本身（GNU 亦然），不缓存；但其数据源 API 都是 O(1) 系统调用，无预算问题。
- 验收形态：回归钉以探针计数断言（测试内以可注入的快照计数器或以「self 打开路径不调 snapshot 函数」的结构钉实现），不用时延断言。

### 4.3 与并发 / 进程树清理的交互

- **PPid 真值是进程树清理的地基**：dsh-ops 受限模式的进程树清理若建在今天的假 PPid 上会清错树。A1 把 PPid 真值化列为 P1 而非润色，理由在此。树遍历消费者纪律（写给下游的契约注记）：pid 可被复用，probe→read 之间进程可换人——遍历必须逐节点重探存活、且以（pid + 映像名/创建时间）双键认人；/proc 本层只保证单次读取的自洽快照，不保证跨读一致性（Linux procfs 同样不保证）。
- **FIFO 端点的清理归属**：命名管道端点名由路径派生、跨会话共享命名空间。纪律：端点随最后句柄关闭而消失（OS 语义），shell **不得**在会话退出时按名强拆端点（可能属于其他会话的活 fifo）；标记文件的生死归 `mkfifo`/`rm` 的文件语义，与会话无关。崩溃残留只有标记文件（普通文件、可 rm），无孤儿 server——server 角色进程死亡即端点消失，对端 open 会落到「按规则重建 server」路径，B2 角色规则须含此恢复形。
- **procsub/coproc 邻接**：B 期不动 procsub 载体，故 GNU `delete_procsubs()`（execute_cmd.c:832）对应的退出清理面本期无新增义务；若未来 procsub 切换 FIFO 载体，其退出清理必须与本节端点归属纪律一并设计（端点是 shell 自建的、归 shell 清理——与用户 mkfifo 的标记恰好相反，此区分须在那时写明）。

### 4.4 待裁点（设计不自决、留主会话/用户）

- ① `/proc/version` 内容：现串把「Linux version 6.1-…」打头、诚实标注收尾。LLM 语料消费者多以 `uname`/version 前缀做环境判定——保 Linux 形（现状）vs 换真实 Windows 版本形，是产品口径抉择。本设计默认**维持现状**（动它需单独立项），仅在此登记。
- ② C1 物化下 `/proc/self` 指代差（shell 自身 vs 读者自身，§3.5）：Q11 计划已默认接受；因 #633 合作方的兼容定义极严（「所有 LLM 按语料习惯书写的命令都不报错」——注意其口径是**不报错**、非「读数必真」），本设计按已接受处理，但建议主会话在与合作方对表时明示此差，避免验收期翻案。
- ③ C3 的 WinuxCmd 链接呈现数据面通道（快照清单 vs 命名管道现问现答）：涉及是否新开一个常驻查询面，安全与复杂度权衡需主会话裁。本设计倾向**快照清单**（与 C1 物化同批落盘的一份 manifest，零新常驻面），倾向仅供参考、非裁定。

---

## 5. 与 S3–S6 既有排产的衔接

| 既有片 | 关系 | 本设计处置 |
|---|---|---|
| S3 映射段合流（path.rs /dev 表并入注册表） | 正交 | A0 注册表扩类（SyntheticLink）与 S3 的映射合流互不依赖、先后皆可；两者都动 special_paths.rs，排产上**错开合并窗口**即可，不合并施工 |
| S4 test/conditional 问注册表 + fd 事实（#1901/#1902） | **强衔接** | A2/B3 的 test 面验收依赖 S4 的通道形状。处置：S4 的 /proc 部分（test 对 SyntheticProc/SyntheticLink 类的谓词答案：-e/-r 对内容类答真、-L 对链接类答真、-p 对标记答真）**并入本设计 A2/B3 随片施工**，S4 只保留 /dev/fd 事实面——避免同一通道两片重复开工。排产上 A2/B3 在 S4 之后或同批，PR 里互相引用 |
| S5 niu 补全 + 高亮同批（#627/#628） | 数据源衔接 | 补全的 `/proc/` 前缀候选源 = A3 的枚举数据源（顶层名表 + self + list_pids），与 S5 的「候选必须问同一注册表」纪律天然合流。处置：**独立排期、数据源先行**——A3 先于或同批于 S5 落地，S5 不改本设计的枚举表 |
| S6 跨仓对表（WinuxCmd native_path） | **B/C 期即 S6 的实体** | 审计 S6 原范围只有 /dev/tty 一处口径对表；B 期的端点名算法对表、C3 的字段契约与 /proc 拼写收编都是跨仓对表工作。处置建议：**把 S6 扩围为「跨仓契约片」**，B/C 期作为其施工主体落地，/dev/tty 老项搭车。S6 由「可选、另裁」升为 B 期前置（端点算法对表先于 B2 编码） |

---

## 6. 口径与验收参照

### 6.1 裁决权
- 语义裁决者恒为 **GNU 行为**（bash 5.3.20 oracle 的重定向/打开语义、coreutils mkfifo/readlink 的错误形状与文案）。/proc 内容本身非 GNU 契约（Q11 计划已定位为 userland 增值层），其**格式**以 Linux procfs 的下游消费习惯为准（字段名、NUL 分隔、State 码）。
- Git Bash / MSYS2 / Cygwin 是**同机参考实现**（/proc 与 FIFO 在三家皆为运行时实物，可作覆盖面与错误形状样本），**不是 oracle**：三家与本设计分歧时，以 GNU/Linux 语义为准，不反向改期望。三家差分须同机同跑（GitHub Windows runner 预装 Git Bash；MSYS2/Cygwin 可得性按审计 §4.7 的成本注记处理）。

### 6.2 每期回归钉清单（命名带 issue 号，断言按上款口径）

**A 期**
- A-1（#1917）：`/proc/version` 形态钉——`cat /proc/version` 首字段 `Linux`、`version` 词在位；`read line < /proc/version` 重定向面同结果。
- A-2（#1917）：cmdline 真值钉——以已知 argv 启动子 shell 后读其 `/proc/<pid>/cmdline`，NUL 分隔逐项等于真 argv（Windows CI 面）；无权读场景的回退形状钉（映像名 + NUL）。
- A-3（#1917）：status PPid 真值钉——子进程读自身 `/proc/self/status` 的 PPid == 派生它的 shell pid；shell 读子 pid 的 PPid 同值。
- A-4（#1900）：`readlink` 引擎面 + `cat /proc/self/exe` 跟随读到的是映像二进制头（MZ 魔数）；`test -e /proc/self/exe`、`test -L /proc/self/exe`、`test -e /proc/self/cwd` 三谓词（随 S4 通道）。
- A-5（#1900）：`/proc/self/cwd` 内容 == 会话 cwd 的 POSIX 拼写；会话内 `cd` 后再读跟随变化（G-02 session cwd 接点钉）。
- A-6（#1917，性能结构钉）：/proc/self/* 打开路径快照调用计数 == 0；单命令周期批量枚举快照计数 ≤ 1（探针计数断言，非计时）。
- A-7（#1917）：未知 pid / 已退出 pid 的 `/proc/<pid>/status` 打开答 ENOENT 形（目录消失语义不回归）。

**B 期**
- B-1（#1903）：rendezvous 往返钉（超时护栏）——`mkfifo f; echo hi > f & cat f` 输出 `hi`；读先开、写先开两序各一。
- B-2（#1903/#1917）：跨侧混开钉——WinuxCmd `cat f` × shell 写、shell `cat f` × WinuxCmd 工具写，两向往返。
- B-3（#1903）：阻塞 open 中断钉——无对端时 `cat f` 阻塞，Ctrl+C 后 shell 存活且重定向以 EINTR 形收口（PTY，与 #1662 同门）。
- B-4（#1917）：引擎 `mkfifo f` 后标记契约钉——文件首字节 `!<fifo>` + SYSTEM 属性在位；重复 mkfifo 同名报 GNU 形 EEXIST 文案。
- B-5（#1902 随 S4）：`test -p f` 对标记答真、对普通文件答假。
- B-6（#1903）：探测不消耗钉——`test -e f` / 存在性探测后 rendezvous 仍成立（#1820 纪律的 Windows 版）。

**C 期**
- C-1（#1905）：真实外部 exe（非 WinuxCmd 工具）以 `/proc/stat`、`/proc/meminfo` 为操作数读出内容；根不可写降级时引擎内面不受影响钉。
- C-2（#1905）：物化新鲜度钉——两次 spawn 间隔内 /proc/uptime 物化值递增（快照时刻语义）。
- C-3（#1917）：`/proc/self/fd` 枚举钉——开/关 fd 后条目集跟随；条目 readlink 文本形状（文件路径 / pipe 合成文本）。
- C-4（#1917，跨仓契约钉）：WinuxCmd `ps` 输出的 PPID 列与 `/proc/<pid>/status` 的 PPid 对同一组进程逐一相等（Windows CI 双仓面）。

### 6.3 字段语义契约表（C3 附表，最小集）

| /proc/<pid>/status 字段 | 数据源（rubash） | ps 对应列 | 同义纪律 |
|---|---|---|---|
| Name | ToolHelp szExeFile / PEB 映像名 | COMM | 均为映像基名，不含路径 |
| Pid | th32ProcessID | PID | — |
| PPid | th32ParentProcessID（A1 后） | PPID | 父进程退出后两边同为原始值，不做重挂伪装 |
| State | 现映射 S 起步 | STAT | Windows 无 zombie/running 全集，映射表变更须双仓同批 |

---

## 7. 风险与未决项

1. **PEB 读取的覆盖面**：`pid_cmdline_from_peb` 对跨位数（32 位进程被 64 位 shell 读）、受保护进程、权限不足一律失败回退——回退率在真实 agent 负载下多高未测。缓解：回退形状与现行一致（不劣化），A1 验收含一个跨位数样本的「回退形状对」钉而非「必读出」钉。
2. **exe 条目「答 None = 不存在」的形状妥协**（§3.2）：与 Linux「条目在、readlink EACCES」有别；若 LLM 语料脚本以 `test -e /proc/<pid>/exe` 判存活再读，会在权限不足场景误判死亡。本期接受，登记为已知差。
3. **命名管道角色规则与 WinuxCmd 既有实现的对表深度**：B2 的「先开者建 server」是从其阻塞 open 语义推的**待核假设**——实施前必须逐行读 WinuxCmd 打开标记的实现（native_path 桥的 server/client 分配与多实例上限），若其规则不同，shell 侧服从之。本设计在此只立「服从同一规则」的纪律，不预断规则内容。
4. **端点名 hash 算法的逐字节对齐**：native_path.cppm:1101-1128 的 hash 输入（canonical 化的确切步骤）须在实施期以互操作钉（B-2）证死；算法漂移的失败模式是「两边各等各的」静默挂死——B-1/B-2 钉的超时护栏就是为它设的。
5. **Ctrl+C 中断阻塞 open**（§3.4 B2）：OVERLAPPED + 中断事件等待是设计规定，但 reedline/宿主的中断事件与执行器会话事件是否为同一物，实施期须与 niu 宿主面核对；若无现成事件，退而求其次的「短轮询 + 中断标志」方案须在 PR 里明示其延迟上界。
6. **物化目录的安全面**：`<root>\proc` 是可预测路径的快照文件（含 cmdline 快照时可能带敏感 argv）。C1 物化集**排除 cmdline**（只物化顶层静态文件 + stat/meminfo/cpuinfo/uptime/loadavg/version），self/&lt;pid&gt; 条目不进 C1——此排除是本设计的安全裁定，验收按此。
7. **/proc/self 指代差**（§4.4-②）：与合作方验收口径的潜在分歧，待裁。
8. **version 串内容**（§4.4-①）：待裁，默认维持。
9. **未覆盖**：`/proc/net/*`、`/proc/mounts` 等其他 LLM 语料高频条目不在四条诉求内，本设计不扩围；若 #633 后续评估提出，按同一注册表机制另立条目（机制已备、增条目是小片）。

---

## 8. 排产建议（给主会话）

1. A0+A1（注册表扩类 + 保真）→ 2. A2（exe/cwd，含 S4 /proc 谓词部分）→ 3. B 期（先 S6 扩围对表、再 B2→B3→B1）→ 4. A3 + S5 数据源衔接 → 5. C 期（C1 → C3 → C2 可选）。
A 与 B 无代码冲突面（proc_vfs/special_paths vs fd_table/shell_options 的 fifo 臂），机器紧张时可串行、窗口宽裕时可并行两簇。全部期次都在 1.5.0 门内可达，但 C2 与 C3-链接呈现（待裁点③）是 C 期内定的可砍尾巴——窗口压缩时先砍这两项，不动 A/B 与 C1。
