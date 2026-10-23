# 需求：终端类型（TERM）选择

- 提出时间：2026-10-23
- 状态：待实现
- 相关模块：`config.rs`、`ssh.rs`、`telnet_codec.rs`、`App.vue`

## 1. 目标

在主机配置里增加「终端类型」下拉，支持：

- 自动（默认）
- xterm-256color
- xterm
- vt220
- vt100
- linux
- ansi

## 2. 现状（代码事实）

| 位置 | 当前行为 |
|---|---|
| `ssh.rs:145` | `channel.request_pty("xterm-256color", ...)` —— 写死 |
| `telnet_codec.rs:204` | TTYPE 协商固定回复 `xterm-256color` |
| `serial_service.rs` | 串口无 TERM 概念，不适用 |
| 前端 | 没有任何终端类型选项 |

## 3. 「自动」能不能探测？—— 结论

### 标准协议层面：不能

- **SSH**：客户端在 `request_pty` 时单向告知 TERM，服务器不返回"我支持哪些终端类型"。
- **Telnet**：TTYPE 协商的方向是服务器问、客户端答，客户端无法主动查询服务器偏好。

所以不存在协议级的"服务器告诉客户端支持什么"。

### 工程层面：SSH 可以间接探测（有代价）

SSH 在认证后、请求 PTY 之前，可以用一次性 `exec` 通道执行：

```sh
command -v infocmp >/dev/null 2>&1 && \
for t in xterm-256color xterm vt220 vt100 linux ansi; do
  infocmp -1 "$t" >/dev/null 2>&1 && { echo "$t"; break; }
done
```

按优先级返回第一个服务器上有 terminfo 条目的类型。

**代价**：
- 每次连接多一次 exec，增加约 50~300 ms 延迟；
- 精简系统（OpenWrt / BusyBox）可能没有 `infocmp`，此时回退 `xterm-256color`；
- 探测结果建议按主机缓存到配置（`term_type_effective`），避免每次重连都探测。

### Telnet：不能探测

只能客户端声明，`自动` 时按 `xterm-256color` 处理（几乎所有 BSD/Linux telnetd 都接受）。

## 4. 实现方案

### 4.1 配置模型（`config.rs`）

`HostItem` 增加：

```rust
pub term_type: Option<String>, // "auto" | 具体类型，None 视为 auto
```

### 4.2 SSH（`ssh.rs`）

1. `start_ssh_session` 增加参数解析：从 `params` 取 `term_type`（需在 `SshConnectParams` 增加字段）。
2. 认证成功后、`channel_session()` 之前，若为 `auto` 且未缓存有效值：
   - 用临时通道执行上面的探测脚本；
   - 结果为空则用 `xterm-256color`。
3. `request_pty(term, ...)` 使用解析出的值。

### 4.3 Telnet（`telnet_codec.rs`）

`TelnetCodec::new(cols, rows)` 增加 `term_name: String` 参数，TTYPE 回复该值；`auto` 传入 `xterm-256color`。

### 4.4 前端（`App.vue`）

- 主机编辑弹窗增加「终端类型」下拉，仅 `protocol === "ssh" | "telnet"` 时显示；
- 选项文案：`自动（推荐）` / `xterm-256color` / `xterm` / `vt220` / `vt100` / `linux` / `ansi`；
- 默认 `自动`；
- 串口协议隐藏该项。

## 5. 验收标准

1. 新建 SSH 主机默认「自动」，连接后 `echo $TERM` 输出服务器上实际可用的类型（常见为 `xterm-256color`）；
2. 手动选 `vt100` 后连接，`echo $TERM` 输出 `vt100`，`infocmp vt100` 正常，终端显示不乱码；
3. Telnet 手动选类型后，抓包可看到 TTYPE 回复对应值；
4. 无 `infocmp` 的服务器（如精简 OpenWrt）自动回退 `xterm-256color`，不影响连接；
5. 串口主机不显示该选项，行为不变。

## 6. 风险与取舍

- 「自动」会让首次连接稍慢（多一次 exec），可通过缓存规避；
- 极少数系统 terminfo 缺失时只能回退，不代表真实支持；
- SCOANSI 不在本次范围（xterm.js 不支持其专有序列，需单独评估）。
