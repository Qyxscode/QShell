# QShell Lite 0.3

**轻量 SSH 终端客户端**：单 exe + 一个 DLL，无 WebView、无运行时依赖，
**Windows XP SP3 ~ Windows 11 通用**（.NET Framework 4.0 / x86）。

和 QShell 桌面版（Win11）**共用同一份配置文件**：
`%APPDATA%\QShell\config.json`，两边改的主机、备忘录、设置都互相可见。

## 文件

| 文件 | 说明 |
|---|---|
| `QShellLite.exe` | 主程序（101 KB） |
| `Renci.SshNet.dll` | SSH 库（必须和 exe 同目录） |

## 功能

### 终端

自写 VT 引擎（不用 PuTTY / xterm）：

- 16 色 / 256 色 / 真彩、粗体、下划线、反显
- 中日韩双宽字符、框线字符
- 备用屏（vim / htop / tmux / less）、滚动区、scrollback
- 滚动条、Shift+PageUp/PageDown 翻历史
- 选择复制（拖动即复制）、双击选词、三击选行、`Ctrl+F` 搜索
- `Ctrl+Shift+C/V/A` 复制粘贴全选、括号粘贴
- OSC 8 超链接（Ctrl+点击打开）、OSC 52 剪贴板
- 鼠标上报（vim / tmux）
- 光标样式与闪烁（`DECSCUSR`）、可配光标颜色
- 方向键 / Home / End / 退格 / F1~F12、SS3 与 CSI 双模式
- Alt+字母/数字（meta 序列）、Ctrl+方向（按词移动）
- 窗口缩放自动同步远端（拖完 400ms 通知，避免反复重绘）
- 大量输出流控（每帧最多 256KB）

### 会话

- 多标签（标签上 × 关闭、中键关、`Ctrl+W`）
- SSH：密码 / 私钥（OpenSSH 与 PuTTY `.ppk`）
- **跳板机**（SSH 隧道，支持密钥）
- **串口**：COM 口、波特率、数据位/校验/停止位/流控、DTR/RTS
- 断线自动重连（3 秒重试，内容保留）
- 会话日志（去 ANSI，可读文本，写到 `%APPDATA%\QShell\logs\`）

### 主机管理

- 树形分组（空分组也显示）、拖拽排序
- 新增 / 编辑 / 克隆 / 删除 / 复制地址
- 备注（`details.memo`）
- **延迟探测**（TCP 握手耗时，显示在树上）

### 备忘录

- 命令分组快捷面板，右侧常驻
- 双击/回车立即执行，「插入到终端」只填不执行
- 新增 / 编辑 / 删除 / 复制命令文本 / 分组管理
- 拖拽调整顺序与分组

### 外观 / 设置

- 字体（只列等宽字体）、字号、粗体、光标样式、闪烁、字色
- 自动重连、会话日志开关
- **备份导入导出**：`.zip`（内含 `qshell_config.json`）或 `.json`，与桌面版通用

## 与桌面版的差异

| 项 | 说明 |
|---|---|
| 分屏 | **不做**（SSH 下本地重排与远端重绘冲突，成本高于收益） |
| 现代 UI | WinForms 原生观感，无圆角/动画/亚克力 |
| 字体连字 | GDI 不支持 |
| SFTP / 端口转发 / 流量统计 | 暂未实现 |
| Telnet / RLogin / Raw | 暂未实现（SSH + 串口够用） |

## 数据安全

- 读写 `config.json` 时**保留所有未知字段**（桌面版新增的字段不会丢）
- 写入前自动备份 `config.json.bak`
- 导入备份前会校验 JSON 并再次备份

## 已知限制

- 窗口拉宽后，历史内容不会重新折行（变窄只是隐藏，内容不丢）
- 串口功能未在真实硬件上验证
- 高 DPI 下字体可能偏小（未做 DPI 感知）

## 构建

```powershell
powershell -ExecutionPolicy Bypass -File .\build.ps1
```

需要 .NET Framework 4.0 的 `csc.exe`（`C:\Windows\Microsoft.NET\Framework\v4.0.30319\`）。
`build.ps1` 必须是 **UTF-8 with BOM**（含中文，否则 PowerShell 5.1 按 GBK 解析报错）。
