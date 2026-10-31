use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static CONFIG_IO_LOCK: Mutex<()> = Mutex::new(());

/// 当前会话的主密码（仅在开启加密并解锁后存在，不落盘）
static CONFIG_PASSWORD: Mutex<Option<String>> = Mutex::new(None);

const CONFIG_LOCKED: &str = "CONFIG_LOCKED";

/// 原子写入：先写同目录临时文件，再替换目标，避免读取方/崩溃时看到半截 JSON。
pub(crate) fn write_file_atomic(path: &Path, content: &[u8]) -> Result<(), String> {
    let _guard = CONFIG_IO_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.json");
    let temp_path = parent.join(format!(".{}.tmp", file_name));

    fs::write(&temp_path, content).map_err(|e| format!("写入临时配置文件失败: {}", e))?;

    if path.exists() {
        let backup_path = parent.join(format!("{}.bak", file_name));
        let _ = fs::copy(path, backup_path);
    }

    if fs::rename(&temp_path, path).is_err() {
        // Windows 上目标文件存在时部分文件系统会拒绝 rename，退化为先删除再替换。
        let _ = fs::remove_file(path);
        if let Err(error) = fs::rename(&temp_path, path) {
            let _ = fs::remove_file(&temp_path);
            return Err(format!("替换配置文件失败: {}", error));
        }
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HostDetails {
    pub memo: Option<String>,
    pub expire_date: Option<String>,
    pub alert_expire_days: Option<i32>,
    pub price: Option<String>,
    pub currency: Option<String>,
    pub traffic_total: Option<String>,
    pub traffic_total_unit: Option<String>,
    pub traffic_total_infinite: Option<bool>,
    pub alert_traffic_pct: Option<i32>,
    pub traffic_used: Option<String>,
    pub traffic_used_unit: Option<String>,
    pub reset_day: Option<i32>,
    pub reset_hour: Option<i32>,
    pub reset_min: Option<i32>,
    pub reset_sec: Option<i32>,
    pub traffic_direction: Option<String>,
    pub target_interface: Option<String>,
    pub traffic_price: Option<String>,
    pub traffic_currency: Option<String>,
    pub traffic_cycle_used_bytes: Option<u64>,
    pub traffic_source: Option<String>,
    pub traffic_custom_command: Option<String>,
    pub traffic_source_effective: Option<String>,
    pub traffic_source_error: Option<String>,
    pub traffic_last_sync_ts: Option<f64>,
    pub traffic_delta_bytes: Option<u64>,
    pub last_raw_bytes: Option<u64>,
    pub traffic_counter_sig: Option<String>,
    pub traffic_boot_id: Option<String>,
    pub last_reset_timestamp: Option<f64>,
    pub traffic_manual_override: Option<bool>,
    pub traffic_iface_baseline: Option<String>,
    pub traffic_vnstat_anchor_bytes: Option<u64>,
    pub traffic_vnstat_anchor_ts: Option<f64>,
    pub traffic_cycle_anchor_used_bytes: Option<u64>,
    pub traffic_force_nic: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HostItem {
    pub name: String,
    #[serde(default = "default_protocol")]
    pub protocol: Option<String>,
    pub host: String,
    pub port: u32,
    pub user: String,
    pub pwd: Option<String>,
    pub key_path: Option<String>,
    pub passphrase: Option<String>,
    pub serial_data_bits: Option<u8>,
    pub serial_parity: Option<String>,
    pub serial_stop_bits: Option<u8>,
    pub serial_flow_control: Option<String>,
    pub serial_newline: Option<String>,
    pub serial_local_echo: Option<bool>,
    pub serial_dtr: Option<bool>,
    pub serial_rts: Option<bool>,
    // 跳板机（ProxyJump）配置，留空表示直连
    #[serde(default)]
    pub proxy_host: Option<String>,
    #[serde(default)]
    pub proxy_port: Option<u16>,
    #[serde(default)]
    pub proxy_user: Option<String>,
    #[serde(default)]
    pub proxy_pwd: Option<String>,
    #[serde(default)]
    pub proxy_key: Option<String>,
    #[serde(default)]
    pub proxy_passphrase: Option<String>,
    /// 终端类型（auto / xterm-256color / xterm / vt220 / vt100 / linux / ansi）
    #[serde(default)]
    pub term_type: Option<String>,
    /// 是否使用系统 ssh-agent 认证
    #[serde(default)]
    pub use_agent: Option<bool>,
    pub details: Option<HostDetails>,
    pub latency_text: Option<String>,
}

fn default_protocol() -> Option<String> {
    Some("ssh".to_string())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeviceGroup {
    pub group: String,
    pub is_standalone_host: Option<bool>,
    pub hosts: Vec<HostItem>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MemoItem {
    pub name: String,
    pub cmd: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MemoGroup {
    pub group: String,
    pub items: Vec<MemoItem>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WebDavSettings {
    pub url: Option<String>,
    pub user: Option<String>,
    pub pwd: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppSettings {
    pub app_font_family: Option<String>,
    pub font_family: Option<String>,
    pub font_size: Option<i32>,
    pub is_bold: Option<bool>,
    pub cursor_style: Option<String>,
    pub cursor_blink: Option<bool>,
    pub blink_interval: Option<i32>,
    pub theme_mode: Option<String>,
    /// 前端历史上一直写入 `term_bg_mode`，这里用别名兼容旧配置，序列化仍输出 `terminal_bg_mode`。
    #[serde(alias = "term_bg_mode")]
    pub terminal_bg_mode: Option<String>,
    pub terminal_fg_color: Option<String>,
    pub terminal_bg_color: Option<String>,
    pub terminal_bg_image: Option<String>,
    pub terminal_bg_image_fit: Option<String>,
    pub terminal_bg_image_dim: Option<f32>,
    pub terminal_bg_image_auto_fg: Option<bool>,
    pub accent_color: Option<String>,
    pub minimize_to_tray: Option<bool>,
    /// 用户偏好的开机自启状态：用于升级/重装后自启项被清掉时自动恢复
    #[serde(default)]
    pub autostart_enabled: Option<bool>,
    /// 是否记录终端会话日志
    #[serde(default)]
    pub session_log_enabled: Option<bool>,
    /// 断线自动重连开关
    #[serde(default)]
    pub auto_reconnect: Option<bool>,
    /// 启动时是否恢复上次会话
    #[serde(default)]
    pub session_restore_enabled: Option<bool>,
    pub webdav: Option<WebDavSettings>,
    /// 告警忽略记录（到期 / 流量分别记录，条件变化时自动失效）。
    /// 结构由前端维护：
    /// { "expire": { "host:port": "到期日期指纹" },
    ///   "traffic": { "host:port": { "pct": 92, "threshold": 90 } } }
    #[serde(default)]
    pub alert_ignores: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub version: Option<String>,
    #[serde(default)]
    pub devices: Vec<DeviceGroup>,
    #[serde(default)]
    pub managed_hosts: Vec<HostItem>,
    pub memos: Option<Vec<MemoGroup>>,
    pub settings: Option<AppSettings>,
}

pub(crate) fn get_config_path() -> PathBuf {
    // 允许测试或高级用户显式指定配置路径
    if let Some(custom) = std::env::var_os("QSHELL_CONFIG_PATH") {
        return PathBuf::from(custom);
    }

    // 开发模式仍使用项目目录，避免影响本机已有的正式配置
    if cfg!(debug_assertions) {
        return dev_config_path();
    }

    let stable = app_config_path();
    migrate_legacy_config(&stable);
    stable
}

fn dev_config_path() -> PathBuf {
    // 优先读取根目录或上一级目录的 config.json，避免在 src-tauri/ 内部读写导致 cargo-watch 重启
    let root_path = PathBuf::from("../../config.json");
    if root_path.exists() {
        return root_path;
    }
    let parent_path = PathBuf::from("../config.json");
    if parent_path.exists() {
        return parent_path;
    }
    let current = PathBuf::from("config.json");
    if current.exists() {
        return current;
    }
    // 默认保存在上级目录（项目根目录），不写在 src-tauri 内部
    parent_path
}

fn app_config_path() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join("QShell").join("config.json");
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("QShell").join("config.json");
    }
    PathBuf::from("config.json")
}

fn is_qshell_config(path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    value.get("managed_hosts").is_some()
        || value.get("devices").is_some()
        || value.get("settings").is_some()
}

fn migration_marker(stable: &Path) -> PathBuf {
    stable
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".config-initialized")
}

fn migrate_legacy_config(stable: &Path) {
    let marker = migration_marker(stable);
    if stable.exists() {
        // A real config already exists. Record that migration has been handled,
        // so deleting config.json later does not resurrect a legacy config.
        let _ = fs::write(&marker, b"");
        return;
    }
    if marker.exists() {
        // The user intentionally removed the configuration after first launch.
        return;
    }

    // 旧安装版实际把配置写在 %LOCALAPPDATA%\config.json
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("config.json"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("config.json"));
        }
    }
    candidates.push(PathBuf::from("../config.json"));
    candidates.push(PathBuf::from("../../config.json"));
    candidates.push(PathBuf::from("config.json"));

    for candidate in candidates {
        if candidate == stable || !candidate.is_file() || !is_qshell_config(&candidate) {
            continue;
        }
        if let Some(parent) = stable.parent() {
            if fs::create_dir_all(parent).is_err() {
                continue;
            }
        }
        if fs::copy(&candidate, stable).is_ok() {
            let _ = fs::write(&marker, b"");
            return;
        }
    }
}

/// 读取配置文件明文内容（备份导出用；加密配置要求已解锁）
pub(crate) fn read_config_plain() -> Result<Vec<u8>, String> {
    let p = get_config_path();
    let raw = fs::read(&p).map_err(|e| format!("读取配置文件失败: {}", e))?;
    if crate::config_crypto::is_encrypted(&raw) {
        let password = CONFIG_PASSWORD
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(password) = password else {
            return Err(CONFIG_LOCKED.to_string());
        };
        crate::config_crypto::decrypt(&raw, &password)
    } else {
        Ok(raw)
    }
}

/// 按当前加密状态写回配置（备份导入用）
pub(crate) fn write_config_plain(plain: &[u8]) -> Result<(), String> {
    let p = get_config_path();
    let password = CONFIG_PASSWORD
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    match password {
        Some(password) => {
            let encrypted = crate::config_crypto::encrypt(plain, &password)?;
            write_file_atomic(&p, &encrypted)
        }
        None => write_file_atomic(&p, plain),
    }
}

#[tauri::command]
pub fn load_hosts_config() -> Result<AppConfig, String> {
    let _guard = CONFIG_IO_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let p = get_config_path();
    if !p.exists() {
        return Ok(AppConfig {
            version: Some("1.1".into()),
            devices: vec![],
            managed_hosts: vec![],
            memos: Some(vec![]),
            settings: None,
        });
    }
    let content = fs::read(&p).map_err(|e| format!("读取配置文件失败: {}", e))?;
    let plain = if crate::config_crypto::is_encrypted(&content) {
        let password = CONFIG_PASSWORD
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(password) = password else {
            // 前端收到这个标记后弹出解锁框
            return Err(CONFIG_LOCKED.to_string());
        };
        crate::config_crypto::decrypt(&content, &password)?
    } else {
        content
    };
    let cfg: AppConfig = serde_json::from_slice(&plain).map_err(|e| format!("解析 JSON 格式错误: {}", e))?;
    Ok(cfg)
}

#[tauri::command]
pub fn save_hosts_config(config: AppConfig) -> Result<(), String> {
    let p = get_config_path();
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {}", e))?;
    }
    let json_str = serde_json::to_string_pretty(&config).map_err(|e| format!("序列化失败: {}", e))?;
    let password = CONFIG_PASSWORD
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let already_encrypted = fs::read(&p)
        .map(|raw| crate::config_crypto::is_encrypted(&raw))
        .unwrap_or(false);

    if already_encrypted && password.is_none() {
        // 已加密但尚未解锁：绝不能把明文写回去
        return Err(CONFIG_LOCKED.to_string());
    }
    if let Some(password) = password {
        let encrypted = crate::config_crypto::encrypt(json_str.as_bytes(), &password)?;
        write_file_atomic(&p, &encrypted)
    } else {
        write_file_atomic(&p, json_str.as_bytes())
    }
}

#[tauri::command]
pub fn is_config_encrypted() -> bool {
    let p = get_config_path();
    fs::read(&p)
        .map(|raw| crate::config_crypto::is_encrypted(&raw))
        .unwrap_or(false)
}

#[tauri::command]
pub fn unlock_config(password: String) -> Result<(), String> {
    let p = get_config_path();
    if !p.exists() {
        return Ok(());
    }
    let content = fs::read(&p).map_err(|e| format!("读取配置文件失败: {}", e))?;
    if !crate::config_crypto::is_encrypted(&content) {
        // 明文配置不需要解锁
        return Ok(());
    }
    crate::config_crypto::decrypt(&content, &password)?;
    *CONFIG_PASSWORD.lock().unwrap_or_else(|e| e.into_inner()) = Some(password);
    Ok(())
}

/// 开启或关闭配置加密。开启时需要当前配置可读（明文或已解锁），关闭时需要验证主密码。
#[tauri::command]
pub fn set_config_encryption(enabled: bool, password: String) -> Result<(), String> {
    let p = get_config_path();
    if !p.exists() {
        return Err("配置文件不存在，请先保存一次主机配置".to_string());
    }
    let content = fs::read(&p).map_err(|e| format!("读取配置文件失败: {}", e))?;
    let encrypted_now = crate::config_crypto::is_encrypted(&content);

    if enabled {
        let plain = if encrypted_now {
            let current = CONFIG_PASSWORD
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let Some(current) = current else {
                return Err(CONFIG_LOCKED.to_string());
            };
            crate::config_crypto::decrypt(&content, &current)?
        } else {
            content
        };
        let encrypted = crate::config_crypto::encrypt(&plain, &password)?;
        write_file_atomic(&p, &encrypted)?;
        *CONFIG_PASSWORD.lock().unwrap_or_else(|e| e.into_inner()) = Some(password);
        Ok(())
    } else {
        if !encrypted_now {
            return Ok(());
        }
        let plain = crate::config_crypto::decrypt(&content, &password)?;
        write_file_atomic(&p, &plain)?;
        *CONFIG_PASSWORD.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }
}
