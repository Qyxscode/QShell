# QShell (Tauri 2 Native Architecture)

## 项目架构与工程分工

- **前端源码与 UI 开发**：统一维护在同级目录 `../qshell-next` 中（Vue 3 + TypeScript + Vite + xterm.js）。
- **原生后端与宿主工程**：本项目（`QShell-WinUI3`）是基于 **Tauri 2 官方标准原生架构**构建的桌面应用宿主，负责窗口管理、系统托盘、自动开机启动、原生崩溃转储与看门狗，以及通过 Rust 原生 `#[path = "..."]` 直连挂载 `qshell-next` 的 SSH、SFTP、流量审计与系统监控核心引擎。

## 构建与同步工作流

在构建时，Tauri 会自动执行项目目录下的 `sync-ui.ps1`：
1. 自动定位执行上下文与项目绝对路径；
2. 自动进入 `../qshell-next` 执行 `npm run build`，确保包含最新的前端代码；
3. 清空本工程 `ui_dist` 目录，杜绝旧静态资源与旧哈希文件残留；
4. 将最新构建产物同步到 `ui_dist` 并完成 Tauri 2 原生二进制打包。
