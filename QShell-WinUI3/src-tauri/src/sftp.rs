use serde::{Deserialize, Serialize};
use std::path::Path;
use std::fs::File;
use std::io::{Read, Write};
use std::sync::{Mutex as StdMutex, OnceLock};
use std::collections::HashSet;
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

/// 用 `temp` 覆盖 `target`。
///
/// Windows 上 rename 不能覆盖已存在的目标，必须先让开位置。直接删除会让
/// 「随后的 rename 失败」演变成新旧文件同时丢失，所以这里先把旧文件改名为
/// `.qshell-old`，新文件就位后再删备份；新文件未能就位则把备份还原。
/// 返回 Err 时，`target` 处原有的文件仍然存在（只要备份这一步成功过）。
pub(crate) fn replace_local_file(temp: &Path, target: &Path) -> Result<(), String> {
    let mut backup_os = target.as_os_str().to_os_string();
    backup_os.push(".qshell-old");
    let backup = std::path::PathBuf::from(backup_os);

    let mut backup_ready = false;
    if target.exists() {
        let _ = std::fs::remove_file(&backup);
        if let Err(e) = std::fs::rename(target, &backup) {
            let _ = std::fs::remove_file(temp);
            return Err(format!("无法替换本地已有文件: {}", e));
        }
        backup_ready = true;
    }

    if let Err(e) = std::fs::rename(temp, target) {
        if backup_ready {
            let _ = std::fs::rename(&backup, target);
        }
        let _ = std::fs::remove_file(temp);
        return Err(format!("写入本地文件失败: {}", e));
    }

    if backup_ready {
        let _ = std::fs::remove_file(&backup);
    }
    Ok(())
}

#[tauri::command]
pub fn cancel_sftp_transfer(task_id: String) -> Result<(), String> {
    if let Ok(mut set) = cancel_registry().lock() {
        set.insert(task_id);
    }
    Ok(())
}

/// 丢弃缓存的 SFTP 通道，下次操作会重新建连。
///
/// SFTP 用的是独立于终端的长连接，空闲久了会被中间设备回收，
/// 此时本地句柄还在但包已经发不出去（Unable to send FXP_OPEN）。
/// 由用户点「重试」触发重建，不打断正在用的终端会话。
#[tauri::command]
pub async fn reset_sftp_channel(
    state: tauri::State<'_, SshManager>,
    session_id: String,
) -> Result<(), String> {
    // 全局会话锁只用来取槽位引用，拿到就释放。
    // 旧实现在持锁状态下等槽位锁：如果此刻正有别的线程在建 SFTP 连接（可能好几秒），
    // 全局锁就一直不释放，write_ssh_session / resize / close 全部排队，界面假死。
    let slot = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话未连接")?;
        act.sftp.clone()
    };
    tokio::task::spawn_blocking(move || {
        *slot.lock() = None;
    })
    .await
    .map_err(|e| format!("执行错误: {}", e))?;
    Ok(())
}

