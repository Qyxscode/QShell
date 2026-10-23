# QShell BUGS.md（BUG-001 ~ BUG-017）代码审查报告

- 审查时间：2026-10-23
- 审查对象：`..\BUGS.md` 中 17 条标记"已修复（待回归验证）"的条目
- 代码基线：`../qshell-next/src/App.vue`（实际 **13718 行 / 434,948 字节**，BUGS.md 记录的 12304 行是旧版）、`../qshell-next/src-tauri/src/*.rs`（核心引擎）、本工程 `src/*.rs`（宿主）
- 方法：逐条定位实现代码 + 边界推演；**未修改任何代码**；Rust 侧用 `cargo test --offline --no-run` 做了编译验证
- 约定：遵守 BUGS.md"未收到确认前不修改代码"

> 重要：仓库里存在**两套后端**。实际编译的是 `QShell-WinUI3/src`（宿主，含托盘/自启/剪贴板/串口），核心引擎通过 `#[path]` 挂到 `../qshell-next/src-tauri/src`。`../qshell-next/src-tauri` 本身还保留一份**过时的完整宿主 lib.rs**（518 行，缺 clipboard 等命令）。审查中以实际编译路径为准。

---

## 一、总体结论

| 条目 | 判定 | 主要缺口 |
|---|---|---|
| BUG-001 标签拖拽排序 | **部分修复** | 排序不持久化；落点残留；失焦后拖拽状态不清理 |
| BUG-002 切标签 SFTP 不刷新 | **部分修复** | 右栏折叠时切换会话，展开后不刷新；260ms 空列表重试不复核序号 |
| BUG-003 偶发"协议不支持" | **部分修复** | 面板四态正确；**右键菜单仍把无会话/连接中/断开统一标成"协议不支持"** |
| BUG-004 关闭全部 SSH 后残留 | **真实修复** | 仅 `isSftpDragOver` 未复位 |
| BUG-005 空状态字重不均 | **与描述不符（待视觉验证）** | title 12px/500、desc 11px/400 是层级化，非"字重一致" |
| BUG-006 预设色块形状 | **真实修复** | 备注已过时；遗留死 CSS |
| BUG-007 白字切日间不可读 | **部分修复** | 对比度兜底只在"全局主题切换"路径；日间直选 #FFFFFF 仍不可读 |
| BUG-008 备忘录"复制到终端" | **部分修复** | 只剥尾部换行→多行命令会**部分立即执行**；未判 read_only；失败静默 |
| BUG-009 子项多余关闭按钮 | **真实修复** | 遗留死 CSS |
| BUG-010 下载用自定义浏览器 | **真实修复** | 但**批量下载只看最后一个文件**（新回归） |
| BUG-011 右键复制后键盘无响应 | **部分修复** | 粘贴的只读/断线早退分支不恢复焦点；旧 term 引用竞态 |
| BUG-012 文件浏览器闪烁 | **部分修复** | 去 blur + 固定高度已达成；loading 时列表整体卸载仍闪 |
| BUG-013 下载同名不提示 | **部分修复** | 冲突检测正确；**失败无 UI 反馈，任务永久 running 且清不掉** |
| BUG-014 上传同名不提示 | **部分修复** | 前端检测到位；**后端无覆盖语义**；失败遗留远端临时文件；缺"跳过" |
| BUG-015 剪贴板权限提示 | **真实修复** | 已核实宿主实现了 arboard 命令并注册（前端回退分支仍在） |
| BUG-016 Ctrl+Shift+V 粘两次 | **真实修复** | 逻辑正确，建议实机确认一次 |
| BUG-017 图片无缩略图 | **真实修复** | 生成 48px（显示 26px）；无尺寸/格式；缓存无上限 |

**统计：真实修复 7 条，部分修复 9 条，与描述不符 1 条。**

另外发现 **1 个编译级缺陷**：`tests/utf8_decoder_test.rs` 引用了不存在的 crate 名，`cargo test` 直接编译失败（已实证，见第四节）。

---

## 二、逐条详情

### BUG-001 标签页拖拽排序 —— 部分修复
- 已落地：`App.vue:5929-5962` `data-tab-index` + `pointerdown`；`App.vue:2718-2728` 落点识别；`App.vue:2899-2919` 结束处理；`App.vue:2952-2957` 拖拽后 350ms 抑制 click；监听成对解绑（`5422-5426` / `5559-5563`）。
- 缺陷 1（期望未落地）：`handleTabDrop`（`185-195`）只改 `sessions.value`，**没有任何持久化**，项目也没有会话恢复机制 → "保存到当前会话布局"未实现。
- 缺陷 2：`2724-2728` 指针离开标签区时不把 `dragOverTabIndex` 置空，松手可能按"最后经过的标签"重排，与视觉落点不符。
- 缺陷 3：拖拽中 Alt+Tab 丢失 `pointerup` 时，`pointerDrag` 与 `document.body.style.userSelect='none'`（`2881`）残留，`onPointerDragCancel` 只监听 `pointercancel`。
- 附带：`:draggable="false"` 下仍绑定 HTML5 `dragstart/over/drop`（`5942-5946`），半死代码。

### BUG-002 切换标签页 SFTP 不刷新 —— 部分修复
- 已落地：`sftpRequestSeq`（`911`），`fetchSftpFiles` 的 success/catch/finally 三路都校验 `requestId + sessionId + alive`（`4404-4429`），旧响应会丢弃；watch 覆盖"切换会话/连接中→在线/重连"（`4434-4444`）。
- 缺陷 1：watch 内 `if (!isRightPanelOpen || rightPanelTab !== 'sftp') return;`（`4439`）→ 右栏折叠时切换标签不刷新，展开后落到"目录为空或无权限"，需手点"跳转"。
- 缺陷 2：`4353-4359` 空列表 260ms 重试**不复核 `requestId`**，只查 session/alive。260ms 窗口内用户手动切路径，会被旧重试覆盖回 home。

### BUG-003 新开终端偶发"协议不支持" —— 部分修复
- 已落地：面板四态 `App.vue:6081-6102` 顺序正确（无会话 → 非 SSH → 连接中/已断开 → loading → error → 空目录）；新建会话先 `alive:false` 再切 active（`3765-3774`），窗口期命中"连接中"。
- 缺陷（与备注不符）：**右键菜单 `App.vue:6526-6534` 仍以 `!canUseSftp` 为条件显示"当前协议不支持 SFTP"**，无会话/连接中/已断开全部被误标；同处"刷新目录"（`sftpRefresh` `4977-4980`）静默 return 无提示。
- 附带：`activeSessionIsSsh`（`4307-4311`）是"非 telnet/serial 即 SSH"，导入数据里 protocol 异常会被当 SSH。

### BUG-004 关闭全部 SSH 后残留错误 —— 真实修复
- `closeSession`（`4292-4296`）→ `activeSessionId=""` + `clearSftpState()`（`4319-4328` 清 files/selection/path/error/loading）；模板 `6081` 首分支优先"请先连接 SSH 主机"。
- 小瑕疵：`clearSftpState` 未复位 `isSftpDragOver`（断线分支 `3911` 是单独清的，说明需要）。

