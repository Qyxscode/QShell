# QShell 桌面版 —— 网络工具箱交接文档

> 写于 2026-11-06，用于换会话继续开发。
> 上一版已发布：**v1.0.2**；本次改动**全部未发版**，最新测试版为 **`QShell_测试版276.exe`**。

---

## 一、项目铁律（必须先读）

这些是用户长期约定，违反会被明确纠正：

| 规则 | 说明 |
|---|---|
| **「记录」** | 只往 `BUGS.md` 写，不改代码、不打包 |
| **「打包」** | 只跑 tauri build 出测试版 exe，不升版本号 |
| **「发版」** | 升 4 处版本号 → 跑 `打包发版.ps1` → 更新 latest.json → commit/push → robocopy 备份到 `D:\SSH-QShell-<版本>` |
| **提交信息** | 只描述「新增了什么」，不写「修复了什么 bug」 |
| **禁止 `git add -A`** | 必须显式列路径；提交后检查 `create mode` 行（历史上有过凭据泄露事故） |
| **不要自作主张打包** | 用户没说「打包」就不要 build exe |
| **不确定就问** | 用户是夜猫子，中文交流 |

**PowerShell 5.1**，不是 pwsh 7。写文件必须用：
```powershell
[IO.File]::WriteAllText($path, $text, (New-Object Text.UTF8Encoding($false)))
```
`Set-Content -Encoding UTF8` 会写 BOM，破坏 JSON/PostCSS。

---

## 二、本次工作范围

把安卓版的「网络工具箱」搬到桌面版。落地形态：

```
系统监控卡片标题右侧  [🔧 工具]  ← 点击
        ↓
左侧浮层菜单（236px→跟随左栏宽度，覆盖左栏，不挤压终端）
        ↓
点击列表项 → 打开可拖动浮窗（可多开、记忆位置、多层级）
```

### 已完成的 5 个工具

| 工具 | 详情页 | 状态 |
|---|---|---|
| **Ping**（单/多目标 · 单次/持续 · 每目标独立开始停止） | ✅ 历史+折线图 | 完成 |
| **TCP Ping**（host:port · 单次/持续） | ✅ 共用历史面板 | 完成 |
| **端口扫描**（常用54个 / 自定义表达式 / 全部65535 · 超时档位 · 并发档位） | ✅ 全端口状态列表 | 完成 |
| **子网扫描**（CIDR / 范围 · 流式显示 · MAC · 设备类型推断） | ✅ IP/MAC/类型/延迟 | 完成 |
| **网络唤醒 WOL**（设备记录 · 探测 · 直接发魔术包） | ❌ 按要求不做 | **唤醒功能有 bug，见第五节** |

---

## 三、代码结构

### 后端 `src-tauri/src/probe.rs`（原 108 行 → 现约 1000 行）

新增命令（全部已在 `lib.rs` 注册）：

| 命令 | 作用 |
|---|---|
| `ping_once(host, timeout_ms)` | 单次 ICMP，返回 `{ok, rtt_ms, status}` |
| `tcp_ping_once(host, port, timeout_ms)` | 单次 TCP 握手测速 |
| `scan_ports(host, ports[], timeout_ms, concurrency)` | 并发扫一批端口 |
| `scan_subnet(cidr, ...)` | 整个网段（旧版，前端已不用） |
| `scan_subnet_batch(ips[], ...)` | 分批扫 IP 列表（**前端主用**） |
| `wake_on_lan(mac, broadcast, port, target_ip)` | 发魔术包 |
| `detect_wol_candidates(cidr, timeout_ms)` | 探测 WOL 候选（ARP+ping） |
| `probe_wol_ip(ip)` | 指定单 IP 探测 |

辅助函数：`parse_ping_output`、`expand_cidr`、`read_arp_table`、`guess_device_hint`、`reverse_dns`、`local_ipv4_networks`。

`PingOutcome` / `PortScanResult` / `SubnetHost` / `WolCandidate` 用 `serde::Serialize`，前端通过 `invoke<T>()` 直接拿结构化数据。

### 前端 `src/App.vue`（现约 13400 行）

关键位置：

| 内容 | 行号（约） |
|---|---|
| `ToolKind` / `TOOL_MENU` / `toolWindows` | 245~275 |
| 浮窗拖动 | 343 |
| Ping 状态与逻辑 | 393~540 |
| 历史面板（Ping/TCP 共用） | 555~660 |
| TCP Ping | 665~800 |
| 端口扫描 | 845~970 |
| 子网扫描 | 975~1100 |
| WOL | 1105~1275 |
| 模板：工具按钮 | 系统监控标题处 |
| 模板：浮层 + 浮窗 | `</aside>` 之后 |

