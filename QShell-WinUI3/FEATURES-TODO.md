# 需求与待办看板

- 更新时间：2026-10-24
- 说明：这里是**待实现/待修复**项的索引，详细分析见对应文档。

## 状态总览

| # | 项目 | 类型 | 状态 | 预估 |
|---|---|---|---|---|
| 1 | IPv6 字面量连接修复 | 缺陷修复 | ✅ 已完成（2026-10-23） | 0.5 小时 |
| 2 | 本地终端（PowerShell / CMD） | 新功能 | ✅ 已完成（2026-10-23） | 1~2 天 |
| 3 | 终端类型（TERM）选择 + 自动探测 | 新功能 | ✅ 已完成（2026-10-23） | 2~3 小时 |
| 4 | 键盘交互式认证 | 新功能 | ✅ 已完成（2026-10-23） | 0.5 天 |
| 5 | Agent 认证（使用系统 ssh-agent） | 新功能 | ✅ 已完成（2026-10-23） | 0.5 天 |
| 6 | 密钥生成向导 + 导入/导出 | 新功能 | ✅ 已完成（2026-10-23） | 1~2 天 |
| 7 | 主密码（配置加密） | 新功能 | ✅ 已完成（2026-10-24，开关默认关闭） | 1 天 |
| 8 | 断线自动重连 | 新功能 | ✅ 已完成（2026-10-24） | 1 天 |
| 9 | 会话恢复（退出后接续标签与终端缓冲） | 新功能 | ✅ 已完成（2026-10-25） | 1~2 天 |
| 11 | 终端搜索（Ctrl+F） | 新功能 | ✅ 已完成（2026-10-24） | 2~3 小时 |
| 13 | 跳板机（ProxyJump） | 新功能 | ✅ 已完成（2026-10-24） | 2~3 天 |
| 15 | 终端日志录制 | 新功能 | ✅ 已完成（2026-10-24） | 0.5 天 |
| 19 | SFTP 目录同步（镜像上传） | 新功能 | ✅ 已完成（2026-10-24） | 2 天 |
| 20 | Raw TCP（裸 TCP 透传） | 新功能 | ✅ 已完成（2026-10-25） | 1 天 |
| 21 | Rlogin（RFC 1282） | 新功能 | ✅ 已完成（2026-10-25） | 1 天 |
| 22 | FTP（接入 SFTP 面板：列表/上传/下载/删除/重命名/新建目录） | 新功能 | ✅ 已完成（2026-10-25） | 3 天 |

> 3~7 详见 [FEATURES-TERM-TYPE.md](./FEATURES-TERM-TYPE.md) 与 [FEATURES-SSH-CAPABILITIES.md](./FEATURES-SSH-CAPABILITIES.md)。

---

## 1. IPv6 字面量连接修复（已完成）

**实现**（2026-10-23）：
- `ssh.rs` 新增 `connect_tcp()`：`(host, port).to_socket_addrs()` + 每个地址 `connect_timeout(8s)` 轮询；`start_ssh_session` 改用它，支持 IPv6 字面量并消除 21 秒卡顿。
- `probe.rs` 改为 `(clean_host, target_port).to_socket_addrs()`。
- 验证：`cargo test` 全绿；域名/IPv4 路径不变。

**原因**：地址用字符串拼接，未处理 IPv6 需要方括号的规则。

| 位置 | 现状 | 修复 |
|---|---|---|
| `ssh.rs:89` | `format!("{}:{}", host, port)` | 改为 `TcpStream::connect((host.as_str(), port))`，或 `to_socket_addrs()` |
| `probe.rs:16` | `format!("{}:{}", clean_host, target_port)` | 改为 `(clean_host.as_str(), target_port).to_socket_addrs()` |

**顺带修复**：SSH 主连接目前使用 `TcpStream::connect`，没有连接超时，不可达地址会卡到系统默认（约 21 秒）。建议改成 `connect_timeout` + 多地址轮询（`sftp.rs:46-54` 已有正确写法）。

