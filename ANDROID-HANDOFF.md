# QShell Android 版开发交接

> 建立时间：2026-10-29
> 来源对话：QShell 桌面版维护（已完成到 v0.99.88）
> 本文档目的：新对话接手 Android 版时，先读完这份再动手，避免重复踩坑。

---

## 0. 一句话说清

QShell 是一个 Tauri 2 + Rust + Vue 3 + xterm.js 的 SSH / Telnet / 串口客户端（桌面版已发到 v0.99.88）。
现在要在**同一个仓库**里加 Android target，做成手机上的运维终端。桌面版继续独立演进，互不影响。

---

## 1. 仓库与环境

| 项 | 值 |
|---|---|
| 仓库根 | `C:\Users\a6704\Desktop\SSH`（git 仓库） |
| 工程目录 | `C:\Users\a6704\Desktop\SSH\QShell-WinUI3` |
| 后端 | `QShell-WinUI3\src-tauri`（Rust，edition 2024） |
| 前端 | `QShell-WinUI3\src`（Vue 3 SFC + Vite 8） |
| 当前版本 | `0.99.88`（4 处同步：`Cargo.toml` / `tauri.conf.json` / `package.json` / `App.vue` 的 `appVersion`） |
| 平台 | Windows（PowerShell 5.1，**没有 pwsh 7**） |

### 环境注意

- 改 `package.json` / `tauri.conf.json` / `App.vue` 后**必须确认没有 BOM**。PS 5.1 的 `Set-Content -Encoding UTF8` 会写 BOM，Vite 的 PostCSS 会直接报错。
  正确写法：
  ```powershell
  [IO.File]::WriteAllText($p, $text, (New-Object Text.UTF8Encoding($false)))
  ```
- 发版脚本 `打包发版.ps1` 反过来**必须有 BOM**，否则 PS 5.1 解析中文报错。
- 构建命令：
  ```powershell
  $env:TAURI_SIGNING_PRIVATE_KEY = 'C:\Users\a6704\Desktop\QShell-更新密钥\qshell-updater.key'
  $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''
  node .\node_modules\@tauri-apps\cli\tauri.js build --bundles nsis
  ```

---

## 2. 用户工作流约定（务必遵守）

| 用户说的话 | 你要做的事 |
|---|---|
| 「记录」 | **只**往 `BUGS.md` 追加条目，不改代码、不打包 |
| 「打包」 | 跑 tauri build，产出测试版 exe |
| 「发版」 | 升 4 处版本号 → 跑 `打包发版.ps1` → 更新 `latest.json` → commit/push → robocopy 备份到 `D:\SSH-QShell-<version>` |
| 「暂时先不打包」 | 只改代码 + `vite build` 验证，不出 exe |

**测试版约定**：`QShell_测试版<N>.exe`（从 `src-tauri\target\release\QShell.exe` 复制，然后
`icacls /setintegritylevel Medium`），编号递增，当前到 **213**。

**其它习惯**：
- 用户是夜猫子，中文交流
- 不确定就问，别猜
- commit message **只写加了什么**，不写修了什么
- 文档体系：`HANDOFF.md`（交接）、`BUGS.md`（缺陷记录）、`QShell-WinUI3\FEATURES-TODO.md`（功能规划）
- 不要为了「顺手」多打包，用户明确说过只在要求时打包

---

## 3. Android 版目标与 MVP 范围

**定位**：手机应急运维 —— 掏出手机、三秒连上、敲两条命令。

**第一版只做这些**：

| 保留 | 不做 |
|---|---|
| SSH 连接（密码 + 私钥） | 分屏、广播台 |
| 单会话终端 + 功能键条 | 串口、本地终端、端口转发 |
| 连接列表 + 分组 | 流量审计、监控图表 |
| 复制粘贴、字号调节 | SFTP（第二版再加） |

**最低系统**：Android 7.0（API 24），这是 Tauri 2 mobile 的下限。

---

## 4. 后端模块盘点（基于 `src-tauri/Cargo.toml` 实测）