### BUG-005 空状态文字粗细 —— 与描述不符（需视觉验证）
- `App.vue:10184-10199`：`.sftp-empty-title` 12px/500，`.sftp-empty-desc` 11px/400。是**层级化**而非"字重一致"；BUGS.md 备注写"统一字重与字号层级"与代码不完全吻合。
- 覆盖检查：两份规则各只出现一次，白名单（`13360-13394`）不含它们，不存在被覆盖的可能。
- 另：错误态（`6098`）与"目录为空"（`6101`）的裸文本未用新类，层级不统一。

### BUG-006 预设色块形状 —— 真实修复（备注过时）
- `App.vue:12829-12837` `.color-swatch-btn { 28px; border-radius:6px }`，与 `components/ColorPicker.vue` 的 `.cp-trigger` 一致。
- 备注所说"自定义彩虹按钮"已不存在（模板 `7647` 用 ColorPicker 取代），相关 `.rainbow-custom-color-btn`（`12850-12875`）、`.accent-swatch`（`12793-12810`）为死 CSS。

### BUG-007 白字切日间不可读 —— 部分修复
- 已落地：`contrastRatio`/`relativeLuminance`（`302-318`）实现正确；`executeThemeChange`（`320-351`）在 `contrastRatio(prevFg, targetBg) < 4.5` 时回退预设色。主场景（暗色选白 → 切日间）确实修好。
- 缺陷 1（关键遗漏）：对比度兜底**只存在于全局主题切换路径**。`applyTerminalSettings`（`3676-3755`）保存设置、`openSession`（`3781-3783`）新建终端都不校验 → **在日间模式直接用取色盘选 #FFFFFF 并保存，仍是白底白字**；该状态切回暗色对比度足够，不会被纠正，不可读会长期保留。
- 缺陷 2：背景图分支用亮度阈值 `effective < 0.45`（`343-345`、`3668-3672`），不是 4.5 对比度，与备注"WCAG 检测"不符；`bg_image_auto_fg` 关闭时走 else-if，会用纯色 `targetBg` 误判并覆盖用户自定义前景。
- 缺陷 3：`hexToRgb`（`292-300`）只接受 `#RRGGBB`，3 位值会被算成亮度 0。

### BUG-008 备忘录"复制到终端" —— 部分修复
- 已落地：菜单项存在（`6455-6460`），实时按 `activeSessionId` 定位会话，后端 `write_ssh_session` 原样写字节不补换行，telnet/serial 置灰 + 函数内 early return。
- 缺陷 1（最严重）：`cmd.replace(/[\r\n]+$/g, "")`（`5075`）**只剥尾部换行**。命令中段/首段的 `\r`/`\n` 原样写入 PTY → shell 收到即执行。例：备忘录存 `cd /opt\n./deploy.sh`，点"复制到终端"后 `cd /opt` 立刻执行，违反"只填充不执行"。
- 缺陷 2：未检查 `cur.read_only`（对比粘贴路径 `5179`、`term.onData` `3880`）→ 只读模式防误操作被绕过。
- 缺陷 3：置灰只是视觉（`.menu-item.disabled` `11646-11655` 无 `pointer-events:none`），点击静默无提示；`invoke` 未 `catch`、未判 `isTauri`。
- 缺陷 4：不做输入行清理，连续两次复制会拼接在已有输入之后；远端前台是 vim/less 时字符会被前台程序消费。

### BUG-009 备忘录子项多余关闭按钮 —— 真实修复
- 子项模板 `6174-6196` 仅剩 pillar / doc / name，无按钮、无删除事件；右键删除路径可用（`6465` → `deleteMemo` `5128-5132`）。
- 遗留：`.memo-del-icon`（`10453-10470`）、`.memo-actions`（`10545-10549`）、`.btn-memo-act`（`10551-10563`）死 CSS。

### BUG-010 下载用应用内文件浏览器 —— 真实修复（含新回归）
- `4779-4786` 改为 `openLocalFileBrowser({action:"save"})`；私钥/备份/背景图同样走应用内浏览器；无系统对话框残留。
- **新回归（严重）**：`sftpTriggerDownloadSelected`（`4494-4498`）`for … void sftpTriggerDownload(file)` 并发发起，而 `openLocalFileBrowser` 开头（`4642-4643`）会 `finishLocalFileBrowser(null)` 取消上一个未结束的选择器 → 多选 N 个文件时前 N-1 个静默退出（`4787` `if (!targetSavePath) return;`），**只有最后一个真正下载**。

### BUG-011 右键复制后键盘无响应 —— 部分修复
- 已落地：`copySelection`（`5200-5207`）`writeClipboardText(sel).finally(() => cur?.termInstance?.focus())`；粘贴路径 finally 也恢复焦点。
- 缺陷 1：粘贴早退分支 `5179-5182`（`!cur || !cur.alive || cur.read_only`）只 `closeContextMenu()` **不恢复焦点**。
- 缺陷 2：`cur` 在 await 前取定，剪贴板等待期间关闭标签会 `term.dispose()`，finally 的 `focus()` 可能抛异常，`pasteClipboard` 未 catch。
- 缺陷 3：`5204` 的 `void writeClipboardText(sel).finally(...)` 无 catch。

### BUG-012 文件浏览器按钮闪烁 —— 部分修复
- 已达成：`.local-picker-overlay` 的 `backdrop-filter:none !important`（`11682-11688`，特异性 (0,2,0) 覆盖基类 blur）；`.local-picker-modal` 固定高度 `min(620px,78vh)`（`13396-13402`）+ body/list `flex:1;min-height:0`，loading/空目录不塌缩。
- 未解决：`loadLocalDir`（`4701-4721`）每次先 `localPickerLoading = true`，模板 `6616` 把整个列表卸载成"读取目录中..."再重建 → 网络盘/大目录仍可见明显闪动。
- 附带：`showLocalThisPc/showLocalNetwork`（`4728-4742`）不递增 `localPickerDirToken`，旧目录缩略图批次继续跑；`11690-11694` 移除了按钮焦点框（无障碍退化）。

### BUG-013 下载同名不提示 —— 部分修复
- 已落地：`4789-4799` 在传输前检测并提供 覆盖/重命名/取消；`availableLocalPath`（`4543-4553`）逐候选 stat，无死循环；后端临时文件 `.qshell-part` + 成功才改名（`sftp.rs:657/699/760-775`），取消时删除。
- 缺陷 1（严重）：下载失败**完全无 UI 反馈**：`4823-4825` 只 `console.error`；后端 stat 失败（`sftp.rs:663`）、`File::create`（`699`）、read（`730`）、`write_all`（`733`）都不 emit `failed` → 任务永久 `running`，而 `clearTransferHistory`（`1012-1015`）只清非 running → **僵尸任务永远删不掉**。
- 缺陷 2：检测 fail-open——`localPathExists` 出错返回 false（`4518-4523`），会直接覆盖。
- 缺陷 3：`joinLocalPath`（`4631-4635`）不校验 `..`、非法字符与 Windows 保留名 → 保存框输入 `..\..\Windows\...` 可写出所选目录。
- 缺陷 4：失败路径不删 `.qshell-part`；`sftp.rs:760` 先 `remove_file(local_path)` 再 rename，rename 失败会丢原文件。

