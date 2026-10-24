use serde::{Deserialize, Serialize};
use std::path::Path;
use std::fs::File;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Mutex as StdMutex, OnceLock};
use std::collections::HashSet;
use std::time::Duration;
use tauri::Emitter;
use crate::ssh::{SftpHandle, SshConnectParams, SshManager};

static TRANSFER_CANCEL: OnceLock<StdMutex<HashSet<String>>> = OnceLock::new();

fn cancel_registry() -> &'static StdMutex<HashSet<String>> {
    TRANSFER_CANCEL.get_or_init(|| StdMutex::new(HashSet::new()))
}

fn is_transfer_cancelled(task_id: &str) -> bool {
    cancel_registry()
        .lock()
        .map(|set| set.contains(task_id))
        .unwrap_or(false)
}

fn clear_transfer_cancel(task_id: &str) {
    if let Ok(mut set) = cancel_registry().lock() {
        set.remove(task_id);
    }
}

#[tauri::command]
pub fn cancel_sftp_transfer(task_id: String) -> Result<(), String> {
    if let Ok(mut set) = cancel_registry().lock() {
        set.insert(task_id);
    }
    Ok(())
}

fn open_sftp_handle(params: &SshConnectParams) -> Result<SftpHandle, String> {
    let addrs = (params.host.as_str(), params.port)
        .to_socket_addrs()
        .map_err(|e| format!("操作失败: {}", e))?;

    let mut tcp = None;
    let mut last_error = String::new();
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, Duration::from_secs(8)) {
            Ok(stream) => {
                tcp = Some(stream);
                break;
            }
            Err(e) => last_error = e.to_string(),
        }
    }
    let tcp = tcp.ok_or_else(|| format!("操作失败: {}", last_error))?;
    let _ = tcp.set_nodelay(true);

    let mut sess = ssh2::Session::new().map_err(|e| format!("初始化 SFTP 会话失败: {}", e))?;
    sess.set_tcp_stream(tcp);
    sess.set_timeout(12_000);
    let _ = sess.method_pref(
        ssh2::MethodType::Kex,
        "ecdh-sha2-nistp256,ecdh-sha2-nistp384,ecdh-sha2-nistp521,diffie-hellman-group14-sha256,diffie-hellman-group16-sha512,diffie-hellman-group-exchange-sha256,diffie-hellman-group14-sha1",
    );
    let _ = sess.method_pref(
        ssh2::MethodType::HostKey,
        "rsa-sha2-512,rsa-sha2-256,ecdsa-sha2-nistp256,ecdsa-sha2-nistp384,ecdsa-sha2-nistp521,ssh-rsa",
    );
    sess.handshake().map_err(|e| format!("SFTP 握手失败: {}", e))?;
    let _ = sess.set_keepalive(true, 30);

    let mut authenticated = false;
    if let Some(ref key_path) = params.private_key {
        if !key_path.trim().is_empty() {
            let p = Path::new(key_path);
            if p.is_file() {
                authenticated = sess
                    .userauth_pubkey_file(&params.username, None, p, params.passphrase.as_deref())
                    .is_ok()
                    && sess.authenticated();
            }
        }
    }
    if !authenticated {
        if let Some(ref pwd) = params.password {
            let _ = sess.userauth_password(&params.username, pwd);
        }
    }
    if !sess.authenticated() {
        return Err("SFTP 认证失败，请检查用户名、密码或私钥".to_string());
    }

    let sftp = sess.sftp().map_err(|e| format!("创建 SFTP 通道失败: {}", e))?;
    Ok(SftpHandle {
        session: std::sync::Arc::new(parking_lot::Mutex::new(sess)),
        sftp: std::sync::Arc::new(parking_lot::Mutex::new(sftp)),
    })
}

