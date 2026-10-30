# QShell

面向 Windows 10 / 11 的一体化终端客户端：**SSH / SFTP / FTP / Telnet / Rlogin / Raw TCP / 串口 / 本地终端**，基于 **Tauri 2 + Rust + Vue 3 + xterm.js**。

单文件便携运行，静态链接 MSVC 运行库，用户端无需安装 VC++ Redistributable。

---

> **🪟 需要支持 Windows XP / 7 / 8.1？**
>
> 主项目依赖 WebView2 与现代 Rust 工具链，**无法在 Windows 7 及更早系统运行**。
> 老系统请使用独立实现的分支版本：**[QShell Lite →](../../releases?q=lite&expanded=true)**
>
> - 单目录部署（`QShellLite.exe` + 一个 DLL），解压即用
> - 自写 VT 引擎，不依赖 PuTTY / WebView2
> - 支持 Windows XP SP3 ~ Windows 11
> - 与主项目**共用同一份配置文件**，主机/备忘录/设置互通
> - ⚠️ 预览版，未做详细测试，请勿用于关键生产环境
>
> 该版本为长期可用的独立分支，不受主线版本更新影响。

---

## 功能

### 连接协议

| 协议 | 说明 |
|---|---|
| **SSH2** | 密码、私钥（RSA / ECDSA / ED25519）、键盘交互式认证、系统 SSH Agent、**跳板机（ProxyJump）** |
| **Telnet** | ECHO / SGA / TTYPE / NAWS 协商、终端类型选择 |
| **Rlogin** | RFC 1282 协议 |
| **Raw TCP** | 裸字节透传（网络设备、调试端口） |
| **串口** | 波特率、数据位、校验位、停止位、流控、DTR/RTS、换行转换 |
| **本地终端** | 内置 PowerShell / CMD（ConPTY） |

### 文件传输

- **SFTP**：浏览、上传、下载、重命名、删除、chmod、**目录同步（镜像上传）**
- **FTP**：复用同一文件面板（列表 / 上传 / 下载 / 删除 / 重命名 / 新建目录）
- **主机级文件传输策略**：自动 / 不启用 / SFTP / FTP
  - 自动模式下 SSH 走 SFTP、Telnet 等协议自动尝试 FTP
  - 非 SSH 会话也能直接使用文件面板
- 传输进度气泡（自动弹出）、冲突处理（覆盖 / 跳过 / 重命名）

### 终端

- 多标签、拖拽排序、标签锁定、只读模式
- **终端搜索**（Ctrl+F，大小写可选）
- **日志录制**（纯文本，自动剥离 ANSI 控制字符）
- **全屏模式**（F11 / Esc / 工具栏按钮）
- **命令输入历史**（按主机过滤、右键复制/删除/发送、密码不记录）
- **快速命令面板**（Ctrl+K 或底部胶囊触发，模糊搜索 + 匹配高亮，备忘录 + 历史双数据源，支持钉住常驻、清空历史）
- **会话恢复**（退出后接续标签与终端缓冲，可选择开关）
- **断线自动重连**（3s / 6s / 12s / 24s / 30s，共 5 次；主动 `exit` / `logout` / Ctrl+D 不重连）

### 运维工具

- **系统监控**：CPU / 内存 / 磁盘 / 网络实时面板与折线图
- **流量审计**：网卡计数器与 vnstat 周期统计（可选）
- **到期提醒**：主机到期与流量阈值告警
- **路由追踪**：逐跳流式显示、延迟条形可视化、IP 归属地、连通状态
- **会话事件通知**：异常掉线（红）/ 手动断开（灰蓝）/ 已重连（绿），通知面板双页签
- **端口转发**：本地端口映射到远端服务
- **命令备忘录**：分组管理常用命令

### 其他

