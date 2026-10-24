use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

pub struct FtpEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mode_str: String,
}

pub struct FtpSession {
    control: TcpStream,
    reader: BufReader<TcpStream>,
    pub host: String,
    pub port: u16,
}

#[derive(Default)]
pub struct FtpManager {
    pub sessions: Arc<Mutex<HashMap<String, FtpSession>>>,
}

impl FtpManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, String> {
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("解析地址失败: {}", e))?;
    let mut last = String::new();
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, Duration::from_secs(8)) {
            Ok(stream) => return Ok(stream),
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("连接 {}:{} 失败: {}", host, port, last))
}

impl FtpSession {
    pub fn connect_and_login(
        host: &str,
        port: u16,
        user: &str,
        pass: &str,
    ) -> Result<Self, String> {
        let control = connect_tcp(host, port)?;
        control
            .set_read_timeout(Some(Duration::from_secs(20)))
            .ok();
        control
            .set_write_timeout(Some(Duration::from_secs(20)))
            .ok();
        let reader = BufReader::new(
            control
                .try_clone()
                .map_err(|e| format!("复制 FTP 控制连接失败: {}", e))?,
        );

        let mut session = Self {
            control,
            reader,
            host: host.to_string(),
            port,
        };

        // 欢迎横幅
        let (code, msg) = session.read_response()?;
        if code != 220 {
            return Err(format!("FTP 服务器未就绪: {} {}", code, msg));
        }

        session.send(&format!("USER {}", user))?;
        let (code, _) = session.read_response()?;
        if code == 331 {
            session.send(&format!("PASS {}", pass))?;
            let (code, msg) = session.read_response()?;
            if code != 230 {
                return Err(format!("FTP 登录失败: {} {}", code, msg));
            }
        } else if code != 230 {
            return Err(format!("FTP 登录失败: {}", code));
        }

        // 二进制模式
        session.send("TYPE I")?;
        let (code, msg) = session.read_response()?;
        if code != 200 {
            return Err(format!("FTP 切换二进制模式失败: {} {}", code, msg));
        }
        Ok(session)
    }

    fn send(&mut self, cmd: &str) -> Result<(), String> {
        self.control
            .write_all(format!("{}\r\n", cmd).as_bytes())
            .map_err(|e| format!("发送 FTP 命令失败: {}", e))
    }