pub(crate) fn ensure_sftp_handle(
    slot: &std::sync::Arc<parking_lot::Mutex<Option<SftpHandle>>>,
    params: &SshConnectParams,
) -> Result<SftpHandle, String> {
    let mut guard = slot.lock();
    if let Some(handle) = guard.as_ref() {
        return Ok(handle.clone());
    }
    let handle = open_sftp_handle(params)?;
    *guard = Some(handle.clone());
    Ok(handle)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SftpFileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: String,
    pub size_bytes: u64,
    pub mode_str: String,
    pub mode_oct: String,
    pub mtime: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileProperties {
    pub name: String,
    pub full_path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub mode_str: String,
    pub mode_oct: String,
    pub mtime_str: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TransferProgressPayload {
    pub task_id: String,
    pub filename: String,
    pub is_upload: bool,
    pub transferred: u64,
    pub total: u64,
    pub speed_bps: u64,
    pub status: String, // "running", "completed", "failed"
    pub error: Option<String>,
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn format_permission(perm: u32, is_dir: bool) -> String {
    let kind = match perm & 0o170000 {
        0o040000 => 'd',
        0o120000 => 'l',
        0o100000 => '-',
        0o060000 => 'b',
        0o020000 => 'c',
        0o140000 => 's',
        0o010000 => 'p',
        _ => if is_dir { 'd' } else { '-' },
    };
    let mode = perm & 0o7777;
    let mut out = String::with_capacity(10);
    out.push(kind);

    fn push_triplet(out: &mut String, bits: u32, special: bool, special_char: char) {
        out.push(if bits & 0o4 != 0 { 'r' } else { '-' });
        out.push(if bits & 0o2 != 0 { 'w' } else { '-' });
        let executable = bits & 0o1 != 0;
        if special {
            out.push(if executable {
                special_char
            } else {
                special_char.to_ascii_uppercase()
            });
        } else {
            out.push(if executable { 'x' } else { '-' });
        }
    }

    push_triplet(&mut out, (mode >> 6) & 0o7, mode & 0o4000 != 0, 's');
    push_triplet(&mut out, (mode >> 3) & 0o7, mode & 0o2000 != 0, 's');
    push_triplet(&mut out, mode & 0o7, mode & 0o1000 != 0, 't');
    out
}

pub fn with_sftp<T, F>(
    slot: &std::sync::Arc<parking_lot::Mutex<Option<SftpHandle>>>,
    params: &SshConnectParams,
    mut op: F,
) -> Result<T, String>
where
    F: FnMut(&ssh2::Sftp) -> Result<T, ssh2::Error>,
{
    let handle = ensure_sftp_handle(slot, params)?;
    let sftp = handle.sftp.lock();
    op(&sftp).map_err(|e| format!("SFTP 操作失败: {}", e))
}

#[tauri::command]
pub async fn sftp_home_dir(
    state: tauri::State<'_, SshManager>,
    session_id: String,
) -> Result<String, String> {
    let (sftp_slot, params) = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话未连接")?;
        (act.sftp.clone(), act.params.clone())
    };

    tokio::task::spawn_blocking(move || {
        let path = with_sftp(&sftp_slot, &params, |sftp| {
            sftp.realpath(Path::new("."))
        })?;
        Ok(path.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| format!("执行错误: {}", e))?
}

#[tauri::command]
pub async fn list_sftp_files(
    state: tauri::State<'_, SshManager>,
    session_id: String,
    remote_path: String,
) -> Result<Vec<SftpFileEntry>, String> {
    let (sftp_slot, params) = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话未连接")?;
        (act.sftp.clone(), act.params.clone())
    };

    tokio::task::spawn_blocking(move || {
        let p = Path::new(&remote_path);
        let handle = ensure_sftp_handle(&sftp_slot, &params)?;
        let sftp = handle.sftp.lock();
        let entries = sftp
            .readdir(p)
            .map_err(|e| format!("操作失败: {}", e))?;
        drop(sftp);

        let mut dirs = Vec::new();
        let mut files = Vec::new();

        for (path, stat) in entries {
            let fname = match path.file_name() {
                Some(name) => name.to_string_lossy().to_string(),
                None => continue,
            };
            if fname == "." || fname == ".." {
                continue;
            }

            let is_dir = stat.is_dir();
            let size_bytes = stat.size.unwrap_or(0);
            let size_str = if is_dir { "-".to_string() } else { format_bytes(size_bytes) };
            let perm = stat.perm.unwrap_or(if is_dir { 0o755 } else { 0o644 });
            let mode_str = format_permission(perm, is_dir);
            let mode_oct = format!("{:04o}", perm & 0o777);
            let mtime = stat.mtime.unwrap_or(0);

            let entry = SftpFileEntry {
                name: fname,
                is_dir,
                size: size_str,
                size_bytes,
                mode_str,
                mode_oct,
                mtime,
            };

            if is_dir {
                dirs.push(entry);
            } else {
                files.push(entry);
            }
        }

        dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        let mut result = dirs;
        result.extend(files);
        Ok(result)
    })
    .await
    .map_err(|e| format!("操作失败: {}", e))?
}

#[tauri::command]
pub async fn sftp_mkdir(
    state: tauri::State<'_, SshManager>,
    session_id: String,
    remote_path: String,
) -> Result<(), String> {
    let (sftp_slot, params) = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话未连接")?;
        (act.sftp.clone(), act.params.clone())
    };

    tokio::task::spawn_blocking(move || {
        with_sftp(&sftp_slot, &params, |sftp| {
            sftp.mkdir(Path::new(&remote_path), 0755)
        })
        .map_err(|e| format!("操作失败: {}", e))
    })
    .await
    .map_err(|e| format!("操作失败: {}", e))?
}

fn remove_dir_all(sftp: &ssh2::Sftp, p: &Path) -> Result<(), ssh2::Error> {
    for (entry_path, stat) in sftp.readdir(p)? {
        let fname = match entry_path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };
        if fname == "." || fname == ".." {
            continue;
        }
        if stat.is_dir() {
            remove_dir_all(sftp, &entry_path)?;
        } else {
            sftp.unlink(&entry_path)?;
        }
    }
    sftp.rmdir(p)
}

#[tauri::command]
pub async fn sftp_remove(
    state: tauri::State<'_, SshManager>,
    session_id: String,
    remote_path: String,
    is_dir: bool,
) -> Result<(), String> {
    let (sftp_slot, params) = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话未连接")?;
        (act.sftp.clone(), act.params.clone())
    };

    tokio::task::spawn_blocking(move || {
        let p = Path::new(&remote_path);
        with_sftp(&sftp_slot, &params, |sftp| {
            if is_dir {
                remove_dir_all(sftp, p)
            } else {
                sftp.unlink(p)
            }
        })
        .map_err(|e| format!("操作失败: {}", e))
    })
    .await
    .map_err(|e| format!("操作失败: {}", e))?
}

