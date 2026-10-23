# 功能评估：SSH 认证与加密能力

- 评估时间：2026-10-23
- 评估对象：当前代码基线（`ssh2` 0.9.6 / libssh2，配合 vendored OpenSSL 3）
- 结论速览：**已实现 5 项 / 可直接扩展 4 项 / 受底层库限制无法实现 5 项**

## 1. 已实现（有代码依据）

| 功能 | 状态 | 代码依据 |
|---|---|---|
| AES128 / 192 / 256 | ✅ | `ssh.rs:109-116` 的 `method_pref` 显式启用 `aes128-ctr/aes192-ctr/aes256-ctr/aes128-cbc/aes256-cbc` |
| 3DES | ✅ | 同上，`3des-cbc` |
| Rijndael | ✅ | 即 AES，同一实现 |
| RSA / ECDSA / ED25519 公钥认证 | ✅ | `ssh.rs:125` `userauth_pubkey_file(username, None, path, passphrase)`，支持带口令私钥 |
| 密码认证 | ✅ | `ssh.rs:132` `userauth_password` |

补充：主机密钥算法已启用 `rsa-sha2-512/256`、`ecdsa-sha2-nistp256/384/521`、`ssh-rsa`；KEX 启用 ecdh-p256/384/521、dh-group14-sha256、group16-sha512、group-exchange-sha256、group14-sha1（`ssh.rs:101-108`）。

## 2. 可以直接扩展（不需要更换 SSH 库）

| 功能 | 可行性 | 实现要点 | 预估 |
|---|---|---|---|
| 键盘交互式认证（keyboard-interactive） | ✅ 容易 | `ssh2` 已提供 `userauth_keyboard_interactive(prompt_handler)`，接一个回调处理多轮提示即可；需在 `start_ssh_session` 的认证链里增加失败回退顺序 | 0.5 天 |
| Agent 认证（使用本地 ssh-agent 里的密钥） | ✅ 容易 | `ssh2` 提供 `userauth_agent(username)` 与 `agent()`；前端增加"使用系统 Agent"认证方式 | 0.5 天 |
| 密钥生成向导 + 导入 / 导出 | ✅ 可以 | 生成 ed25519 / rsa / ecdsa 并导出 OpenSSH 或 PEM 格式；可引入 `ed25519-dalek`、`rsa` 或调用系统 `ssh-keygen`；导入需解析 OpenSSH / PEM / PuTTY(ppk) | 1~2 天 |
| 主密码（配置加密） | ✅ 可以 | 用 AES-GCM 加密 `%APPDATA%\QShell\config.json`，启动时解锁；与 SSH 层无关，属应用层功能 | 1 天 |

## 3. 受 libssh2 限制，当前无法实现

| 功能 | 结论 | 原因 |
|---|---|---|
| OpenSSH CA 证书认证 | ❌ | libssh2 不支持 `ssh-rsa-cert-v01@openssh.com` 等证书类型公钥算法，`userauth_pubkey_file` 只接受普通密钥 |
| PKCS#11（硬件令牌 / 智能卡） | ❌ | libssh2 认证只接受**文件路径**私钥，没有"外部签名回调"接口，无法把签名交给令牌 |
| Kerberos / GSSAPI | ❌ | libssh2 编译层不含 GSSAPI 支持 |
| Xagent 代理转发（远端使用本地 agent） | ❌ | libssh2 没有 `auth-agent-req@openssh.com` 与 channel 转发 API。**注意**：用本地 agent **认证**可以做（见第 2 节），**转发**不行 |
| Blowfish / CAST128 / ArcFour | ⚠️ 实际不可用 | libssh2 有实现，但需要 OpenSSL 3 的 legacy provider（当前 vendored 构建未启用）；且 OpenSSH 8.8+ 已移除这些算法，服务器端基本不支持 |

## 4. 加密算法现状明细

```
AES128/192/256 (CTR/CBC)  ✅ 已启用，现代服务器通用
3DES                      ✅ 已启用（老设备兼容，新服务器常禁用）
Rijndael = AES            ✅ 同一算法
Blowfish / CAST128        ❌ 需 legacy provider，且服务器已弃用
ArcFour                   ❌ 全行业已移除
```

## 5. 如果要补齐"无法实现"的项

需要把传输层从 `ssh2`（libssh2 同步 API）换成 `russh`：

| 换库后可获得 | 说明 |
|---|---|
| OpenSSH 证书认证 | russh 支持 cert 类型公钥 |
| Agent 转发 | russh 支持 agent forwarding |
| 更完整的 KEX / 加密算法 | 包含现代算法与 post-quantum 扩展 |
| keyboard-interactive | 原生支持 |

**代价**：
- `ssh.rs`、`sftp.rs`、`traffic.rs`、`monitor.rs` 全部基于 ssh2 的同步/阻塞调用，需要重写传输层；
- 预计 1~2 周，且需重新回归 SFTP 传输、流量审计、监控采集；
- PKCS#11 即使换库也要额外接 `cryptoki` 做签名回调。

## 6. 建议优先级

1. **键盘交互式认证**（最容易，很多老服务器与双因素场景需要）
2. **Agent 认证**（免密钥文件，体验提升明显）
3. **密钥生成向导 + 导入导出**
4. **主密码**
5. CA 证书 / Kerberos / PKCS#11 / Agent 转发：结论是**不做**（换库成本远高于收益），在界面上明确标注"不支持"

## 7. 验证方法（便于验收）

| 功能 | 验证 |
|---|---|
| AES / 3DES | `ssh -vv` 看协商结果，或服务端 `/var/log/auth.log` |
| 键盘交互式 | 用 `sshd` 开启 `AuthenticationMethods keyboard-interactive` 测试 |
| Agent 认证 | 本机 `ssh-add` 后连接，确认弹出指纹认证 |
| 密钥生成 | 生成后在服务器 `authorized_keys` 追加，能登录；`ssh-keygen -l -f` 校验指纹 |
| 主密码 | 加密后 `config.json` 应为密文；错误主密码应拒绝加载 |
| CA / PKCS#11 / Kerberos | 当前预期**连接失败或不可选**，作为"不支持"基线 |