    fn read_response(&mut self) -> Result<(u16, String), String> {
        let mut first = String::new();
        self.reader
            .read_line(&mut first)
            .map_err(|e| format!("读取 FTP 响应失败: {}", e))?;
        let first = first.trim_end_matches(['\r', '\n']).to_string();
        if first.is_empty() {
            return Err("FTP 服务器关闭了连接".to_string());
        }
        let code: u16 = first
            .get(0..3)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("无法解析 FTP 响应: {}", first))?;
        // 多行响应：以 "code-" 开头，直到 "code " 结束
        let is_multiline = first.len() > 3 && first.as_bytes()[3] == b'-';
        let mut message = first;
        if is_multiline {
            loop {
                let mut line = String::new();
                self.reader
                    .read_line(&mut line)
                    .map_err(|e| format!("读取 FTP 响应失败: {}", e))?;
                let line = line.trim_end_matches(['\r', '\n']).to_string();
                message.push('\n');
                message.push_str(&line);
                if line.starts_with(&format!("{} ", code)) || line.is_empty() {
                    break;
                }
            }
        }
        Ok((code, message))
    }

    fn pasv(&mut self) -> Result<TcpStream, String> {
        self.send("PASV")?;
        let (code, msg) = self.read_response()?;
        if code != 227 {
            return Err(format!("FTP PASV 失败: {} {}", code, msg));
        }
        // 227 Entering Passive Mode (h1,h2,h3,h4,p1,p2)
        let start = msg.find('(').ok_or("PASV 响应格式错误")?;
        let end = msg.find(')').ok_or("PASV 响应格式错误")?;
        let nums: Vec<u16> = msg[start + 1..end]
            .split(',')
            .filter_map(|s| s.trim().parse::<u16>().ok())
            .collect();
        if nums.len() < 6 {
            return Err("PASV 响应缺少地址信息".to_string());
        }
        let ip = format!("{}.{}.{}.{}", nums[0], nums[1], nums[2], nums[3]);
        let port = nums[4] * 256 + nums[5];
        let data = connect_tcp(&ip, port)?;
        data.set_read_timeout(Some(Duration::from_secs(30))).ok();
        Ok(data)
    }

    pub fn list(&mut self, path: &str) -> Result<Vec<FtpEntry>, String> {
        // 先切换目录
        if !path.is_empty() && path != "." {
            self.send(&format!("CWD {}", path))?;
            let (code, msg) = self.read_response()?;
            if code != 250 {
                return Err(format!("进入目录失败: {} {}", code, msg));
            }
        }
        let mut data = self.pasv()?;
        self.send("LIST")?;
        let (code, msg) = self.read_response()?;
        if code != 150 && code != 125 {
            return Err(format!("LIST 失败: {} {}", code, msg));
        }
        let mut raw = Vec::new();
        data.read_to_end(&mut raw)
            .map_err(|e| format!("读取目录数据失败: {}", e))?;
        let (code, msg) = self.read_response()?;
        if code != 226 && code != 250 {
            return Err(format!("目录传输未完成: {} {}", code, msg));
        }
        Ok(parse_list_output(&raw))
    }

    pub fn download(&mut self, remote: &str, local: &Path) -> Result<u64, String> {
        let mut data = self.pasv()?;
        self.send(&format!("RETR {}", remote))?;
        let (code, msg) = self.read_response()?;
        if code != 150 && code != 125 {
            return Err(format!("下载失败: {} {}", code, msg));
        }
        let mut file = std::fs::File::create(local).map_err(|e| format!("创建本地文件失败: {}", e))?;
        let mut buf = [0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let n = data.read(&mut buf).map_err(|e| format!("读取下载数据失败: {}", e))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])
                .map_err(|e| format!("写入本地文件失败: {}", e))?;
            total += n as u64;
        }
        let (code, msg) = self.read_response()?;
        if code != 226 && code != 250 {
            return Err(format!("下载未完成: {} {}", code, msg));
        }
        Ok(total)
    }

    pub fn upload(&mut self, remote: &str, local: &Path) -> Result<u64, String> {
        let mut data = self.pasv()?;
        self.send(&format!("STOR {}", remote))?;
        let (code, msg) = self.read_response()?;
        if code != 150 && code != 125 {
            return Err(format!("上传失败: {} {}", code, msg));
        }
        let mut file = std::fs::File::open(local).map_err(|e| format!("打开本地文件失败: {}", e))?;
        let mut buf = [0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let n = file.read(&mut buf).map_err(|e| format!("读取本地文件失败: {}", e))?;
            if n == 0 {
                break;
            }
            data.write_all(&buf[..n])
                .map_err(|e| format!("写入上传数据失败: {}", e))?;
            total += n as u64;
        }
        drop(data);
        let (code, msg) = self.read_response()?;
        if code != 226 && code != 250 {
            return Err(format!("上传未完成: {} {}", code, msg));
        }
        Ok(total)
    }

    pub fn remove(&mut self, path: &str, is_dir: bool) -> Result<(), String> {
        self.send(&format!("{} {}", if is_dir { "RMD" } else { "DELE" }, path))?;
        let (code, msg) = self.read_response()?;
        if code != 250 && code != 200 {
            return Err(format!("删除失败: {} {}", code, msg));
        }
        Ok(())
    }

    pub fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        self.send(&format!("RNFR {}", from))?;
        let (code, msg) = self.read_response()?;
        if code != 350 {
            return Err(format!("重命名失败（源文件）: {} {}", code, msg));
        }
        self.send(&format!("RNTO {}", to))?;
        let (code, msg) = self.read_response()?;
        if code != 250 {
            return Err(format!("重命名失败: {} {}", code, msg));
        }
        Ok(())
    }

    pub fn mkdir(&mut self, path: &str) -> Result<(), String> {
        self.send(&format!("MKD {}", path))?;
        let (code, msg) = self.read_response()?;
        if code != 257 && code != 250 {
            return Err(format!("创建目录失败: {} {}", code, msg));
        }
        Ok(())
    }
    pub fn close(mut self) {
        let _ = self.send("QUIT");
    }
}

