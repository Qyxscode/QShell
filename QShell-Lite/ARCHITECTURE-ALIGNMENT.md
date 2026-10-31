# QShell 两个版本：架构对照与 Lite 版边界

> 目的：在继续开发 Lite 版之前，先把 Win11 桌面版的架构、数据结构、功能边界梳理清楚，
> 避免"做一半发现理解不对"的返工。

## 一、桌面版（QShell-WinUI3）架构

### 技术栈

| 层 | 技术 |
|---|---|
| 壳 | Tauri 2（Rust） |
| 前端 | Vue 3 + TypeScript，单文件 `App.vue`（11,144 行） |
| 终端 | `@xterm/xterm` + addon：fit / search / serialize |
| 后端 | Rust，26 个模块，注册 **92 个 Tauri 命令** |

### 前端结构

```
src/
  App.vue             11,144 行，全部界面与状态
  components/         14 个弹窗/控件（ColorPicker、FontSelector、KeyGen、
                      Encryption、Unlock、Forward、Trace、Update…）
  themes.ts           主题预设
  options.ts          协议/终端类型/串口参数等选项
  utils/              格式化、模糊匹配、ANSI、路径
```

### 界面布局

```
┌──────────────────────────────────────────────────────────────┐
│ 标题栏: QShell v1.0.0 │ [主机管理] 🌙 🔔 ↕ ⚡ 🖥 分屏 ⚙ ▣ │ －□✕ │
├───────────┬──────────────────────────────────┬───────────────┤
│ 系统监控   │                                  │ [备忘录][SFTP][历史] │
│  流量/CPU  │          终端区（多标签）          │  ┌──────────┐ │
├───────────┤      支持 1~4 屏分屏（Alt+1~4）    │  │ 分组      │ │
│ 设备列表   │                                  │  │  命令项   │ │
│  (树形)    │                                  │  └──────────┘ │
│            │                                  │  [+ 添加命令项]│
└───────────┴──────────────────────────────────┴───────────────┘
```

- 右侧栏可折叠（280ms 过渡动画，折叠时重算终端尺寸）
- `currentView`: `workspace`（会话） / `mgmt`（主机管理视图）切换
- 终端全屏：F11

### 数据模型（`%APPDATA%\QShell\config.json`）

```jsonc
{
  "version": "...",
  "devices": [
    {
      "group": "VPS",
      "is_standalone_host": false,
      "hosts": [HostItem]
    }
  ],
  "managed_hosts": [HostItem],          // 主机管理里"未分组"的独立主机
  "memos": [
    { "group": "VPS常用", "items": [ { "name": "DNS解锁检测", "cmd": "..." } ] }
  ],
  "settings": AppSettings
}
```

**HostItem**
```
name, protocol(ssh/telnet/rlogin/raw/serial/local_shell),
host, port, user, pwd, key_path, passphrase,
serial_data_bits/parity/stop_bits/flow_control/newline/local_echo/dtr/rts,
proxy_host/proxy_port/proxy_user/proxy_pwd/proxy_key/proxy_passphrase,   // 跳板机
term_type, use_agent,
details: HostDetails,
latency_text
```

**HostDetails**
```
memo, expire_date, alert_expire_days, price, currency,
traffic_total/traffic_total_unit/traffic_total_infinite,
alert_traffic_pct, traffic_used/traffic_used_unit,
reset_day/hour/min/sec, traffic_direction, target_interface,
traffic_price/currency, traffic_cycle_used_bytes, traffic_source,
traffic_custom_command, traffic_source_effective/error,
traffic_last_sync_ts, traffic_delta_bytes, last_raw_bytes,
traffic_counter_sig, traffic_boot_id, last_reset_timestamp,
traffic_manual_override, traffic_iface_baseline,
traffic_vnstat_anchor_bytes/ts, traffic_cycle_anchor_used_bytes,
traffic_force_nic
```

**AppSettings**
```
app_font_family, font_family, font_size, is_bold,
cursor_style, cursor_blink, blink_interval,
theme_mode, term_bg_mode, terminal_fg_color, terminal_bg_color,
terminal_bg_image, terminal_bg_image_fit/dim/auto_fg,
accent_color, minimize_to_tray, autostart_enabled,
session_log_enabled, auto_reconnect, session_restore_enabled,
webdav { url, user, pwd, path }
```

### 后端模块（Rust）

| 模块 | 行数 | 职责 |
|---|---|---|
| `config.rs` | 416 | 配置读写、原子写入、加密入口 |
| `config_crypto.rs` | 95 | 配置加密/解密（主密码） |
| `ssh.rs` | 581 | SSH 会话（连接、写、resize、关闭） |
| `generic_*` | 341 | Telnet / 串口 / 本地 Shell / Raw 的统一会话层 |
| `telnet_service.rs` + `telnet_codec.rs` | 409 | Telnet 协议与协商 |
| `serial_service.rs` | 124 | 串口 |
| `raw_rlogin_service.rs` | 181 | Raw TCP / RLogin |
| `local_shell_service.rs` | 111 | 本地 Shell |
| `sftp.rs` | 1009 | SFTP 浏览/传输/同步 |
| `ftp.rs` | 393 | FTP |
| `port_forward.rs` | 735 | 本地/远程/SOCKS 转发 |
| `traffic.rs` | 1224 | 流量统计（大模块） |
| `monitor.rs` | 499 | 系统指标（CPU/内存/磁盘/网络） |
| `probe.rs` | 95 | 延迟探测 |
| `trace_route.rs` | 193 | 路由追踪 |
| `backup.rs` | 427 | 本地备份导出/导入 + WebDAV |
| `keygen.rs` | 184 | 密钥生成/检查 |
| `session_log.rs` | 87 | 会话日志 |
| `session_state.rs` | 33 | 会话状态保存（恢复上次会话） |
| `local_fs.rs` | 233 | 本地文件系统（SFTP 面板用） |
| `lib.rs` | 566 | 命令注册、窗口效果 |

