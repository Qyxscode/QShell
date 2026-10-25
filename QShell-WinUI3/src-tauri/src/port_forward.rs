use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tauri::State;

use crate::ssh::{SshConnectParams, SshManager};

pub struct ForwardTask {
    pub id: String,
    pub session_id: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub stop: Arc<AtomicBool>,
    pub params: SshConnectParams,
}

pub struct PortForwardManager {
    pub tasks: Mutex<HashMap<String, ForwardTask>>,
}

impl PortForwardManager {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
        }
    }
}

fn stop_task(manager: &PortForwardManager, forward_id: &str) {
    let mut tasks = manager.tasks.lock();
    if let Some(task) = tasks.remove(forward_id) {
        task.stop.store(true, Ordering::SeqCst);
    }
}

/// 临时诊断日志：写到 %TEMP%\qshell_fwd.log
fn fwd_log(message: &str) {
    use std::io::Write as _;
    let path = std::env::temp_dir().join("qshell_fwd.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{}", message);
    }
}

fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, String> {
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("解析地址失败: {}", e))?;
    let mut last_error = String::new();
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, Duration::from_secs(8)) {
            Ok(stream) => return Ok(stream),
            Err(e) => last_error = e.to_string(),
        }
    }
    Err(format!("连接 {}:{} 失败: {}", host, port, last_error))
}

struct PasswordPrompt(String);

impl ssh2::KeyboardInteractivePrompt for PasswordPrompt {
    fn prompt(
        &mut self,
        _username: &str,
        _instructions: &str,
        prompts: &[ssh2::Prompt<'_>],
    ) -> Vec<String> {
        prompts.iter().map(|_| self.0.clone()).collect()
    }
}

/// 为转发单独建立一条 SSH 连接（阻塞模式），避免影响终端会话的非阻塞状态。
fn open_forward_session(params: &SshConnectParams) -> Result<ssh2::Session, String> {
    let tcp = connect_tcp(&params.host, params.port)?;
    tcp.set_nodelay(true).ok();
    let mut sess = ssh2::Session::new().map_err(|e| format!("初始化 SSH 会话失败: {}", e))?;
    sess.set_tcp_stream(tcp);
    sess.set_timeout(15_000);
    sess.handshake()
        .map_err(|e| format!("转发连接握手失败: {}", e))?;

    let user = &params.username;
    let mut authenticated = false;

    if let Some(key) = params.private_key.as_deref().filter(|k| !k.trim().is_empty()) {
        let _ = sess.userauth_pubkey_file(
            user,
            None,
            std::path::Path::new(key),
            params.passphrase.as_deref(),
        );
        authenticated = sess.authenticated();
    }
    if !authenticated && params.use_agent.unwrap_or(false) {
        let _ = sess.userauth_agent(user);
        authenticated = sess.authenticated();
    }
    if !authenticated {
        if let Some(pwd) = params.password.as_deref().filter(|p| !p.is_empty()) {
            let _ = sess.userauth_password(user, pwd);
            authenticated = sess.authenticated();
        }
    }
    if !authenticated {
        if let Some(pwd) = params.password.as_deref().filter(|p| !p.is_empty()) {
            let mut handler = PasswordPrompt(pwd.to_string());
            let _ = sess.userauth_keyboard_interactive(user, &mut handler);
            authenticated = sess.authenticated();
        }
    }
    if !authenticated {
        return Err("转发连接认证失败（请检查密码或私钥）".to_string());
    }
    Ok(sess)
}

fn is_retryable(error: &ssh2::Error) -> bool {
    let message = error.message().to_lowercase();
    message.contains("timed out")
        || message.contains("would block")
        || message.contains("eagain")
        || message.contains("again")
}

fn is_retryable_io(error: &std::io::Error) -> bool {
    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) {
        return true;
    }
    let message = error.to_string().to_lowercase();
    message.contains("timed out")
        || message.contains("would block")
        || message.contains("eagain")
        || message.contains("again")
}

/// 一条本地连接：独立 SSH 连接 + 独立通道，短超时轮询双向转发。
fn forward_connection(
    mut tcp: TcpStream,
    params: &SshConnectParams,
    remote_host: &str,
    remote_port: u16,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let sess = open_forward_session(params)?;
    let mut channel = sess
        .channel_direct_tcpip(remote_host, remote_port, None)
        .map_err(|e| format!("建立转发通道失败: {}", e))?;
    // 建好通道后改为短超时，让两个方向都能轮询
    sess.set_timeout(30);
    pump_streams(tcp, channel, &stop);
    Ok(())
}

