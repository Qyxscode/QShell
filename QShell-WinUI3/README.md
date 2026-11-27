# QShell

QShell 是一个面向 Windows 的 SSH / SFTP / Telnet / 串口 / 本地终端一体化客户端，基于 **Tauri 2 + Rust + Vue 3 + xterm.js**。

## 源码唯一性说明

**本目录（`QShell-WinUI3`）是 QShell 唯一的宿主源码位置**，`src/` 与 `src-tauri/` 均只在此处维护。

同级目录中另有几个名称相近的内容，容易误改，特此标注：

| 路径 | 性质 | 是否参与本产品构建 |
| --- | --- | --- |
| `QShell-WinUI3/` | 当前产品源码（Tauri 2 + Rust + Vue 3） | 是 |
| `../QShell-Lite/` | 早期 C# WinForms 版本源码，独立技术栈 | 否 |
| `../QShellLite-测试/` | 上述 Lite 版本的构建产物（exe） | 否 |

历史遗留的旧副本 `../qshell-next/`（含一份过时的 `src-tauri/src/lib.rs`）已删除，不再存在第二份 Rust 宿主源码。若在其他位置发现 `src-tauri`，一律视为非权威副本，不得在其上修改。


```
QShell-WinUI3/
├── src/                  # 前端（Vue 3 + TypeScript）
├── public/               # 静态资源
├── src-tauri/            # Rust 后端
│   ├── Cargo.toml
│   ├── build.rs
│   ├── tauri.conf.json
│   ├── capabilities/
│   ├── icons/
│   └── src/
│       ├── lib.rs / main.rs
│       ├── ssh.rs / sftp.rs / traffic.rs / monitor.rs / probe.rs
│       ├── config.rs / config_crypto.rs / backup.rs / keygen.rs
│       ├── local_fs.rs / port_forward.rs
│       └── generic_*.rs / telnet_*.rs / serial_service.rs
├── package.json
├── vite.config.ts
└── index.html
```

## 功能

- **SSH2**：密码、私钥（RSA / ECDSA / ED25519）、键盘交互式认证、系统 SSH Agent
- **SFTP**：浏览、上传、下载、重命名、删除、chmod，带进度与取消
- **Telnet**：ECHO / SGA / TTYPE / NAWS 协商、终端类型选择
- **串口**：波特率、数据位、校验位、停止位、流控、DTR/RTS、换行转换
- **本地终端**：内置 PowerShell / CMD（ConPTY）
- **端口转发**：本地端口映射到远端服务
- **会话管理**：多标签、拖拽排序、标签锁定、只读模式
- **系统监控**：CPU / 内存 / 磁盘 / 网络实时面板与折线图
- **流量审计**：基于网卡计数器与 vnstat 的周期统计
- **配置加密**：可选主密码（PBKDF2-SHA256 + AES-256-GCM）
- **自动更新**：检查、下载、安装、重启
- **其他**：IPv6、命令备忘录、系统托盘、开机自启、本地/WebDAV 备份

## 构建

```powershell
npm install
npm run build
npm run tauri build -- --bundles nsis
```

产物：
- 可执行文件：`src-tauri/target/release/QShell.exe`
- 安装包：`src-tauri/target/release/bundle/nsis/QShell_<版本>_x64-setup.exe`

## 系统要求

- Windows 10 / 11 x64
- WebView2 Runtime（Windows 11 自带）
- 构建需要：Rust（MSVC 工具链）、Node.js 20+

## License

[MIT](../LICENSE)