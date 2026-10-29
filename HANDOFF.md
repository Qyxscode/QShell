# QShell 项目交接文档

> 给新会话/新协作者用。读这份文档即可承接当前进度，不必翻完整历史。

## 项目概况

- 路径：`C:\Users\a6704\Desktop\SSH`（仓库根），应用在 `QShell-WinUI3/`
- 技术栈：Tauri 2 + Rust + Vue 3 + xterm.js，Windows 10/11
- 当前版本：**0.99.89**（已发布）
- 前端：`src/App.vue`（约 8500 行，单体）、`src/components/`（12 个组件）、`src/styles/app.css`（约 8900 行）
- 后端：`src-tauri/src/`（ssh/sftp/ftp/monitor/traffic/port_forward/trace_route 等）

## 用户工作习惯（重要）

1. **说「记录」= 只写入 `BUGS.md`，不改代码、不打包**
2. **说「打包」才执行 `tauri build`**（用户电脑配置低，打包耗 CPU/时间，不要主动打包）
3. **说「发版」**才升版本号、生成 latest.json、提交推送、D 盘备份
4. 提交信息只写「新增了什么」，不写修复明细
5. 用户是夜猫子（白天睡觉），凌晨活跃正常
6. 遇到不确定的细节**直接问**，不要反复盲改

## 打包与发版流程

```powershell
# 打包（测试版）
$env:TAURI_SIGNING_PRIVATE_KEY='C:\Users\a6704\Desktop\QShell-更新密钥\qshell-updater.key'
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD=''
cd C:\Users\a6704\Desktop\SSH\QShell-WinUI3
node .\node_modules\@tauri-apps\cli\tauri.js build --bundles nsis
# 产物复制为 QShell_测试版N.exe，并 icacls 设置 Medium 完整性

# 发版：改 4 处版本号（Cargo.toml / tauri.conf.json / package.json / App.vue 的 appVersion 兜底）
# 生成 latest.json（含签名）、提交推送、robocopy 备份到 D:\SSH-QShell-<版本>
```

签名私钥：`C:\Users\a6704\Desktop\QShell-更新密钥\qshell-updater.key`（空密码）

## 当前功能状态

| 模块 | 状态 |
|---|---|
| SSH（密钥/Agent/跳板机/IPv6） | ✅ |
| Telnet / Rlogin / Raw TCP / 串口 / 本地终端 | ✅ |
| SFTP / FTP 文件面板 | ✅ |
| 端口转发 -L / -R / -D(SOCKS5) | ✅ |
| 终端分屏 1/2/3/4 屏 | ✅ 已收尾 |
| 配色主题（5 套 × 明暗） | ✅ |
| 快速命令面板（抽屉式推拉） | ✅ |
| 系统监控 / 流量审计 / 到期提醒 / 路由追踪 | ✅ |
| 会话恢复 / 断线重连 / 命令历史 | ✅ |
| 拖拽上传（拖到哪个窗格传到哪个会话） | ✅ |

## 分屏架构（易错点）

核心变量（`App.vue`）：

```ts
paneLayoutSetting  // 用户设定的屏数（localStorage 记忆）
paneLayout         // 当前实际布局（随会话数自动变化）
paneSessions       // { p1..p4: sessionId }
paneKeys           // computed：当前布局的合法槽位
```

关键函数：

- `reshufflePanes(n)`：切换布局 + 回收超出槽位 + 重新填充（**核心**）
- `applyAutoLayout()`：按会话数自动调整（0 → setting，否则 min(setting, count)）
- `assignPaneToSession(sid)`：新会话分配（无空槽时自动扩屏 2→3→4）
- `paneKeyOfSession(sid)`：会话在哪个槽位
- `paneBoxStyle(sid)`：返回 `gridArea` 行内样式

监听：`watch(() => sessions.value.length, applyAutoLayout)` + `closeSession` 末尾 `nextTick(applyAutoLayout)`

**布局 CSS 铁律**（踩过很多坑）：

- `.pane-grid` 必须 `position: absolute; inset: 0`（改成 relative 会塌陷）
- grid 行列用 `minmax(0, 1fr)`（`1fr` 会被内容撑开）
- 窗格 `height: 100%`（auto 会取内容高度，底部差 1~2 行）
- 分割线：`pane-grid-lines` 绝对定位线层 + `--arrow`；窗格不描边
- 修改布局相关 CSS 前，先 diff `D:\SSH-QShell-<版本>` 备份对照