新增组件：**`src/components/ToolSelect.vue`** —— 自定义下拉，替代原生 `<select>`（原生下拉在 Windows 上是系统灰底白字，与界面风格冲突）。

### 样式 `src/styles/app.css`

末尾追加了几段（按 `/* ===== 网络工具箱` 搜索可定位）：
- 工具按钮 / 浮层 / 浮窗 / 表格 / 进度条 / 下拉组件 / 子网统计卡 / WOL 表单

---

## 四、关键设计决策（别推翻）

1. **浮层宽度跟随 `leftPanelWidth`**（`:style="{ width: (leftPanelWidth + 10) + 'px' }"`），`left: -10px` 抵消 `.main-body` 的 padding；高度 `top: 0; bottom: 0` 与终端卡片齐平。

2. **不做真实模糊**：整个项目禁止 `backdrop-filter` / `will-change` / 常驻 `transform`，会让文字丢失 ClearType。

3. **每个 Ping 目标独立运行**：`PingRow.running` + `PingRow.timerId`，全局 `pingRunning` 是 computed（`some(r => r.running)`）。

4. **关闭浮窗自动清空**：`closeToolWindow` 里按 type 分派清理（`pinghistory` → 清该目标历史；`portscan`/`scandetail` → 清扫描结果）。

5. **流式显示**：子网扫描和端口扫描都是「前端分批调后端」，每批回来立即 append 并推进度条，这样能显示进度也能随时中止。

6. **详情面板通用化**：`HistoryRowLike` 接口 + `historyKind` 区分 Ping/TCP，新增工具只要数据行满足形状就能复用。

---

## 五、当前唯一的阻塞问题：WOL 唤不醒

### 现象

用户在程序中点「唤醒」，提示「魔术包已发往 xxx」，但目标电脑无反应。
**同样一个 MAC（`74:56:3C:42:76:F1`），用 OpenWrt 的 etherwake 可以唤醒；用我写的 PowerShell 测试脚本也能唤醒。**

### 已经排除的

- ✅ 魔术包格式正确（6×0xFF + 16×MAC = 102 字节，本地回环验证过）
- ✅ 目标 MAC 正确（从 OpenWrt 抄来的）
- ✅ 用户映射的端口 9 和 7 都发
- ✅ 记录里的 `255.255.255.255` 已改为走自动分支（v275）

### 关键线索（v276 尚未验证）

用户 275 版截图提示：

```
shiro：魔术包已发往 255.255.255.255（端口 9 和 7 各一份）
```

**只有 `255.255.255.255`，没有 `192.168.254.255`** → `local_ipv4_networks()` 返回空列表。

v276 已做两处修复（**未验证**）：
1. `ipconfig` 改用绝对路径 `%SystemRoot%\System32\ipconfig.exe`（怀疑打包后 PATH 不含 System32）
2. 枚举失败时用 `target_ip` 按 /24 推断广播地址兜底

### 下一步验证步骤

1. 装 **276**，把 shiro 设备记录补上 IP（或删掉重新探测添加，探测会带 IP）
2. 点唤醒，看提示里有没有 `192.168.254.255`
3. **如果出现了但还是唤不醒** → 说明 UDP 封装方式目标网卡不认
   - 最后手段：用 **Npcap** 发原始以太网帧（EtherType `0x0842`），与 `etherwake` 完全等价
   - 需要用户安装 Npcap，Rust 侧加 `pnet`/`pcap` crate，约 1 天工作量
   - **必须先征得用户同意**

### 环境事实（用户机器）

```
100.95.185.88/32    Tailscale 虚拟网卡
192.168.254.31/24   真实内网（目标电脑在同一网段）
192.168.56.1/24     VirtualBox Host-Only
网关 192.168.254.1（OpenWrt，可反解成 qyxs.top）
目标电脑 74:56:3C:42:76:F1，IP 用户在 192.168.254.x 段（具体值问用户）
```

---

## 六、另一个未决问题

**Ping 历史面板的自动清空可能过于激进**：关闭浮窗就丢弃数据，用户如果想"收起再看"会丢。

已按用户要求实现（他说"如果不清理会影响判断"），但没反馈是否好用。如果要调整，选项：
- 只清明细表格、保留统计摘要
- 或加"关闭时询问"

