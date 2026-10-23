# QShell

QShell 是一个面向 Windows 的 SSH / SFTP / Telnet / 串口一体化终端，基于 **Tauri 2 + Rust + Vue 3 + xterm.js** 构建。

## 功能

- **SSH2**：密码、私钥（RSA / ECDSA / ED25519）、键盘交互式认证、系统 SSH Agent
- **SFTP**：浏览、上传、下载、重命名、删除、chmod，带进度与取消
- **Telnet**：ECHO / SGA / TTYPE / NAWS 协商
- **串口**：波特率、数据位、校验位、停止位、流控、DTR/RTS、换行转换
- **本地终端**：内置 PowerShell / CMD（ConPTY）
- **终端类型**：自动探测（`infocmp`）/ xterm-256color / xterm / vt220 / vt100 / linux / ansi
- **会话管理**：多标签、拖拽排序、标签锁定、只读模式
- **系统监控**：CPU / 内存 / 磁盘 / 网络实时面板与折线图
- **流量审计**：基于网卡计数器与 vnstat 的周期统计（可选）
- **配置加密**：可选主密码（PBKDF2-SHA256 + AES-256-GCM）
- **其他**：IPv6、命令备忘录、系统托盘、开机自启、本地/WebDAV 备份

## 系统要求

- Windows 10 / 11 x64
- WebView2 Runtime（Windows 11 自带；Windows 10 可能需要安装）

## 目录结构

```
.
├── QShell-WinUI3/      # Tauri 宿主（窗口、托盘、命令注册）
└── qshell-next/        # 前端（Vue 3）与核心引擎源码（Rust）
    └── src-tauri/src/  # SSH / SFTP / Telnet / 串口 / 监控 / 流量审计
```

> 注意：`QShell-WinUI3/src/lib.rs` 通过 `#[path = "../../qshell-next/src-tauri/src/..."]` 直接引用核心引擎，
> 因此 **两个目录必须保持同级**，否则编译会报找不到文件。

## 构建

### 环境准备

- Rust（stable，MSVC 工具链）
- Node.js 20+
- WebView2 Runtime

### 1. 安装前端依赖

```powershell
cd qshell-next
npm install
```

### 2. 构建前端

```powershell
npm run build
```

### 3. 编译宿主程序

```powershell
cd ..\QShell-WinUI3
pwsh .\sync-ui.ps1        # 将 qshell-next/dist 同步到 ui_dist
cargo build --release    # 产物：target\release\QShell.exe
```

### 4. 打包安装程序（可选）

```powershell
cd QShell-WinUI3
npx --prefix ..\qshell-next tauri build --bundles nsis
```

产物位于 `QShell-WinUI3\target\release\bundle\nsis\`。

## 第三方组件

| 组件 | 许可证 |
|---|---|
| Tauri | Apache-2.0 / MIT |
| Rust `ssh2` / libssh2 | MIT / BSD |
| OpenSSL | Apache-2.0 |
| `portable-pty` | MIT |
| Vue 3 / Vite | MIT |
| xterm.js | MIT |
| lucide-vue-next | ISC |

完整许可证见各依赖仓库。

## 免责声明

本软件按“现状”提供，不附带任何担保。用于连接远程设备时请遵守目标设备与网络的管理规定。

## License

[MIT](LICENSE)
