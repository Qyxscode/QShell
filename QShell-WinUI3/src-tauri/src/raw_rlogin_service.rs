use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};

use crate::generic_terminal::generic_manager::GenericStreamManager;
use crate::generic_terminal::generic_session::{GenericSession, ProtocolType, Utf8ChunkDecoder};

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

fn register_session(
    state: &tauri::State<'_, GenericStreamManager>,
    session_id: &str,
    instance_id: u64,
    protocol: ProtocolType,
    stream: &TcpStream,
    stop_flag: Arc<AtomicBool>,
) -> Result<Arc<Mutex<Box<dyn Write + Send>>>, String> {
    let writer = stream
        .try_clone()
        .map_err(|e| format!("复制套接字失败: {}", e))?;
    let writer_arc: Arc<Mutex<Box<dyn Write + Send>>> =
        Arc::new(Mutex::new(Box::new(writer) as Box<dyn Write + Send>));
    let shutdown_stream = stream.try_clone().map_err(|e| format!("复制套接字失败: {}", e))?;

    let sess = GenericSession {
        instance_id,
        protocol,
        writer: writer_arc.clone(),
        tcp_stream: Some(shutdown_stream),
        stop_flag: stop_flag.clone(),
        telnet_codec: None,
        newline_mode: "raw".to_string(),
        local_echo: false,
        pty_master: None,
        pty_child: None,
    };
    state.register_session(session_id.to_string(), sess);
    Ok(writer_arc)
}

fn spawn_reader(
    app: AppHandle,
    manager: GenericStreamManager,
    sid: String,
    instance_id: u64,
    mut reader: TcpStream,
    stop_flag: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let _ = reader.set_read_timeout(Some(Duration::from_millis(200)));
        let mut buf = [0u8; 8192];
        let mut decoder = Utf8ChunkDecoder::new();
        loop {
            if stop_flag.load(Ordering::SeqCst) {
                break;
            }
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let text = decoder.decode(&buf[..n]);
                    if !text.is_empty() {
                        crate::session_log::log_chunk(&sid, &text);
                        let _ = app.emit(&format!("ssh-data-{}", sid), text);
                    }
                }
                Err(ref e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break,
            }
        }
        let _ = app.emit(&format!("ssh-closed-{}", sid), ());
        manager.cleanup_if_current(&sid, instance_id);
    });
}

/// Raw TCP：连接后直接透传原始字节（用于网络设备、调试端口等）。
#[tauri::command]
pub async fn start_raw_tcp_session(
    app: AppHandle,
    state: tauri::State<'_, GenericStreamManager>,
    session_id: String,
    host: String,
    port: u16,
) -> Result<String, String> {
    let sid = session_id.clone();
    let instance_id = state.next_instance_id();
    let stop_flag = Arc::new(AtomicBool::new(false));

    let stream = connect_tcp(&host, port)?;
    stream.set_nodelay(true).ok();

    register_session(&state, &sid, instance_id, ProtocolType::Raw, &stream, stop_flag.clone())?;
    spawn_reader(
        app,
        state.inner().clone(),
        sid,
        instance_id,
        stream.try_clone().map_err(|e| e.to_string())?,
        stop_flag,
    );
    Ok("Raw TCP 已连接".to_string())
}

/// Rlogin（RFC 1282）：发送 4 个 \0 结尾字段完成握手，然后透传。
#[tauri::command]
pub async fn start_rlogin_session(
    app: AppHandle,
    state: tauri::State<'_, GenericStreamManager>,
    session_id: String,
    host: String,
    port: Option<u16>,
    client_user: String,
    server_user: String,
    term_type: Option<String>,
) -> Result<String, String> {
    let sid = session_id.clone();
    let instance_id = state.next_instance_id();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let target_port = port.unwrap_or(513);

    let mut stream = connect_tcp(&host, target_port)?;
    stream.set_nodelay(true).ok();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;

    let term = term_type.unwrap_or_else(|| "xterm-256color".to_string());
    let client = if client_user.trim().is_empty() {
        "qshell".to_string()
    } else {
        client_user.trim().to_string()
    };
    let server = server_user.trim().to_string();

    // 客户端字段：client_user\0 server_user\0 terminal_type\0 terminal_speed\0
    for part in [client.as_str(), server.as_str(), term.as_str(), "38400"] {
        stream
            .write_all(part.as_bytes())
            .map_err(|e| format!("发送 Rlogin 握手失败: {}", e))?;
        stream
            .write_all(&[0])
            .map_err(|e| format!("发送 Rlogin 握手失败: {}", e))?;
    }

    // 服务器回 1 字节：0 表示成功，非 0 表示拒绝，随后是错误消息直到断开
    let mut ack = [0u8; 1];
    match stream.read_exact(&mut ack) {
        Ok(()) if ack[0] == 0 => {}
        Ok(()) => {
            let mut message = Vec::new();
            let _ = stream.read_to_end(&mut message);
            return Err(format!(
                "Rlogin 被服务器拒绝: {}",
                String::from_utf8_lossy(&message).trim()
            ));
        }
        Err(e) => return Err(format!("读取 Rlogin 握手响应失败: {}", e)),
    }
    let _ = stream.set_read_timeout(None);

    register_session(
        &state,
        &sid,
        instance_id,
        ProtocolType::Rlogin,
        &stream,
        stop_flag.clone(),
    )?;
    spawn_reader(
        app,
        state.inner().clone(),
        sid,
        instance_id,
        stream.try_clone().map_err(|e| e.to_string())?,
        stop_flag,
    );
    Ok(format!("Rlogin 已连接 {}@{}", server, host))
}