#[tauri::command]
pub async fn sftp_rename(
    state: tauri::State<'_, SshManager>,
    session_id: String,
    old_path: String,
    new_path: String,
) -> Result<(), String> {
    let (sftp_slot, params) = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话未连接")?;
        (act.sftp.clone(), act.params.clone())
    };

    tokio::task::spawn_blocking(move || {
        with_sftp(&sftp_slot, &params, |sftp| {
            sftp.rename(Path::new(&old_path), Path::new(&new_path), None)
        })
        .map_err(|e| format!("操作失败 {}", e))
    })
    .await
    .map_err(|e| format!("操作失败: {}", e))?
}

#[tauri::command]
pub async fn sftp_chmod(
    state: tauri::State<'_, SshManager>,
    session_id: String,
    remote_path: String,
    mode_oct: String,
) -> Result<(), String> {
    let (sftp_slot, params) = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话未连接")?;
        (act.sftp.clone(), act.params.clone())
    };

    let mode_val = u32::from_str_radix(&mode_oct, 8).map_err(|e| format!("权限八进制格式错误: {}", e))?;

    tokio::task::spawn_blocking(move || {
        let p = Path::new(&remote_path);
        with_sftp(&sftp_slot, &params, |sftp| {
            let mut stat = sftp.stat(p)?;
            stat.perm = Some(mode_val);
            sftp.setstat(p, stat)
        })
        .map_err(|e| format!("修改权限失败: {}", e))
    })
    .await
    .map_err(|e| format!("执行异常: {}", e))?
}