fn parse_list_output(raw: &[u8]) -> Vec<FtpEntry> {
    let text = String::from_utf8_lossy(raw);
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        // Unix 格式：drwxr-xr-x 2 ftp ftp 4096 Jan 01 12:00 name
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 9 {
            continue;
        }
        let perms = parts[0];
        if perms.len() < 10 {
            continue;
        }
        let is_dir = perms.starts_with('d');
        let size: u64 = parts[4].parse().unwrap_or(0);
        // 名字可能含空格：从第 9 个字段开始拼接
        let name = parts[8..].join(" ");
        if name == "." || name == ".." {
            continue;
        }
        entries.push(FtpEntry {
            name,
            is_dir,
            size,
            mode_str: perms.to_string(),
        });
    }
    entries
}

// ================= Tauri 命令 =================

#[tauri::command]
pub fn ftp_connect(
    state: tauri::State<'_, FtpManager>,
    session_id: String,
    host: String,
    port: u16,
    username: String,
    password: String,
) -> Result<(), String> {
    let session = FtpSession::connect_and_login(&host, port, &username, &password)?;
    let mut map = state.sessions.lock();
    if let Some(old) = map.remove(&session_id) {
        old.close();
    }
    map.insert(session_id, session);
    Ok(())
}

#[tauri::command]
pub fn ftp_list(
    state: tauri::State<'_, FtpManager>,
    session_id: String,
    path: String,
) -> Result<Vec<serde_json::Value>, String> {
    let mut map = state.sessions.lock();
    let session = map.get_mut(&session_id).ok_or("FTP 会话不存在")?;
    let entries = session.list(&path)?;
    Ok(entries
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "name": e.name,
                "is_dir": e.is_dir,
                "size": e.size,
                "mode_str": e.mode_str,
            })
        })
        .collect())
}

#[tauri::command]
pub fn ftp_download(
    state: tauri::State<'_, FtpManager>,
    session_id: String,
    remote_path: String,
    local_path: String,
) -> Result<u64, String> {
    let mut map = state.sessions.lock();
    let session = map.get_mut(&session_id).ok_or("FTP 会话不存在")?;
    session.download(&remote_path, Path::new(&local_path))
}

#[tauri::command]
pub fn ftp_close(state: tauri::State<'_, FtpManager>, session_id: String) -> Result<(), String> {
    let mut map = state.sessions.lock();
    if let Some(session) = map.remove(&session_id) {
        session.close();
    }
    Ok(())
}

#[tauri::command]
pub fn ftp_upload(
    state: tauri::State<'_, FtpManager>,
    session_id: String,
    remote_path: String,
    local_path: String,
) -> Result<u64, String> {
    let mut map = state.sessions.lock();
    let session = map.get_mut(&session_id).ok_or("FTP 会话不存在")?;
    session.upload(&remote_path, Path::new(&local_path))
}

#[tauri::command]
pub fn ftp_remove(
    state: tauri::State<'_, FtpManager>,
    session_id: String,
    remote_path: String,
    is_dir: bool,
) -> Result<(), String> {
    let mut map = state.sessions.lock();
    let session = map.get_mut(&session_id).ok_or("FTP 会话不存在")?;
    session.remove(&remote_path, is_dir)
}

#[tauri::command]
pub fn ftp_rename(
    state: tauri::State<'_, FtpManager>,
    session_id: String,
    from_path: String,
    to_path: String,
) -> Result<(), String> {
    let mut map = state.sessions.lock();
    let session = map.get_mut(&session_id).ok_or("FTP 会话不存在")?;
    session.rename(&from_path, &to_path)
}

#[tauri::command]
pub fn ftp_mkdir(
    state: tauri::State<'_, FtpManager>,
    session_id: String,
    remote_path: String,
) -> Result<(), String> {
    let mut map = state.sessions.lock();
    let session = map.get_mut(&session_id).ok_or("FTP 会话不存在")?;
    session.mkdir(&remote_path)
}