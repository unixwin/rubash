# 跨三平台差距台账（rubash 引擎 + niubash 产品层）

> 2026-09-26 复查基线：rubash @f12aa756（unix 进程域接线已落地，见下），
> niubash 1.1.4（crates/niubash-runtime，rubash 走 git master 依赖）。
> 每项附 file:line 证据与修复方向；行号随并行修复漂移，动手前先 grep 复核。
> 状态标记：✅ 已落地 / 🔴 阻塞 / 🟡 语义偏差 / ⚪ 低优先或测试层。

## 已完成（f12aa756，本台账的起点）

| 项 | 内容 | GNU 锚点 |
|---|---|---|
| ✅ A | fd/unix.rs `spawn_whitelisted` 消费 `spec.extra_handles`（fd≥3 继承修复）+ 删 `set_extra_handles` 死代码 + read_some EINTR 重试 | （纯 bug，无语义变更） |
| ✅ B | PATH 两档制 FS_EXEC_PREFERRED + file_to_lose_on（非可执行→126 非 127） | findcmd.c:623/580/591/695/156 |
| ✅ C | `wait_status::process_exit_status` 单点解码，替换 14 处 `code().unwrap_or(1)` | jobs.c:2958 |
| ✅ D | unix 信号后端（signal-hook：INT/TERM/HUP/QUIT/USR1/USR2 喂共享队列；trap 派发零改动） | sig.c:102/133 |

验证：win build 零警告 + 439/439 lib 测试；`--tests` 交叉检查 linux-gnu 与
aarch64-apple-darwin 干净；5 条 WSL GNU Bash 5.3.0 基线对齐。

## 引擎层缺口（rubash）

- **E1 🔴 信号名↔编号表 Linux-x86 硬编码**（`src/builtins/kill.rs:8` SIGNALS 表、
  `src/builtins/trap.rs` 名字表）。Darwin 实际编号 17=STOP/18=TSTP/19=CONT/20=CHLD、
  30=USR1/31=USR2，且 7/10/12/16 段不同、无 STKFLT/PWR/RT 信号（有 EMT/INFO）。
  macOS 上 `kill -USR1` 会真发 SIGBUS(10)、`kill -CONT` 实发 TSTP——主动有害。
  **修法**：`#[cfg(unix)]` 表改由 libc 常量按 target 生成（`libc::SIGUSR1 as i32`），
  RTMIN 尾部（34+）`cfg(target_os = "linux")`；**Windows 字面量表不动**——mailbox
  线格式就是 Linux 编号，设计如此。同构先例：bash 构建期由
  `support/mksignames.c` 从目标机 `<signal.h>` 生成 signames.c
  （third_party/bash/Makefile.in:602-783），zsh 同模式（configure 探测）。
  这也是 macOS 路线唯一的真语义阻塞。
- **E2 🟡 `command -v`/`type` 自有查找器**（`src/builtins/command.rs:209` 用
  `env::split_paths`；`src/executor/lookup_paths.rs:14` 对 mv/cat/ls 硬编码
  `/usr/bin/mv`、`/bin/{name}`）。未接 B 的两档制，`type cat` 输出失真。
  修法：builtin 查找统一委托 `find_user_command`（引擎单一查找路径）。
- **E3 🟡 `--version` 硬编码 `x86_64-pc-msys`**（`src/main.rs:505`、
  `src/builtins/help.rs:298`），Linux/macOS 上报 Windows 身份。修法：按
  target triple 报（windows=msys 兼容串保留，unix 报 `*-linux-gnu`/`*-apple-darwin`）。
- **E4 🟡 TMPDIR 强制默认**（`src/executor/init.rs:364`、`local_helpers.rs:192`）：
  bash 不设置 TMPDIR，unix 下可能得到 `<cwd>/target` 假值。修法：unix 分支不注入。
- **E5 🟡 `standard_path`（`command -p`）非 Windows 硬编码
  `/usr/local/bin:/usr/bin:/bin`**（`src/executor/path.rs:341,363`、
  `builtins/command.rs:245`）。bash 用 `confstr(_CS_PATH)`：macOS 实际
  `/bin:/sbin:/usr/bin:/sbin`。修法：unix 走 `nix::unistd::confstr(_CS_PATH)`。