#[tauri::command]
pub async fn sftp_stat(
    state: tauri::State<'_, SshManager>,
    session_id: String,
    remote_path: String,
) -> Result<FileProperties, String> {
    let (sftp_slot, params) = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话未连接")?;
        (act.sftp.clone(), act.params.clone())
    };

    let remote_path_clone = remote_path.clone();
    tokio::task::spawn_blocking(move || {
        let p = Path::new(&remote_path_clone);
        let stat = with_sftp(&sftp_slot, &params, |sftp| {
            sftp.stat(p)
        })
        .map_err(|e| format!("操作失败 {}", e))?;

        let fname = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| remote_path_clone.clone());
        let is_dir = stat.is_dir();
        let size_bytes = stat.size.unwrap_or(0);
        let perm = stat.perm.unwrap_or(if is_dir { 0o755 } else { 0o644 });
        let mode_oct = format!("{:04o}", perm & 0o777);
        let mode_str = format_permission(perm, is_dir);
        let mtime = stat.mtime.unwrap_or(0);

        let mtime_str = if mtime > 0 {
            format!("{}", mtime)
        } else {
            "-".to_string()
        };

        Ok(FileProperties {
            name: fname,
            full_path: remote_path_clone,
            is_dir,
            size_bytes,
            mode_str,
            mode_oct,
            mtime_str,
        })
    })
    .await
    .map_err(|e| format!("操作失败: {}", e))?
}

