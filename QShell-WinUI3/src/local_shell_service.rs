use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use parking_lot::Mutex;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tauri::{AppHandle, Emitter};

use crate::generic_terminal::generic_manager::GenericStreamManager;
use crate::generic_terminal::generic_session::{GenericSession, ProtocolType, Utf8ChunkDecoder};

/// 启动本地终端（PowerShell / CMD），底层使用 Windows ConPTY。
#[tauri::command]
pub async fn start_local_shell_session(
    app: AppHandle,
    state: tauri::State<'_, GenericStreamManager>,
    session_id: String,
    shell: String,
    cols: Option<u16>,
    rows: Option<u16>,
    cwd: Option<String>,
) -> Result<String, String> {
    let sid = session_id.clone();
    let term_cols = cols.unwrap_or(80).max(20);
    let term_rows = rows.unwrap_or(24).max(5);
    let instance_id = state.next_instance_id();
    let stop_flag = Arc::new(AtomicBool::new(false));

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: term_rows,
            cols: term_cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("创建伪终端失败: {}", e))?;

    let use_cmd = shell.eq_ignore_ascii_case("cmd");
    let mut cmd = if use_cmd {
        let mut c = CommandBuilder::new("cmd.exe");
        c.arg("/Q");
        c
    } else {
        let mut c = CommandBuilder::new("powershell.exe");
        c.arg("-NoLogo");
        c
    };
    if let Some(dir) = cwd.as_deref() {
        if !dir.trim().is_empty() {
            cmd.cwd(dir.trim());
        }
    }
    // 出错时不要继承宿主的控制台句柄
    cmd.env("TERM", "xterm-256color");

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("启动本地 Shell 失败: {}", e))?;
    drop(pair.slave);

    let master = pair.master;
    let reader = master
        .try_clone_reader()
        .map_err(|e| format!("读取伪终端失败: {}", e))?;
    let writer = master
        .take_writer()
        .map_err(|e| format!("写入伪终端失败: {}", e))?;

    let writer_arc = Arc::new(Mutex::new(Box::new(writer) as Box<dyn std::io::Write + Send>));
    let master_arc = Arc::new(Mutex::new(master));
    let child_arc = Arc::new(Mutex::new(child));

    let sess = GenericSession {
        instance_id,
        protocol: ProtocolType::Local,
        writer: writer_arc,
        tcp_stream: None,
        stop_flag: stop_flag.clone(),
        telnet_codec: None,
        // 本地 PTY 已经是原始终端，不做换行转换
        newline_mode: "raw".to_string(),
        local_echo: false,
        pty_master: Some(master_arc),
        pty_child: Some(child_arc),
    };
    state.register_session(sid.clone(), sess);

    spawn_local_reader_thread(app, state.inner().clone(), sid, instance_id, reader, stop_flag);

    Ok(if use_cmd {
        "CMD 已启动".to_string()
    } else {
        "PowerShell 已启动".to_string()
    })
}

fn spawn_local_reader_thread(    app: AppHandle,
    manager: GenericStreamManager,
    sid: String,
    instance_id: u64,
    mut reader: Box<dyn Read + Send>,
    stop_flag: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut decoder = Utf8ChunkDecoder::new();
        while !stop_flag.load(Ordering::SeqCst) {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let text = decoder.decode(&buf[..n]);
                    if !text.is_empty() {
                        let _ = app.emit(&format!("ssh-data-{}", sid), text);
                    }
                }
                Err(_) => break,
            }
        }
        let _ = app.emit(&format!("ssh-closed-{}", sid), ());
        manager.cleanup_if_current(&sid, instance_id);
    });
}