- **E6 🟡 `/bin/X` miss 回退按 basename 搜 PATH**（`src/executor/path.rs:281-285`）：
  Windows 的命名空间桥接，Linux 上偏离 bash not-found 语义。修法：`cfg!(windows)` 门控。
- **E7 🟡 无门控的 Windows 路径残迹**：`has_path_separator` 认 `\`（`path.rs:1052`）、
  `shell_path_to_process` 的 `/`→`\` replace（`path.rs:628`，现调用点均在
  windows 分支，属未来雷点）。修法：补 cfg 门。
- **E8 ⚪ env builtin 无条件复制 SystemRoot/WINDIR/ComSpec**
  （`src/executor/external_inner.rs:247,895`）——unix 上环境残留时才可见。
- **E9 🟡 `~user` 读虚拟 `/etc/passwd`，无 getpwnam**（`src/expand/tilde/tilde.rs:74`）：
  unix 上 `~root` 展开依赖部署的 passwd 文件。修法：`#[cfg(unix)]` 加
  `libc::getpwnam_r` 分支，文件路径保留为嵌入场景回退。
- **E10 🟡 交互 Ctrl-C 在 read 阻塞中不弃行**：unix 信号后端装了 handler 后
  read EINTR 重试（fd/unix.rs），信号在下一命令边界派发；bash 是立即弃行重绘。
  归入终端层 P1（见 E14），与 Windows 同缺，不是 unix 独有。
- **E11 🟡 进程替换是临时文件串行仿真**（`src/executor/external_setup.rs:539`）：
  失去并发/流式语义。unix 修法：改走 `/dev/fd/N` 管道（fd/unix.rs `disk_file_path`
  已有 /proc/self/fd 基础；macOS 原生 /dev/fd 也可用）。Windows 维持现状。
- **E12 ⚪ `exec` builtin 不换进程映像**（spawn+wait 代替）：PID 变化等可观察
  偏差，已决策接受并记录（fork 在多线程运行时下不安全，respawn 是终态架构）。
- **E13 ⚪ 作业控制进程组模块**（setpgid/killpg/tcsetpgrp，拟 `src/jobs/pgrp.rs`）：
  仅交互 `set -m` 完整语义需要；脚本模式 per-pid 模型已是 POSIX 正确
  （无 set -m 时 bash 也不建新 pgrp）。
- **E14 ⚪ termios raw mode / 真行编辑**：readline 目前是行提交后解释，
  双平台同缺；终端抽象缝做一次两侧受益（crossterm/termios 后端）。
- **E15 ⚪ macOS APFS 大小写不敏感 vs nocaseglob/glob 测试**：测试层，需
  macOS runner 真跑后归因。
- **E16 ⚪ Windows NTSTATUS→信号映射**：落点已收在 `wait_status.rs`
  （`process_exit_status` 单点），需要时补映射表即可。

## 重审计新增（2026-09-26，Audit 代理，wave-1 派发后合并）

引擎层：
- **E17 🟡 CRLF 剥除无门控（4 处一致）**：lexer 脚本/heredoc 行剥 CR
  （`src/lexer/mod.rs:214`，同文件 :178 注释自证 GNU 不剥）；命令替换捕获剥成对 `\r`
  （`command_subst_helpers.rs:235-259`）；`read` 行尾剥 `\r`（`read_helpers.rs:404,414`，
  :406 NOTE 自认偏差）；`read -t` 超时路径（`pipeline_exec.rs:16-22`）。unix 上均为
  GNU 偏差（83 套件可观察）。状态：后 3 处已派 A2 追加（cfg!(windows) 门控，
  Windows 零变化）；lexer 处留 wave-2（文件被并行 crew 占用）。
- **E18 🟡 `/bin/bash` 字面路径先走 PATH 搜索**（`src/executor/path.rs:252-263`）：
  `is_standard_unix_bash_path` 命中后 `find_user_command("bash")` 先于字面文件探测且无门；
  macOS 上 homebrew bash 顶替 `/bin/bash`。与 E6（miss 后回退）不同：这是命中前的
  优先级倒置。状态：已派 A2 追加（回退链 cfg!(windows)，unix 先探字面文件）。