### BUG-014 上传同名不提示 —— 部分修复
- 已落地：`4561-4572` 传输前检测并提供 覆盖/重命名/取消；后端 `sftp_upload_file(remote_path)` 支持自定义目标名，前端传改写后的路径。
- 缺陷 1：`availableRemoteName`（`4532-4541`）只查内存 `sftpFiles`，**不做远端 stat 二次确认** → 列表过期/未加载时返回原名，后端 rename 静默覆盖（"重命名"退化为覆盖）。
- 缺陷 2（与备注要求不符）：后端**没有覆盖/不覆盖语义**——`sftp.rs:490` 建 `remote_path.qshell-tmp-taskId`，`519-523` create，`596-614` 一律 rename；无 exists/`CREATE_NEW` 参数，检测与 rename 之间有 TOCTOU。
- 缺陷 3：失败路径遗留远端临时文件——`sftp.rs:548`（本地读失败）、`557`（write 返回 0）、`563-564`（10s stall 超时）、`568`（写错误）四处 `return Err` 都不 `unlink(temp_remote)`、不 emit failed、不 `clear_transfer_cancel`。大文件服务端卡顿 >10s 就会在远端目录堆积垃圾。
- 缺陷 4：只有 覆盖/重命名/取消，缺 BUGS.md 期望的"跳过"。

### BUG-015 剪贴板权限提示 —— 真实修复（已纠正子代理误判）
- 前端 `readClipboardText`（`5153-5162`）/`writeClipboardText`（`5164-5175`）优先走原生 `invoke`。
- **宿主确实实现了**：`QShell-WinUI3/src/lib.rs:75-84` 用 `arboard` 实现 `read_clipboard_text`/`write_clipboard_text`，并在 `515-516` 注册；`Cargo.toml` 含 `arboard`。
- 残留：原生失败时仍回退 `navigator.clipboard.readText()`（`5161`），理论上仍可能触发 WebView2 权限；正常路径已不触发。

### BUG-016 Ctrl+Shift+V 粘两次 —— 真实修复
- `App.vue:3836-3872` `attachCustomKeyEventHandler` 粘贴分支同时 `preventDefault + stopPropagation` 并 `return false`，只调用一次自定义写入；全文件无第二个 paste/keydown 监听。建议实机确认一次。

### BUG-017 图片缩略图 —— 真实修复
- 已落地：后缀白名单（`4673-4675`）、`for (offset += 4)` 的 4 路并发（`4677-4699`）、每批与写入前校验 `localPickerDirToken`（旧目录丢弃、不串图）、损坏图片回退普通图标；后端 `local_fs.rs:35-55` spawn_blocking + `image::thumbnail`。
- 差距：生成 48px 但 CSS 显示 26px（`13546-13553`，与备注"48px 缩略图"口径不同）；未显示图片尺寸/格式（列表只有 `formatMetricBytes(entry.size)`，且 `844-849` 对 <1MB 四舍五入成 "0 MB"）；缓存无上限无淘汰；后端不限像素尺寸（大图解压炸弹风险）。

---

## 三、额外发现（BUGS.md 未记录）

### 安全 / 数据完整性（建议优先）

1. **[严重·已编译验证] 测试不可编译**：`tests/utf8_decoder_test.rs:1` 引用 `qshell_winui3_lib`，但 `Cargo.toml:11` 的 lib 名是 `qshell_lib` → `cargo test` 直接 `error[E0433]`（本次已实测）。
2. **[严重] taskId 用 `Date.now()` 毫秒碰撞**：`App.vue:4574`（`up-${Date.now()}`）、`4803`（`down-${Date.now()}`）。拖拽多文件时 `5503-5509` 同步连发，同毫秒会产生相同 taskId → 后端 `temp_remote` 相同 → 两个上传写同一临时文件，rename 后内容交错；前端 `v-for :key="t.task_id"`（`6402`）重复 key。下载侧共用 `local_path + ".qshell-part"` 同理。
3. **[严重] 批量删除循环内重取 activeSessionId**：`sftpDeleteItemsConfirmed`（`5024-5046`，`5033` 每轮 `await` 后重取）→ 用户中途切标签，后续删除会打到另一台主机的同名路径。同类模式还有 `4589/4818/4837`。
4. **[中] 路径穿越**：`joinLocalPath` 允许 `..`（见 BUG-013 缺陷 3）；`local_fs.rs:5-68` 所有命令无根目录白名单，任意路径可读；`tauri.conf.json:31-33` `"csp": null`。当前 `App.vue` 无 `v-html/innerHTML`（0 命中），利用面主要来自应用自身。
5. **[中] 设置字段名不匹配导致静默丢失**：前端保存 `settings.term_bg_mode`（`3740`）、`settings.terminal_bg_color`（`3742`），而 Rust `AppSettings` 只有 `terminal_bg_mode`（`config.rs:140`）、没有 `terminal_bg_color`。`save_hosts_config` 用强类型反序列化，**未知字段被丢弃**，重载后 `cfg.settings.term_bg_mode`（`3377`）永远为空 → 用户选择的终端暗/亮背景模式**无法持久化**。
6. **[中] 旧配置解析失败**：`config.rs:151-158` 的 `AppConfig` 中 `devices`/`managed_hosts` 无 `#[serde(default)]`，只有 `devices` 的旧版配置会 `解析 JSON 格式错误`，而非容错。
7. **[中] 冲突弹窗无互斥**：`askTransferConflict`（`1353-1364`）直接覆盖 `transferConflictModal`，并发调用时前一个 Promise 永久 pending。

### 后端 / 工程

8. **[中] 每次传输都新建 SSH 连接**：`sftp.rs:503/658` 直接 `open_sftp_handle`，绕过 `ensure_sftp_handle`（`100-111`）的句柄复用。批量上传 = 每条 SSH 连接 + 一个 blocking 线程，易触发服务端 MaxStartups。
9. **[中] local_fs 同步命令阻塞 IPC**：`local_path_exists`/`list_local_dir`/`read_local_image_data_url`（`local_fs.rs:5-103`）是同步命令，浏览网络盘会卡住 WebView IPC 线程；后者还同步读最多 25MB。
10. **[中] 错误文案混乱**：`sftp.rs:58` `"操作失败SFTP SSH 操作失败"`、`69`、`90`、`93`；`list_sftp_files` 会话缺失返回 `"SSH 操作失败"`（`237`），其余命令返回 `"SSH 会话未连接"`（`215/425/476/646`），前端原样拼进 UI。
11. **[中] 构建脚本路径依赖 cwd**：`tauri.conf.json:10` `beforeBuildCommand` 写 `-File QShell-WinUI3\sync-ui.ps1`，只有 cwd 是 `Desktop\SSH` 时成立；在 `QShell-WinUI3` 目录执行 `cargo tauri build` 会找不到脚本（`beforeDevCommand` 有兜底，`beforeBuildCommand` 没有）。
12. **[中] 最小窗口尺寸不一致**：`tauri.conf.json:20-21` `minWidth:1080/minHeight:640`，但 `src/lib.rs:465` `set_min_size(980×600)`，运行时以 980×600 为准。
13. **[中] 传输失败无状态通知（下载侧）**：见 BUG-013 缺陷 1；上游 upload 侧前端 catch 有置 failed，但后端不 emit。
14. **[轻微] 会话覆盖不 stop 旧实例**：`generic_manager.rs:25-30` `register_session` 直接 `map.insert`，同 sid 重连时旧读线程的 socket 不主动 shutdown。
15. **[轻微] watchdog 无退避**：`src/lib.rs:367-418` 崩溃便重启，无次数/退避上限，且每次正常启动常驻一个子进程。
16. **[轻微] probe 细节**：`probe.rs:16` 的 `format!("{}:{}")` 对 IPv6 字面量不适用；`probe.rs:66` 注释说 GBK 容错但实际用 `from_utf8_lossy`（靠 ASCII 数字兜底）。
17. **[信息] 两套宿主源码并存**：`../qshell-next/src-tauri/src/lib.rs`（518 行，旧）与实际宿主 `src/lib.rs`（556 行，含 clipboard/自启/托盘）不一致，`generic_*` 副本相同。容易误改/误判（本次子代理就踩了）。
18. **[信息] 产物版本落后**：`ui_dist` 已更新到 20:03:57，但 `QShell.exe` 仍是 18:38 编译 —— **当前 exe 不含最新前端**，回归验证前必须重新打包。
18a. **[轻微] 单测会真的弹系统通知**：`src/lib.rs:550-555` 的 `test_notify_toast` 在 `cargo test` 时会真实调用 `notify_tray`（本次实测确实弹出），在 CI 或无人值守环境会失败；建议加 `#[ignore]`，或只断言 `get_embedded_icon_path` 的产物存在。

