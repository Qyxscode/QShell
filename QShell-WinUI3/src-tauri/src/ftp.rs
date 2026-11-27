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

/// FTP 会话表。
///
/// 每个会话单独包一层 Mutex：FTP 是单连接串行协议，同一会话的操作必须排队，
/// 但不同会话之间不该互相阻塞。全局表只在查表/增删时短暂加锁。
#[derive(Default)]
pub struct FtpManager {
    pub sessions: Arc<Mutex<HashMap<String, Arc<Mutex<FtpSession>>>>>,
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

/// 远端临时文件后缀：时间戳 + 进程内自增序号，避免同一毫秒内并发上传撞名。
fn next_temp_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{}-{}", ts, seq)
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
        // 先落到 `.qshell-part`，完整收完再替换目标文件：
        // 中途断线时原来的同名文件不会被截成半截，与 SFTP 的下载语义保持一致。
        let mut temp_os = local.as_os_str().to_os_string();
        temp_os.push(".qshell-part");
        let temp = std::path::PathBuf::from(temp_os);

        // 上次进程在「旧文件已移开、新文件未就位」之间中断时，先把原文件恢复回去，
        // 否则用户的原文件会凭空消失，只剩一个没人认得的 .qshell-old。
        let mut backup_os = local.as_os_str().to_os_string();
        backup_os.push(".qshell-old");
        let backup = std::path::PathBuf::from(backup_os);
        if !local.exists() && backup.exists() {
            let _ = std::fs::rename(&backup, local);
        }