fn open_sftp_handle(params: &SshConnectParams) -> Result<SftpHandle, String> {
    // 统一走 ssh 模块的建连逻辑：跳板机、认证、算法偏好全在那里处理。
    // 这里若自己直连 params.host，跳板机模式会绕过跳板机，
    // 撞上本机网络里同名的内网地址就会连到别的设备（监控数据显示成别的机器）。
    let mut sess = crate::ssh::open_session(params)?;
    let sftp = sess
        .sftp()
        .map_err(|e| format!("创建 SFTP 通道失败: {}", e))?;
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

/// 传输失败后丢弃会话的传输连接。
///
/// 空闲连接被中间设备回收后，本地句柄看起来还在，但包已经发不出去
/// （表现为 `Unable to send FXP_OPEN` / `FXP_WRITE`）。留着它会让后面每一次
/// 传输都失败，所以失败即丢弃，下次传输重新建连即可自愈。
/// 用 try_lock：取不到说明另一个传输正占着它，交给那次传输处理。
fn drop_transfer_after_failure(
    slot: &std::sync::Arc<parking_lot::Mutex<Option<SftpHandle>>>,
    failed: bool,
) {
    if !failed {
        return;
    }
    if let Some(mut guard) = slot.try_lock() {
        *guard = None;
    }
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
    // 传输走会话级的专用连接：同一会话的多个传输串行复用它（FTP/SFTP 单连接
    // 本来就不能并发），浏览目录用的另一条连接不受影响，也不会每个文件都新建
    // 一条 SSH 连接。
    let (transfer_slot, params) = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话未连接")?;
        (act.transfer.clone(), act.params.clone())
    };

    let local_file_path = local_path.clone();
    let fname = Path::new(&local_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());

    let slot_health = transfer_slot.clone();
    let result = tokio::task::spawn_blocking(move || {
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

        let handle = ensure_sftp_handle(&transfer_slot, &params)?;
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
    .map_err(|e| format!("操作失败: {}", e))?;
    drop_transfer_after_failure(&slot_health, result.is_err());
    result
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
    // 传输走会话级的专用连接：同一会话的多个传输串行复用它（FTP/SFTP 单连接
    // 本来就不能并发），浏览目录用的另一条连接不受影响，也不会每个文件都新建
    // 一条 SSH 连接。
    let (transfer_slot, params) = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话未连接")?;
        (act.transfer.clone(), act.params.clone())
    };

    let fname = Path::new(&remote_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());

    let slot_health = transfer_slot.clone();
    let result = tokio::task::spawn_blocking(move || {
        clear_transfer_cancel(&task_id);
        let temp_local = format!("{}.qshell-part", local_path);
        // 覆盖下载前旧文件的临时存放位置，用于「新文件未能就位」时还原
        let backup_local = format!("{}.qshell-old", local_path);
        // 上次进程在「旧文件已移开、新文件未就位」之间中断：先把原文件恢复回去，
        // 否则用户的原文件会凭空消失，只剩一个没人认得的 .qshell-old。
        if !Path::new(&local_path).exists() && Path::new(&backup_local).exists() {
            let _ = std::fs::rename(&backup_local, &local_path);
        }
        let handle = ensure_sftp_handle(&transfer_slot, &params)?;
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

        // 半截文件写完后整体替换到目标位置；替换失败会还原用户原有的文件
        if let Err(msg) = replace_local_file(Path::new(&temp_local), Path::new(&local_path)) {
            let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                task_id: task_id.clone(),
                filename: fname.clone(),
                is_upload: false,
                transferred,
                total: total_size,
                speed_bps: 0,
                status: "failed".into(),
                error: Some(msg.clone()),
            });
            clear_transfer_cancel(&task_id);
            return Err(msg);
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
    .map_err(|e| format!("操作失败: {}", e))?;
    drop_transfer_after_failure(&slot_health, result.is_err());
    result
}

/// 递归收集远端目录下的相对路径：目录（含空目录）与文件（相对路径 + 字节数）。
/// 相对路径统一用 `/` 分隔，落到本地时再替换成平台分隔符。
fn collect_remote_tree(
    sftp: &ssh2::Sftp,
    root: &str,
    rel: &str,
    dirs: &mut Vec<String>,
    files: &mut Vec<(String, u64)>,
    depth: usize,
) -> Result<(), String> {
    // 防御异常深的目录（符号链接循环或恶意构造），避免栈溢出
    if depth > 64 {
        return Err("远端目录层级超出限制".to_string());
    }
    let dir_path = if rel.is_empty() {
        root.to_string()
    } else {
        format!("{}/{}", root, rel)
    };
    let label = if rel.is_empty() { root } else { rel };
    let entries = sftp
        .readdir(Path::new(&dir_path))
        .map_err(|e| format!("读取远端目录失败（{}）: {}", label, e))?;

    for (path, stat) in entries {
        let name = match path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };
        if name == "." || name == ".." {
            continue;
        }
        let child_rel = if rel.is_empty() {
            name
        } else {
            format!("{}/{}", rel, name)
        };
        if stat.is_dir() {
            dirs.push(child_rel.clone());
            collect_remote_tree(sftp, root, &child_rel, dirs, files, depth + 1)?;
        } else {
            files.push((child_rel, stat.size.unwrap_or(0)));
        }
    }
    Ok(())
}

/// 下载单个远端文件到本地，经 `.qshell-part` 中转后再安全替换目标文件。
fn copy_remote_file_to_local(sftp: &ssh2::Sftp, remote: &str, local: &Path) -> Result<u64, String> {
    let mut remote_file = sftp
        .open(Path::new(remote))
        .map_err(|e| format!("打开远端文件失败: {}", e))?;
    let temp_local = format!("{}.qshell-part", local.to_string_lossy());
    let mut local_file = File::create(&temp_local).map_err(|e| format!("创建本地文件失败: {}", e))?;

    let mut buffer = [0u8; 256 * 1024];
    let mut transferred: u64 = 0;
    loop {
        match remote_file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                if let Err(e) = local_file.write_all(&buffer[..n]) {
                    drop(local_file);
                    let _ = std::fs::remove_file(&temp_local);
                    return Err(format!("写入本地文件失败: {}", e));
                }
                transferred += n as u64;
            }
            Err(e) => {
                drop(local_file);
                let _ = std::fs::remove_file(&temp_local);
                return Err(format!("下载失败: {}", e));
            }
        }
    }
    drop(local_file);
    replace_local_file(Path::new(&temp_local), local)?;
    Ok(transferred)
}