### 前端体验 / 维护

19. 死代码：标签指示条 `tabListRef/updateTabIndicator`（`559-594`，模板没绑 ref）、旧 HTML5 标签拖拽（`5942-5946`）、`.accent-swatch`/`.rainbow-custom-color-btn`/`.memo-*` 死 CSS。
20. 监听泄漏：`App.vue:5457-5481` resize 用匿名函数，`onUnmounted` 无法移除。
21. `copyText`（`2632-2637`）只走 `navigator.clipboard.writeText` 且无 catch，与 `writeClipboardText` 不一致。
22. `sendMemoCommand`（`5060-5067`）对 telnet/serial 也调 `write_ssh_session` → 后端报错且无提示；未判 read_only；非 alive 时静默复制且无反馈。
23. SFTP 成功后路径与列表的写入顺序：`4416` 成功才写 `currentSftpPath`，失败不清旧列表 → 错误文案与旧列表并存。
24. `4848` `new Date(mtime_str * 1000)`，后端 `mtime<=0` 返回 `"-"`（`sftp.rs:445-449`）→ Invalid Date。
25. `3778-3779` `openSession` 取不到 mountEl 直接 return，`is_connecting` 永久 true（面板永久"连接中"）。
26. `formatMetricBytes`（`844-849`）<1MB 显示 "0 MB"；进度对 `total=0` 用 `||1` 兜底，空文件恒 100%。

---

## 四、实证记录

```
$ cargo test --offline --no-run
error[E0433]: cannot find module or crate `qshell_winui3_lib` in this scope
 --> tests\utf8_decoder_test.rs:1:5
1 | use qshell_winui3_lib::generic_terminal::generic_session::Utf8ChunkDecoder;
  |     ^^^^^^^^^^^^^^^^^ use of unresolved module or unlinked crate `qshell_winui3_lib`
error: could not compile `qshell` (test "utf8_decoder_test")
```

```
$ cargo test --offline --lib
running 31 tests
test generic_terminal::generic_control::tests::*        ... ok (4)
test generic_terminal::generic_session::utf8_tests::*   ... ok (5)
test generic_terminal::telnet_codec::tests::*           ... ok (7)
test sftp::tests::formats_linux_permissions             ... ok
test traffic::tests::*                                  ... ok (10)
test backup::tests::restore_keeps_current_webdav_credentials ... ok
test backup::tests::empty_local_password_keeps_cloud_password ... ok
test tests::test_embedded_icon_path                     ... ok
test tests::test_notify_toast                           ... ok
test result: ok. 31 passed; 0 failed

$ cargo test --offline --test telnet_codec_test
test result: ok. 10 passed; 0 failed
```

**结论：除 `utf8_decoder_test.rs` 外，Rust 侧 31 + 10 个测试全部通过。** 也就是说测试基础设施是健康的，只是那一个集成测试的 crate 名过期（`qshell_winui3_lib` 是包目录名，不是 lib 名）。修好它就是 1 行。

补充实证：`Cargo.toml` lib 名 `qshell_lib`（`src/main.rs:5` 同用此名）；`ui_dist/index.html` 20:03:57 vs `QShell.exe` 18:38；`src/lib.rs:75-84` 存在 arboard 剪贴板命令并注册于 `515-516`；前端 `package.json:8` 的 `build` 脚本是 `vue-tsc --noEmit && vite build`，而 `ui_dist` 是 20:03 由该脚本产出的，说明**前端类型检查当前是通的**（无需另行跑 vue-tsc）。

---

## 五、建议修复批次（按"收益/风险"排序，待你确认后再动代码）

- **批次 1（必修，编译/数据）**：测试 crate 名；`AppSettings` 补 `term_bg_mode` 别名与 `terminal_bg_color` 字段；`AppConfig` 补 serde default；修正 `beforeBuildCommand` 路径；统一 min_size。
- **批次 2（传输可靠性）**：taskId 改唯一 ID；上传失败路径清理远端临时文件 + emit failed + 清 cancel 标记；下载失败置 failed 并允许清理；批量下载串行化（复用选择器）；`availableRemoteName` 加远端 stat；后端上传加覆盖语义。
- **批次 3（终端交互）**：BUG-008 多行命令清洗 + read_only 拦截；BUG-011 早退分支恢复焦点；BUG-007 抽公共对比度兜底并覆盖"保存/新建终端"路径；BUG-003 右键菜单状态修正。
- **批次 4（SFTP 生命周期）**：折叠展开补刷、260ms 重试复核序号、批量删除固定 sessionId、错误态与列表一致性。
- **批次 5（体验/清理）**：BUG-005 字重确认、BUG-012 loading 不卸载列表、缩略图缓存上限与像素上限、死代码/死 CSS、无障碍焦点。

---

## 六、批次 1 的落地补丁（待确认后才动手）

以下四组改动互相独立、风险低，建议作为第一批一次做完。

### 1) 修好测试编译（1 行）
- `tests/utf8_decoder_test.rs:1`：`qshell_winui3_lib` → `qshell_lib`
- 依据：`Cargo.toml:11` `name = "qshell_lib"`；`src/main.rs:5` 也用的 `qshell_lib::run()`，改测试是唯一正确方向（改 Cargo.toml 反而要连带改 main.rs）。
- 验收：`cargo test --offline` 通过（含 `generic_control`/`telnet_codec`/`backup` 的既有单测）。

### 2) 修好设置持久化（`../qshell-next/src-tauri/src/config.rs` + `App.vue`）
- Rust：`AppSettings.terminal_bg_mode` 加 `#[serde(alias = "term_bg_mode")]`；新增字段 `terminal_bg_color: Option<String>`（与前端 `3742` 对齐）。
- 前端：读取处 `3377` 改为 `cfg.settings.terminal_bg_mode || cfg.settings.term_bg_mode`（兼容旧盘上可能已存在的 `terminal_bg_mode`）；`3379` 保持不变。
- 可选加固：给 `AppSettings` 加 `#[serde(flatten)] extra: serde_json::Map<String, Value>`，以后前端新增字段不会静默丢失（代价是序列化会多带一层未知字段，需确认无副作用）。
- 验收：改终端背景暗/亮 → 重启 → 设置仍在；`config.json` 里能看到 `terminal_bg_mode`/`terminal_bg_color`。

