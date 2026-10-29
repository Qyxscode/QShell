use std::sync::Arc;
use parking_lot::Mutex;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use std::net::{TcpStream, ToSocketAddrs};
use std::io::{Read, Write};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProxyConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
    #[serde(default)]
    pub use_agent: Option<bool>,
}

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
    /// 跳板机（ProxyJump），为 None 时直连
    #[serde(default)]
    pub proxy: Option<ProxyConfig>,
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

/// 所有涉及 SSH 通道的操作都排进这个队列，由唯一的 IO 线程串行执行。
///
/// libssh2 的 session 不是线程安全的：即使对 channel 加锁，读线程和写线程
/// 交替进入 transport 层仍有可能把内部状态搞坏 —— 网络抖动时表现为
/// "读取失败: transport read"，以及数据流错位导致的光标漂移。
/// 改成「单 IO 线程 + 命令队列」后，读写永远在同一个线程里发生。
pub enum ChannelCommand {
    Write(Vec<u8>),
    Resize(u32, u32),
    Close,
}

pub struct ActiveSession {
    pub channel: Arc<Mutex<ssh2::Channel>>,
    /// 发往 IO 线程的命令队列
    pub tx: std::sync::mpsc::Sender<ChannelCommand>,
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
fn apply_session_prefs(sess: &ssh2::Session) {
    // 优先启用现代 KEX / 主机密钥 / 加密算法，兼容新旧服务端
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
}

fn authenticate(
    sess: &ssh2::Session,
    username: &str,
    password: Option<&str>,
    private_key: Option<&str>,
    passphrase: Option<&str>,
    use_agent: bool,
) -> Result<(), String> {
    if let Some(key_path) = private_key.filter(|k| !k.trim().is_empty()) {
        let path = std::path::Path::new(key_path);
        if path.is_file() {
            let _ = sess.userauth_pubkey_file(username, None, path, passphrase);
        }
    }
    if !sess.authenticated() && use_agent {
        let _ = sess.userauth_agent(username);
    }
    if !sess.authenticated() {
        if let Some(pwd) = password.filter(|p| !p.is_empty()) {
            let _ = sess.userauth_password(username, pwd);
        }
    }
    if !sess.authenticated() {
        if let Some(pwd) = password.filter(|p| !p.is_empty()) {
            let mut handler = PasswordPromptHandler { password: pwd };
            let _ = sess.userauth_keyboard_interactive(username, &mut handler);
        }
    }
    if !sess.authenticated() {
        return Err("身份认证未通过，请检查用户名、密码、私钥或 SSH Agent".to_string());
    }
    Ok(())
}

fn open_direct_session(params: &SshConnectParams) -> Result<ssh2::Session, String> {
    let tcp = connect_tcp(&params.host, params.port)?;
    tcp.set_nodelay(true).ok();
    let mut sess = ssh2::Session::new().map_err(|e| format!("初始化 SSH 会话失败: {}", e))?;
    sess.set_tcp_stream(tcp);
    sess.set_timeout(15000);
    apply_session_prefs(&sess);
    sess.handshake().map_err(|e| format!("SSH 握手失败: {}", e))?;
    let _ = sess.set_keepalive(true, 30);
    authenticate(
        &sess,
        &params.username,
        params.password.as_deref(),
        params.private_key.as_deref(),
        params.passphrase.as_deref(),
        params.use_agent.unwrap_or(false),
    )?;
    Ok(sess)
}

/// 连接目标主机：有跳板机时先连跳板，再在跳板通道上跑第二层 SSH。
fn open_session(params: &SshConnectParams) -> Result<ssh2::Session, String> {
    let proxy = match &params.proxy {
        Some(p) if !p.host.trim().is_empty() => p,
        _ => return open_direct_session(params),
    };

    // 1. 连接并认证跳板机
    let jump = open_direct_session(&SshConnectParams {
        session_id: String::new(),
        host: proxy.host.clone(),
        port: proxy.port,
        username: proxy.username.clone(),
        password: proxy.password.clone(),
        private_key: proxy.private_key.clone(),
        passphrase: proxy.passphrase.clone(),
        cols: 80,
        rows: 24,
        use_agent: proxy.use_agent,
        term_type: None,
        proxy: None,
    })
    .map_err(|e| format!("跳板机连接失败: {}", e))?;

    // 2. 在跳板机上开一条到目标的通道
    let channel = jump
        .channel_direct_tcpip(&params.host, params.port, None)
        .map_err(|e| format!("跳板机转发通道建立失败: {}", e))?;

    // 3. 本机监听一个随机端口，把目标会话的 TCP 流量经由该通道转发
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("跳板机本地中转监听失败: {}", e))?;
    let local_addr = listener
        .local_addr()
        .map_err(|e| format!("读取本地中转地址失败: {}", e))?;

