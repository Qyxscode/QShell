# QShell

面向 Windows 10 / 11 的 SSH / SFTP / Telnet / 串口 / 本地终端一体化客户端，基于 **Tauri 2 + Rust + Vue 3 + xterm.js**。

## 功能

- **SSH2**：密码、私钥（RSA / ECDSA / ED25519）、键盘交互式认证、系统 SSH Agent
- **SFTP**：浏览、上传、下载、重命名、删除、chmod，带进度与取消
- **Telnet**：ECHO / SGA / TTYPE / NAWS 协商、终端类型选择
- **串口**：波特率、数据位、校验位、停止位、流控、DTR/RTS、换行转换
- **本地终端**：内置 PowerShell / CMD（ConPTY）
- **端口转发**：本地端口映射到远端服务
- **终端类型**：自动探测（`infocmp`）/ xterm-256color / xterm / vt220 / vt100 / linux / ansi
- **会话管理**：多标签、拖拽排序、标签锁定、只读模式
- **系统监控**：CPU / 内存 / 磁盘 / 网络实时面板与折线图
- **流量审计**：基于网卡计数器与 vnstat 的周期统计（可选）
- **配置加密**：可选主密码（PBKDF2-SHA256 + AES-256-GCM）
- **自动更新**：检查、下载、安装、重启
- **其他**：IPv6、命令备忘录、系统托盘、开机自启、本地/WebDAV 备份

## 目录结构

```
QShell/
├── QShell-WinUI3/          # 完整项目（Tauri 标准布局）
│   ├── src/                # Vue 3 前端
│   ├── src-tauri/          # Rust 后端
│   │   ├── src/            # SSH / SFTP / Telnet / 串口 / 监控 / 流量审计 / 端口转发
│   │   └── tauri.conf.json
│   ├── package.json
│   └── vite.config.ts
├── LICENSE
└── README.md
```

## 构建

```powershell
cd QShell-WinUI3
npm install
npm run build
npm run tauri build -- --bundles nsis
```

产物：
- 可执行文件：`QShell-WinUI3/src-tauri/target/release/QShell.exe`
- 安装包：`QShell-WinUI3/src-tauri/target/release/bundle/nsis/QShell_<版本>_x64-setup.exe`

## 系统要求

- Windows 10 / 11 x64
- WebView2 Runtime（Windows 11 自带）
- 构建需要：Rust（MSVC 工具链）、Node.js 20+

## 第三方组件

| 组件 | 许可证 |
|---|---|
| Tauri | Apache-2.0 / MIT |
| Rust `ssh2` / libssh2 | MIT / BSD |
| OpenSSL | Apache-2.0 |
| `portable-pty` | MIT |
| Vue 3 / Vite | MIT |
| xterm.js | MIT |

## 免责声明

本软件按“现状”提供，不附带任何担保。用于连接远程设备时请遵守目标设备与网络的管理规定。

## License

[MIT](LICENSE)