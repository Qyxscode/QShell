# QShell BUGS.md 状态更新稿（2026-10-23 审查结论）

> 用法：把本文件对应段落覆盖回 `../BUGS.md` 中的"状态/备注"字段。原文件约定"先记录、确认后再改代码"，因此本稿只提供文字，不改 `../BUGS.md`、不改任何源码。
> 依据：[CODE-REVIEW-2026-10-23.md](./CODE-REVIEW-2026-10-23.md)。行号基于 `App.vue` 当前 13718 行版本。

## 一、原有条目状态改写

| 条目 | 新状态 | 一句话结论 |
|---|---|---|
| BUG-001 标签拖拽排序 | 部分修复（需补丁） | 排序本身可用，但未持久化、落点会残留、失焦后拖拽状态不清 |
| BUG-002 切标签 SFTP 不刷新 | 部分修复（需补丁） | 序号校验成立；右栏折叠时切换不刷新、260ms 重试不复核序号 |
| BUG-003 偶发协议不支持 | 部分修复（需补丁） | 面板四态正确；右键菜单仍把无会话/连接中/断开统一误标 |
| BUG-004 关闭全部 SSH 残留 | 已修复 | 首分支优先"请先连接 SSH 主机"；仅拖拽态未复位 |
| BUG-005 空状态字重不均 | 待视觉确认 | 实现为 title 12px/500 + desc 11px/400，是层级化而非"字重一致" |
| BUG-006 预设色块形状 | 已修复 | 6px 圆角与强调色一致；备注"彩虹按钮"已过时 |
| BUG-007 白字切日间不可读 | 部分修复（需补丁） | 主题切换路径有对比度兜底；日间直接选白保存仍不可读 |
| BUG-008 备忘录复制到终端 | 部分修复（需补丁） | 多行命令会部分立即执行；未拦只读；失败静默 |
| BUG-009 子项多余关闭按钮 | 已修复 | 模板/事件已移除；仅余死 CSS |
| BUG-010 下载用自定义浏览器 | 已修复 | 已统一；但批量下载只下发最后一个文件（转 BUG-020） |
| BUG-011 右键复制后键盘无响应 | 部分修复（需补丁） | 正常复制已恢复焦点；粘贴的只读/断线早退不恢复 |
| BUG-012 文件浏览器闪烁 | 部分修复（需补丁） | 去 blur + 固定高度已达成；loading 仍整列表卸载 |
| BUG-013 下载同名不提示 | 部分修复（需补丁） | 冲突检测正确；失败无反馈、任务永久 running |
| BUG-014 上传同名不提示 | 部分修复（需补丁） | 前端检测到位；后端无覆盖语义、失败遗留远端临时文件 |
| BUG-015 剪贴板权限提示 | 已修复 | 宿主 `src/lib.rs:75-84` arboard 命令已实现并注册 |
| BUG-016 Ctrl+Shift+V 粘两次 | 已修复 | 单次 preventDefault 写入，无第二路径 |
| BUG-017 图片无缩略图 | 已修复 | 4 路并发 + token 隔离；缓存/像素上限待加固 |

## 二、新增条目

## BUG-018 测试目标无法编译

**状态：** 待修复

**现象：**

`cargo test` 直接失败：

```text
error[E0433]: cannot find module or crate `qshell_winui3_lib` in this scope
 --> tests\utf8_decoder_test.rs:1:5
```

**原因：** `tests/utf8_decoder_test.rs:1` 引用 `qshell_winui3_lib`，而 `Cargo.toml:11` 的 lib 名为 `qshell_lib`（`src/main.rs:5` 同样使用 `qshell_lib`）。

**修复：** 测试文件改为 `qshell_lib::generic_terminal::generic_session::Utf8ChunkDecoder`。

---

## BUG-019 终端背景明暗模式无法持久化

**状态：** 待修复

**现象：** 偏好设置里把"终端背景"从暗色切到亮色，重启后回到暗色。

**原因：**

- 前端保存 `settings.term_bg_mode`（`App.vue:3740`）与 `settings.terminal_bg_color`（`3742`）；
- 后端 `AppSettings`（`config.rs:131-149`）字段名是 `terminal_bg_mode`，且没有 `terminal_bg_color`；
- `save_hosts_config` 用强类型反序列化，未知字段被丢弃，重载时 `cfg.settings.term_bg_mode`（`3377`）永远为空。

**修复：** `terminal_bg_mode` 加 `#[serde(alias = "term_bg_mode")]`；新增 `terminal_bg_color: Option<String>`；前端读取兼容两个键名。

---

## BUG-020 批量下载多个文件只下载最后一个

**状态：** 待修复

**现象：** SFTP 多选 N 个文件点"下载选中"，只有最后一个文件出现在本地。

