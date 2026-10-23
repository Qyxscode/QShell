use std::sync::Arc;
use parking_lot::Mutex;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use std::net::{TcpStream, ToSocketAddrs};
use std::io::{Read, Write};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SshConnectParams {
    pub session_id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
    pub cols: u32,
    pub rows: u32,
    /// 是否使用系统 ssh-agent 里的密钥认证
    #[serde(default)]
    pub use_agent: Option<bool>,
    /// 终端类型：auto / xterm-256color / xterm / vt220 / vt100 / linux / ansi
    #[serde(default)]
    pub term_type: Option<String>,
}

/// 键盘交互式认证的应答器：服务器给几轮提示就回几次密码。
/// 适用于把 PasswordAuthentication 换成 keyboard-interactive 的服务器。
struct PasswordPromptHandler<'a> {
    password: &'a str,
}

impl ssh2::KeyboardInteractivePrompt for PasswordPromptHandler<'_> {
    fn prompt(
        &mut self,
        _username: &str,
        _instructions: &str,
        prompts: &[ssh2::Prompt<'_>],
    ) -> Vec<String> {
        prompts.iter().map(|_| self.password.to_string()).collect()
    }
}

fn terminal_flush_interval(pending_len: usize) -> std::time::Duration {
    let millis = if pending_len >= 8 * 1024 * 1024 {
        3000
    } else if pending_len >= 2 * 1024 * 1024 {
        500
    } else if pending_len >= 512 * 1024 {
        120
    } else if pending_len >= 128 * 1024 {
        40
    } else {
        16
    };
    std::time::Duration::from_millis(millis)
}

pub struct SftpHandle {
    pub session: Arc<Mutex<ssh2::Session>>,
    pub sftp: Arc<Mutex<ssh2::Sftp>>,
}

impl Clone for SftpHandle {
    fn clone(&self) -> Self {
        Self {
            session: self.session.clone(),
            sftp: self.sftp.clone(),
        }
    }
}

pub struct ActiveSession {
    pub channel: Arc<Mutex<ssh2::Channel>>,
    // Keeps the interactive SSH session alive for the lifetime of its channel.
    #[allow(dead_code)]
    pub session: Arc<Mutex<ssh2::Session>>,
    pub sftp: Arc<Mutex<Option<SftpHandle>>>,
    pub monitor: Arc<Mutex<Option<SftpHandle>>>,
    pub params: SshConnectParams,
}

pub struct SshManager {
    pub sessions: Arc<Mutex<HashMap<String, ActiveSession>>>,
}

impl SshManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// 统一 TCP 连接：支持域名、IPv4、IPv6 字面量，并为每个地址设置连接超时，
/// 避免不可达地址卡到系统默认的约 21 秒。
fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, String> {
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("解析地址失败 ({}:{}): {}", host, port, e))?;
    let mut last_error = String::new();
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(8)) {
            Ok(stream) => return Ok(stream),
            Err(e) => last_error = e.to_string(),
        }
    }
    if last_error.is_empty() {
        Err(format!("连接失败 ({}:{}): 没有可用的地址", host, port))
    } else {
        Err(format!("连接失败 ({}:{}): {}", host, port, last_error))
    }
}

/// 解析终端类型：`auto` 时探测服务器上存在的 terminfo，失败回退 xterm-256color。
fn resolve_term_type(requested: Option<&str>, sess: &ssh2::Session) -> String {
    let requested = requested.unwrap_or("auto").trim();
    if !requested.is_empty() && requested != "auto" {
        return requested.to_string();
    }
    let probe = "for t in xterm-256color xterm vt220 vt100 linux ansi; do \
                 command -v infocmp >/dev/null 2>&1 || break; \
                 infocmp -1 \"$t\" >/dev/null 2>&1 && { echo \"$t\"; break; }; done";
    if let Ok(mut ch) = sess.channel_session() {
        if ch.exec(probe).is_ok() {
            let mut out = String::new();
            let mut buf = [0u8; 64];
            loop {
                match ch.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => out.push_str(&String::from_utf8_lossy(&buf[..n])),
                    Err(_) => break,
                }
            }
            let _ = ch.close();
            let candidate = out.trim();
            if !candidate.is_empty() {
                return candidate.to_string();
            }
        }
    }
    "xterm-256color".to_string()
}