#[tauri::command]
pub async fn sftp_upload_file(
    app: tauri::AppHandle,
    state: tauri::State<'_, SshManager>,
    session_id: String,
    local_path: String,
    remote_path: String,
    task_id: String,
    overwrite: Option<bool>,
) -> Result<(), String> {
    let params = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话未连接")?;
        act.params.clone()
    };

    let local_file_path = local_path.clone();
    let fname = Path::new(&local_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());

    tokio::task::spawn_blocking(move || {
        clear_transfer_cancel(&task_id);
        let mut local_file = File::open(&local_file_path).map_err(|e| format!("操作失败: {}", e))?;
        let total_size = local_file.metadata().map(|m| m.len()).unwrap_or(0);
        let temp_remote = format!("{}.qshell-tmp-{}", remote_path, task_id);

        let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
            task_id: task_id.clone(),
            filename: fname.clone(),
            is_upload: true,
            transferred: 0,
            total: total_size,
            speed_bps: 0,
            status: "running".into(),
            error: None,
        });

        let handle = open_sftp_handle(&params)?;
        // 任何失败路径都必须删除远端临时文件并把 failed 状态回传前端，
        // 否则远端目录会堆积 .qshell-tmp-*，前端任务还会永久停在 running。
        let fail_upload = |message: String, transferred: u64| -> Result<(), String> {
            {
                let sftp = handle.sftp.lock();
                let _ = sftp.unlink(Path::new(&temp_remote));
            }
            let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                task_id: task_id.clone(),
                filename: fname.clone(),
                is_upload: true,
                transferred,
                total: total_size,
                speed_bps: 0,
                status: "failed".into(),
                error: Some(message.clone()),
            });
            clear_transfer_cancel(&task_id);
            Err(message)
        };
        // 后端不再无条件覆盖：未明确选择"覆盖"时目标已存在就直接失败，
        // 避免前端检查与 rename 之间的 TOCTOU 窗口把别人的文件静默盖掉。
        if !overwrite.unwrap_or(false) {
            let exists = {
                let sftp = handle.sftp.lock();
                sftp.stat(Path::new(&remote_path)).is_ok()
            };
            if exists {
                return fail_upload("目标文件已存在（未选择覆盖）".to_string(), 0);
            }
        }
        if is_transfer_cancelled(&task_id) {
            let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                task_id: task_id.clone(),
                filename: fname.clone(),
                is_upload: true,
                transferred: 0,
                total: total_size,
                speed_bps: 0,
                status: "cancelled".into(),
                error: None,
            });
            clear_transfer_cancel(&task_id);
            return Ok(());
        }

        let mut remote_file = {
            let sftp = handle.sftp.lock();
            sftp.create(Path::new(&temp_remote))
                .map_err(|e| format!("操作失败: {}", e))?
        };

        let mut buffer = [0u8; 256 * 1024];
        let mut transferred: u64 = 0;
        let mut last_emit = std::time::Instant::now();
        let mut last_transferred: u64 = 0;

        loop {
            if is_transfer_cancelled(&task_id) {
                drop(remote_file);
                let sftp = handle.sftp.lock();
                let _ = sftp.unlink(Path::new(&temp_remote));
                let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                    task_id: task_id.clone(),
                    filename: fname.clone(),
                    is_upload: true,
                    transferred,
                    total: total_size,
                    speed_bps: 0,
                    status: "cancelled".into(),
                    error: None,
                });
                clear_transfer_cancel(&task_id);
                return Ok(());
            }
            let bytes_read = match local_file.read(&mut buffer) {
                Ok(n) => n,
                Err(e) => {
                    drop(remote_file);
                    return fail_upload(format!("上传失败: {}", e), transferred);
                }
            };
            if bytes_read == 0 {
                break;
            }

            let mut offset = 0;
            let mut stall_started = std::time::Instant::now();
            while offset < bytes_read {
                match remote_file.write(&buffer[offset..bytes_read]) {
                    Ok(0) => {
                        drop(remote_file);
                        return fail_upload("上传失败：远端未接收数据".to_string(), transferred);
                    }
                    Ok(n) => {
                        offset += n;
                        stall_started = std::time::Instant::now();
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if stall_started.elapsed() > std::time::Duration::from_secs(10) {
                            drop(remote_file);
                            return fail_upload("上传超时：远端长时间未接收数据".to_string(), transferred);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(e) => {
                        drop(remote_file);
                        return fail_upload(format!("上传失败: {}", e), transferred);
                    }
                }
            }

            transferred += bytes_read as u64;

            if last_emit.elapsed().as_millis() >= 200 || transferred == total_size {
                let elapsed = last_emit.elapsed().as_secs_f64();
                let speed = if elapsed > 0.0 {
                    ((transferred - last_transferred) as f64 / elapsed) as u64
                } else {
                    0
                };
                let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                    task_id: task_id.clone(),
                    filename: fname.clone(),
                    is_upload: true,
                    transferred,
                    total: total_size,
                    speed_bps: speed,
                    status: "running".into(),
                    error: None,
                });
                last_emit = std::time::Instant::now();
                last_transferred = transferred;
            }
        }

        drop(remote_file);
        {
            let sftp = handle.sftp.lock();
            if let Err(e) = sftp.rename(Path::new(&temp_remote), Path::new(&remote_path), None) {
                let _ = sftp.unlink(Path::new(&temp_remote));
                let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                    task_id: task_id.clone(),
                    filename: fname.clone(),
                    is_upload: true,
                    transferred,
                    total: total_size,
                    speed_bps: 0,
                    status: "failed".into(),
                    error: Some(format!("操作失败 {}", e)),
                });
                clear_transfer_cancel(&task_id);
                return Err(format!("操作失败 {}", e));
            }
        }

        // 操作失败
        let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
            task_id: task_id.clone(),
            filename: fname.clone(),
            is_upload: true,
            transferred: total_size,
            total: total_size,
            speed_bps: 0,
            status: "completed".into(),
            error: None,
        });
        clear_transfer_cancel(&task_id);

        Ok(())
    })
    .await
    .map_err(|e| format!("操作失败: {}", e))?
}

