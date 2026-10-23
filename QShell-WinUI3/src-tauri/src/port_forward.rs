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

    tcp.set_read_timeout(Some(Duration::from_millis(30)))
        .map_err(|e| e.to_string())?;
    tcp.set_write_timeout(Some(Duration::from_secs(15)))
        .map_err(|e| e.to_string())?;

    let mut buf = [0u8; 16 * 1024];
    let mut pending: Vec<u8> = Vec::new();

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }

        // 本地 -> 远端
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

        // 远端 -> 本地
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
    let _ = tcp.shutdown(std::net::Shutdown::Both);
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