/// 启动一条本地端口转发。local_port 传 0 时由系统分配，返回实际监听端口。
#[tauri::command]
pub async fn start_local_forward(
    ssh: State<'_, SshManager>,
    fwd: State<'_, PortForwardManager>,
    forward_id: String,
    session_id: String,
    local_port: u16,
    remote_host: String,
    remote_port: u16,
) -> Result<u16, String> {
    if session_id.trim().is_empty() || remote_host.trim().is_empty() {
        return Err("会话或目标地址为空".to_string());
    }
    if remote_port == 0 {
        return Err("目标端口不能为 0".to_string());
    }

    let params = {
        let map = ssh.sessions.lock();
        let active = map
            .get(&session_id)
            .ok_or("SSH 会话不存在或已断开")?;
        active.params.clone()
    };

    fwd_log(&format!(
        "[start] id={} session={} local_port={} remote={}:{}",
        forward_id, session_id, local_port, remote_host, remote_port
    ));

    stop_task(&fwd, &forward_id);

    let listener = TcpListener::bind(("127.0.0.1", local_port))
        .map_err(|e| format!("监听本地端口 {} 失败: {}", local_port, e))?;
    fwd_log(&format!("[bind] ok, requested={}", local_port));
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("设置监听属性失败: {}", e))?;
    let actual_port = listener
        .local_addr()
        .map_err(|e| format!("读取监听端口失败: {}", e))?
        .port();
    fwd_log(&format!("[bind] actual local_addr port={}", actual_port));

    let stop = Arc::new(AtomicBool::new(false));
    let remote_host_trimmed = remote_host.trim().to_string();
    fwd.tasks.lock().insert(
        forward_id.clone(),
        ForwardTask {
            id: forward_id.clone(),
            session_id,
            local_port: actual_port,
            remote_host: remote_host_trimmed.clone(),
            remote_port,
            stop: stop.clone(),
            params: params.clone(),
        },
    );

    let accept_stop = stop.clone();
    std::thread::spawn(move || {
        fwd_log(&format!("[accept] thread started, port={}", actual_port));
        while !accept_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((tcp, _addr)) => {
                    fwd_log(&format!("[accept] connection from {}", _addr));
                    let params = params.clone();
                    let conn_stop = accept_stop.clone();
                    let host = remote_host_trimmed.clone();
                    std::thread::spawn(move || {
                        match forward_connection(tcp, &params, &host, remote_port, conn_stop) {
                            Ok(()) => fwd_log("[conn] closed normally"),
                            Err(e) => fwd_log(&format!("[conn] error: {}", e)),
                        }
                    });
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(30));
                }
                Err(e) => {
                    fwd_log(&format!("[accept] fatal error: {}", e));
                    break;
                }
            }
        }
        fwd_log("[accept] thread exited");
    });

    Ok(actual_port)
}

#[tauri::command]
pub fn stop_local_forward(
    fwd: State<'_, PortForwardManager>,
    forward_id: String,
) -> Result<(), String> {
    stop_task(&fwd, &forward_id);
    Ok(())
}

#[derive(serde::Serialize)]
pub struct ForwardInfo {
    pub id: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
}

#[tauri::command]
pub fn list_running_forwards(fwd: State<'_, PortForwardManager>) -> Vec<ForwardInfo> {
    fwd.tasks
        .lock()
        .values()
        .map(|task| ForwardInfo {
            id: task.id.clone(),
            local_port: task.local_port,
            remote_host: task.remote_host.clone(),
            remote_port: task.remote_port,
        })
        .collect()
}


// ================= 远程端口转发 (-R) =================

pub struct RemoteForwardTask {
    pub id: String,
    pub session_id: String,
    pub remote_bind_port: u16,
    pub local_host: String,
    pub local_port: u16,
    pub stop: Arc<AtomicBool>,
    pub handle: Option<std::thread::JoinHandle<()>>,
}

pub struct RemoteForwardManager {
    pub tasks: Mutex<HashMap<String, RemoteForwardTask>>,
}

impl RemoteForwardManager {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
        }
    }
}