#[tauri::command]
pub async fn start_ssh_session(
    app: AppHandle,
    state: tauri::State<'_, SshManager>,
    params: SshConnectParams,
) -> Result<String, String> {
    let session_id = params.session_id.clone();
    let host = params.host.clone();
    let port = params.port;
    let username = params.username.clone();
    let password = params.password.clone();
    let private_key = params.private_key.clone();
    let passphrase = params.passphrase.clone();
    let cols = params.cols;
    let rows = params.rows;
    let use_agent = params.use_agent.unwrap_or(false);
    let term_type = params.term_type.clone();

    let sessions_map = state.sessions.clone();
    
    // 建连、握手与认证在阻塞线程中执行
    tokio::task::spawn_blocking(move || {
        let tcp = connect_tcp(&host, port)?;
        tcp.set_nodelay(true).ok();

        let mut sess = ssh2::Session::new().map_err(|e| format!("初始化 SSH 会话失败: {}", e))?;
        sess.set_tcp_stream(tcp);
        sess.set_timeout(15000); // 15 秒读写超时
        // 优先启用现代 KEX / 主机密钥算法，兼容新旧服务端
        let _ = sess.method_pref(
            ssh2::MethodType::Kex,
            "ecdh-sha2-nistp256,ecdh-sha2-nistp384,ecdh-sha2-nistp521,diffie-hellman-group14-sha256,diffie-hellman-group16-sha512,diffie-hellman-group-exchange-sha256,diffie-hellman-group14-sha1",
        );
        let _ = sess.method_pref(
            ssh2::MethodType::HostKey,
            "rsa-sha2-512,rsa-sha2-256,ecdsa-sha2-nistp256,ecdsa-sha2-nistp384,ecdsa-sha2-nistp521,ssh-rsa",
        );
        let _ = sess.method_pref(
            ssh2::MethodType::CryptCs,
            "aes128-ctr,aes192-ctr,aes256-ctr,aes128-cbc,aes256-cbc,3des-cbc",
        );
        let _ = sess.method_pref(
            ssh2::MethodType::CryptSc,
            "aes128-ctr,aes192-ctr,aes256-ctr,aes128-cbc,aes256-cbc,3des-cbc",
        );

        sess.handshake().map_err(|e| format!("SSH 握手失败: {}", e))?;
        let _ = sess.set_keepalive(true, 30);

        if let Some(ref key_path) = private_key {
            if !key_path.trim().is_empty() {
                let p = std::path::Path::new(key_path);
                if p.is_file() {
                    let _ = sess.userauth_pubkey_file(&username, None, p, passphrase.as_deref());
                }
            }
        }

        // 系统 ssh-agent（需在主机配置里勾选）
        if !sess.authenticated() && use_agent {
            let _ = sess.userauth_agent(&username);
        }

        // 密码认证
        if !sess.authenticated() {
            if let Some(ref pwd) = password {
                let _ = sess.userauth_password(&username, pwd);
            }
        }

        // 键盘交互式：很多服务器（CentOS/Debian 默认配置）用它代替 PasswordAuthentication
        if !sess.authenticated() {
            if let Some(ref pwd) = password {
                let mut handler = PasswordPromptHandler { password: pwd };
                let _ = sess.userauth_keyboard_interactive(&username, &mut handler);
            }
        }

        if !sess.authenticated() {
            return Err("身份认证未通过，请检查用户名、密码、私钥或 SSH Agent".to_string());
        }

        // SFTP uses a separate connection, created lazily on first use.
        let sftp_slot: Arc<Mutex<Option<SftpHandle>>> = Arc::new(Mutex::new(None));
        let monitor_slot: Arc<Mutex<Option<SftpHandle>>> = Arc::new(Mutex::new(None));

        let term = resolve_term_type(term_type.as_deref(), &sess);
        let mut channel = sess.channel_session().map_err(|e| format!("创建 SSH 通道失败: {}", e))?;
        channel.request_pty(&term, None, Some((cols, rows, 0, 0))).map_err(|e| format!("申请 PTY 失败: {}", e))?;
        channel.shell().map_err(|e| format!("启动远程 Shell 失败: {}", e))?;

        // Keep the interactive session non-blocking.
        sess.set_blocking(false);

        let channel_arc = Arc::new(Mutex::new(channel));
        let sess_arc = Arc::new(Mutex::new(sess));

        // 注册会话
        sessions_map.lock().insert(
            session_id.clone(),
            ActiveSession {
                channel: channel_arc.clone(),
                session: sess_arc,
                sftp: sftp_slot,
                monitor: monitor_slot,
                params,
            },
        );

        // 输出读取线程（对标 ssh8.py _ssh_session_reader：合并小块输出，降低 IPC 频率）
        let read_channel = channel_arc.clone();
        let cleanup_channel = channel_arc.clone();
        let cleanup_sessions = sessions_map.clone();
        let app_handle = app.clone();
        let sid = session_id.clone();

        std::thread::spawn(move || {
            let mut buf = [0u8; 32768];
            let mut pending = String::new();
            // 按 UTF-8 边界解码：直接对每个 32KB 块做 from_utf8_lossy 会把
            // 跨块的中文字符拆坏成 U+FFFD（终端显示方块）。
            let mut decoder = crate::generic_terminal::generic_session::Utf8ChunkDecoder::new();
            let mut last_emit = std::time::Instant::now();
            loop {
                let read_res = {
                    let mut ch = read_channel.lock();
                    ch.read(&mut buf)
                };
                match read_res {
                    Ok(0) => break,
                    Ok(n) => {
                        pending.push_str(&decoder.decode(&buf[..n]));
                        if pending.len() >= 64 * 1024
                            || last_emit.elapsed() >= terminal_flush_interval(pending.len())
                        {
                            let event_name = format!("ssh-data-{}", sid);
                            let _ = app_handle.emit(&event_name, pending.clone());
                            pending.clear();
                            last_emit = std::time::Instant::now();
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if !pending.is_empty()
                            && last_emit.elapsed() >= terminal_flush_interval(pending.len())
                        {
                            let event_name = format!("ssh-data-{}", sid);
                            let _ = app_handle.emit(&event_name, pending.clone());
                            pending.clear();
                            last_emit = std::time::Instant::now();
                        }
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
            if !pending.is_empty() {
                let _ = app_handle.emit(&format!("ssh-data-{}", sid), pending);
            }
            let _ = app_handle.emit(&format!("ssh-closed-{}", sid), ());

            // The shell exited or the channel died. Drop this session (including
            // its separate SFTP/monitor connections) immediately, but only if it
            // is still the same channel instance. This prevents an old reader
            // thread from deleting a freshly reconnected session with the same id.
            {
                let mut sessions = cleanup_sessions.lock();
                let same_session = sessions
                    .get(&sid)
                    .map(|active| Arc::ptr_eq(&active.channel, &cleanup_channel))
                    .unwrap_or(false);
                if same_session {
                    sessions.remove(&sid);
                }
            }
            // 监控/网络采样缓存以 session_id 为键，会话消失时必须一并清理，否则会无界增长。
            crate::monitor::clear_session_caches(&sid);
        });

        Ok("SSH 会话建立成功".to_string())
    })
    .await
    .map_err(|e| format!("SSH 会话任务异常: {}", e))?
}

#[tauri::command]
pub fn write_ssh_session(
    state: tauri::State<'_, SshManager>,
    session_id: String,
    data: String,
) -> Result<(), String> {
    if let Some(active) = state.sessions.lock().get(&session_id) {
        let mut ch = active.channel.lock();
        // 会话被设为非阻塞，write_all 遇到 WouldBlock 会直接失败并丢数据；
        // 这里改为循环写入 + 超时，粘贴大段文本时才不会半途而废。
        let bytes = data.as_bytes();
        let mut offset = 0usize;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while offset < bytes.len() {
            match ch.write(&bytes[offset..]) {
                Ok(0) => return Err("写入终端失败：远端通道已关闭".to_string()),
                Ok(n) => offset += n,
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() > deadline {
                        return Err("写入终端超时：远端暂时无法接收数据".to_string());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => return Err(format!("写入终端失败: {}", e)),
            }
        }
        // flush 同样可能 WouldBlock，此时数据已写入通道，忽略即可。
        let _ = ch.flush();
        Ok(())
    } else {
        Err("SSH session not found".to_string())
    }
}

#[tauri::command]
pub fn resize_ssh_session(
    state: tauri::State<'_, SshManager>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    if let Some(active) = state.sessions.lock().get(&session_id) {
        let mut ch = active.channel.lock();
        ch.request_pty_size(cols, rows, None, None).map_err(|e| format!("调整 PTY 尺寸失败: {}", e))?;
        Ok(())
    } else {
        Err("SSH session not found".to_string())
    }
}

#[tauri::command]
pub fn close_ssh_session(
    state: tauri::State<'_, SshManager>,
    session_id: String,
) -> Result<(), String> {
    crate::monitor::clear_session_caches(&session_id);
    if let Some(active) = state.sessions.lock().remove(&session_id) {
        let mut ch = active.channel.lock();
        ch.close().ok();
        Ok(())
    } else {
        Ok(())
    }
}