- **E19 ⚪ 继承 TMPDIR/HOME 无条件 `\`→`/` 重写**（`src/executor/init.rs:357-363`）：
  Linux 文件名合法含 `\`（`/home/a\b` 被改写）。状态：已派 A2 追加（cfg!(windows) 门）。
- **E20 ⚪→发布层🔴 bash shim 无平台门**（`src/bin/bash.rs` + Cargo.toml
  `[[bin]] name="bash"`）：unix 构建产出 `bash` 二进制且 `locate_shell` 只探
  `niu.exe`/`winuxsh.exe` → 必 127；`cargo install`/unix tarball 会遮蔽系统 bash
  （Windows 侧 package-release.ps1:47-109 依赖该 bin 产 bash.exe，不能删）。
  修法：`locate_shell` 平台分叉候选名（unix 裸 `niu`）+ unix 发布不装 shim。归 C2 批次。
- **E21 ✅ 引擎 ctrlc 死依赖已删**（959961a9）。
- E9 修订：`~` 的 USERPROFILE 兜底（`tilde.rs:22-28`）随 getpwnam 改造一并 getpwuid；
  unix HOME 未设时 `~` 现展开为空串。
- E7 修订：`support_names.rs:668` 另有一个无门控的同名 `executable_extensions()`
  （PATHEXT、`;` 切分），唯一调用点已被 cfg!(windows) 包裹——目前良性，建议改名防误用。

niubash 层：
- **N10 ✅ 命令补全只认 Windows 后缀**（`completion/command.rs:154-196`）：PATH 扫描
  仅收 .exe/.bat/.cmd/.ps1/.sh/.bash/.zsh/.niubash 且去后缀；unix 无后缀可执行全漏，
  补全退化为硬编码 Windows 命令表——unix 上第一个用户可感知的功能失效。
  已修（niubash b54a346）：unix 按可执行位收录（`metadata()` **跟随符号链接**——与
  GNU findcmd.c `file_status` 的 stat 语义一致，homebrew/alternatives 目录正确）、
  不去后缀、常用命令表按平台分叉；Windows 分支字节不变。
- **N11 ✅ which_tool 后缀探测无门控**（`completion/external.rs:988-1000`）：unix 上
  `foo.exe` 抢先于真 `foo`。已修（niubash b54a346）：后缀表与 admission cfg 配对。
- **N12 ✅ windows_terminal 模块全平台编译**（`lib.rs:32` 无门）：已修（niubash
  b54a346）：模块 `#[cfg(windows)]`；`--install-wt-profile` unix 下明确报
  Windows-only；fonts/setup_wizard 的 WT 触点 unix no-op 配对。
- N1 修订：`repl.rs:97` 已漂移为 `spawn_self_update`（原 file:line 失效）；宿主直接
  spawn 全量清单：`completion/runtime.rs:125`、`completion/external.rs:546`、
  `shell.rs:2023/2100/2123/2162/4418/4534`（direnv/zoxide/thefuck/fzf/进程插件，
  统一 `resolve_native_command_path*` :4677-4723）、`git_status.rs:476`、`prompt.rs:976`、
  `plugins/external.rs:159/313`——全部并入"委托引擎"改造。
- N6 修订：crossterm feature 固定 `["event-stream","windows"]` 且 default-features=false
  ——关掉了 bracketed-paste、WinAPI 后端写死；feature 集按平台审，验证随 N9 runner。
- 已验证无 API 编译断点：`set_winuxcmd_path`/`set_shell_root`/`set_compatible_shell_path`
  无 cfg 但 unix 可编译（仅写 env）；`set_elevation_handler` 是 cfg(windows) 且 niubash 零调用。

## CI 与发布缺口

- **C1 🔴 rubash CI 无 macOS runner**（`.github/workflows/ci.yml` 全 ubuntu；
  cross-target-check 只证明"编译 FOR darwin"，unix 门控测试只在 ubuntu 执行）。
  修法：加 `macos-latest` 跑 `cargo test --lib`。
- **C2 🔴 rubash release 无 darwin 产物**（`release.yml` grep darwin/apple 零命中；
  linux tarball 已有）。修法：release matrix 加 aarch64/x86_64-apple-darwin。
- **C3 🔴 f12aa756 未 push**：niubash 以 `rubash = { git, branch=master }` 消费，
  引擎 unix 接线对 niubash 不可见，直到 push（captain 决策）。
- **C4 🟡 83 套件从未在任何 unix 真二进制上跑过**。修法：CI 加 linux
  （先 ubuntu 自举编译）smoke slice，diff 账本进 `target/issue-suites/results/`。

