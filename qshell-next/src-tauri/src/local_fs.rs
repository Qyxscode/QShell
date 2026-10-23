use serde::Serialize;
use std::path::{Path, PathBuf};
use base64::Engine;

#[tauri::command]
pub async fn read_local_image_data_url(path: String) -> Result<String, String> {
    // 读取最大 25MB 图片并做 base64 编码是阻塞操作，放到 blocking 线程池，
    // 避免浏览网络盘/大图时卡住 WebView 的 IPC 线程。
    tokio::task::spawn_blocking(move || {
        const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;
        let file_path = Path::new(&path);
        let meta = std::fs::metadata(file_path).map_err(|e| format!("读取背景图失败: {}", e))?;
        if !meta.is_file() {
            return Err("背景图不是文件".into());
        }
        if meta.len() > MAX_IMAGE_BYTES {
            return Err("背景图超过 25 MB，请选择更小的图片".into());
        }
        let bytes = std::fs::read(file_path).map_err(|e| format!("读取背景图失败: {}", e))?;
        let ext = file_path
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("")
            .to_lowercase();
        let mime = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "bmp" => "image/bmp",
            _ => return Err("仅支持 PNG / JPG / WEBP / GIF / BMP 背景图".into()),
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(format!("data:{};base64,{}", mime, encoded))
    })
    .await
    .map_err(|e| format!("读取背景图任务失败: {}", e))?
}

#[tauri::command]
pub async fn local_image_thumbnail(path: String, max_size: u32) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        const MAX_SOURCE_BYTES: u64 = 50 * 1024 * 1024;
        let file_path = Path::new(&path);
        let meta = std::fs::metadata(file_path).map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() > MAX_SOURCE_BYTES {
            return Err("图片不存在或超过 50 MB".into());
        }
        let mut reader = image::ImageReader::open(file_path).map_err(|e| e.to_string())?;
        reader = reader.with_guessed_format().map_err(|e| e.to_string())?;
        let mut limits = image::Limits::default();
        // 防止超高分辨率图片（解压炸弹）在 4 路并发缩略图时把内存打满。
        limits.max_alloc = Some(256 * 1024 * 1024);
        limits.max_image_width = Some(16_384);
        limits.max_image_height = Some(16_384);
        reader.limits(limits);
        let image = reader.decode().map_err(|e| e.to_string())?;
        let edge = max_size.clamp(16, 128);
        let thumb = image.thumbnail(edge, edge);
        let mut png = Vec::new();
        thumb
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
        Ok(format!("data:image/png;base64,{}", encoded))
    })
    .await
    .map_err(|e| format!("缩略图任务失败: {}", e))?
}

#[derive(Debug, Serialize, Clone)]
pub struct LocalEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[tauri::command]
pub async fn local_path_exists(path: String) -> bool {
    // 网络盘/慢盘上的 exists() 是阻塞调用，放到 blocking 线程池。
    tokio::task::spawn_blocking(move || Path::new(&path).exists())
        .await
        .unwrap_or(false)
}

#[tauri::command]
pub async fn list_local_dir(path: String) -> Result<Vec<LocalEntry>, String> {
    tokio::task::spawn_blocking(move || {
        let dir = PathBuf::from(&path);
        if !dir.is_dir() {
            return Err("目录不存在".into());
        }

        let mut entries = Vec::new();
        let read_dir = std::fs::read_dir(&dir).map_err(|e| format!("读取目录失败: {}", e))?;
        for item in read_dir.flatten() {
            let file_name = item.file_name().to_string_lossy().to_string();
            if file_name == "." || file_name == ".." {
                continue;
            }
            let metadata = match item.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let entry_path = item.path();
            entries.push(LocalEntry {
                name: file_name,
                path: entry_path.to_string_lossy().to_string(),
                is_dir: metadata.is_dir(),
                size: if metadata.is_file() { metadata.len() } else { 0 },
            });
        }

        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(entries)
    })
    .await
    .map_err(|e| format!("读取目录任务失败: {}", e))?
}

#[derive(Debug, Serialize, Clone)]
pub struct LocalKnownFolder {
    pub id: String,
    pub name: String,
    pub path: String,
    pub icon: String,
}

#[tauri::command]
pub fn local_known_folders() -> Vec<LocalKnownFolder> {
    let mut folders = Vec::new();
    let home = std::env::var("USERPROFILE")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .or_else(|| std::env::var("HOME").ok())
        .unwrap_or_default();

    let mut push = |id: &str, name: &str, path: String, icon: &str| {
        if !path.trim().is_empty() && Path::new(&path).is_dir() {
            folders.push(LocalKnownFolder {
                id: id.into(),
                name: name.into(),
                path,
                icon: icon.into(),
            });
        }
    };

    if !home.is_empty() {
        push("home", "主文件夹", home.clone(), "folder");
    }

    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let shell = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders")
            .ok();
        let read = |key: &str| -> Option<String> {
            let value: String = shell.as_ref()?.get_value(key).ok()?;
            Some(expand_env_vars(&value))
        };
        if let Some(p) = read("Desktop") {
            push("desktop", "桌面", p, "folder");
        }
        if let Some(p) = read("{374DE290-123F-4565-9164-39C4925E467B}") {
            push("downloads", "下载", p, "folder");
        }
        if let Some(p) = read("Personal") {
            push("documents", "文档", p, "folder");
        }
        if let Some(p) = read("My Pictures") {
            push("pictures", "图片", p, "folder");
        }
    }

    #[cfg(not(windows))]
    {
        if !home.is_empty() {
            push("desktop", "桌面", format!("{}/Desktop", home), "folder");
            push("downloads", "下载", format!("{}/Downloads", home), "folder");
            push("documents", "文档", format!("{}/Documents", home), "folder");
            push("pictures", "图片", format!("{}/Pictures", home), "folder");
        }
    }

    folders
}

#[cfg(windows)]
fn expand_env_vars(value: &str) -> String {
    let mut result = value.to_string();
    let mut start = 0;
    while let Some(open) = result[start..].find('%') {
        let open = start + open;
        let Some(close_rel) = result[open + 1..].find('%') else {
            break;
        };
        let close = open + 1 + close_rel;
        let name = &result[open + 1..close];
        if let Ok(replacement) = std::env::var(name) {
            result.replace_range(open..=close, &replacement);
            start = open + replacement.len();
        } else {
            start = close + 1;
        }
    }
    result
}

#[tauri::command]
pub fn local_roots() -> Vec<LocalEntry> {
    let mut roots = Vec::new();

    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetLogicalDrives() -> u32;
        }
        let mask = unsafe { GetLogicalDrives() };
        for i in 0..26 {
            if mask & (1 << i) != 0 {
                let letter = (b'A' + i as u8) as char;
                let root = format!("{}:\\", letter);
                if Path::new(&root).exists() {
                    roots.push(LocalEntry {
                        name: format!("{}:", letter),
                        path: root,
                        is_dir: true,
                        size: 0,
                    });
                }
            }
        }
    }

    #[cfg(not(windows))]
    {
        roots.push(LocalEntry {
            name: "/".into(),
            path: "/".into(),
            is_dir: true,
            size: 0,
        });
    }

    roots
}