#[tauri::command]
pub async fn sftp_download_file(
    app: tauri::AppHandle,
    state: tauri::State<'_, SshManager>,
    session_id: String,
    remote_path: String,
    local_path: String,
    task_id: String,
) -> Result<(), String> {
    let params = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话未连接")?;
        act.params.clone()
    };

    let fname = Path::new(&remote_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());

    tokio::task::spawn_blocking(move || {
        clear_transfer_cancel(&task_id);
        let temp_local = format!("{}.qshell-part", local_path);
        let handle = open_sftp_handle(&params)?;
        // 下载失败同样要清理半截 .qshell-part 并回传 failed，否则前端任务会永久停在 running。
        let fail_download = |message: String, transferred: u64, total: u64| -> Result<(), String> {
            let _ = std::fs::remove_file(&temp_local);
            let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                task_id: task_id.clone(),
                filename: fname.clone(),
                is_upload: false,
                transferred,
                total,
                speed_bps: 0,
                status: "failed".into(),
                error: Some(message.clone()),
            });
            clear_transfer_cancel(&task_id);
            Err(message)
        };
        let p = Path::new(&remote_path);
        let stat = match {
            let sftp = handle.sftp.lock();
            sftp.stat(p)
        } {
            Ok(stat) => stat,
            Err(e) => return fail_download(format!("读取远端文件信息失败: {}", e), 0, 0),
        };
        let total_size = stat.size.unwrap_or(0);

        let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
            task_id: task_id.clone(),
            filename: fname.clone(),
            is_upload: false,
            transferred: 0,
            total: total_size,
            speed_bps: 0,
            status: "running".into(),
            error: None,
        });

        if is_transfer_cancelled(&task_id) {
            let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                task_id: task_id.clone(),
                filename: fname.clone(),
                is_upload: false,
                transferred: 0,
                total: total_size,
                speed_bps: 0,
                status: "cancelled".into(),
                error: None,
            });
            clear_transfer_cancel(&task_id);
            return Ok(());
        }

        let mut remote_file = match {
            let sftp = handle.sftp.lock();
            sftp.open(p)
        } {
            Ok(file) => file,
            Err(e) => return fail_download(format!("打开远端文件失败: {}", e), 0, total_size),
        };

        let mut local_file = match File::create(&temp_local) {
            Ok(file) => file,
            Err(e) => return fail_download(format!("创建本地文件失败: {}", e), 0, total_size),
        };
        let mut buffer = [0u8; 256 * 1024];
        let mut transferred: u64 = 0;
        let mut last_emit = std::time::Instant::now();
        let mut last_transferred: u64 = 0;

        loop {
            if is_transfer_cancelled(&task_id) {
                drop(remote_file);
                drop(local_file);
                let _ = std::fs::remove_file(&temp_local);
                let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                    task_id: task_id.clone(),
                    filename: fname.clone(),
                    is_upload: false,
                    transferred,
                    total: total_size,
                    speed_bps: 0,
                    status: "cancelled".into(),
                    error: None,
                });
                clear_transfer_cancel(&task_id);
                return Ok(());
            }
            let bytes_read = match remote_file.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                Err(e) => {
                    drop(remote_file);
                    drop(local_file);
                    return fail_download(format!("下载失败: {}", e), transferred, total_size);
                }
            };

            if let Err(e) = local_file.write_all(&buffer[..bytes_read]) {
                drop(remote_file);
                drop(local_file);
                return fail_download(format!("写入本地文件失败: {}", e), transferred, total_size);
            }
            transferred += bytes_read as u64;

            if last_emit.elapsed().as_millis() >= 200 || transferred == total_size {
                let elapsed = last_emit.elapsed().as_secs_f64();
                let speed = if elapsed > 0.0 {
                    ((transferred - last_transferred) as f64 / elapsed) as u64
                } else {
                    0
                };
                let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                    task_id: task_id.clone(),
                    filename: fname.clone(),
                    is_upload: false,
                    transferred,
                    total: total_size,
                    speed_bps: speed,
                    status: "running".into(),
                    error: None,
                });
                last_emit = std::time::Instant::now();
                last_transferred = transferred;
            }
        }

        drop(remote_file);
        drop(local_file);
        let _ = std::fs::remove_file(&local_path);
        if let Err(e) = std::fs::rename(&temp_local, &local_path) {
            let _ = std::fs::remove_file(&temp_local);
            let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                task_id: task_id.clone(),
                filename: fname.clone(),
                is_upload: false,
                transferred,
                total: total_size,
                speed_bps: 0,
                status: "failed".into(),
                error: Some(format!("操作失败 {}", e)),
            });
            clear_transfer_cancel(&task_id);
            return Err(format!("操作失败 {}", e));
        }

        let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
            task_id: task_id.clone(),
            filename: fname.clone(),
            is_upload: false,
            transferred: total_size,
            total: total_size,
            speed_bps: 0,
            status: "completed".into(),
            error: None,
        });
        clear_transfer_cancel(&task_id);

        Ok(())
    })
    .await
    .map_err(|e| format!("操作失败: {}", e))?
}