    // 通道建立后改用短超时轮询，供双向转发
    jump.set_timeout(30);
    let _ = jump.set_blocking(false);

    std::thread::spawn(move || {
        if let Ok((tcp, _)) = listener.accept() {
            pump_proxy_stream(tcp, channel);
        }
    });

    // 4. 目标会话连接本地中转端口
    let tcp = std::net::TcpStream::connect(local_addr)
        .map_err(|e| format!("连接本地中转失败: {}", e))?;
    tcp.set_nodelay(true).ok();

    let mut target = ssh2::Session::new().map_err(|e| format!("初始化 SSH 会话失败: {}", e))?;
    target.set_tcp_stream(tcp);
    target.set_timeout(15000);
    apply_session_prefs(&target);
    target
        .handshake()
        .map_err(|e| format!("目标主机 SSH 握手失败: {}", e))?;
    let _ = target.set_keepalive(true, 30);
    authenticate(
        &target,
        &params.username,
        params.password.as_deref(),
        params.private_key.as_deref(),
        params.passphrase.as_deref(),
        params.use_agent.unwrap_or(false),
    )
    .map_err(|e| format!("目标主机{}", e))?;
    Ok(target)
}

fn pump_proxy_stream(mut tcp: std::net::TcpStream, mut channel: ssh2::Channel) {
    tcp.set_read_timeout(Some(std::time::Duration::from_millis(30))).ok();
    tcp.set_write_timeout(Some(std::time::Duration::from_secs(15))).ok();
    let mut buf = [0u8; 16 * 1024];
    let mut pending: Vec<u8> = Vec::new();
    loop {
        match tcp.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => pending.extend_from_slice(&buf[..n]),
            Err(ref e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
        if !pending.is_empty() {
            match channel.write(&pending) {
                Ok(n) if n > 0 => {
                    pending.drain(..n);
                }
                Ok(_) => {}
                Err(ref e) if is_retryable_io(e) => {}
                Err(_) => break,
            }
        }
        match channel.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if tcp.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
            Err(ref e) if is_retryable_io(e) => {}
            Err(_) => break,
        }
    }
    let _ = channel.close();
}