### 3) 旧配置容错（`config.rs:151-158`）
- `AppConfig` 的 `devices` / `managed_hosts` / `memos` / `settings` / `version` 全部加 `#[serde(default)]`，`devices`/`managed_hosts` 用 `Vec::new()` 默认。
- 验收：手工删掉 `managed_hosts` 字段后启动，不再报"解析 JSON 格式错误"，而是空列表 + 正常启动（随后保存会补全字段）。

### 4) 配置一致性（`tauri.conf.json` + `src/lib.rs`）
- 构建命令：`tauri.conf.json:10` 的 `-File QShell-WinUI3\sync-ui.ps1` 与 cwd 强耦合。建议先实测一次 `cargo tauri build`（在 `QShell-WinUI3` 目录执行）确认失败形态，再决定改成 `-File .\sync-ui.ps1` 还是保留原样并补 `cd`。脚本本身用 `$MyInvocation.MyCommand.Path` 定位，改路径不会影响它找 `../qshell-next`。
- 最小尺寸：`tauri.conf.json:20-21`（1080×640）与 `src/lib.rs:465`（`set_min_size(980×600)`）二选一。需要你定口径：**以配置文件为准**（删掉 `set_min_size`，或改成与 conf 一致），还是以运行时代码为准（改 conf）。

---

## 七、批次 2~5 的落地补丁（函数级，待确认后按批执行）

### 批次 2：传输可靠性

| 改动点 | 位置 | 具体做法 |
|---|---|---|
| taskId 唯一化 | `App.vue:4574`、`4803`（以及所有 `taskId =` 生成处） | 抽 `makeTaskId(prefix)`：`${prefix}-${Date.now()}-${++taskSeq}`（`taskSeq` 模块级自增）。必须同时保证同毫秒多文件不同 ID，否则后端 `temp_remote` 相同 |
| 上传失败统一清理 | `sftp.rs:548/557/563-568` 四个 `return Err` | 抽 `fn fail_upload(app, task_id, fname, transferred, total, temp_remote, handle, msg)`：`unlink(temp_remote)` + emit `status:"failed"` + `clear_transfer_cancel` + 返回 Err。四处替换为调用它 |
| 下载失败 emit | `sftp.rs:663/699/730/733` | 同样统一走 `fail_download`，并删除 `.qshell-part` |
| 下载失败前端反馈 | `App.vue:4823-4825` | `catch` 中把对应 `activeTransfers` 项置 `status:"failed"` + `error`（复用上传的 4595-4605 写法） |
| 批量下载串行 | `App.vue:4494-4498` | 改为 `for (const f of files) { const p = await sftpTriggerDownload(f); if (!p) break; }`；`sftpTriggerDownload` 返回 `Promise<boolean>`（成功/用户取消） |
| 重命名二次校验 | `App.vue:4532-4541` | 改 `async function availableRemoteName(name)`：候选名逐个 `await remotePathExists(base + candidate)`，最多 20 次后回落时间戳 |
| 后端覆盖语义 | `sftp.rs:466-473` | 命令加 `overwrite: Option<bool>`；`false`/`None` 时先 `stat(remote_path)`，存在则返回 `目标已存在` 错误（前端可在 TOCTOU 时给明确提示）。前端 4561 的选择结果显式传入 |
| 缺"跳过" | `App.vue:7382-7403`（冲突弹窗按钮） | 增加第 4 个按钮，冲突返回 `"skip"`；批量上传/下载遇 skip 直接下一个 |

### 批次 3：终端交互

| 改动点 | 位置 | 具体做法 |
|---|---|---|
| BUG-008 多行命令 | `App.vue:5070-5079` | **推荐方案 A（安全）**：检测到 `\r`/`\n` 时只填充首行，并用提示告知"命令含 N 行，已仅填充首行（不会执行）"。**方案 B（体验更好但有前提）**：用 bracketed paste 包裹 —— `"\x1b[200~" + cmd + "\x1b[201~"`。注意这不是客户端的自由开关：**是否生效取决于远端 shell 是否已开启 `DECSET 2004`**，未开启时 `\x1b[200~` 会作为普通输入显示成乱码。因此 B 方案必须先在前端记录远端是否发送过 `\x1b[?2004h`（在 `ssh-data` 写入 `term.write()` 的路径上顺带扫描），只有检测到已开启才用，否则回退方案 A。**可行性已核对**：`App.vue:3894-3895` 是唯一的 `ssh-data-*` 消费点（`term.write(event.payload)`），在此处扫描 `\x1b[?2004h`/`\x1b[?2004l` 即可维护该标志，无需改动后端。同时把 `replace(/[\r\n]+$/g,"")` 换成 `replace(/\r\n/g,"\n")`（规范化而非吞内容） |
| BUG-008 只读拦截 | `App.vue:5071-5072` | 增加 `if (cur.read_only) return;`，菜单 disabled 条件同步加 `activeSession?.read_only` |
| BUG-008 失败提示 | `App.vue:5076` | `invoke(...).catch(e => alert/提示("写入终端失败: " + e))`，并对 `!isTauri` 走模拟分支 |
| BUG-011 早退恢复焦点 | `App.vue:5179-5182` | 早退前执行 `cur?.termInstance?.focus()`（`cur` 可能为空，用可选链） |
| BUG-011 旧 term 竞态 | `App.vue:5196`、`5204` | `finally` 内先校验 `sessions.value.some(s => s.id === cur?.id && s.alive)` 再 focus；给 `pasteClipboard`/`copySelection` 加 `.catch(() => {})` |
| BUG-007 统一兜底 | `App.vue:3676-3755`（`applyTerminalSettings`）、`3781-3783`（`openSession`） | 抽 `ensureReadableFg(fg, bg, preset)`：`contrastRatio(fg,bg) < 4.5` 时返回 `preset`。保存与新建终端都过一遍；背景图模式用"图片亮度×遮罩"换算成等效背景色后再算对比度，而不是固定 0.45 |
| BUG-003 右键菜单 | `App.vue:6526-6534` | 抽 `sftpPanelState` computed（`no-session` / `non-ssh` / `connecting` / `disconnected` / `ready`），菜单按状态给出与面板一致的文案；"刷新目录"在非 ready 时 disabled |
| BUG-001 持久化 | `App.vue:185-195` | 若确认"会话布局"需跨重启：在 config 增 `settings.tab_order`（存 host 并列 id 或名称序列），`handleTabDrop` 后 `save_hosts_config`；重启恢复时按序排列。若只需运行时顺序，则在 BUGS.md 注明口径 |
| BUG-001 落点残留 | `App.vue:2723-2728` | `else { dragOverTabIndex.value = null; }` |
| BUG-001 失焦残留 | `App.vue:5427`（window blur） | blur 时调用一次 `onPointerDragCancel()`（它已做状态清理） |

### 批次 4：SFTP 生命周期

