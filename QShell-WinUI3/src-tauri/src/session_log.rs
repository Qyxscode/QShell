use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

static LOG_ENABLED: AtomicBool = AtomicBool::new(false);

/// 会话日志目录：%APPDATA%\QShell\logs
pub fn log_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("QShell").join("logs")
}

pub fn set_enabled(enabled: bool) {
    LOG_ENABLED.store(enabled, Ordering::SeqCst);
}

pub fn is_enabled() -> bool {
    LOG_ENABLED.load(Ordering::SeqCst)
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    while let Some(&next) = chars.peek() {
                        chars.next();
                        if ('\x40'..='\x7e').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '\x07' {
                            break;
                        }
                        if next == '\x1b' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            }
            continue;
        }
        if c == '\r' {
            continue;
        }
        out.push(c);
    }
    out
}
/// 把一段终端输出追加到该会话的日志文件。
pub fn log_chunk(session_id: &str, text: &str) {
    if !is_enabled() || text.is_empty() {
        return;
    }
    let dir = log_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let safe_id: String = session_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let path = dir.join(format!("{}.log", safe_id));
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let plain = strip_ansi(text);
        let _ = file.write_all(plain.as_bytes());
    }
}

#[tauri::command]
pub fn set_session_logging(enabled: bool) -> Result<(), String> {
    set_enabled(enabled);
    Ok(())
}

#[tauri::command]
pub fn get_session_logging() -> bool {
    is_enabled()
}