**原因：** `sftpTriggerDownloadSelected`（`App.vue:4494-4498`）并发调用 `sftpTriggerDownload`，而 `openLocalFileBrowser`（`4642-4643`）会 `finishLocalFileBrowser(null)` 取消上一个未结束的选择器 → 前 N-1 个命中 `4787` 的 `if (!targetSavePath) return;` 静默退出。

**修复：** 串行 `await` 下载；或把选择器改造成队列。

---

## BUG-021 上传失败后远端残留临时文件

**状态：** 待修复

**现象：** 上传大文件时若网络卡顿超过 10 秒，远端目录出现 `xxx.qshell-tmp-up-<ts>` 垃圾文件，且失败时界面无提示。

**原因：** `sftp.rs:548`（本地读失败）、`557`（write 返回 0）、`563-564`（10s stall 超时）、`568`（写错误）四处 `return Err` 都不 `unlink(temp_remote)`、不 emit `failed`、不 `clear_transfer_cancel`。

**修复：** 抽统一失败处理函数，四处替换。

---

## BUG-022 下载失败后任务永久停留在"传输中"

**状态：** 待修复

**现象：** 下载失败（远端无权限、本地无法创建文件等）后，传输面板中该任务一直显示进行中，进度 0%，且"清空记录"清不掉。

**原因：** 前端 `App.vue:4823-4825` 只 `console.error`；后端 `sftp.rs:663/699/730/733` 失败路径不 emit `failed`；`clearTransferHistory`（`1012-1015`）只清非 running。

**修复：** 后端所有失败路径 emit `status:"failed"`；前端 catch 置 failed。

---

## BUG-023 批量删除可能删除到另一台主机的同名文件

**状态：** 待修复

**现象：** 在 A 主机确认批量删除后，若在删除过程中切换到 B 主机，后续文件可能被删在 B 主机的同名路径上。

**原因：** `sftpDeleteItemsConfirmed`（`App.vue:5024-5046`）在 `for` 循环内每轮读取 `activeSessionId.value`（`5033`），而 `base` 已在循环外算好。

**修复：** 循环外取一次 `sessionId` 并在整个循环内使用。

---

## BUG-024 并发传输 taskId 毫秒碰撞

**状态：** 待修复

**现象：** 拖拽多个文件上传时，偶发进度互相覆盖，或最终文件内容交错。

**原因：** `App.vue:4574` `up-${Date.now()}`、`4803` `down-${Date.now()}`；拖拽入口 `5503-5509` 同步 `forEach` 连发，同毫秒内 taskId 相同 → 后端 `temp_remote`（`sftp.rs:490`）相同；前端 `v-for :key="t.task_id"`（`6402`）重复 key。

**修复：** taskId 改为 `时间戳 + 自增序号`（或 `crypto.randomUUID()`）。

---

## BUG-025 SSH 中文输出被 32KB 块边界截断

**状态：** 待修复

**现象：** `cat` 大文件、`journalctl`、`docker logs` 等连续中文输出时，偶尔出现方块/问号（U+FFFD）。

**原因：** `ssh.rs:185` 读线程用 `String::from_utf8_lossy(&buf[..n])` 逐块转换，一个 3 字节汉字跨在 32KB 边界上时，残字节被替换。telnet/串口路径已用 `generic_session.rs:45-87` 的 `Utf8ChunkDecoder` 正确处理，SSH 路径未复用。

**修复：** 改用 `Utf8ChunkDecoder::decode`。

---

## BUG-026 非阻塞会话上调用 write_all 可能丢数据

**状态：** 待修复

**现象：** 粘贴大段文本时偶发"写入失败"，内容缺失。

**原因：** `ssh.rs:149` `sess.set_blocking(false)`，而 `ssh.rs:242-245` 的 `ch.write_all(...)` 遇到 `WouldBlock` 直接返回错误、不重试。

**修复：** 写路径增加 WouldBlock 重试与总超时；或对写操作单独加锁并临时阻塞。

---

## BUG-027 监控回退连接把域名主机连到 127.0.0.1:22

**状态：** 待修复

**现象：** 主机填域名时，监控面板偶发显示本机指标；本机 SSH 日志出现大量失败登录。

**原因：** `monitor.rs:122-125` `format!("{}:{}", host, port).parse()` 的目标类型被推断为 `&SocketAddr`，只接受 IP 字面量；域名/IPv6 解析失败后 `unwrap_or_else(|_| "127.0.0.1:22".parse().unwrap())` 回退本机，并用远端凭据认证。

**修复：** 使用 `to_socket_addrs()`（参考 `traffic.rs:667`、`sftp.rs:40`），删除本机兜底。

---