## niubash 产品层缺口（跨平台升级）

> 定位：niu.exe = 宿主层（crates/niubash-runtime），嵌入 rubash 引擎；
> 基线文档 `docs/niu-product-baseline-20260922.md`（P0-P5 是 Windows 侧欠账，
> 跨平台改造顺势一起还）。

- **N1 🔴 自建 spawn 旁路**（`crates/niubash-runtime/src/repl.rs:97`
  `std::process::Command::new(exe)`）：不走引擎 PATH 解析，Windows 侧已造成
  ~87 行套件差异（基线 P1），跨平台上必须委托引擎（引擎的 per-platform 查找
  已就位，宿主旁路会把平台差异全部重新引入）。
- **N2 🔴 ctrl_c.rs 宿主自带 Ctrl-C 处理**：与引擎新 unix 信号后端并存，
  unix 上有双处理/吞信号风险。修法：信号语义统一归引擎，宿主只做 UI 层反应
  （重绘提示符），或明确声明对引擎 `take_pending_signals` 的只读订阅。
- **N3 🟡 console_guard.rs / terminal.rs / text_style.rs 的 Windows 控制台假设**
  （UTF-8 codepage、VT 开关）：unix 需 no-op 或对齐 termios/ANSI 默认。
- **N4 🟡 fonts.rs `curl.exe` 下载、doctor.rs 体检、setup_wizard 的 Windows 假设**：
  unix 用系统 `curl`/跳过。
- **N5 🟡 self_update.rs WinHttp 自更新**（顶层 windows-sys WinHttp）：
  unix 需 https 通道（ureq/curl）+ 安装位置约定。
- **N6 ⚪ reedline 0.50 / crossterm 0.27 本身跨平台**（crossterm features 需
  验证 unix 后端编译）；fd-lock/dirs/globset 均跨平台，无阻。
- **N7 ⚪ git_status/prompt 段调用 `git` 子进程**（`prompt.rs:976`）：跨平台可用，
  但 spawn 应统一走引擎（并入 N1）。
- **N8 ⚪ installer/ 与发布形态**：Windows 安装器之外需 tarball/homebrew 形态
  （并入 C2 产物矩阵）。
- **N9 ⚪ CI**：niubash ci.yml 仅 windows-latest；加 ubuntu + macos 编译与
  冒烟（等 C1/C3 就绪）。

## 建议顺序

> **Wave-1 已派发（2026-09-26，并行子代理，worktree 隔离）**：
> f12aa756 已 push（origin/master，经 `http.version=HTTP/1.1` 绕过 HTTP/2 TLS 故障）；
> C1/C2 由 captain 直接落地；A1=E1（worktree rubash-wt-e1，kill/trap/trap_exec/
> job_builtins 信号常量化）；A2=E2-E9（worktree rubash-wt-e2，机械门控）；
> A4=niubash N3/N4/N5/N9（宿主平台门控+CI）；A5=niubash N2（ctrl_c 对齐）；
> Audit=双仓重审计（新发现进 wave-2）。N1（spawn 委托）留 wave-2：需引擎侧
> 公共 API 先落地。

1. C3 push f12aa756（解锁 niubash 侧一切）；
2. E1 信号表按 target 生成（macOS 唯一语义阻塞，bash mksignames 同构）；
3. C1/C2 CI + release 补 macOS/linux runner 与产物；
4. N1+N2（宿主 spawn 委托引擎 + Ctrl-C 归一）——niubash 跨平台的核心两刀；
5. E2-E9 机械清欠（每项一处 cfg/委托，独立可验证）；
6. C4 linux 83-suite smoke，账本驱动后续。

**Wave-2 素材（A5/N2 回收时新增，2026-09-26）**：
- 引擎加**非消费** `peek_pending_signals()`——宿主提示符时机反应的正确接口；
  `take_pending_signals` 是破坏性排空（kill.rs `mem::take` + `Signals::pending()`），
  REPL 轮询它会在引擎派发前偷走信号（等同吞没）。
- `--noediting` 回退路径（无 raw mode）unix 下 ^C 产生真内核 SIGINT，
  `read_line` 的 `ErrorKind::Interrupted` 需显式验证/处理。
- ~~删 rubash `ctrlc` 死依赖~~（已完成，防宿主/引擎内误用引发处理器替换或叠加）。
