# Rubash Host & Engine Contract（宿主与引擎合约）

状态：owner 裁决 2026-09-09。本文是宿主层（niubash/niu.exe）与 rubash 引擎层
之间的权威合约记录 + 通用化路线图。优先级：**排在 GNU 平价战役之后**，
但所有裁决必须记录在案（本文）。

## 0. Owner 裁决

1. **License 统一 MIT**。Cargo.toml 已是 MIT；仓库内残留 GPL-3.0-or-later
   文件头需要一次性清扫（低优先级 TODO，本条即记录）。
2. **品牌命名空间 = NIU_\***（正名）；WINUXSH_\* = 遗留兜底别名；
   `__RUBASH_\*` = 引擎内部通道（清单不可见），维持不变。
3. rubash 定位：通用 bash 语义引擎（可被任意宿主嵌入/调用），宿主无关为终态。

## 1. 环境变量合约（目标态）

| 目标变量 | 遗留别名 | 语义 |
|---|---|---|
| NIU_SHELL_ROOT | WINUXSH_ROOT, RUBASH_ROOT | 支撑 `/` 逻辑根的原生目录（bin/,usr/bin/,etc/,tmp/,var/）|
| NIU_SHELL_PATH_STYLE=host\|slash-drive | WINUXSH_SHELL_PATH_STYLE | PWD 显示拼写 |
| NIU_DISPATCHER | WINUXCMD, WINUXCMD_PATH | WinuxCmd 命令 dispatcher 可执行文件 |
| NIU_SHELL | WINUXSH_SHELL（bash shim 转发目标）| shim/引擎指向 |

解析链（已修空值遮蔽 bug，commit 7eb9403d）：按 NIU_SHELL_ROOT →
WINUXSH_ROOT → RUBASH_ROOT 顺序取**第一个非空值**；迁移完成后 NIU_* 唯一。
嵌入 API 通道 `__RUBASH_SHELL_ROOT` 恒为最优先（清单不可见，命名空间正确）。

## 2. 宿主↔引擎职责分层（NIU-1 审计确认的现状 + 目标）

- 宿主（niu.exe，静态嵌入 rubash）：reedline 编辑/补全候选/prompt 渲染/
  历史**存储**（~/.niubash_history，经 HistoryProvider 注入）/高亮/插件。
- 引擎（rubash）：全部 shell 语义；history/fc 内建（走注入的 Provider）；
  complete/compgen 语义（GNU 纯表）；prompt 模板展开。
- 合约铁律：**执行侧表**保留 Windows 扩展超集（igncr 等接受为 no-op），
  **补全候选表**保持 GNU 纯净——此分层已实现，用测试锁死（P3-B）。
- rubash 历史扩展（!word/histchars）：门控在**会话内历史列表有无条目**，
  不门控在交互式 history 选项/HISTFILE（宿主强制 history=false 不得静默
  禁用脚本侧扩展）——已下发实施代理（2026-09-09）。

## 3. 待办（降优先级，按此顺序执行）

### 宿主侧（owner 已授权在 NIU 仓同步更改）
- P1-A 重编 niu.exe 带最新 rubash（path 依赖，纯构建即得）。
- P1-B 修 bash shim 偏斜：宿主启动 export NIU_SHELL=<niu.exe>（或 WINUXSH_SHELL
  过渡），让 PATH 上的 bash 脚本不再跑 8/21 旧引擎。
- P2-A WINUXSH_HIST_IGNORE_DUPS/_SPACE 死信道：宿主零读者已实锤 →
  rubash 停写（或宿主映射 reedline history_exclusion_prefix + 去重）。
- P2-D rubash 读 NIU_* 优先 + WINUXSH_* 兜底 → 宿主 shell.rs 删旧名双写。
- P3-A 清理 niu1.exe / usr/bin/__old_0.17.0/ / tmp 残留；
  ⚠️ ~/.niubashrc 含真实 API 密钥——任何文档/打包严禁引用其内容。

### 引擎侧（rubash）
- GPL 文件头 → MIT 清扫（一次性，低优先）。
- set_shell_root/set_winuxcmd_path 停止**导出**品牌变量（改只设 __RUBASH_*
  隐藏通道；宿主确认不读导出值后执行）。
- WINUXSH_UNSUPPORTED_DEVICE sentinel 改名 NIU_（T1 机械）。
- init.rs 与 cd/paths.rs 的 PWD 风格双实现合并为单一所有权（T2）。
- Cargo features：windows-host / test-support 门控（NIU-4 Phase 1）。
- `bash` bin 在非 Windows 上必须是真 shell（现为 winuxsh shim，127）。
- 跨平台：CI 装 x86_64-unknown-linux-gnu + aarch64-apple-darwin 跑
  `cargo check --target`（本机无该 target，真实错误清单待采集）；
  嵌入 API 面：Shell facade / ShellIo trait / 信号注入 / env 隔离构造器；
  __RUBASH_* env-IPC（537+ 处/~180 名）最终替换为结构化 API（NIU-4 Phase 4）。

## 4. 证据源

- NIU-1：托管链/所有权矩阵/依赖盘点（2026-09-09，read-only）。
- NIU-2：命令集/逐套件宿主归因/env 合约/版本偏斜（同日）。
- NIU-3：WINUX 94 处逐站点盘点 + 合约提案（同日）。
- NIU-4：跨平台就绪度/嵌入 API 缺口/六阶段路线（同日）。
- 空值遮蔽修复：7eb9403d（已验证 cd / && pwd 经逻辑根解析）。