/// 下载整个远端目录到本地。`local_path` 是**最终目标目录**（前端已按重名情况
/// 决定是沿用原名还是「另存为新文件夹」），内容直接落在该目录下。
/// 单个文件失败不中断整体，最后汇总报告。
#[tauri::command]
pub async fn sftp_download_dir(
    app: tauri::AppHandle,
    state: tauri::State<'_, SshManager>,
    session_id: String,
    remote_path: String,
    local_path: String,
    task_id: String,
) -> Result<(), String> {
    // 传输走会话级的专用连接：同一会话的多个传输串行复用它（FTP/SFTP 单连接
    // 本来就不能并发），浏览目录用的另一条连接不受影响，也不会每个文件都新建
    // 一条 SSH 连接。
    let (transfer_slot, params) = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话未连接")?;
        (act.transfer.clone(), act.params.clone())
    };

    let folder_name = Path::new(&remote_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "folder".to_string());
    let display_name = format!("{}/", folder_name);

    let slot_health = transfer_slot.clone();
    let result = tokio::task::spawn_blocking(move || {
        clear_transfer_cancel(&task_id);
        let handle = ensure_sftp_handle(&transfer_slot, &params)?;
        let sftp = handle.sftp.lock();

        let root = remote_path.trim_end_matches('/').to_string();
        let mut dirs: Vec<String> = Vec::new();
        let mut files: Vec<(String, u64)> = Vec::new();
        collect_remote_tree(&sftp, &root, "", &mut dirs, &mut files, 0)?;

        let total_bytes: u64 = files.iter().map(|(_, size)| *size).sum();
        // 空目录时总量为 0，进度条会除零，这里兜底为 1
        let total_for_bar = total_bytes.max(1);

        let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
            task_id: task_id.clone(),
            filename: display_name.clone(),
            is_upload: false,
            transferred: 0,
            total: total_for_bar,
            speed_bps: 0,
            status: "running".into(),
            error: None,
        });

        let dest_root = std::path::PathBuf::from(&local_path);
        if let Err(e) = std::fs::create_dir_all(&dest_root) {
            let message = format!("创建本地目录失败: {}", e);
            let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                task_id: task_id.clone(),
                filename: display_name.clone(),
                is_upload: false,
                transferred: 0,
                total: total_for_bar,
                speed_bps: 0,
                status: "failed".into(),
                error: Some(message.clone()),
            });
            clear_transfer_cancel(&task_id);
            return Err(message);
        }

        let mut failures: Vec<String> = Vec::new();
        // 先把目录建出来，保证端上的空目录也能在本地出现
        for dir in &dirs {
            let target = dest_root.join(dir.replace('/', std::path::MAIN_SEPARATOR_STR));
            if let Err(e) = std::fs::create_dir_all(&target) {
                failures.push(format!("{}/（创建目录失败: {}）", dir, e));
            }
        }

        let mut transferred: u64 = 0;
        let mut last_emit = std::time::Instant::now();
        let mut last_transferred: u64 = 0;

        for (rel, _size) in &files {
            if is_transfer_cancelled(&task_id) {
                let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                    task_id: task_id.clone(),
                    filename: display_name.clone(),
                    is_upload: false,
                    transferred,
                    total: total_for_bar,
                    speed_bps: 0,
                    status: "cancelled".into(),
                    error: None,
                });
                clear_transfer_cancel(&task_id);
                return Ok(());
            }

            let remote_file = format!("{}/{}", root, rel);
            let dest = dest_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match copy_remote_file_to_local(&sftp, &remote_file, &dest) {
                Ok(n) => transferred += n,
                Err(e) => failures.push(format!("{}: {}", rel, e)),
            }

            let elapsed = last_emit.elapsed().as_secs_f64();
            if elapsed >= 0.2 || transferred == total_bytes {
                let speed = if elapsed > 0.0 {
                    ((transferred - last_transferred) as f64 / elapsed) as u64
                } else {
                    0
                };
                let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                    task_id: task_id.clone(),
                    filename: display_name.clone(),
                    is_upload: false,
                    transferred,
                    total: total_for_bar,
                    speed_bps: speed,
                    status: "running".into(),
                    error: None,
                });
                last_emit = std::time::Instant::now();
                last_transferred = transferred;
            }
        }

        if failures.is_empty() {
            let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                task_id: task_id.clone(),
                filename: display_name.clone(),
                is_upload: false,
                transferred: total_for_bar,
                total: total_for_bar,
                speed_bps: 0,
                status: "completed".into(),
                error: None,
            });
            clear_transfer_cancel(&task_id);
            Ok(())
        } else {
            let head: Vec<String> = failures.iter().take(3).cloned().collect();
            let message = format!("{} 项未能完成下载：{}", failures.len(), head.join("；"));
            let _ = app.emit("sftp-transfer-progress", TransferProgressPayload {
                task_id: task_id.clone(),
                filename: display_name.clone(),
                is_upload: false,
                transferred,
                total: total_for_bar,
                speed_bps: 0,
                status: "failed".into(),
                error: Some(message.clone()),
            });
            clear_transfer_cancel(&task_id);
            Err(message)
        }
    })
    .await
    .map_err(|e| format!("下载目录任务失败: {}", e))?;
    drop_transfer_after_failure(&slot_health, result.is_err());
    result
}