## 已知未修 / 观察中

见 `BUGS.md`（约 52 条，绝大多数已修复）。

仍存在的：

- 按钮精简：**已达成**（2026-10-29 用户确认，上下文显隐后顶栏图标数量已可接受，不再做视觉分组/合并）
- xterm 历史 reflow 导致分屏后旧内容「挤压」（库特性，不处理）

BUGS.md 中原标记「待修复」的 BUG-061 / 062 / 063 / 065 已由用户确认修复，状态已更新。

## 拖拽上传实现要点（2026-10-27）

- 落点判定：`updateDropUploadTarget()` 用 `pane-slot-box` 的 `getBoundingClientRect()` 做**几何命中**，不用 `elementFromPoint`（延迟/断开浮层、空窗格遮罩会挡住命中）；拖放坐标是物理像素，需除以 `devicePixelRatio`。
- 目标目录：`sftpPathBySession`（每会话记忆上次浏览目录），没有则 `sftp_home_dir`；不要依赖全局 `currentSftpPath`。
- 上传：文件走 `sftp_upload_file` / `ftp_upload`，目录走 `sftp_sync_directory`（FTP 不支持目录）；冲突确认复用 `askTransferConflict`。
- 命中窗格但会话不支持文件传输时**静默不传**，不回退到活动会话，避免传错机器。
- 提示层：新组件 `src/components/DropUploadOverlay.vue`，作为 `.pane-grid` 的 grid 子项，用 `gridArea` 与目标窗格对齐，`pointer-events: none`。

## 已解决（2026-10-27 晚）

- 广播台（多会话群发命令，底部抽屉，v0.99.86 已发版）

- 通知气泡三角：气泡改为跟随按钮定位（--popover-left），三角实测对齐
- 告警头部按钮组：改为靠右（space-between + margin-left:auto）
- 配色主题卡片：等分网格，与下方输入框左对齐
- 视图切换动画：位移 20px + 缩放 + 模糊 0.3s → **已被轻量版替换**（见下方历史记录）
- 新应用图标（SVG 生成的 ico/png 全套）
- 终端拖拽上传：拖到哪个窗格就传到那个会话（详见下文实现要点）

## 历史问题记录

**主页面 ↔ 主机管理切换卡顿**（2026-10-27 晚，第二次修）：

- 旧实现：`.workspace-layout` 过渡 `filter: blur(5px)` + `scale(0.985)`，时长 0.3~0.34s，`mgmt-view` 同时播放入场动画。
- 卡的原因：
  1. `filter: blur()` 作用在**整个主页面**（含 xterm canvas 与大列表），过渡期间每一帧都要离屏重绘 + 模糊采样；
  2. `scale()` 会重新栅格化文字，xterm 用 WebGL 时更贵；
  3. 两段动画同时跑，合成线程争用。
- **最终方案（覆盖式抽屉，2026-10-27 第三次调整，v169）**：
  - `.workspace-layout` 切走后**不位移**，只 `pointer-events:none`；`.mgmt-view` 是覆盖层（`absolute; inset:10px; z-index:1; background:--bg-app`），由 `<Transition name="view-slide">` 驱动；
  - 方向：**原路往返** —— 打开 `-100%-20px → 0`，收起 `0 → -100%-20px`（v169 曾让收起也向下推走，用户否掉，v170 改回）；
  - 每次只有一个元素在动 —— 这是关键，见下方第 2 条坑。
- **三个必记的坑**：
  1. `overflow: hidden` 裁剪的是 **padding box**：位移只写 `-100%` 时，元素底边停在内容区顶边，`main-body` 的 10px 内边距里会露出残影（用户截图抓到过）。要么带 `-20px` 余量，要么干脆不位移。
  2. **不要让两层视图同时反向运动**（一层向上撤离、一层向下压入）—— 肉眼就是"弹开再回弹"。只让上层动。
  3. `mgmt-view` 的 `inset: 10px` 必须跟 `.main-body` 的 `padding: 10px` 一致。