| 模块 | 文件 | Android 可行性 | 处理方式 |
|---|---|---|---|
| SSH 主协议 | `ssh.rs` (19.8KB) | ⚠️ **最大卡点** | 现用 `ssh2`（libssh2 + vendored OpenSSL），交叉编译到 NDK 很痛苦 |
| SFTP | `sftp.rs` (38.5KB) | ✅ 依赖 ssh 通道 | 跟随 SSH 方案 |
| 端口转发 | `port_forward.rs` (25.4KB) | ✅ 纯 TCP | 可直接用 |
| Telnet / RLogin / Raw | `telnet_service.rs` 等 | ✅ 纯 TCP + codec | 可直接用 |
| 监控 / 流量审计 | `monitor.rs` (24.3KB) / `traffic.rs` (44.8KB) | ✅ 走 SSH exec | 逻辑可复用 |
| 备份 / 加密 | `backup.rs` / `config_crypto.rs` | ✅ 纯 Rust | 可直接用 |
| 本地终端 | `local_shell_service.rs` | ❌ | `portable-pty` 依赖 fork/pty，Android 没有 → cfg 排除 |
| 串口 | `serial_service.rs` | ❌ | `serialport` 需要 USB OTG 专用库 → cfg 排除 |
| 路由追踪 | `trace_route.rs` | ⚠️ | raw socket 需 root → 改为调 `ping` 子进程 |
| 桌面专属 | 托盘、自启、更新、剪贴板、通知 | ❌ | `cfg(target_os = "android")` 排除；Android 用插件替代 |

**已在 `cfg(windows)` 下、不影响的**：`winreg`、`webview2-com`、`windows-core`、`tauri-winrt-notification`。

---

## 5. 前端复用与移动端改造

前端 Vue 代码 **80% 可复用**，需要处理的是「触摸 + 软键盘」这一层。

### 必须做的适配

1. **功能键条**（最关键）
   手机键盘没有 Ctrl / Esc / Tab / 方向键，而终端全靠这些。需要在键盘上方加一条按钮带：
   `Ctrl` `Esc` `Tab` `↑↓←→` `Ctrl+C` `Ctrl+D` `/` `-` `|`
   - Ctrl 建议做成「粘滞 Ctrl」：点一下变亮，再点字母就发 Ctrl+字母
2. **软键盘弹出时的 resize**
   监听 `visualViewport` 或 `resize`，键盘弹起时终端要收缩，不能让内容被键盘盖住。
3. **触摸滚动与选择**
   xterm 的滚动在移动端要调 `touchAction`；文本选择要支持长按 + 原生选择手柄。
4. **IME 组合输入**
   中文输入法拼字过程中不能把拼音直接发给终端，要等 `compositionend`。
5. **响应式布局**
   竖屏窄屏：左侧会话栏改成抽屉（现在桌面版的 `isRightPanelOpen` 抽屉做法可以照抄）。

### 可以砍掉的

分屏相关（`.pane-*`）、顶栏凹陷槽、窗口控制按钮、托盘菜单、自动更新。

---

## 6. 核心技术决策（动手前必须先定）

### 决策 1：SSH 库换不换？（最重要）

| 方案 | 优点 | 缺点 |
|---|---|---|
| **保留 `ssh2`** | 代码零改动 | Android 交叉编译 OpenSSL，坑多；`vendored-openssl` 需要配 NDK 工具链 |
| **换成 `russh` + `rustls`** | 纯 Rust，`cargo build --target aarch64-linux-android` 直接过 | `ssh.rs` + `sftp.rs` + `port_forward.rs` 要重写（约 2~3 周） |
| 双实现（cfg 切换） | 桌面用 ssh2、Android 用 russh | 两套逻辑要一起维护，容易分叉 |

**倾向**：如果确定长期要做移动端，换 `russh` 更值。但先用 1~2 小时做 spike，确认交叉编译到底有多痛再定。

### 决策 2：一个仓库还是两个前端？

建议**同一个仓库**、同一份 Vue 源码，用不同 Vite 入口或 `import.meta.env` 分支做移动布局。
不要复制一份前端代码出来 —— 那会变成两套维护。