- 配置加密：可选主密码（PBKDF2-SHA256 + AES-256-GCM）
- 自动更新：检查、下载、安装、重启
- 系统托盘、开机自启、本地 / WebDAV 备份
- IPv6 字面量连接支持
- 静态链接 CRT，免装 VC++ 运行库
- 深色 / 浅色主题，字体、字号、光标、背景图、强调色可调

## 快捷键

| 快捷键 | 功能 |
|---|---|
| `F11` | 终端全屏 / 退出全屏 |
| `Esc` | 退出全屏、关闭搜索、关闭命令面板 |
| `Ctrl+F` | 终端搜索 |
| `Ctrl+K` | 快速命令面板 |
| `Enter`（断开后） | 重新连接 |

## 目录结构

```
SSH/
├── QShell-WinUI3/              # Tauri 标准布局
│   ├── src/                    # Vue 3 前端（App.vue + components + styles）
│   ├── src-tauri/
│   │   ├── src/                # Rust 后端
│   │   │   ├── ssh.rs          # SSH 连接 / 认证 / ProxyJump / 本地转发
│   │   │   ├── sftp.rs         # SFTP 文件操作 / 目录同步
│   │   │   ├── ftp.rs          # FTP 客户端
│   │   │   ├── telnet*.rs      # Telnet 与服务端
│   │   │   ├── serial.rs       # 串口
│   │   │   ├── raw_rlogin_service.rs  # Raw TCP / Rlogin
│   │   │   ├── trace_route.rs  # 路由追踪
│   │   │   ├── monitor.rs      # 系统监控
│   │   │   ├── traffic.rs      # 流量审计
│   │   │   ├── session_state.rs    # 会话恢复快照
│   │   │   └── session_log.rs      # 日志录制
│   │   └── tauri.conf.json
│   ├── tests/                  # 本地测试服务器（PowerShell mock）
│   ├── package.json
│   └── vite.config.ts
├── BUGS.md                     # 缺陷记录
├── FEATURES-TODO.md            # 功能清单与进度
├── LICENSE
└── README.md
```

## 构建

```powershell
cd QShell-WinUI3
npm install
npm run build                     # 前端构建（vue-tsc + vite）
npm run tauri build -- --bundles nsis   # 完整打包（含签名与 NSIS 安装包）
```

产物：

- 可执行文件：`QShell-WinUI3/src-tauri/target/release/QShell.exe`
- 安装包：`QShell-WinUI3/src-tauri/target/release/bundle/nsis/QShell_<版本>_x64-setup.exe`
- 更新签名：同名 `.sig` 文件，配合仓库根目录的 `latest.json`

> 静态 CRT 配置位于 `QShell-WinUI3/.cargo/config.toml`（`target-feature=+crt-static`）。

## 本地测试

`tests/` 下提供两个无需第三方依赖的 PowerShell 模拟服务器：

```powershell
# Raw TCP / Rlogin 回显服务器
powershell -ExecutionPolicy Bypass -File tests/mock-server.ps1 -Mode raw
powershell -ExecutionPolicy Bypass -File tests/mock-server.ps1 -Mode rlogin

# FTP 模拟服务器（支持 LIST / RETR / STOR / DELE / MKD / RNFR+RNTO）
powershell -ExecutionPolicy Bypass -File tests/ftp-mock.ps1
```

## 系统要求

**运行**

- Windows 10 / 11 x64
- WebView2 Runtime（Windows 11 自带；Windows 10 多数已随 Edge 安装）

**构建**

- Rust（MSVC 工具链）
- Node.js 20+

## 第三方组件

| 组件 | 许可证 |
|---|---|
| Tauri | Apache-2.0 / MIT |
| Rust `ssh2` / libssh2 | MIT / BSD |
| OpenSSL | Apache-2.0 |
| `portable-pty` | MIT |
| Vue 3 / Vite | MIT |
| xterm.js（含 addon-fit / addon-search / addon-serialize） | MIT |

## 免责声明

本软件按“现状”提供，不附带任何担保。用于连接远程设备时请遵守目标设备与网络的管理规定。

## License

[MIT](LICENSE)