- **铁律**：不要对这个区域用 blur / scale / drop-shadow 过渡。位移本身不贵，贵的是模糊与缩放引起的重绘/重栅格化。

**通知气泡三角位置 / 头部按钮布局**（2026-10-27 晚）：

- 气泡 `.alert-popover`：`position:absolute; right:15px; width:360px`
- 三角：`.transfer-monitor-popover::before/::after`，用 `--arrow-right` 定位
- 问题：铃铛按钮位置可能**落在气泡左边缘之外**，三角被钳在 20px 处，无法对准
- 彻底修法：气泡改为**跟随按钮定位**（行内 `left`，去掉固定 `right: 15px`），使按钮中心始终在气泡内

**文字发虚（浅色模式最明显）**（2026-10-28）：

- 现象：浅色主题下大量中文小字笔画发虚、像缺像素；深色下不明显。
- 真正原因（**两条叠加**，都不是"缺字体阴影"，加阴影只会更糊）：
  1. `body, html, #app { -webkit-font-smoothing: antialiased }` —— 在 Windows 上强制灰度抗锯齿，**关掉系统 ClearType**；
  2. 大面积容器常驻合成层：`.workspace-layout { transform: translateZ(0) }`、`.right-panel-card { will-change: width,flex-basis,transform,opacity }`、`.mgmt-view { will-change: transform }`、`.terminal-viewport { translateZ(0) }` —— **Chromium 在合成层内禁用 ClearType**。
- 修法与铁律：
  - `-webkit-font-smoothing: subpixel-antialiased` + `text-rendering: auto`（**不要再写 antialiased**）；
  - 不要给大面积容器常驻 `translateZ(0)` / `will-change`，动画时浏览器会自动提升图层；
  - 字体栈：西文 `Segoe UI Variable Text → Segoe UI`，中文 **显式** `Microsoft YaHei UI → Microsoft YaHei`（不写死会回退宋体/等宽，最糊）；
  - 中文文本**不要**用 `monospace` 承载（等宽无中文，回退字体极糊）；
  - 字号下限：中文 10px 仍偏糊，次要信息尽量 ≥11px，避免 `letter-spacing` 配合小字号（会把笔画拆散）；
  - `options.ts` 的「系统推荐默认」必须与 `App.vue` 的 `defaultGlobalFont` 同步，旧栈由 `LEGACY_DEFAULT_FONT` 自动迁移。
- 保留项：`.center-terminal-card` 的 `translateZ(0)`（修圆角闪烁，终端内是等宽英文，影响小）；`.history-cmd` 的 Consolas（命令文本有意等宽）。

## 代码风格约定

- 注释用中文，说明「为什么」而不是「做了什么」
- 新增功能一律新文件（组件 / composable / 数据文件），不要继续塞进 `App.vue`
- 布局类改动同步写入 `BUGS.md` 或本文件，避免新会话重复踩坑

**底部抽屉（命令面板 / 广播台）动画期间的线条与空白**（2026-10-29，踩了三次才修好）：

- 现象：面板展开/收起过程中"没有描边"，动画结束才出现；同时终端底部露出一截容器底色。
- **三个独立原因，任何一个都会造成同一观感**：
  1. **opacity 过渡**：`.quick-palette { opacity: 0; transition: ... opacity .6s }` 会把整个元素（含 inset 描边）一起淡掉。已删除 opacity 过渡，只保留 height/max-height 动画（面板本来就有 overflow:hidden 能完全隐藏）。
  2. **终端没跟上**：面板高度动画 0.8s 期间终端区变高，但 xterm 只在动画结束 fit 一次 → 底部露底色。已改为 `fitPanesDuringPanelAnimation()`：动画期间 requestAnimationFrame 逐帧 fit，结束时统一发一次 resize。
  3. **class 时序**：线画在 `.is-open` 的 box-shadow 上，收起时类先移除、动画才开始 → 线提前消失。已把上下两条线改成 **元素自身的 background-image**（`linear-gradient` + `background-size: 100% 1px`，位置 0 0 / 0 100%），不挂任何 class，高度为 0 时自动不可见。
- **铁律**：抽屉类元素的边界线不要挂在"展开态 class"上，用元素自身背景或常驻伪元素画；同时不要给整个面板加 opacity 过渡。