### 决策 3：终端渲染器

xterm 有 DOM / Canvas / WebGL 三种渲染器。手机低端机上：
- Canvas 通常最稳
- WebGL 可能更快但耗电
建议默认 Canvas，提供开关。

---

## 7. 已知风险与陷阱

| 风险 | 说明 | 应对 |
|---|---|---|
| **`edition = "2024"`** | 需要 Rust ≥ 1.85，别指望降版本兼容老工具链 | Android 用新工具链反而没问题，不用管 |
| **OpenSSL 交叉编译** | `vendored-openssl` + NDK 是最可能卡住的地方 | 先 spike，或直接换 russh |
| **后台被杀** | Android 会杀后台进程，SSH 连接直接断 | 前台服务（Foreground Service）+ 常驻通知；切移动网络时自动重连 |
| **国产 ROM 限制** | MIUI/EMUI 等对后台管得更严 | 引导用户加白名单；连接断开时给明确提示 |
| **存储权限** | Android 10+ 分区存储 | SFTP 下载走 SAF（`ACTION_OPEN_DOCUMENT_TREE`）或应用私有目录 |
| **WebView 版本** | 取决于系统和 Android System WebView 更新 | 检查 `navigator.userAgent` 里的 Chrome 版本，低于 90 给出提示 |
| **`ProcessPrng` 教训** | 桌面版在 Win7 上就因为这个崩溃（Rust 1.78+ 的产物要求 Win10） | 说明：跑不了就是跑不了，别浪费时间降级；Android 不受影响，但别指望把桌面版代码搬去老系统 |

---

## 8. 从桌面版继承的踩坑清单（一定要看，能省掉几十小时）

这些是桌面版已经踩过并修复的问题，Android 版**设计时就要避开**：

### 8.1 面板/抽屉的线条与动画（踩了三次才修好）

1. **不要给整个面板加 `opacity` 过渡** —— 会把内部的描边一起淡掉，动画过程中看起来「没有边」
2. **尺寸动画期间要逐帧 fit** —— 面板高度从 340 收成 0 时，终端容器在变高变矮，如果只在动画结束时 resize，中间会露出容器底色
3. **边界线不要挂在「展开态 class」上** —— class 一移除线就消失，而动画还在跑。正确做法是用元素自身的 `background-image: linear-gradient(...)` 画线，高度为 0 时自动不可见
4. 顺序：优先用 `background-image` 画线 → 需要时才用 `inset box-shadow` → **永远不要**用 `border` 画收起态的分隔线（border 即使透明也占位，会漏出几像素灰带）

### 8.2 性能

- **不要用 `content-visibility: auto` 做列表虚拟化** —— 项目列表只有几十项时，滚动条快速拖动会让元素反复进出视口，反而更卡
- 避免常驻的 `will-change` / `transform: translateZ(0)` 合成层 —— 会破坏 Windows 的 ClearType（Android 无此问题，但层数过多一样吃内存）
- 大块数据（`cat` 大文件）不要做逐行正则处理，设个 8KB 上限

### 8.3 xterm 相关

- 链接识别要**逐 cell 取文本**（`line.getCell(i)` + `getWidth()`），直接用 `translateToString()` 的字符索引当列号，遇到中文（宽字符）会整体错位
- 输出高亮不要整块处理，要**逐行**判断：SSH 数据包里常混着带颜色的提示符，整块跳过会连带跳过纯文本行
- 插入 ANSI 颜色时用 `\x1b[39m`（只重置前景色），不要用 `\x1b[0m`（会重置所有属性）

### 8.4 主题系统

- 颜色一律走 CSS 变量：`--bg-app --bg-card --bg-sub-card --bg-capsule --bg-track --border-card --border-capsule --text-main --text-sub --color-primary`
- 反白文字（彩色按钮上的白字）**必须写死白色**，换成变量会在大按钮上变成深色
- 状态语义色（延迟红黄绿、错误红）也不该跟随主题漂移

### 8.5 其它约定