| 改动点 | 位置 | 具体做法 |
|---|---|---|
| 折叠展开补刷 | `App.vue:4434-4444` watch | 依赖数组加 `isRightPanelOpen` 与 `rightPanelTab`；或保留原 watch，另加 `watch([isRightPanelOpen, rightPanelTab], …)` 在变为 `true/'sftp'` 时 `void fetchSftpHome()` |
| 260ms 重试竞态 | `App.vue:4353-4359` | 重试前后校验 `requestId === sftpRequestSeq`；用 `fetchSftpFiles` 的返回值判定，而不是看 `sftpFiles.length` |
| 批量删除错会话 | `App.vue:5030-5036` | 循环外取 `const sessionId = activeSessionId.value;`，循环内不变；若中途会话失效，`invoke` 会报错并计入 `failCount` |
| 错误态一致性 | `App.vue:4416-4424` | 失败时保留旧列表但加"路径无效"标注，或清空列表 + 保留 `currentSftpPath` 输入值（二选一，避免"错误文案 + 旧列表"并存） |
| `isSftpDragOver` 复位 | `App.vue:4319-4328` | `clearSftpState` 内加 `isSftpDragOver.value = false` |
| mtime 异常 | `App.vue:4848` | `const t = Number(props.mtime_str); mtime_str: Number.isFinite(t) && t > 0 ? new Date(t*1000).toLocaleString() : "—"` |

### 批次 5：体验与清理

| 改动点 | 位置 | 具体做法 |
|---|---|---|
| 缩略图缓存上限 | `App.vue:926`、`4691-4692` | 改 LRU（如 300 条），超限删最旧键；再加负缓存集合（损坏路径 5 分钟内不重试） |
| 后端像素上限 | `local_fs.rs:35-55` | 用 `image::ImageReader::with_guessed_format` + `limits`（如 `max_alloc = 256MB`、`max_dimension = 16384`），超限返回"图片过大"而不是 OOM |
| BUG-012 loading 不卸载 | `App.vue:6616-6619` | loading 期间保留列表 DOM（只加 `opacity`/骨架条），不要 `v-if` 替换整个列表 |
| 焦点可见性 | `App.vue:11690-11694` | 改为 `outline: 2px solid var(--accent); outline-offset: 2px`（键盘可见），保留 `:focus:not(:focus-visible)` 去掉鼠标点击外框 |
| 死代码/死 CSS | `App.vue:559-594`、`5942-5946`、`10453-10470`、`10545-10563`、`12793-12810`、`12850-12875` | 删除或注释说明；删除前 grep 确认无引用 |
| resize 监听泄漏 | `App.vue:5457-5481` | 抽具名 `onWindowResize`，`onUnmounted` 里 `removeEventListener` |
| 统一剪贴板 | `App.vue:2632-2637` | `copyText` 改为 `await writeClipboardText(text)` 并 catch |
| 格式函数 | `App.vue:844-849` | <1MB 用 KB 保留 1 位小数（`45 KB` 而非 `0 MB`） |

---

## 八、BUGS.md 状态更新建议

原 `../BUGS.md` 在工作区外、且约定"先记录不擅改"，因此另出 `BUGS-STATUS-2026-10-23.md`（同目录），可直接复制对应段落覆盖 `BUGS.md` 中的"状态/备注"。核心变更：

- BUG-001/002/003/007/008/011/012/013/014 → `部分修复（需补丁）`
- BUG-005 → `待视觉确认（实现与备注措辞不一致）`
- BUG-004/006/009/010/015/016/017 → `已修复`
- 新增 `BUG-018 测试不可编译`、`BUG-019 终端背景模式无法持久化`、`BUG-020 批量下载只下发一个文件`、`BUG-021 上传失败遗留远端临时文件`、`BUG-022 下载失败任务永久 running`、`BUG-023 批量删除可能跨会话`、`BUG-024 taskId 毫秒碰撞`

---

## 九、扩展审查：SSH 连接与数据通道（`ssh.rs`，BUGS.md 未覆盖）

审查 17 条时顺带通读了 `ssh.rs` 全文，发现两个**会影响日常使用**的缺陷，建议登记为 BUG-025、BUG-026。

### BUG-025 UTF-8 字符被 32KB 块边界截断（中等）
- 位置：`ssh.rs:173-201` 读线程，`ssh.rs:185` `pending.push_str(&String::from_utf8_lossy(&buf[..n]))`
- 触发：单次 `read` 返回 32768 字节，而一个 3 字节中文字恰好跨在块边界上 → 前 1~2 字节被单独 `from_utf8_lossy` → 输出 `U+FFFD`，终端显示乱码/方块。
- 为什么容易中：`cat` 大文件、`journalctl`、`docker logs` 这类连续中文输出，32KB 边界每 32KB 出现一次，命中概率随输出量线性上升。
- 讽刺的是**同项目的 telnet/串口路径已经解决了这个问题**：`generic_session.rs:45-87` 的 `Utf8ChunkDecoder` 专门处理跨块残留，且 5 个单测全过。SSH 路径没用它。
- 修复：把 `String::from_utf8_lossy` 换成 `Utf8ChunkDecoder::decode`（复用现成实现，无需新写），或改用 `String::from_utf8_lossy` 前先按 UTF-8 边界切分并保留尾部残字节。

### BUG-026 非阻塞会话上调用 `write_all`（中等）
- 位置：`ssh.rs:149` `sess.set_blocking(false)`；`ssh.rs:242-245` `ch.write_all(data.as_bytes())?` + `ch.flush()?`
- 问题：`write_all` 内部循环调 `write`，一旦底层返回 `WouldBlock`（非阻塞模式下的正常现象）就立刻返回 `Err`，**不做重试**。粘贴大段文本、或远端回显/网络抖动导致发送窗口满了，就会出现"写入失败"且内容丢失。
- 修复：写入路径改为"循环 + WouldBlock 时短暂 sleep 重试 + 总超时"，或对写操作临时切回阻塞模式（注意 `set_blocking` 是 Session 级，会影响读线程，需用通道级 `set_blocking`/`set_timeout` 或加写锁）。
- 需运行验证：取决于粘贴体量与网络 RTT；小文本几乎不触发。

### 其它（轻微）
- `ssh.rs:94` `TcpStream::connect` **无连接超时**，不可达地址会卡到 OS 默认（约 21s），期间 UI 一直显示"连接中"。建议 `connect_timeout(8s)` + 多地址轮询（`sftp.rs:46-54` 已有正确写法可复用）。
- `ssh.rs:204` 读循环 `sleep(2ms)` 忙等；单会话可接受，多开 10 个会话时是 10 个线程持续唤醒。
- `ssh.rs:97/99/100/118/132/137/144-146/154/166` 的中文文案与注释被批量替换成了"操作失败"（如 `"操作失败SSH 操作失败"`、第 100 行整行注释只剩"操作失败"），用户看到的报错无法定位，注释也失效。属替换事故，建议统一重写。
- `ssh.rs:155` 同 `session_id` 重连时直接覆盖旧条目，旧读线程要到 EOF 才退出（有 `Arc::ptr_eq` 保护不会误删新会话，但仍短暂并存）。
- 每个会话固定占用 3 条 SSH 连接（主通道 / SFTP `sftp.rs` / 监控 `monitor_slot`），传输时再新建第 4 条 —— 与"每次传输新建连接"叠加，多主机多任务时连接数增长较快。

---

## 十、扩展审查：流量审计与系统监控（`traffic.rs` / `monitor.rs`）

这两个模块不在 BUGS.md 范围内，但承担主机详情里 20 余个 traffic 字段与实时监控面板，且此前从未审查。**发现 4 个 P0**，其中两条会让"监控指标"来自错误的主机。