## BUG-028 监控回退会话无超时，逐步耗尽 Tokio 线程池

**状态：** 待修复

**现象：** 应用长时间运行（约 20~30 分钟）后，SFTP 传输、流量审计、监控一起卡死。

**原因：** `monitor.rs:127-157` 的 aux 会话未 `set_timeout`；前端 `App.vue:4225` 每 3 秒 `setInterval(pollMetrics)` 且不等待上一次完成 → 挂起的 `spawn_blocking` 任务累积，池（512）耗尽。

**修复：** aux 会话 `set_timeout(8s)`；前端加 in-flight 去重。

---

## BUG-029 32 位架构上把计数归零误判为回绕，虚增约 3.4 GiB

**状态：** 待修复

**现象：** 32 位 ARM 软路由上，pppoe 重拨后流量统计突增数 GB。

**原因：** `traffic.rs:181-190` 保留了 32 位内核回绕分支，而 `/proc/net/dev` 在所有 Linux 上都是 u64；`prev`/`cur` 均小于 `u32::MAX` 时归零被算成 `(4294967295-prev)+cur+1`。单测（`1239-1244`）直接传 `is_64_bit=false`，绕过真实推断链。

**修复：** 删除该分支；`arch` 为空时返回 false。

---

## BUG-030 流量审计的连接风暴、无主机密钥校验与自定义命令注入面

**状态：** 待修复

**现象/风险：**
- 每次审计新建 1~2 条完整 SSH 连接（`traffic.rs:738`），多主机定时审计持续新建；
- 握手后无 known_hosts 校验（`traffic.rs:710`，`ssh.rs` 同样）；
- `traffic_custom_command` 原样 `exec`（`traffic.rs:946`），该字段可经导入配置/WebDAV 恢复写入（`backup.rs:107-139`）→ 恶意备份包可在所有主机执行任意命令。

**修复：** 复用 `ensure_sftp_handle` 式连接；记录并校验 host key；自定义命令增加显式确认或白名单。

---

## BUG-031 流量周期与漏计问题

**状态：** 待修复

**现象：** 月末/非 0 点结算时流量统计不准；某些网卡长期不计流量。

**原因：**
- `traffic.rs:245-264`/`607-614` vnstat 周期起点硬编码 `0,0,0`，忽略 `reset_hour/min/sec` → 每周期多算"0 点到结算点"；
- `traffic.rs:232-243` 只比较"本月结算点"，`reset_day=31` 在 2 月等短月漏结算；
- `traffic.rs:753/773` 截断输出被当完整快照，`910` 用部分 counters 覆盖 `iface_baseline` → 缺失网卡后续 delta 记 0，永久漏计且不报错；
- `traffic.rs:804-812` 快照失败清空基线，静默丢一周期。

**修复：** 周期起点使用配置时分秒；短月做月末 clamp；校验 `exit_status` 与快照完整性后再更新基线。

---

## BUG-032 监控 CPU 百分比偏低、会话缓存泄漏

**状态：** 待修复

**现象：** 虚拟化宿主机上 CPU 使用率显示偏低；长时间多会话后内存缓慢增长。

**原因：**
- `monitor.rs:232-235` `total = fields.iter().sum()` 把 guest/guest_nice 重复计入（内核文档明确它们已含在 user/nice 中）；
- `monitor.rs:17/28/484` 的 `SESSION_NET_CACHE`/`SESSION_METRIC_CACHE` 以 session_id 为键且无任何 remove，每次重连泄漏一条。

**修复：** `fields[..8].sum()`；在 `close_ssh_session`/`ssh-closed` 路径清理缓存。

---

## BUG-033 流量报警忽略自定义阈值且不换算单位

**状态：** 待修复

**现象：** 关闭了流量提醒仍收到报警；或明明快超了却不报警；百分比显示成 500%。

**原因：** `App.vue:1074-1085` 硬编码 `used/total >= 0.9`，未使用 `alert_traffic_pct`（`config.rs:46`）；`parseFloat` 直接相除，不换算 `traffic_used_unit` 与 `traffic_total_unit`。

**修复：** 读取 `alert_traffic_pct`（0 视为关闭），按单位统一换算后再计算比例。

---

## 三、其它建议（不单列为 BUG）

- `beforeBuildCommand` 的 `QShell-WinUI3\sync-ui.ps1` 依赖 cwd，建议实测后统一。
- `minWidth/minHeight`（1080×640）与 `src/lib.rs:465` 的 `set_min_size(980×600)` 不一致，需定口径。
- `QShell.exe` 编译于 18:38，`ui_dist` 更新于 20:03 —— 回归验证前需重新打包。
- `../qshell-next/src-tauri/src/lib.rs` 是过时的旧宿主副本，容易误改，建议删除或加注释说明。