- 用户明确不喜欢「把功能藏进菜单/折叠」（多一步点击就否），宁可做上下文显隐（没有会话时不显示相关按钮）
- 不要在菜单里加一大段快捷键提示（「太占地」）
- 分屏比例记忆用 `localStorage`（`qshell_pane_split`），布局记忆是 `qshell_pane_layout`

---

## 9. 实施顺序建议

```
第 0 步（1~2 小时）：spike
  ├── 加 cfg 分层：#![cfg(not(target_os = "android"))] 包住 pty / serial / 桌面插件
  ├── cargo check --target aarch64-linux-android
  └── 记录编译错误清单 → 决定 ssh2 去留

第 1 步（1 周）：工程骨架
  ├── tauri android init
  ├── NDK 配置 + 签名 keystore
  └── 空壳 App 能在真机跑起来（显示一个 WebView 页面）

第 2 步（2~3 周）：SSH 通路
  ├── 迁移或交叉编译 SSH 库
  ├── 打通「连接 → 终端显示 → 输入回传」
  └── 连接列表读取 QShell 的 config.json（格式见下）

第 3 步（2~3 周）：移动端体验
  ├── 功能键条 + 粘滞 Ctrl
  ├── 软键盘 resize / IME 组合输入
  ├── 触摸滚动、长按选择
  └── 竖屏布局（侧栏改抽屉）

第 4 步（1 周）：Android 专项
  ├── 前台服务保活
  ├── 网络切换自动重连
  └── SAF 存储（如果做 SFTP）
```

**总计 6~8 周**。

---

## 10. 配置文件格式（Android 版要能读同一份）

路径：`%APPDATA%\QShell\config.json`（桌面）；Android 版可以支持从文件导入。

结构（只列关键字段）：

```json
{
  "devices": [
    {
      "group": "VPS",
      "is_standalone_host": false,
      "hosts": [
        {
          "name": "东京-Acck",
          "protocol": "ssh",
          "host": "45.192.200.39",
          "port": 2281,
          "user": "root",
          "pwd": "xxx",
          "key_path": null,
          "passphrase": null,
          "proxy_host": null,
          "term_type": null,
          "use_agent": null,
          "details": { }
        }
      ]
    }
  ]
}
```

`protocol` 取值：`ssh` / `telnet` / `rlogin` / `raw` / `serial` / `local`。

---

## 11. 第一步验证清单（spike 阶段要回答的问题）

1. `cargo check --target aarch64-linux-android` 除了 ssh2/openssl 还报多少错？
2. `ssh2` 的 vendored OpenSSL 能否用 NDK 工具链编过？如果 2 小时内搞不定 → 直接换 `russh`
3. `tauri android init` 生成的工程能否在真机上跑起一个空 WebView？
4. xterm 在 Android WebView 里的渲染效果与输入是否正常？
5. 前端 Vue 应用体积（当前 `dist` 单文件 ~776KB gzip 215KB）在低端机上首屏要多久？

这五个问题有明确答案后，再决定是否投 6~8 周。

---

## 12. 桌面版现状（供参考，不要动）

- 版本：v0.99.88 已发布（GitHub Release）
- 测试版编号：213
- 已实现：SSH/Telnet/RLogin/Raw/串口/本地终端、分屏（1~4 屏 + 可拖动比例 + Alt+1~4）、广播台（逐台结果 + 发送确认）、SFTP/FTP、流量审计、监控、主题系统（5 套配色 × 明暗）、会话恢复、断线重连、主密码、备份（本地/WebDAV）、自动更新、托盘
- 明确不做：拖会话到空窗格、嵌套分屏、按钮精简、快捷键一览/自定义
- 遗留：xterm 历史 reflow 导致的旧内容挤压（库特性，不修）

---

## 13. 沟通约定

- 用户会直接说「记录」「打包」「发版」，按第 2 节的表处理
- 提方案时给**成本和取舍**，不要只给一个选项
- 用户不喜欢过度设计，能砍就砍
- 动手前如果涉及架构变更，先说清楚影响面再动手
- 做完了要能被验证：每轮改动都给出「你该重点看哪几点」