**验收**：
- 填 `2001:db8::1` 能连接，且 `[2001:db8::1]` 旧写法仍兼容；
- 域名、IPv4 行为不变；
- 不可达 IPv6 地址 8 秒内报错，不再长时间卡住。

---

## 2. 本地终端（PowerShell / CMD）支持（已完成）

**需求**：在「添加主机」里增加本地 Shell 类型，可选 **PowerShell** 与 **CMD**，打开后是一个本地终端会话，行为与现有终端一致（颜色、光标、复制粘贴、字体、主题等）。

### 2.1 数据模型

- 协议枚举从 `"ssh" | "telnet" | "serial"` 扩展为 `"ssh" | "telnet" | "serial" | "local"`
- 本地类型增加子字段 `local_shell: "powershell" | "cmd"`（默认 `powershell`）
- 可选：`local_cwd`（起始目录）、`local_args`（额外参数）

### 2.2 后端

- 新增 `local_shell_service.rs`，与 `serial_service.rs` 同构：
  - 用 **ConPTY**（Windows 10 1809+）创建伪终端，推荐 `portable-pty` crate（wezterm 维护），它会自动使用 ConPTY；
  - 进程：`powershell.exe -NoLogo` 或 `cmd.exe`；工作目录取 `local_cwd`；
  - 复用 `GenericStreamManager`（writer + 读线程 + `ssh-data-{sid}` / `ssh-closed-{sid}` 事件）；
  - 终端尺寸通过 `resize` 同步到 PTY（ConPTY 支持）；
  - 结束会话时确保子进程被终止（避免僵尸进程）。
- 命令注册：`start_local_shell_session` / `write_generic_session`（复用）/ `resize_generic_session`（复用）/ `close_generic_session`（复用）。

### 2.3 前端

- 「添加主机」协议下拉增加 **本地终端**，选中后显示「Shell 类型：PowerShell / CMD」；
- 会话创建时走本地启动命令；
- 本地会话**隐藏**右侧 SFTP 面板与监控卡片（无远程主机概念），或显示"本地会话不支持"；
- 标题栏/标签页显示 `PowerShell` 或 `CMD` 图标与名称；
- 底部状态栏不显示远端延迟。

### 2.4 兼容与限制

- 仅 Windows 支持（ConPTY）；其它平台可用 `forkpty`（本次不做）；
- 本地终端不受 SSH 主题/字体影响？——应共用现有 `terminalSettings`，保持体验一致；
- 安全性：本地终端等于直接执行命令，属预期行为，但需在首次使用时提示一次。

### 2.5 验收

1. 添加本地主机，选 PowerShell，打开后能执行 `Get-ChildItem`，颜色正常；
2. 选 CMD，执行 `dir` 正常；
3. 调整窗口大小，`mode con` / `$Host.UI.RawUI.WindowSize` 随之变化；
4. 关闭标签，任务管理器里 `powershell.exe`/`cmd.exe` 子进程被结束；
5. 本地会话不显示 SFTP 与监控面板。

### 2.6 工作量拆分

| 步骤 | 预估 |
|---|---|
| 数据模型 + 表单 UI | 0.5 天 |
| `portable-pty` 接入 + 会话服务 | 0.5 天 |
| 前端协议分支与面板联动 | 0.5 天 |
| 回归与边界（多开、关闭清理） | 0.5 天 |

---

## 3~7 已记录项（摘要）

- **终端类型**：下拉 + `自动`；SSH 用 `infocmp` 探测（缓存结果），Telnet 回退 `xterm-256color`。详见 FEATURES-TERM-TYPE.md。
- **键盘交互式认证**：`ssh2` 已有 `userauth_keyboard_interactive`，接回调。
- **Agent 认证**：`userauth_agent`，前端增加认证方式选项。
- **密钥生成/导入导出**：生成 ed25519/rsa/ecdsa，解析 OpenSSH/PEM/PPK。
- **主密码**：AES-GCM 加密 `config.json`。