fn stop_remote_task(manager: &RemoteForwardManager, forward_id: &str) {
    let task = {
        let mut tasks = manager.tasks.lock();
        tasks.remove(forward_id)
    };
    if let Some(task) = task {
        task.stop.store(true, Ordering::SeqCst);
        // 等线程真正退出（会触发远端监听取消 + 会话关闭），保证端口立即释放
        if let Some(handle) = task.handle {
            let _ = handle.join();
        }
    }
}

/// 远端连接进来后，在通道与本地目标之间双向转发（串行处理）。
fn pump_remote_channel(
    channel: &mut ssh2::Channel,
    tcp: &mut TcpStream,
    stop: &AtomicBool,
) {
    tcp.set_read_timeout(Some(Duration::from_millis(30))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(15))).ok();

    let mut buf = [0u8; 16 * 1024];
    let mut pending: Vec<u8> = Vec::new();

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }

        // 远端通道 -> 本地目标
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

        // 本地目标 -> 远端通道
        match tcp.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => pending.extend_from_slice(&buf[..n]),
            Err(ref e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
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
    }

    let _ = channel.close();
    let _ = tcp.shutdown(std::net::Shutdown::Both);
}

/// 启动一条远程端口转发：请求远端监听 remote_bind_port，连接转发到本地 local_host:local_port。
#[tauri::command]
pub async fn start_remote_forward(
    ssh: State<'_, SshManager>,
    fwd: State<'_, RemoteForwardManager>,
    forward_id: String,
    session_id: String,
    remote_bind_port: u16,
    local_host: String,
    local_port: u16,
) -> Result<u16, String> {
    if session_id.trim().is_empty() {
        return Err("会话为空".to_string());
    }
    if remote_bind_port == 0 {
        return Err("远端监听端口不能为 0".to_string());
    }
    if local_host.trim().is_empty() || local_port == 0 {
        return Err("本地目标地址或端口无效".to_string());
    }

    let params = {
        let map = ssh.sessions.lock();
        let active = map
            .get(&session_id)
            .ok_or("SSH 会话不存在或已断开")?;
        active.params.clone()
    };

    stop_remote_task(&fwd, &forward_id);

    let stop = Arc::new(AtomicBool::new(false));
    let local_host_trimmed = local_host.trim().to_string();
    let (tx, rx) = std::sync::mpsc::channel::<Result<u16, String>>();

    fwd.tasks.lock().insert(
        forward_id.clone(),
        RemoteForwardTask {
            id: forward_id.clone(),
            session_id,
            remote_bind_port,
            local_host: local_host_trimmed.clone(),
            local_port,
            stop: stop.clone(),
            handle: None,
        },
    );

    let task_stop = stop.clone();
    let thread_handle = std::thread::spawn(move || {
        let sess = match open_forward_session(&params) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };

        let (mut listener, bound_port) = match sess.channel_forward_listen(remote_bind_port, None, None) {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(Err(format!("远端监听失败（端口可能已被占用，或服务器未开启 AllowTcpForwarding）：{}", e)));
                return;
            }
        };
        let _ = tx.send(Ok(bound_port));

        // 短超时轮询远端连接
        sess.set_timeout(200);

        while !task_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok(mut channel) => {
                    match connect_tcp(&local_host_trimmed, local_port) {
                        Ok(mut tcp) => {
                            fwd_log(&format!(
                                "[remote] inbound -> {}:{}",
                                local_host_trimmed, local_port
                            ));
                            pump_remote_channel(&mut channel, &mut tcp, &task_stop);
                        }
                        Err(e) => {
                            fwd_log(&format!("[remote] local target connect failed: {}", e));
                        }
                    }
                }
                Err(ref e) if is_retryable(&e) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    fwd_log(&format!("[remote] accept fatal: {}", e));
                    break;
                }
            }
        }
        fwd_log("[remote] listener thread exited");
    });
    {
        let mut map = fwd.tasks.lock();
        if let Some(task) = map.get_mut(&forward_id) {
            task.handle = Some(thread_handle);
        }
    }

    match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(Ok(port)) => Ok(port),
        Ok(Err(e)) => {
            stop_remote_task(&fwd, &forward_id);
            Err(e)
        }
        Err(_) => {
            stop_remote_task(&fwd, &forward_id);
            Err("远端监听超时（服务器可能不允许远程转发）".to_string())
        }
    }
}

#[tauri::command]
pub fn stop_remote_forward(
    fwd: State<'_, RemoteForwardManager>,
    forward_id: String,
) -> Result<(), String> {
    stop_remote_task(&fwd, &forward_id);
    Ok(())
}