## 二、桌面版功能清单（按模块）

| 模块 | 能力 |
|---|---|
| **连接协议** | SSH / Telnet / RLogin / Raw TCP / 串口 / 本地 Shell |
| **会话** | 多标签、1~4 屏分屏（Alt+1~4）、终端全屏（F11）、自动重连、恢复上次会话 |
| **认证** | 密码 / 私钥 / passphrase / ssh-agent |
| **跳板机** | ProxyJump（host/port/user/pwd/key/passphrase） |
| **终端** | xterm.js：真彩、连字、鼠标、超链接、搜索、序列化 |
| **端口转发** | 本地 / 远程 / SOCKS |
| **SFTP** | 目录浏览、上传下载、拖拽、改权限、同步目录、传输队列 |
| **FTP** | 连接、浏览、上传下载、增删改名 |
| **备忘录** | 命令分组快捷面板 |
| **历史** | 会话历史记录 |
| **流量/到期** | 流量统计、到期提醒、通知铃铛 |
| **系统监控** | CPU/内存/磁盘/网络吞吐 |
| **配置** | 主密码加密、本地备份、WebDAV 云备份 |
| **密钥工具** | 生成 SSH 密钥、检查公钥 |
| **外观** | 主题（暗/亮 + 预设）、字体、字号、光标样式/闪烁、终端背景图/色 |
| **系统集成** | 托盘、最小化到托盘、开机自启、更新检查、通知 |
| **诊断** | 路由追踪、延迟探测 |
| **剪贴板** | 读写（OSS 52 也走这里） |

## 三、Lite 版现状（QShell-Lite）

| 模块 | 状态 |
|---|---|
| 技术栈 | C# / WinForms / .NET 4.0 / 自写 VT 终端 / SSH.NET 2020 |
| 平台 | XP SP3 ~ Win11，单 exe + 1 个 DLL |
| 连接协议 | **仅 SSH** |
| 终端 | 自写：真彩、CJK、备用屏、滚动条、搜索、选择复制、超链接、鼠标、光标样式 |
| 会话 | 多标签（可关）、无分屏、无自动重连、无会话恢复 |
| 认证 | 密码 / 私钥（.ppk + OpenSSH） |
| 主机管理 | 增删改、分组树、克隆、复制地址、备注（details.memo） |
| 备忘录 | 右侧面板，读写 config.json 的 `memos` |
| 配置 | 读写同一份 config.json，保留未知字段，写前 .bak |
| 右侧面板 | 备忘录 +（临时）主机详情 |

## 四、差距矩阵

### A. 建议对齐（Lite 能做好，价值高）

| 功能 | 说明 | 工作量 |
|---|---|---|
| **外观设置** | 字体/字号/光标样式/颜色/主题（读写 `settings`） | 中 |
| **自动重连** | 断线检测 + 重试（读 `settings.auto_reconnect`） | 小 |
| **会话日志** | 写日志文件（读 `settings.session_log_enabled`） | 小 |
| **SFTP 基础** | 浏览 + 上传下载（SSH.NET 有 SftpClient） | 大 |
| **端口转发** | 本地转发 + SOCKS（SSH.NET 有 ForwardedPort） | 中 |
| **跳板机** | ProxyJump（SSH.NET 支持 ProxyTypes） | 中 |
| **延迟探测** | TCP 探测，写 `latency_text` | 小 |
| **分屏** | 1~2 屏（SplitContainer） | 中 |
| **历史记录** | 简单版（SQLite 或 JSON） | 中 |

### B. 需简化或换方案

| 桌面版 | Lite 版做法 |
|---|---|
| xterm.js 连字 | GDI 不支持，放弃 |
| 终端背景图片 | 可选，GDI 实现成本高，建议放弃 |
| 现代 UI（圆角/动画/亚克力） | WinForms 做不了，保持朴素 |
| 流量统计（1200 行 Rust） | 工作量过大，先不做 |
| WebDAV 备份 | 可选，后置 |
| 多协议（Telnet/串口/本地 Shell） | 按需，SSH 优先 |

### C. 明确不做（架构不匹配）

- 主题市场式的现代视觉
- 连字 / 背景图 / 透明
- 6el / Sixel

## 五、建议的 Lite 版功能边界（第一版）

**目标**：能在 XP~Win11 上稳定用的轻量 SSH 客户端，数据和桌面版互通。

1. SSH 连接（密码 / 私钥 / 跳板机）
2. 多标签终端（自写 VT）
3. 主机管理（分组树、增删改、备注）
4. 备忘录（命令快捷，已做）
5. 外观设置（字体/字号/光标/配色，读写 `settings`）
6. 自动重连 + 会话日志
7. 点击主机详情面板（显示到期/流量等 `details` 只读信息）

**第二版可加**：SFTP 基础、端口转发、分屏、延迟探测、历史记录。

## 六、待确认

1. **右侧栏是否需要 SFTP / 历史两个 tab**（现在只有备忘录）？
2. **主机详情面板**保留吗（三栏后中间区域变窄）？
3. 外观设置做**完整设置窗口**，还是只做几项常用的？
4. 是否要**主机管理视图**（桌面版的 `mgmt` 全屏视图），还是保持左侧树？