fn is_retryable_io(error: &std::io::Error) -> bool {
    if matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ) {
        return true;
    }
    let message = error.to_string().to_lowercase();
    message.contains("timed out")
        || message.contains("would block")
        || message.contains("eagain")
        || message.contains("again")
}

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
    let cols = params.cols;
    let rows = params.rows;
    let term_type = params.term_type.clone();

    let sessions_map = state.sessions.clone();
    
    // 建连、握手与认证在阻塞线程中执行
    tokio::task::spawn_blocking(move || {
        let mut sess = open_session(&params)?;

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

        // 命令队列：写 / 改尺寸 / 关闭都交给 IO 线程串行执行
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<ChannelCommand>();

        // 注册会话
        sessions_map.lock().insert(
            session_id.clone(),
            ActiveSession {
                channel: channel_arc.clone(),
                tx: cmd_tx,
                session: sess_arc,
                sftp: sftp_slot,
                monitor: monitor_slot,
                params,
            },
        );

        // 单一 IO 线程：读写 + 改尺寸 + 关闭全部在这里串行执行，
        // 避免 libssh2 session 被多线程交替访问（网络抖动时会导致 transport 层错乱）。
        let io_channel = channel_arc.clone();
        let cleanup_channel = channel_arc.clone();
        let cleanup_sessions = sessions_map.clone();
        let app_handle = app.clone();
        let sid = session_id.clone();

        std::thread::spawn(move || {
            let mut buf = [0u8; 32768];
            let mut pending = String::new();
            // 断开原因：传给前端显示，便于判断是远端退出、网络问题还是内部错误
            let mut close_reason = String::from("远端结束了会话");
            // 按 UTF-8 边界解码：直接对每个 32KB 块做 from_utf8_lossy 会把
            // 跨块的中文字符拆坏成 U+FFFD（终端显示方块）。
            let mut decoder = crate::generic_terminal::generic_session::Utf8ChunkDecoder::new();
            let mut last_emit = std::time::Instant::now();
            // 网络抖动（丢包/高延迟）时 libssh2 会冒出短暂错误（例如 transport read）。
            // 旧逻辑一遇到非 WouldBlock 的错误就 break，等于把一次抖动当成断线处理；
            // 现在给它一个容忍窗口：连续出错超过约 2 秒才真正放弃。
            let mut consecutive_errors = 0u32;
            // 待写入远端的数据：非阻塞写可能只写出去一部分，剩下的下一轮继续
            let mut out_buf: Vec<u8> = Vec::new();
            let mut pending_resize: Option<(u32, u32)> = None;
            loop {
                // ---- 1. 处理命令队列（写 / 改尺寸 / 关闭）----
                let mut want_close = false;
                loop {
                    match cmd_rx.try_recv() {
                        Ok(ChannelCommand::Write(bytes)) => out_buf.extend_from_slice(&bytes),
                        Ok(ChannelCommand::Resize(c, r)) => pending_resize = Some((c, r)),
                        Ok(ChannelCommand::Close) => want_close = true,
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            want_close = true;
                            break;
                        }
                    }
                }
                if want_close {
                    close_reason = String::from("本地关闭了会话");
                    break;
                }

                // ---- 2. 应用尺寸变更（EAGAIN 时下一轮继续）----
                if let Some((c, r)) = pending_resize {
                    let mut ch = io_channel.lock();
                    match ch.request_pty_size(c, r, None, None) {
                        Ok(()) => pending_resize = None,
                        Err(ref e) if matches!(e.code(), ssh2::ErrorCode::Session(-37)) => {}
                        Err(_) => pending_resize = None,
                    }
                }

                // ---- 3. 写（与读同线程，不存在并发访问）----
                if !out_buf.is_empty() {
                    let mut ch = io_channel.lock();
                    match ch.write(&out_buf) {
                        Ok(0) => {}
                        Ok(n) => {
                            out_buf.drain(..n);
                        }
                        Err(ref e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                            ) => {}
                        Err(_) => out_buf.clear(),
                    }
                }

                let read_res = {
                    let mut ch = io_channel.lock();
                    ch.read(&mut buf)
                };
                match read_res {
                    Ok(0) => {
                        close_reason = String::from("远端结束了会话");
                        break;
                    }
                    Ok(n) => {
                        consecutive_errors = 0;
                        pending.push_str(&decoder.decode(&buf[..n]));
                        if pending.len() >= 64 * 1024
                            || last_emit.elapsed() >= terminal_flush_interval(pending.len())
                        {
                            let event_name = format!("ssh-data-{}", sid);
                            crate::session_log::log_chunk(&sid, &pending);
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
                            crate::session_log::log_chunk(&sid, &pending);
                            let _ = app_handle.emit(&event_name, pending.clone());
                            pending.clear();
                            last_emit = std::time::Instant::now();
                        }
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    // 非阻塞模式下偶发的中断/超时不是致命错误：
                    // 高频输入（比如长按 Ctrl+←/→）时读写线程抢同一把锁，
                    // 旧逻辑会在这里直接 break，把整个会话当成断开处理。
                    Err(ref e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::Interrupted | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    Err(ref e) => {
                        consecutive_errors += 1;
                        if consecutive_errors > 200 {
                            close_reason = format!("读取失败: {}", e);
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
            }
            if !pending.is_empty() {
                crate::session_log::log_chunk(&sid, &pending);
                let _ = app_handle.emit(&format!("ssh-data-{}", sid), pending);
            }
            // 关闭通道（非阻塞下可能失败，忽略即可）
            {
                let mut ch = io_channel.lock();
                let _ = ch.close();
            }
            // 把断开原因一起带出去，前端能直接显示，省得只能看到「连接已断开」
            let _ = app_handle.emit(&format!("ssh-closed-{}", sid), close_reason);

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
    // 不再直接操作通道：排进 IO 线程的命令队列，由它串行写入。
    // 这样写永远和读在同一个线程里发生，libssh2 的 session 不会被并发访问。
    let tx = match state.sessions.lock().get(&session_id) {
        Some(active) => active.tx.clone(),
        None => return Err("SSH session not found".to_string()),
    };
    tx.send(ChannelCommand::Write(data.into_bytes()))
        .map_err(|_| "会话已关闭，无法写入".to_string())
}

#[tauri::command]
pub fn resize_ssh_session(
    state: tauri::State<'_, SshManager>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    // 尺寸变更同样排队，由 IO 线程在两次读取之间应用
    let tx = match state.sessions.lock().get(&session_id) {
        Some(active) => active.tx.clone(),
        None => return Err("SSH session not found".to_string()),
    };
    tx.send(ChannelCommand::Resize(cols, rows))
        .map_err(|_| "会话已关闭，无法调整尺寸".to_string())
}

#[tauri::command]
pub fn close_ssh_session(
    state: tauri::State<'_, SshManager>,
    session_id: String,
) -> Result<(), String> {
    crate::monitor::clear_session_caches(&session_id);
    if let Some(active) = state.sessions.lock().remove(&session_id) {
        // 通知 IO 线程收尾（它会关闭通道并清理会话）
        let _ = active.tx.send(ChannelCommand::Close);
        Ok(())
    } else {
        Ok(())
    }
}