#[cfg(test)]
mod tests {
    use super::{format_permission, replace_local_file};
    use std::path::PathBuf;

    fn case_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("qshell_sftp_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn backup_of(target: &PathBuf) -> PathBuf {
        let mut s = target.as_os_str().to_os_string();
        s.push(".qshell-old");
        PathBuf::from(s)
    }

    #[test]
    fn replace_creates_file_when_target_absent() {
        let dir = case_dir("new");
        let temp = dir.join("a.qshell-part");
        let target = dir.join("a.txt");
        std::fs::write(&temp, b"new").unwrap();

        replace_local_file(&temp, &target).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!temp.exists(), "临时文件应已就位并消失");
        assert!(!backup_of(&target).exists(), "不应残留备份文件");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_overwrites_existing_and_drops_backup() {
        let dir = case_dir("overwrite");
        let temp = dir.join("a.qshell-part");
        let target = dir.join("a.txt");
        std::fs::write(&temp, b"fresh content").unwrap();
        std::fs::write(&target, b"old content").unwrap();

        replace_local_file(&temp, &target).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"fresh content");
        assert!(!backup_of(&target).exists(), "成功后必须清理备份，否则留下垃圾文件");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_keeps_original_when_new_file_missing() {
        // 模拟「新文件无法就位」：temp 不存在 → 第二次 rename 必然失败
        let dir = case_dir("rollback");
        let temp = dir.join("a.qshell-part");
        let target = dir.join("a.txt");
        std::fs::write(&target, b"original").unwrap();

        let result = replace_local_file(&temp, &target);

        assert!(result.is_err(), "新文件缺失时必须报错");
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"original",
            "替换失败后原文件必须还原，不能凭空消失"
        );
        assert!(!backup_of(&target).exists(), "还原后不应残留备份");
        let _ = std::fs::remove_dir_all(&dir);
    }

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
    // 传输走会话级的专用连接：同一会话的多个传输串行复用它（FTP/SFTP 单连接
    // 本来就不能并发），浏览目录用的另一条连接不受影响，也不会每个文件都新建
    // 一条 SSH 连接。
    let (transfer_slot, params) = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话未连接")?;
        (act.transfer.clone(), act.params.clone())
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

    let slot_health = transfer_slot.clone();
    let result = tokio::task::spawn_blocking(move || {
        clear_transfer_cancel(&task_id);
        let handle = ensure_sftp_handle(&transfer_slot, &params)?;

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
    .map_err(|e| format!("目录同步任务失败: {}", e))?;
    drop_transfer_after_failure(&slot_health, result.is_err());
    result
}
