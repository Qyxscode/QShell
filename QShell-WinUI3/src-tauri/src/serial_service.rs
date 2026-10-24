use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use parking_lot::Mutex;
use serde::Deserialize;
use serialport::{DataBits, FlowControl, Parity, StopBits};
use tauri::{AppHandle, Emitter};

use crate::generic_terminal::generic_manager::GenericStreamManager;
use crate::generic_terminal::generic_session::{GenericSession, ProtocolType, Utf8ChunkDecoder};

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SerialOptions {
    pub data_bits: Option<u8>,       // 5, 6, 7, 8
    pub parity: Option<String>,      // "none", "odd", "even"
    pub stop_bits: Option<u8>,       // 1, 2
    pub flow_control: Option<String>, // "none", "software", "hardware"
    pub dtr: Option<bool>,
    pub rts: Option<bool>,
    pub newline: Option<String>,     // "crlf", "cr", "lf"
    pub local_echo: Option<bool>,
}

fn parse_serial_params(opts: &SerialOptions) -> (DataBits, Parity, StopBits, FlowControl) {
    let db = match opts.data_bits.unwrap_or(8) {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        _ => DataBits::Eight,
    };
    let par = match opts.parity.as_deref().unwrap_or("none") {
        "odd" => Parity::Odd,
        "even" => Parity::Even,
        _ => Parity::None,
    };
    let sb = match opts.stop_bits.unwrap_or(1) {
        2 => StopBits::Two,
        _ => StopBits::One,
    };
    let fc = match opts.flow_control.as_deref().unwrap_or("none") {
        "software" => FlowControl::Software,
        "hardware" => FlowControl::Hardware,
        _ => FlowControl::None,
    };
    (db, par, sb, fc)
}

#[tauri::command]
pub async fn start_serial_session(
    app: AppHandle,
    state: tauri::State<'_, GenericStreamManager>,
    session_id: String,
    port_name: String,
    baud_rate: u32,
    options: Option<SerialOptions>,
) -> Result<String, String> {
    let sid = session_id.clone();
    let opts = options.unwrap_or_default();
    let (db, par, sb, fc) = parse_serial_params(&opts);
    let dtr = opts.dtr.unwrap_or(true);
    let rts = opts.rts.unwrap_or(true);
    let newline_mode = opts.newline.unwrap_or_else(|| "cr".to_string());
    let local_echo = opts.local_echo.unwrap_or(false);
    let instance_id = state.next_instance_id();
    let stop_flag = Arc::new(AtomicBool::new(false));

    let mut port = serialport::new(&port_name, baud_rate)
        .data_bits(db)
        .parity(par)
        .stop_bits(sb)
        .flow_control(fc)
        .timeout(Duration::from_millis(100))
        .open()
        .map_err(|e| format!("打开串口 {} 失败: {}", port_name, e))?;

    let _ = port.write_data_terminal_ready(dtr);
    let _ = port.write_request_to_send(rts);

    let reader_port = port.try_clone().map_err(|e| format!("克隆串口句柄失败: {}", e))?;
    let writer_port = Arc::new(Mutex::new(Box::new(port) as Box<dyn Write + Send>));

    let sess = GenericSession {
        instance_id,
        protocol: ProtocolType::Serial,
        writer: writer_port,
        tcp_stream: None,
        stop_flag: stop_flag.clone(),
        telnet_codec: None,
        newline_mode,
        local_echo,
        pty_master: None,
        pty_child: None,
    };
    state.register_session(sid.clone(), sess);

    spawn_serial_reader_thread(app, state.inner().clone(), sid, instance_id, reader_port, stop_flag);
    Ok("串口连接成功".to_string())
}

fn spawn_serial_reader_thread(
    app: AppHandle,
    manager: GenericStreamManager,
    sid: String,
    instance_id: u64,
    mut reader: Box<dyn serialport::SerialPort>,
    stop_flag: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        let mut decoder = Utf8ChunkDecoder::new();

        while !stop_flag.load(Ordering::SeqCst) {
            match reader.read(&mut buf) {
                Ok(0) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(n) => {
                    let text = decoder.decode(&buf[..n]);
                    if !text.is_empty() {
                        crate::session_log::log_chunk(&sid, &text);
                        let _ = app.emit(&format!("ssh-data-{}", sid), text);
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    continue;
                }
                Err(_) => break,
            }
        }

        let _ = app.emit(&format!("ssh-closed-{}", sid), ());
        manager.cleanup_if_current(&sid, instance_id);
    });
}