---

## 七、其他本次改动（非工具箱）

1. **流量显示单位改为按主机独立 + 5 档循环切换**（`auto` / `raw` / `MB` / `GB` / `TB`）
   - 存储：`localStorage["qshell_traffic_unit_mode_map"]`，键为 `host:port`
   - 旧全局键 `qshell_traffic_unit_auto` 自动迁移成 `__default__` 默认档
   - 按钮上显示当前档位文字（`⇄` / `原` / `MB` / `GB` / `TB`）

2. **TCP Ping 计时修正**：原来复用了 `probe_latency` 的"预读首包"逻辑，导致 HTTP/HTTPS 这类「客户端先说话」的服务被算进 600ms 超时。现在只计 TCP 握手 RTT。

---

## 八、踩过的坑（避免重犯）

### 1. PowerShell 写 TS 正则的双反斜杠

```powershell
# 错误：单引号字符串里的 \\s 会原样写入，TS 里变成"匹配反斜杠或字母 s"
'    .split(/[\\s,;]+/)'
# 正确
'    .split(/[\s,;]+/)'
```

因为这个 bug，`qyxs.top` 被按字母 `s` 拆成了两半。

### 2. 行号扫描插入函数会插进块中间

用「找到函数头 → 往下找 `}`」的方式插入代码，会匹配到 `if` 块里的 `}`，把新函数插到旧函数内部，导致模板找不到该函数。**两次把 App.vue 写坏**（重复了整个文件）。

**教训**：用 `edit` 工具的字符串匹配，不要用行号切片。写坏了用 `git checkout -- <file>` + 重做。

### 3. `send_to` 返回 Ok ≠ 包送达

绑定到具体网卡地址（`bind((ip, 0))`）时发广播，`send_to` 返回 Ok，本机回环也能收到，但真实目标收不到。必须 `bind("0.0.0.0:0")`。

### 4. `spawn_blocking` 的返回值多一层

```rust
tokio::task::spawn_blocking(|| -> Result<T, String> { ... })
    .await
    .map_err(...)   // 得到 Result<Result<T,String>, JoinError>
    .map_err(...)?; // 记得最后加 ?
```

### 5. 同名 CSS 类冲突

给原生 `<select>` 写的 `.tool-select` 样式，和新的 ToolSelect 组件容器同名 → 出现双重边框。替换组件时必须删旧样式。

---

## 九、快速上手命令

```powershell
cd C:\Users\a6704\Desktop\SSH\QShell-WinUI3

# 类型检查
npx vue-tsc --noEmit

# 前端构建
npx vite build

# 打包测试版（不升版本号）
$env:TAURI_SIGNING_PRIVATE_KEY = 'C:\Users\a6704\Desktop\QShell-更新密钥\qshell-updater.key'
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''
node .\node_modules\@tauri-apps\cli\tauri.js build --bundles nsis
# 产物：src-tauri\target\release\QShell.exe → 改名为 QShell_测试版<N>.exe
# 记得 icacls 设置 Medium 完整性级别
```

测试版编号当前到 **276**。

---

## 十、建议的下一步优先级

1. **【必做】验证 WOL v276** —— 看 `local_ipv4_networks` 的绝对路径修复是否生效
2. 若仍失败 → 决定是否引入 Npcap 发原始帧（**先问用户**）
3. 工具箱整体验收（5 个工具挨个走一遍）
4. 发版（当前 v1.0.2 → v1.1.0，因为新增了整个工具箱模块）
5. 清理 `src/App.vue.bak-recover`（本次误操作留下的备份）

---

## 十一、文件清单

| 路径 | 说明 |
|---|---|
| `QShell-WinUI3/src/App.vue` | 主逻辑（~13400 行，工具箱代码在 245~1275） |
| `QShell-WinUI3/src/components/ToolSelect.vue` | 新增：自定义下拉组件 |
| `QShell-WinUI3/src-tauri/src/probe.rs` | 新增全部网络探测命令 |
| `QShell-WinUI3/src-tauri/src/lib.rs` | 命令注册（第 26~29 行） |
| `QShell-WinUI3/src/styles/app.css` | 末尾追加工具箱样式 |
| `HANDOFF.md` | 主交接文档（项目级信息、发版流程） |
| `BUGS.md` | Bug 记录 |
| `QShell-WinUI3/src/App.vue.bak-recover` | **临时文件，可删** |