#[cfg(test)]
mod tests {
    use super::format_permission;

    #[test]
    fn formats_linux_permissions() {
        assert_eq!(format_permission(0o40775, true), "drwxrwxr-x");
        assert_eq!(format_permission(0o100644, false), "-rw-r--r--");
        assert_eq!(format_permission(0o100600, false), "-rw-------");
        assert_eq!(format_permission(0o104755, false), "-rwsr-xr-x");
        assert_eq!(format_permission(0o041777, true), "drwxrwxrwt");
        assert_eq!(format_permission(0o100000, false), "----------");
    }
}

// ===== SFTP 目录同步（镜像上传）=====

fn ensure_remote_dir(sftp: &ssh2::Sftp, path: &str) {
    let mut current = String::new();
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        current.push('/');
        current.push_str(segment);
        let _ = sftp.mkdir(Path::new(&current), 0o755);
    }
}

fn collect_local_files(dir: &Path, base: &Path, out: &mut Vec<(std::path::PathBuf, u64)>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("读取本地目录失败: {}", e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取目录项失败: {}", e))?;
        let path = entry.path();
        let meta = entry.metadata().map_err(|e| format!("读取文件属性失败: {}", e))?;
        if meta.is_dir() {
            collect_local_files(&path, base, out)?;
        } else if meta.is_file() {
            let _ = base;
            out.push((path, meta.len()));
        }
    }
    Ok(())
}