#[derive(serde::Serialize)]
pub struct RemoteForwardInfo {
    pub id: String,
    pub remote_bind_port: u16,
    pub local_host: String,
    pub local_port: u16,
}

#[tauri::command]
pub fn list_remote_forwards(fwd: State<'_, RemoteForwardManager>) -> Vec<RemoteForwardInfo> {
    fwd.tasks
        .lock()
        .values()
        .map(|task| RemoteForwardInfo {
            id: task.id.clone(),
            remote_bind_port: task.remote_bind_port,
            local_host: task.local_host.clone(),
            local_port: task.local_port,
        })
        .collect()
}


// ================= 动态转发 SOCKS5 (-D) =================

pub struct SocksForwardTask {
    pub id: String,
    pub session_id: String,
    pub local_port: u16,
    pub stop: Arc<AtomicBool>,
    pub handle: Option<std::thread::JoinHandle<()>>,
}

pub struct SocksForwardManager {
    pub tasks: Mutex<HashMap<String, SocksForwardTask>>,
}

impl SocksForwardManager {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
        }
    }
}

fn stop_socks_task(manager: &SocksForwardManager, forward_id: &str) {
    let task = {
        let mut tasks = manager.tasks.lock();
        tasks.remove(forward_id)
    };
    if let Some(task) = task {
        task.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = task.handle {
            let _ = handle.join();
        }
    }
}

/// 读取 SOCKS5 请求头：返回 (目标地址, 目标端口)。
fn read_socks5_request(tcp: &mut TcpStream) -> Option<(String, u16)> {
    fwd_log("[socks] client connected");
    // 握手：VER NMETHODS METHODS...
    let mut header = [0u8; 2];
    if let Err(e) = tcp.read_exact(&mut header) {
        fwd_log(&format!("[socks] read handshake failed: {}", e));
        return None;
    }
    fwd_log(&format!("[socks] handshake ver={} methods={}", header[0], header[1]));
    if header[0] != 0x05 {
        fwd_log("[socks] not socks5");
        return None;
    }
    let method_count = header[1] as usize;
    if method_count == 0 {
        return None;
    }
    let mut methods = vec![0u8; method_count];
    tcp.read_exact(&mut methods).ok()?;
    // 选择"无认证"
    if tcp.write_all(&[0x05, 0x00]).is_err() {
        fwd_log("[socks] write method failed");
        return None;
    }
    fwd_log("[socks] no-auth accepted");

    // 请求：VER CMD RSV ATYP
    let mut req = [0u8; 4];
    if let Err(e) = tcp.read_exact(&mut req) {
        fwd_log(&format!("[socks] read request failed: {}", e));
        return None;
    }
    fwd_log(&format!("[socks] request ver={} cmd={} atyp={}", req[0], req[1], req[3]));
    if req[0] != 0x05 || req[1] != 0x01 {
        // 只支持 CONNECT
        let _ = tcp.write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
        return None;
    }
    let host = match req[3] {
        0x01 => {
            let mut addr = [0u8; 4];
            tcp.read_exact(&mut addr).ok()?;
            format!("{}.{}.{}.{}", addr[0], addr[1], addr[2], addr[3])
        }
        0x03 => {
            let mut len = [0u8; 1];
            tcp.read_exact(&mut len).ok()?;
            let mut domain = vec![0u8; len[0] as usize];
            tcp.read_exact(&mut domain).ok()?;
            String::from_utf8_lossy(&domain).to_string()
        }
        0x04 => {
            let mut addr = [0u8; 16];
            tcp.read_exact(&mut addr).ok()?;
            std::net::Ipv6Addr::from(addr).to_string()
        }
        _ => return None,
    };
    let mut port_buf = [0u8; 2];
    tcp.read_exact(&mut port_buf).ok()?;
    let port = u16::from_be_bytes(port_buf);
    fwd_log(&format!("[socks] target {}:{}", host, port));
    Some((host, port))
}