### P0-1 监控回退连接只认 IP 字面量，域名会连到本机（严重）
```rust
// monitor.rs:122-125
&format!("{}:{}", params.host, params.port)
    .parse()
    .unwrap_or_else(|_| "127.0.0.1:22".parse().unwrap())
```
- `TcpStream::connect_timeout` 要的是 `&SocketAddr`，所以 `.parse()` 的目标类型被推断为 `SocketAddr`，**只接受 IP 字面量**（`1.2.3.4:22`、`[::1]:22`）。域名、IPv6 写法一律解析失败 → 回退 `127.0.0.1:22`。
- 触发：主机用域名（最常见配置），且主通道本次未执行成功（`!executed || output.trim().is_empty()`，`monitor.rs:121`）→ 用**远端凭据**去认证本机 127.0.0.1:22。本机有 OpenSSH 时：同名同密会连上本机，把本机 `/proc` 指标当远端显示；不同密则产生本机 SSH 失败日志/锁定风险。
- 触发链路（精度补充）：`monitor.rs:61` 的 `ensure_sftp_handle(...)?` 失败会**直接返回 Err**，不会进 aux；aux 只在主路径 `!executed || output.trim().is_empty()`（`monitor.rs:121`）时才走。命令里包含 `echo "Cores:"`/`echo "DF:"`，输出几乎不可能为空，所以实际触发条件是"1.5 秒内连续 10 次开 channel/exec 失败"（会话繁忙、libssh2 状态冲突，恰与 M-3 的并发问题相关）。**频率不高，但一旦命中后果重**：用远端凭据认证本机。
- 修复：改用 `to_socket_addrs()` 解析（`traffic.rs:667`、`sftp.rs:40` 已有正确写法），并去掉 `127.0.0.1` 兜底。

### P0-2 回退会话无超时，配合 3 秒轮询会耗尽线程池（严重）
- `monitor.rs:127-157`：`Session::new()` 后**没有 `set_timeout`**（对比 `sftp.rs:60` 12s、`traffic.rs:693` 10s）；`connect_timeout` 只管建连。banner 不回、握手挂起或 `ch.read` 无数据时永久阻塞。
- 前端 `App.vue:4225` `setInterval(pollMetrics, 3000)` **不 await**，失败/挂起不会阻止下一次。
- 后果：每 3 秒新增一个挂在 `spawn_blocking` 里的任务，Tokio blocking 池（默认 512）被逐步抽干 → 之后所有 `spawn_blocking` 命令（SFTP 传输、流量审计、monitor）一起卡死。约 25 分钟长时间运行即可触发。
- 修复：`aux_sess.set_timeout(8_000)`；前端加 in-flight 去重（上次未完成就跳过本轮）。

### P0-3 32 位架构上把"计数归零"误判为 u32 回绕（严重，条件性）
- `traffic.rs:181-190`，调用点 `859/885-886`。`/proc/net/dev` 在所有 Linux 上都是 u64，但代码保留了"32 位内核回绕"分支；当 `arch_is_64_bit=false`（如 32 位 ARM 软路由）且 `prev`/`cur` 都小于 `u32::MAX` 时，一次接口归零被算成 `(4294967295-prev)+cur+1` ≈ **3.4 GiB 虚增**。
- 单测（`traffic.rs:1239-1244`）直接传 `is_64_bit=false`，绕过了 `arch_is_64_bit` 的真实推断链，所以测不出来。
- 附带：`traffic.rs:170-179` 在 `arch` 为空（uname 不在 PATH）时**按 64 位处理**，回退分支会把全部历史当增量。
- 修复：删除 32 位分支（Linux 早已统一 u64），或仅当 `prev > u32::MAX * 0.99` 才视为回绕；`arch` 为空时返回 false。

### P0-4 流量审计的连接风暴与命令注入面（严重）
- `traffic.rs:738` 每次都 `open_session` 新建完整 SSH 连接（`probe_snapshot` 一次 + `custom` 再一次），对比 `monitor.rs:61` 的 `ensure_sftp_handle` 复用模式，这里是反例。多主机定时审计会持续新建连接。
- `traffic.rs:710` 握手后**没有 known_hosts/主机密钥校验**（`ssh.rs` 同样没有），中间人无法被发现。
- `traffic.rs:946` `traffic_custom_command` 原样 `exec`；该字段可经"导入配置/WebDAV 恢复"写入（`backup.rs:107-139`）→ 恶意备份包可在所有主机上执行任意命令。属于信任边界缺陷。
- 正面确认（不必改）：`target_interface`/`host` **不进 shell**，probe 脚本是 `concat!` 常量，接口名无注入面；vnstat 走 `--json` 不依赖 locale。

### P1 / 中等（traffic.rs）
- **截断输出被当完整快照**（`753` `Err(_) => break` 不看 `exit_status`，`773` 一律 Ok）：若截断发生在 `/proc/net/dev` 中途，`910` 用部分 counters 覆盖 `iface_baseline`；缺失的网卡下一轮 `previous_baseline.get(name)` 为 None → delta 记 0（`884-893`），**永久漏计且不报错**。这是单测覆盖不到的静默数据错误。
- **快照失败清空基线**（`804-812` 把 `last_raw`/`sig`/`boot_id`/`iface_baseline` 全清 + `816` 清 error）：custom 成功而 probe 失败时，下一轮 `has_previous_baseline=false`，静默丢一个周期。
- **vnstat 周期起点忽略时分秒**（`245-264`、`607-614` 硬编码 `0,0,0`，而 `reset_info` `232-243` 用了时分秒）：结算时刻非 0 点时，每周期系统性多算"0 点到结算点"的流量。
- **`reset_day=31` 跨短月漏结算**（`232-243` 只比"本月结算点"）：`last_reset=1/31`、3/1 审计时下一结算点算成 3/31 → 2/28 的结算被跳过，1/31 到 3/31 合并成一个周期。
- **自定义命令高水位永不下降**（`817-824`）：`measured < cycle_used` 时保留旧高值；若 `should_reset` 未识别（未配 reset_day 等），流量条永久卡满。
- **旧配置迁移分支对多网卡和值做回绕判定**（`873`），和值 > `u32::MAX` 时落入最后一行 `current` → 整笔虚增。

### P1 / 中等（monitor.rs）
- **并发读写同一 libssh2 session**（`monitor.rs:62` + `74-107`）：锁只包住 `channel_session/exec`，`read` 循环在锁外；并发 `fetch_system_metrics` 会同时操作同一 session（libssh2 非线程安全）。与前端 3 秒轮询叠加更易触发。
- **部分输出被当正常值**（`88-121`）：WouldBlock 累计 600ms 后 break 却把 `executed=true`，`!executed || output.is_empty()` 的回退条件不成立 → 残缺输出直接解析（MemAvailable 缺失走高估分支 `403-411`，`/proc/net/dev` 缺失则速度归零）。
- **CPU 计算把 guest 重复计入**（`232-235` `total = fields.iter().sum()`）：内核文档明确 guest 已含在 user、guest_nice 已含在 nice → 虚拟化宿主机上 `d_total` 偏大、CPU 百分比系统性偏低。应取 `fields[..8].sum()`。
- **会话缓存无清理**（`17/28/484`）：`SESSION_NET_CACHE`/`SESSION_METRIC_CACHE` 以 session_id 为键，文件内**没有任何 remove**，`ssh.rs:218-227` 只移除 sessions → 每次重连泄漏一条，无界增长。
- **全失败仍返回 Ok(0)**（`498-512`）：cpu/mem/disk=0 且无错误通道，前端 `4198` 无条件写入 → 无法区分"机器空闲"与"采集失败"。