        match self.download_to(remote, &temp) {
            Ok(total) => {
                crate::sftp::replace_local_file(&temp, local)?;
                Ok(total)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&temp);
                Err(e)
            }
        }
    }

    fn download_to(&mut self, remote: &str, target: &Path) -> Result<u64, String> {
        let mut data = self.pasv()?;
        self.send(&format!("RETR {}", remote))?;
        let (code, msg) = self.read_response()?;
        if code != 150 && code != 125 {
            return Err(format!("下载失败: {} {}", code, msg));
        }
        let mut file = std::fs::File::create(target).map_err(|e| format!("创建本地文件失败: {}", e))?;
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
        drop(data);
        let (code, msg) = self.read_response()?;
        if code != 226 && code != 250 {
            return Err(format!("下载未完成: {} {}", code, msg));
        }
        Ok(total)
    }

    pub fn upload(&mut self, remote: &str, local: &Path) -> Result<u64, String> {
        // 与 SFTP 一致：先传成远端临时文件，成功后再改名覆盖，
        // 避免传输中断把服务器上的原文件写成半截。
        let temp = format!("{}.qshell-tmp-{}", remote, next_temp_suffix());
        match self.upload_to(&temp, local) {
            Ok(total) => match self.rename(&temp, remote) {
                Ok(()) => Ok(total),
                // 有些 FTP 服务器不允许 RNTO 覆盖已存在的目标：删掉目标再试一次。
                // 第二次仍失败时保留临时文件，并在错误里给出文件名，避免数据丢失。
                Err(_) => {
                    let _ = self.remove(remote, false);
                    match self.rename(&temp, remote) {
                        Ok(()) => Ok(total),
                        Err(e) => Err(format!(
                            "上传已完成，但无法写入目标文件（{}）；内容保留在临时文件 {} 中",
                            e, temp
                        )),
                    }
                }
            },
            Err(e) => {
                let _ = self.remove(&temp, false);
                Err(e)
            }
        }
    }

    fn upload_to(&mut self, remote: &str, local: &Path) -> Result<u64, String> {
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
    /// 礼貌性地发 QUIT 并关闭控制连接。
    pub fn close(&mut self) {
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
//
// 全部是 async + spawn_blocking：FTP 是同步阻塞协议，一次 LIST/RETR 可能持续
// 数秒到数分钟。同步 Tauri 命令跑在主线程上，如果还持有全局会话锁做网络 IO，
// 会把整个界面连同其他会话一起卡住。现在全局锁只用于取会话，实际 IO 在阻塞
// 线程池里执行，且只锁该会话自身。

fn session_of(
    state: &tauri::State<'_, FtpManager>,
    session_id: &str,
) -> Result<Arc<Mutex<FtpSession>>, String> {
    state
        .sessions
        .lock()
        .get(session_id)
        .cloned()
        .ok_or_else(|| "FTP 会话不存在".to_string())
}

fn join_error(e: tokio::task::JoinError) -> String {
    format!("FTP 操作任务异常: {}", e)
}

#[tauri::command]
pub async fn ftp_connect(
    state: tauri::State<'_, FtpManager>,
    session_id: String,
    host: String,
    port: u16,
    username: String,
    password: String,
) -> Result<(), String> {
    let session = tokio::task::spawn_blocking(move || {
        FtpSession::connect_and_login(&host, port, &username, &password)
    })
    .await
    .map_err(join_error)??;

    let old = state
        .sessions
        .lock()
        .insert(session_id, Arc::new(Mutex::new(session)));
    if let Some(old) = old {
        // 旧连接的重连场景：QUIT 也要走网络，放到阻塞线程里做
        let _ = tokio::task::spawn_blocking(move || {
            if let Some(mut sess) = old.try_lock() {
                sess.close();
            }
        })
        .await;
    }
    Ok(())
}

#[tauri::command]
pub async fn ftp_list(
    state: tauri::State<'_, FtpManager>,
    session_id: String,
    path: String,
) -> Result<Vec<serde_json::Value>, String> {
    let session = session_of(&state, &session_id)?;
    tokio::task::spawn_blocking(move || {
        let mut sess = session.lock();
        let entries = sess.list(&path)?;
        Ok::<_, String>(
            entries
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "name": e.name,
                        "is_dir": e.is_dir,
                        "size": e.size,
                        "mode_str": e.mode_str,
                    })
                })
                .collect(),
        )
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn ftp_download(
    state: tauri::State<'_, FtpManager>,
    session_id: String,
    remote_path: String,
    local_path: String,
) -> Result<u64, String> {
    let session = session_of(&state, &session_id)?;
    tokio::task::spawn_blocking(move || {
        let mut sess = session.lock();
        sess.download(&remote_path, Path::new(&local_path))
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn ftp_close(
    state: tauri::State<'_, FtpManager>,
    session_id: String,
) -> Result<(), String> {
    let removed = state.sessions.lock().remove(&session_id);
    if let Some(session) = removed {
        let _ = tokio::task::spawn_blocking(move || {
            // 若仍有传输在跑，这里是排队等待传输结束；拿不到锁说明已被占用，
            // 结构随最后一个引用释放而关闭即可。
            if let Some(mut sess) = session.try_lock() {
                sess.close();
            }
        })
        .await;
    }
    Ok(())
}

#[tauri::command]
pub async fn ftp_upload(
    state: tauri::State<'_, FtpManager>,
    session_id: String,
    remote_path: String,
    local_path: String,
) -> Result<u64, String> {
    let session = session_of(&state, &session_id)?;
    tokio::task::spawn_blocking(move || {
        let mut sess = session.lock();
        sess.upload(&remote_path, Path::new(&local_path))
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn ftp_remove(
    state: tauri::State<'_, FtpManager>,
    session_id: String,
    remote_path: String,
    is_dir: bool,
) -> Result<(), String> {
    let session = session_of(&state, &session_id)?;
    tokio::task::spawn_blocking(move || {
        let mut sess = session.lock();
        sess.remove(&remote_path, is_dir)
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn ftp_rename(
    state: tauri::State<'_, FtpManager>,
    session_id: String,
    from_path: String,
    to_path: String,
) -> Result<(), String> {
    let session = session_of(&state, &session_id)?;
    tokio::task::spawn_blocking(move || {
        let mut sess = session.lock();
        sess.rename(&from_path, &to_path)
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn ftp_mkdir(
    state: tauri::State<'_, FtpManager>,
    session_id: String,
    remote_path: String,
) -> Result<(), String> {
    let session = session_of(&state, &session_id)?;
    tokio::task::spawn_blocking(move || {
        let mut sess = session.lock();
        sess.mkdir(&remote_path)
    })
    .await
    .map_err(join_error)?
}