/// 处理一个 SOCKS5 客户端连接：握手后通过独立 SSH 连接转发到目标。
fn handle_socks5_client(
    mut tcp: TcpStream,
    params: &SshConnectParams,
    stop: &AtomicBool,
) {
    let Some((target_host, target_port)) = read_socks5_request(&mut tcp) else {
        return;
    };

    // 关键：先建立 SSH 连接与目标通道，成功后才回复 SOCKS5 成功；失败回复错误码
    let sess = match open_forward_session(params) {
        Ok(s) => s,
        Err(e) => {
            fwd_log(&format!("[socks] ssh connect failed: {}", e));
            let _ = tcp.write_all(&[0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
            return;
        }
    };
    let mut channel = match sess.channel_direct_tcpip(&target_host, target_port, None) {
        Ok(c) => c,
        Err(e) => {
            fwd_log(&format!("[socks] direct-tcpip failed: {} (target {}:{})", e, target_host, target_port));
            let _ = tcp.write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
            return;
        }
    };
    fwd_log(&format!("[socks] ok -> {}:{}", target_host, target_port));
    if tcp.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).is_err() {
        return;
    }
    sess.set_timeout(30);
    pump_streams(tcp, channel, stop);
}

/// 启动 SOCKS5 动态转发：本地监听 local_port，所有连接经 SSH 拉到远端目标。
#[tauri::command]
pub async fn start_socks_forward(
    ssh: State<'_, SshManager>,
    fwd: State<'_, SocksForwardManager>,
    forward_id: String,
    session_id: String,
    local_port: u16,
) -> Result<u16, String> {
    if session_id.trim().is_empty() {
        return Err("会话为空".to_string());
    }
    let params = {
        let map = ssh.sessions.lock();
        let active = map.get(&session_id).ok_or("SSH 会话不存在或已断开")?;
        active.params.clone()
    };

    stop_socks_task(&fwd, &forward_id);

    let listener = TcpListener::bind(("127.0.0.1", local_port))
        .map_err(|e| format!("监听本地端口 {} 失败: {}", local_port, e))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("设置监听属性失败: {}", e))?;
    let actual_port = listener
        .local_addr()
        .map_err(|e| format!("读取监听端口失败: {}", e))?
        .port();

    let stop = Arc::new(AtomicBool::new(false));
    fwd.tasks.lock().insert(
        forward_id.clone(),
        SocksForwardTask {
            id: forward_id.clone(),
            session_id,
            local_port: actual_port,
            stop: stop.clone(),
            handle: None,
        },
    );

    let accept_stop = stop.clone();
    let handle = std::thread::spawn(move || {
        while !accept_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((tcp, _addr)) => {
                    let params = params.clone();
                    let conn_stop = accept_stop.clone();
                    std::thread::spawn(move || {
                        handle_socks5_client(tcp, &params, &conn_stop);
                    });
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(30));
                }
                Err(_) => break,
            }
        }
    });
    if let Some(task) = fwd.tasks.lock().get_mut(&forward_id) {
        task.handle = Some(handle);
    }

    Ok(actual_port)
}

#[tauri::command]
pub fn stop_socks_forward(
    fwd: State<'_, SocksForwardManager>,
    forward_id: String,
) -> Result<(), String> {
    stop_socks_task(&fwd, &forward_id);
    Ok(())
}

#[derive(serde::Serialize)]
pub struct SocksForwardInfo {
    pub id: String,
    pub local_port: u16,
}

#[tauri::command]
pub fn list_socks_forwards(fwd: State<'_, SocksForwardManager>) -> Vec<SocksForwardInfo> {
    fwd.tasks
        .lock()
        .values()
        .map(|task| SocksForwardInfo {
            id: task.id.clone(),
            local_port: task.local_port,
        })
        .collect()
}


/// 双向转发：本地 TCP <-> SSH 通道。本地转发、远程转发、SOCKS5 共用同一实现。
fn pump_streams(mut tcp: TcpStream, mut channel: ssh2::Channel, stop: &AtomicBool) {
    tcp.set_read_timeout(Some(Duration::from_millis(30))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(15))).ok();

    let mut buf = [0u8; 16 * 1024];
    let mut pending: Vec<u8> = Vec::new();

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }

        // 本地 -> 通道
        match tcp.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => pending.extend_from_slice(&buf[..n]),
            Err(ref e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
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

        // 通道 -> 本地
        match channel.read(&mut buf) {
            Ok(0) => {
                if channel.eof() {
                    fwd_log("[pump] channel eof");
                    break;
                }
            }
            Ok(n) => {
                if tcp.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
            Err(e) => {
                if channel.eof() {
                    fwd_log(&format!("[pump] channel closed: {}", e));
                    break;
                }
            }
        }
    }

    let _ = channel.close();
    let _ = tcp.shutdown(std::net::Shutdown::Both);
}