/// 递归把本地目录同步到远端目录。默认跳过「远端已存在且大小相同」的文件。
#[tauri::command]
pub async fn sftp_sync_directory(
    app: tauri::AppHandle,
    state: tauri::State<'_, SshManager>,
    session_id: String,
    local_dir: String,
    remote_dir: String,
    task_id: String,
    skip_existing: Option<bool>,
) -> Result<(), String> {
    let params = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话未连接")?;
        act.params.clone()
    };

    let local_root = std::path::PathBuf::from(&local_dir);
    if !local_root.is_dir() {
        return Err("本地目录不存在或不是文件夹".to_string());
    }
    // 在远端创建与本地目录同名的文件夹，再同步其内容
    let local_dir_name = local_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "sync".to_string());
    let remote_root = format!("{}/{}", remote_dir.trim_end_matches('/'), local_dir_name);
    let skip = skip_existing.unwrap_or(true);

    tokio::task::spawn_blocking(move || {
        clear_transfer_cancel(&task_id);
        let handle = open_sftp_handle(&params)?;

        let mut files = Vec::new();
        collect_local_files(&local_root, &local_root, &mut files)?;
        let total_files = files.len();
        let total_bytes: u64 = files.iter().map(|(_, size)| *size).sum();

        let mut done_bytes: u64 = 0;
        let mut done_files = 0usize;
        let mut errors: Vec<String> = Vec::new();

        for (local_path, size) in &files {
            if is_transfer_cancelled(&task_id) {
                break;
            }
            let rel = local_path
                .strip_prefix(&local_root)
                .unwrap_or(local_path)
                .to_string_lossy()
                .replace('\\', "/");
            let remote_path = format!("{}/{}", remote_root, rel);
            let fname = rel.clone();

            if let Some(parent) = std::path::Path::new(&remote_path).parent() {
                let sftp = handle.sftp.lock();
                ensure_remote_dir(&sftp, &parent.to_string_lossy());
            }

            if skip {
                let same = {
                    let sftp = handle.sftp.lock();
                    sftp.stat(Path::new(&remote_path))
                        .map(|st| st.size == Some(*size))
                        .unwrap_or(false)
                };
                if same {
                    done_bytes += *size;
                    done_files += 1;
                    let _ = app.emit(
                        "sftp-transfer-progress",
                        TransferProgressPayload {
                            task_id: task_id.clone(),
                            filename: fname,
                            is_upload: true,
                            transferred: done_bytes,
                            total: total_bytes.max(1),
                            speed_bps: 0,
                            status: "running".into(),
                            error: None,
                        },
                    );
                    continue;
                }
            }

            let temp_remote = format!("{}.qshell-tmp-{}", remote_path, task_id);
            let result: Result<u64, String> = (|| {
                let mut local_file = File::open(local_path)
                    .map_err(|e| format!("打开本地文件失败: {}", e))?;
                let mut remote_file = {
                    let sftp = handle.sftp.lock();
                    sftp.create(Path::new(&temp_remote))
                        .map_err(|e| format!("创建远端文件失败: {}", e))?
                };
                let mut buffer = [0u8; 256 * 1024];
                let mut transferred_in_file: u64 = 0;
                loop {
                    if is_transfer_cancelled(&task_id) {
                        return Err("cancelled".to_string());
                    }
                    let n = local_file.read(&mut buffer).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    remote_file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
                    transferred_in_file += n as u64;
                    let _ = app.emit(
                        "sftp-transfer-progress",
                        TransferProgressPayload {
                            task_id: task_id.clone(),
                            filename: fname.clone(),
                            is_upload: true,
                            transferred: done_bytes + transferred_in_file,
                            total: total_bytes.max(1),
                            speed_bps: 0,
                            status: "running".into(),
                            error: None,
                        },
                    );
                }
                drop(remote_file);
                {
                    let sftp = handle.sftp.lock();
                    sftp.rename(Path::new(&temp_remote), Path::new(&remote_path), None)
                        .map_err(|e| format!("重命名失败: {}", e))?;
                }
                Ok(transferred_in_file)
            })();

            match result {
                Ok(n) => {
                    done_bytes += n;
                    done_files += 1;
                }
                Err(e) if e == "cancelled" => {
                    {
                        let sftp = handle.sftp.lock();
                        let _ = sftp.unlink(Path::new(&temp_remote));
                    }
                    let _ = app.emit(
                        "sftp-transfer-progress",
                        TransferProgressPayload {
                            task_id: task_id.clone(),
                            filename: fname,
                            is_upload: true,
                            transferred: done_bytes,
                            total: total_bytes.max(1),
                            speed_bps: 0,
                            status: "cancelled".into(),
                            error: None,
                        },
                    );
                    clear_transfer_cancel(&task_id);
                    return Ok(());
                }
                Err(e) => {
                    {
                        let sftp = handle.sftp.lock();
                        let _ = sftp.unlink(Path::new(&temp_remote));
                    }
                    errors.push(format!("{}: {}", fname, e));
                }
            }
        }

        let cancelled = is_transfer_cancelled(&task_id);
        let _ = app.emit(
            "sftp-transfer-progress",
            TransferProgressPayload {
                task_id: task_id.clone(),
                filename: format!("目录同步（{}/{} 个文件）", done_files, total_files),
                is_upload: true,
                transferred: done_bytes,
                total: total_bytes.max(1),
                speed_bps: 0,
                status: if cancelled { "cancelled" } else { "completed" }.into(),
                error: if errors.is_empty() { None } else { Some(errors.join("; ")) },
            },
        );
        clear_transfer_cancel(&task_id);
        Ok(())
    })
    .await
    .map_err(|e| format!("目录同步任务失败: {}", e))?
}