### P1 / 中等（前端关联）
- **流量报警忽略用户阈值且不看单位**（`App.vue:1074-1085`）：硬编码 `used/total >= 0.9`，未使用 `alert_traffic_pct`（`config.rs:46`）；`parseFloat` 直接相除、不换算 `traffic_used_unit` 与 `traffic_total_unit`。例：total=1TB、used=900GB → 0.0009 → **漏报**；total=100GB、used=500MB → 5.0 → **误报 500%**。
- **`pollMetrics` 无 in-flight 去重**（`App.vue:4225`）：与 P0-2 叠加，是线程池耗尽的前端一侧原因。

### P2 / 轻微
- `traffic_mult` 未知单位静默按 GiB（`139-148`）；`f64→u64` 饱和无上限（`150-157/661`）；切 `target_interface` 后增量仍从旧全量基线起算（`856`）；无默认路由时回退 `eth0`/全部非虚拟网卡、`br0` 未过滤导致桥接重复计（`343-349/410-423`）；时间回拨跨结算点永久漏一次（`130-136`，但**不会产生负增量**）；`run_remote_script` 从不读 stderr（`747-757`），命令大量写 stderr 会阻塞到超时；`monitor.rs` 锁中毒静默返回 0（`440`）；`/proc/meminfo` 存在导致 `free -m` 回退是死代码（`259-268`）；任意以 `%` 结尾的行都当磁盘占用（`269-274`）。

### 建议优先级
- **P0**：`monitor.rs:122-125`（连错主机）、`monitor.rs:127-157`（线程池耗尽）、`traffic.rs:181-190`（虚增 3.4 GiB）、`traffic.rs:738/710/946`（连接风暴 + 无主机密钥校验 + 自定义命令注入面）。
- **P1**：M-3/M-4/M-5、T-4/T-5/T-6/T-7、前端流量报警。
- **P2**：其余。

---

## 十一、修复进度（2026-10-23 20:40 更新）

> 本节记录实际落地的代码改动。全部改动均已通过 `cargo test`（31 + 10 + 2 全绿）与 `npm run build`（vue-tsc + vite）验证，前端产物已同步到 `ui_dist`。

### 已修复

| 编号 | 内容 | 文件 |
|---|---|---|
| BUG-018 | 测试 crate 名 `qshell_winui3_lib` → `qshell_lib` | `tests/utf8_decoder_test.rs` |
| BUG-019 | `terminal_bg_mode` 加 `alias = "term_bg_mode"`、新增 `terminal_bg_color`；前端读写字段名统一 | `config.rs`、`App.vue` |
| — | `AppConfig.devices` / `managed_hosts` 加 `#[serde(default)]` | `config.rs` |
| — | 最小窗口尺寸统一为 1080×640 | `src/lib.rs` |
| BUG-024 | 新增 `makeTaskId()`（时间戳 + 自增序号） | `App.vue` |
| BUG-021 | 上传四处失败路径统一 `fail_upload`：删远端临时文件 + emit failed + 清 cancel 标记 | `sftp.rs` |
| BUG-022 | 下载四处失败路径统一 `fail_download` + 前端 catch 置 failed | `sftp.rs`、`App.vue` |
| BUG-020 | 批量下载改为串行 `await`，取消即停止后续 | `App.vue` |
| BUG-014 | 上传加 `overwrite` 语义（未选覆盖时目标存在直接失败）；重命名候选名二次 `sftp_stat` 校验；新增"跳过" | `sftp.rs`、`App.vue` |
| — | 冲突弹窗并发保护（旧请求 resolve 为 cancel，避免 Promise 悬挂） | `App.vue` |
| BUG-008 | 多行命令只填充首行并提示；只读拦截；写入失败提示 | `App.vue` |
| BUG-011 | 粘贴早退分支恢复终端焦点 | `App.vue` |
| BUG-007 | 抽 `ensureReadableFg`，保存设置与新建终端也做 4.5 对比度兜底 | `App.vue` |
| BUG-003 | 新增 `sftpPanelState` / `sftpPanelStateText`，右键菜单按状态给文案，刷新目录非 ready 时禁用 | `App.vue` |
| BUG-001 | 拖拽离开标签区清空落点；窗口失焦时清理拖拽状态 | `App.vue` |
| BUG-002 | 右栏折叠/页签切换纳入刷新 watch；260ms 重试复核请求序号并复位 loading | `App.vue` |
| BUG-023 | 批量删除固定开始时解析的 sessionId | `App.vue` |
| — | mtime 为 `-` 时显示 `—` 而不是 Invalid Date；`clearSftpState` 复位拖拽态 | `App.vue` |
| P0-1 | 监控 aux 连接改用 `to_socket_addrs`，删除 127.0.0.1 兜底 | `monitor.rs` |
| P0-2 | aux 会话 `set_timeout(8s)`；前端 `metricsPollInFlight` 防重入 | `monitor.rs`、`App.vue` |
| P0-3 | 删除 32 位回绕分支（含单测更新） | `traffic.rs` |
| — | CPU total 改为 `fields[..8]`（排除 guest/guest_nice） | `monitor.rs` |
| — | 新增 `clear_session_caches`，会话关闭/断开时清理监控缓存 | `monitor.rs`、`ssh.rs` |
| BUG-033 | 流量报警改用 `getTrafficProgress` + `alert_traffic_pct` 阈值 | `App.vue` |
| BUG-025 | SSH 读线程改用 `Utf8ChunkDecoder`，修复中文跨块截断 | `ssh.rs` |
| BUG-026 | `write_ssh_session` 改为循环写 + WouldBlock 重试 + 5s 超时 | `ssh.rs` |
| — | 缩略图缓存上限 300 条；后端 `image::Limits`（max_alloc/max_width/max_height） | `App.vue`、`local_fs.rs` |
| — | `local_path_exists` / `list_local_dir` / `read_local_image_data_url` 改为 async + spawn_blocking | `local_fs.rs` |
| — | `formatMetricBytes` 小于 1MB 用 KB；`copyText` 统一走 `writeClipboardText`；文件浏览器恢复键盘焦点框 | `App.vue` |
| BUG-031(部分) | `reset_info` 在本月结算点未到时回落到上月结算点（短月 clamp）；vnstat 周期起点改用配置的时分秒 | `traffic.rs` |
| — | 本机文件名净化：`joinLocalPath` 只取最后一段、过滤非法字符与 Windows 保留名，无效时给出提示 | `App.vue` |
| BUG-032(部分) | 监控主通道的取通道/exec/读取全部在同一把锁内完成，消除 libssh2 并发访问 | `monitor.rs` |
| BUG-031(剩余) | 流量快照末尾加 `__QSHELL_DONE__` 哨兵，截断输出不再覆盖网卡基线（避免永久漏计） | `traffic.rs` |

### 未修复（建议后续，均已在正文标注）

- BUG-030 的流量审计连接复用与 host key 校验（改动大，需设计）
- `local_fs` 根目录白名单、`csp: null`（会改变行为，需产品确认）
- 死代码/死 CSS 清理、resize 监听解绑（根组件，风险低）
- `beforeBuildCommand` 的 cwd 依赖（需先确认你的打包方式）

> 本报告只读产出，未改动任何源码；`BUGS.md` 也未修改（按约定等你确认）。
