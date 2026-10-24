use std::io::Write;
use tauri::{AppHandle, Emitter, State};
use crate::generic_terminal::generic_manager::GenericStreamManager;
use crate::generic_terminal::generic_session::ProtocolType;

fn convert_serial_newlines(data: &[u8], newline_mode: &str) -> Vec<u8> {
    let mut bytes_to_write = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i] == b'\r' || data[i] == b'\n' {
            if data[i] == b'\r' && i + 1 < data.len() && data[i + 1] == b'\n' {
                i += 1;
            }
            match newline_mode {
                "crlf" => bytes_to_write.extend_from_slice(b"\r\n"),
                "lf" => bytes_to_write.push(b'\n'),
                _ => bytes_to_write.push(b'\r'),
            }
        } else {
            bytes_to_write.push(data[i]);
        }
        i += 1;
    }
    bytes_to_write
}

#[tauri::command]
pub fn write_generic_session(
    app: AppHandle,
    state: State<'_, GenericStreamManager>,
    session_id: String,
    data: String,
) -> Result<(), String> {
    let sess = {
        let map = state.sessions.lock();
        map.get(&session_id).cloned()
    };

    if let Some(sess) = sess {
        let mut bytes_to_write = Vec::new();
        match sess.protocol {
            ProtocolType::Telnet => {
                let bytes = data.as_bytes();
                let mut i = 0;
                while i < bytes.len() {
                    if bytes[i] == b'\r' {
                        if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                            bytes_to_write.extend_from_slice(b"\r\n");
                            i += 2;
                            continue;
                        } else {
                            bytes_to_write.extend_from_slice(b"\r\n");
                            i += 1;
                            continue;
                        }
                    } else if bytes[i] == 255 {
                        bytes_to_write.extend_from_slice(&[255, 255]);
                    } else {
                        bytes_to_write.push(bytes[i]);
                    }
                    i += 1;
                }
            }
            ProtocolType::Serial => {
                bytes_to_write = convert_serial_newlines(data.as_bytes(), &sess.newline_mode);
                if sess.local_echo {
                    let echo_text = String::from_utf8_lossy(&bytes_to_write).to_string();
                    let _ = app.emit(&format!("ssh-data-{}", session_id), echo_text);
                }
            }
            ProtocolType::Local => {
                // 本地 PTY 直接接收原始终端字节（由 ConPTY 处理回显与换行）
                bytes_to_write = data.as_bytes().to_vec();
            }
            ProtocolType::Raw | ProtocolType::Rlogin => {
                // 裸 TCP / Rlogin 直接透传原始字节
                bytes_to_write = data.as_bytes().to_vec();
            }
        }

        let mut w = sess.writer.lock();
        w.write_all(&bytes_to_write).map_err(|e| format!("写入失败: {}", e))?;
        w.flush().map_err(|e| format!("刷新失败: {}", e))?;
        Ok(())
    } else {
        Err("会话不存在".to_string())
    }
}

#[tauri::command]
pub fn resize_generic_session(
    state: State<'_, GenericStreamManager>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let sess = {
        let map = state.sessions.lock();
        map.get(&session_id).cloned()
    };

    if let Some(sess) = sess {
        if matches!(sess.protocol, ProtocolType::Local) {
            sess.resize_pty(cols, rows);
        }
        if let Some(subneg_bytes) = sess.resize_telnet(cols, rows) {
            let mut w = sess.writer.lock();
            let _ = w.write_all(&subneg_bytes);
            let _ = w.flush();
        }
        Ok(())
    } else {
        Err("会话不存在".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::convert_serial_newlines;

    #[test]
    fn serial_crlf_keeps_single_crlf() {
        assert_eq!(
            convert_serial_newlines(b"a\r\nb", "crlf"),
            b"a\r\nb".to_vec()
        );
    }

    #[test]
    fn serial_lf_mode_normalizes_bare_cr_and_lf() {
        assert_eq!(convert_serial_newlines(b"a\rb", "lf"), b"a\nb".to_vec());
        assert_eq!(convert_serial_newlines(b"a\nb", "lf"), b"a\nb".to_vec());
    }

    #[test]
    fn serial_cr_mode_normalizes_bare_lf() {
        assert_eq!(convert_serial_newlines(b"a\nb", "cr"), b"a\rb".to_vec());
    }

    #[test]
    fn serial_does_not_touch_other_bytes() {
        assert_eq!(
            convert_serial_newlines(&[0x00, 0x41, 0x7F], "crlf"),
            vec![0x00, 0x41, 0x7F]
        );
    }
}

#[tauri::command]
pub fn close_generic_session(
    state: State<'_, GenericStreamManager>,
    session_id: String,
) -> Result<(), String> {
    let sess = {
        let mut map = state.sessions.lock();
        map.remove(&session_id)
    };

    if let Some(sess) = sess {
        sess.stop();
        // 如果是 Telnet，写关闭信令让对端/读线程尽早唤醒并释放
        let mut w = sess.writer.lock();
        let _ = w.flush();
    }
    Ok(())
}
