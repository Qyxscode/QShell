use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};

use crate::generic_terminal::generic_manager::GenericStreamManager;
use crate::generic_terminal::generic_session::{GenericSession, ProtocolType, Utf8ChunkDecoder};
use crate::generic_terminal::telnet_codec::TelnetCodec;

#[tauri::command]
pub async fn start_telnet_session(
    app: AppHandle,
    state: tauri::State<'_, GenericStreamManager>,
    session_id: String,
    host: String,
    port: u16,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<String, String> {
    let sid = session_id.clone();
    let addr_str = if host.contains(':') && !host.starts_with('[') {
        format!("[{}]:{}", host, port)
    } else {
        format!("{}:{}", host, port)
    };

    let instance_id = state.next_instance_id();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let term_cols = cols.unwrap_or(80);
    let term_rows = rows.unwrap_or(24);
    let codec = Arc::new(Mutex::new(TelnetCodec::new(term_cols, term_rows)));

    let socket_addr = addr_str
        .to_socket_addrs()
        .map_err(|e| format!("解析地址失败 ({}): {}", addr_str, e))?
        .next()
        .ok_or_else(|| "无法获取有效套接字地址".to_string())?;

    let stream = tokio::task::spawn_blocking(move || {
        TcpStream::connect_timeout(&socket_addr, Duration::from_secs(6))
            .map_err(|e| format!("连接 Telnet 超时或失败: {}", e))
    })
    .await
    .map_err(|e| format!("调度连接任务失败: {}", e))??;

    stream.set_nodelay(true).ok();
    let reader_stream = stream.try_clone().map_err(|e| format!("克隆套接字失败: {}", e))?;
    let shutdown_stream = stream.try_clone().ok();
    let writer_stream = Arc::new(Mutex::new(Box::new(stream) as Box<dyn Write + Send>));

    // 主动发送初始协商：声明支持 TTYPE, NAWS 并抑制 Go Ahead
    {
        let mut w = writer_stream.lock();
        let init_negotiations = [
            crate::generic_terminal::telnet_codec::IAC, crate::generic_terminal::telnet_codec::WILL, crate::generic_terminal::telnet_codec::OPT_TTYPE,
            crate::generic_terminal::telnet_codec::IAC, crate::generic_terminal::telnet_codec::WILL, crate::generic_terminal::telnet_codec::OPT_NAWS,
            crate::generic_terminal::telnet_codec::IAC, crate::generic_terminal::telnet_codec::WILL, crate::generic_terminal::telnet_codec::OPT_SGA,
            crate::generic_terminal::telnet_codec::IAC, crate::generic_terminal::telnet_codec::DO, crate::generic_terminal::telnet_codec::OPT_SGA,
        ];
        let _ = w.write_all(&init_negotiations);
        let _ = w.flush();
    }

    let sess = GenericSession {
        instance_id,
        protocol: ProtocolType::Telnet,
        writer: writer_stream.clone(),
        tcp_stream: shutdown_stream,
        stop_flag: stop_flag.clone(),
        telnet_codec: Some(codec.clone()),
        newline_mode: "crlf".to_string(),
        local_echo: false,
    };
    state.register_session(sid.clone(), sess);

    spawn_telnet_reader_thread(app, state.inner().clone(), sid, instance_id, reader_stream, writer_stream, codec, stop_flag);
    Ok("Telnet 连接成功".to_string())
}

fn spawn_telnet_reader_thread(
    app: AppHandle,
    manager: GenericStreamManager,
    sid: String,
    instance_id: u64,
    mut reader: TcpStream,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    codec: Arc<Mutex<TelnetCodec>>,
    stop_flag: Arc<AtomicBool>,
) {
    let _ = reader.set_read_timeout(Some(Duration::from_millis(200)));
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut decoder = Utf8ChunkDecoder::new();
        let mut reply = Vec::with_capacity(128);

        while !stop_flag.load(Ordering::SeqCst) {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    reply.clear();
                    let clean_bytes = {
                        let mut c = codec.lock();
                        c.process_input(&buf[..n], &mut reply)
                    };

                    if !reply.is_empty() {
                        let mut w = writer.lock();
                        let _ = w.write_all(&reply);
                        let _ = w.flush();
                    }

                    if !clean_bytes.is_empty() {
                        let text = decoder.decode(&clean_bytes);
                        if !text.is_empty() {
                            let _ = app.emit(&format!("ssh-data-{}", sid), text);
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock => {
                    continue;
                }
                Err(_) => break,
            }
        }

        let _ = app.emit(&format!("ssh-closed-{}", sid), ());
        manager.cleanup_if_current(&sid, instance_id);
    });
}
