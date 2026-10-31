use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WebDavConfig {
    pub url: String,
    pub user: String,
    pub pwd: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupResult {
    pub success: bool,
    pub message: String,
    /// 备份里携带的界面偏好（localStorage 快照的 JSON 字符串），导入时回传给前端应用
    #[serde(default)]
    pub ui_state: Option<String>,
}

fn get_config_file_path() -> PathBuf {
    crate::config::get_config_path()
}

/// 把前端收集的界面偏好（动画开关、分屏布局/比例、面板列数、上次目录、命令历史）
/// 挂到备份 JSON 的 `ui_state` 字段上。老版本 QShell 读到未知字段会忽略，向后兼容。
fn attach_ui_state(content: Vec<u8>, ui_state: Option<String>) -> Vec<u8> {
    let Some(raw) = ui_state else {
        return content;
    };
    let Ok(mut root) = serde_json::from_slice::<serde_json::Value>(&content) else {
        return content;
    };
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return content;
    };
    if let Some(object) = root.as_object_mut() {
        object.insert("ui_state".to_string(), payload);
    }
    serde_json::to_vec(&root).unwrap_or(content)
}

/// 从备份 JSON 取出 `ui_state`，并把该字段从配置本体里摘掉
/// （AppConfig 没有这个字段，留在里面会被当成未知字段丢掉，不如显式清理干净）
fn take_ui_state(content: &mut Vec<u8>) -> Option<String> {
    let mut root: serde_json::Value = serde_json::from_slice(content).ok()?;
    let ui_state = root
        .as_object_mut()
        .and_then(|object| object.remove("ui_state"))?;
    // 写回"去掉 ui_state"的版本，后续走正常的配置导入流程
    if let Ok(cleaned) = serde_json::to_vec(&root) {
        *content = cleaned;
    }
    serde_json::to_string(&ui_state).ok()
}

/// 恢复备份时保留当前 WebDAV 连接信息，避免云端旧备份把域名/账号/密码覆盖为空。
fn preserve_webdav_settings(cfg_path: &Path, imported: &[u8]) -> Result<Vec<u8>, String> {
    let mut imported_json: serde_json::Value =
        serde_json::from_slice(imported).map_err(|e| format!("备份文件非有效 JSON: {}", e))?;

    if let Ok(local_bytes) = fs::read(cfg_path) {
        if let Ok(local_json) = serde_json::from_slice::<serde_json::Value>(&local_bytes) {
            if let Some(local_webdav) = local_json
                .get("settings")
                .and_then(|settings| settings.get("webdav"))
                .and_then(|webdav| webdav.as_object())
            {
                let object = imported_json
                    .as_object_mut()
                    .ok_or_else(|| "备份配置根节点不是 JSON 对象".to_string())?;
                let settings = object
                    .entry("settings")
                    .or_insert_with(|| serde_json::Value::Object(Default::default()));
                if !settings.is_object() {
                    *settings = serde_json::Value::Object(Default::default());
                }
                let settings = settings.as_object_mut().unwrap();
                let mut merged = settings
                    .get("webdav")
                    .and_then(|webdav| webdav.as_object())
                    .cloned()
                    .unwrap_or_default();

                for (key, value) in local_webdav {
                    let keep_local = match value {
                        serde_json::Value::String(text) => !text.trim().is_empty(),
                        serde_json::Value::Null => false,
                        _ => true,
                    };
                    if keep_local {
                        merged.insert(key.clone(), value.clone());
                    }
                }
                settings.insert(
                    "webdav".to_string(),
                    serde_json::Value::Object(merged),
                );
            }
        }
    }

    serde_json::to_vec_pretty(&imported_json)
        .map_err(|e| format!("合并 WebDAV 配置失败: {}", e))
}

#[tauri::command]
pub async fn export_local_backup(
    target_path: String,
    ui_state: Option<String>,
) -> Result<BackupResult, String> {
    let cfg_path = get_config_file_path();
    if !cfg_path.exists() {
        return Err("本地配置文件不存在，无法导出".into());
    }

    let content = attach_ui_state(
        crate::config::read_config_plain().map_err(|e| format!("读取配置失败: {}", e))?,
        ui_state,
    );

    // 如果目标路径是 .zip，打包为标准 zip 格式（对标 ssh8.py qshell_config.json 规范）
    if target_path.to_lowercase().ends_with(".zip") {
        let file = fs::File::create(&target_path).map_err(|e| format!("创建 zip 文件失败: {}", e))?;
        let mut zip_writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        
        use std::io::Write;
        zip_writer.start_file("qshell_config.json", options)
            .map_err(|e| format!("写入 zip 条目失败: {}", e))?;
        zip_writer.write_all(&content)
            .map_err(|e| format!("压缩写入数据失败: {}", e))?;
        zip_writer.finish()
            .map_err(|e| format!("完成 zip 打包失败: {}", e))?;
    } else {
        fs::write(&target_path, content).map_err(|e| format!("导出文件失败: {}", e))?;
    }

    Ok(BackupResult {
        success: true,
        message: format!("已成功导出配置备份至: {}", target_path),
        ui_state: None,
    })
}

#[tauri::command]
pub async fn import_local_backup(source_path: String) -> Result<BackupResult, String> {
    let file_bytes = fs::read(&source_path).map_err(|e| format!("读取导入文件失败: {}", e))?;
    let mut content = if source_path.to_lowercase().ends_with(".zip") {
        let reader = std::io::Cursor::new(file_bytes);
        let mut zip_archive = zip::ZipArchive::new(reader).map_err(|e| format!("无效的 ZIP 备份包: {}", e))?;
        use std::io::Read;
        let mut file = if let Ok(f) = zip_archive.by_name("qshell_config.json") {
            f
        } else if let Ok(f) = zip_archive.by_name("config.json") {
            f
        } else {
            return Err("备份包中未发现合法的 qshell_config.json 或 config.json 配置文件".into());
        };
        let mut decompressed = Vec::new();
        file.read_to_end(&mut decompressed).map_err(|e| format!("解压备份文件失败: {}", e))?;
        decompressed
    } else {
        file_bytes
    };

    // 验证是否是合法 JSON
    let _: serde_json::Value = serde_json::from_slice(&content).map_err(|e| format!("备份文件非有效 JSON: {}", e))?;

    // 先把界面偏好摘出来（配置本体里不该保留它），再走正常的导入流程
    let ui_state = take_ui_state(&mut content);

    let cfg_path = get_config_file_path();
    let merged_content = preserve_webdav_settings(&cfg_path, &content)?;
    crate::config::write_config_plain(&merged_content)
        .map_err(|e| format!("写入配置失败: {}", e))?;

    Ok(BackupResult {
        success: true,
        message: "已成功导入配置并更新本地存储".into(),
        ui_state,
    })
}

#[tauri::command]
pub async fn test_webdav_connection(cfg: WebDavConfig) -> Result<BackupResult, String> {
    tokio::task::spawn_blocking(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .map_err(|e| format!("创建网络请求客户端失败: {}", e))?;

        let req = client.request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &cfg.url);
        let req = if let Some(pwd) = cfg.pwd {
            req.basic_auth(&cfg.user, Some(pwd))
        } else {
            req.basic_auth(&cfg.user, None::<String>)
        };

        match req.send() {
            Ok(resp) => {
                if resp.status().is_success() || resp.status().as_u16() == 207 {
                    Ok(BackupResult {
                        success: true,
                        message: "WebDAV 服务器连接成功！状态码: 207 Multi-Status".into(),
                        ui_state: None,
                    })
                } else {
                    Ok(BackupResult {
                        success: false,
                        message: format!("服务器响应状态码: {}", resp.status()),
                        ui_state: None,
                    })
                }
            }
            Err(e) => Err(format!("连接 WebDAV 失败: {}", e)),
        }
    })
    .await
    .map_err(|e| format!("任务执行异常: {}", e))?
}

const DEFAULT_WEBDAV_BACKUP_PATH: &str = "/QShell/qshell_backup.zip";

fn normalized_webdav_path(path: Option<&str>) -> String {
    let candidate = path.map(str::trim).unwrap_or("");
    // The old default was a JSON file directly under the WebDAV root.
    // Treat it as the legacy default, not as a deliberate custom path.
    let candidate = if candidate.is_empty() || candidate == "/qshell_backup.json" {
        DEFAULT_WEBDAV_BACKUP_PATH
    } else {
        candidate
    };
    let mut normalized = candidate.replace('\\', "/");
    if !normalized.starts_with('/') {
        normalized.insert(0, '/');
    }
    while normalized.contains("//") {
        normalized = normalized.replace("//", "/");
    }
    normalized
}

fn webdav_join(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn ensure_webdav_parent_dirs(
    client: &reqwest::blocking::Client,
    cfg: &WebDavConfig,
    remote_file: &str,
) -> Result<(), String> {
    let clean = remote_file.trim_matches('/');
    let parts: Vec<&str> = clean.split('/').filter(|part| !part.is_empty()).collect();
    if parts.len() <= 1 {
        return Ok(());
    }

    let mut current = String::new();
    for part in &parts[..parts.len() - 1] {
        current.push('/');
        current.push_str(part);
        let mut dir_url = webdav_join(&cfg.url, &current);
        if !dir_url.ends_with('/') {
            dir_url.push('/');
        }

        let mut req = client.request(reqwest::Method::from_bytes(b"MKCOL").unwrap(), &dir_url);
        req = if let Some(pwd) = cfg.pwd.as_deref() {
            req.basic_auth(&cfg.user, Some(pwd))
        } else {
            req.basic_auth(&cfg.user, None::<String>)
        };

        let response = req
            .send()
            .map_err(|e| format!("创建 WebDAV 目录失败 [{}]: {}", current, e))?;
        let status = response.status();
        // 201 = created, 405 = already exists, 200/204 = server accepted it.
        if !(status.is_success() || status.as_u16() == 201 || status.as_u16() == 204 || status.as_u16() == 405) {
            return Err(format!(
                "创建 WebDAV 目录失败 [{}]: 服务器状态 {}",
                current, status
            ));
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn webdav_upload_backup(
    cfg: WebDavConfig,
    ui_state: Option<String>,
) -> Result<BackupResult, String> {
    let content = attach_ui_state(
        crate::config::read_config_plain().map_err(|e| format!("读取本地配置失败: {}", e))?,
        ui_state,
    );

    let remote_file = normalized_webdav_path(cfg.path.as_deref());
    let url = webdav_join(&cfg.url, &remote_file);

    let upload_bytes = if remote_file.to_lowercase().ends_with(".zip") {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip_writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            use std::io::Write;
            zip_writer.start_file("qshell_config.json", options)
                .map_err(|e| format!("写入 zip 条目失败: {}", e))?;
            zip_writer.write_all(&content)
                .map_err(|e| format!("压缩写入失败: {}", e))?;
            zip_writer.finish()
                .map_err(|e| format!("完成 zip 压缩失败: {}", e))?;
        }
        buffer.into_inner()
    } else {
        content
    };

    let base_cfg = cfg.clone();
    tokio::task::spawn_blocking(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| format!("客户端初始化失败: {}", e))?;

        ensure_webdav_parent_dirs(&client, &base_cfg, &remote_file)?;

        let mut req = client.put(&url).body(upload_bytes);
        if let Some(pwd) = base_cfg.pwd.as_deref() {
            req = req.basic_auth(&base_cfg.user, Some(pwd));
        } else {
            req = req.basic_auth(&base_cfg.user, None::<String>);
        }

        match req.send() {
            Ok(resp) => {
                if resp.status().is_success() || resp.status().as_u16() == 201 || resp.status().as_u16() == 204 {
                    Ok(BackupResult {
                        success: true,
                        message: format!("云端备份上传成功: {}", remote_file),
                        ui_state: None,
                    })
                } else {
                    Err(format!("上传失败，服务器状态: {}", resp.status()))
                }
            }
            Err(e) => Err(format!("上传请求异常: {}", e)),
        }
    })
    .await
    .map_err(|e| format!("任务执行异常: {}", e))?
}

#[tauri::command]
pub async fn webdav_download_backup(cfg: WebDavConfig) -> Result<BackupResult, String> {
    let remote_file = normalized_webdav_path(cfg.path.as_deref());
    let url = webdav_join(&cfg.url, &remote_file);

    let raw_bytes = tokio::task::spawn_blocking(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| format!("客户端初始化失败: {}", e))?;

        let mut req = client.get(&url);
        if let Some(pwd) = cfg.pwd {
            req = req.basic_auth(&cfg.user, Some(pwd));
        } else {
            req = req.basic_auth(&cfg.user, None::<String>);
        }

        let resp = req.send().map_err(|e| format!("网络请求失败: {}", e))?;
        if !resp.status().is_success() {
            return Err(format!("下载失败，服务器状态: {}", resp.status()));
        }
        let bytes = resp.bytes().map_err(|e| format!("读取内容失败: {}", e))?;
        Ok(bytes.to_vec())
    })
    .await
    .map_err(|e| format!("任务执行异常: {}", e))??;

    let content = if remote_file.to_lowercase().ends_with(".zip") {
        let reader = std::io::Cursor::new(raw_bytes);
        let mut zip_archive = zip::ZipArchive::new(reader).map_err(|e| format!("远端 ZIP 包解析失败: {}", e))?;
        use std::io::Read;
        let mut file = if let Ok(f) = zip_archive.by_name("qshell_config.json") {
            f
        } else if let Ok(f) = zip_archive.by_name("config.json") {
            f
        } else {
            return Err("远端 ZIP 包内不包含有效的 qshell_config.json 文件".into());
        };
        let mut decompressed = Vec::new();
        file.read_to_end(&mut decompressed).map_err(|e| format!("解压远端备份文件失败: {}", e))?;
        decompressed
    } else {
        raw_bytes
    };

    // 验证合法性并写回本地
    let _: serde_json::Value = serde_json::from_slice(&content).map_err(|e| format!("云端文件解析失败: {}", e))?;
    // 界面偏好单独取出回传前端，配置本体里不保留
    let mut content = content;
    let ui_state = take_ui_state(&mut content);

    let cfg_path = get_config_file_path();
    let merged_content = preserve_webdav_settings(&cfg_path, &content)?;
    crate::config::write_config_plain(&merged_content)
        .map_err(|e| format!("覆盖写入本地配置失败: {}", e))?;

    Ok(BackupResult {
        success: true,
        message: "云端备份恢复成功！本地配置已更新".into(),
        ui_state,
    })
}

#[cfg(test)]
mod tests {
    use super::preserve_webdav_settings;
    use std::fs;

    #[test]
    fn restore_keeps_current_webdav_credentials() {
        let dir = std::env::temp_dir().join(format!(
            "qshell_webdav_test_{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let config_path = dir.join("config.json");
        fs::write(
            &config_path,
            r#"{"settings":{"webdav":{"url":"https://local.example/dav/","user":"alice","pwd":"secret","path":"/local.zip"}},"devices":[]}"#,
        )
        .unwrap();

        let imported = br#"{"settings":{"webdav":{"url":"https://old.example/dav/","user":"old","pwd":"","path":"/old.zip"}},"devices":[{"group":"imported","hosts":[]}]}"#;
        let merged = preserve_webdav_settings(&config_path, imported).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&merged).unwrap();
        let webdav = &value["settings"]["webdav"];
        assert_eq!(webdav["url"], "https://local.example/dav/");
        assert_eq!(webdav["user"], "alice");
        assert_eq!(webdav["pwd"], "secret");
        assert_eq!(webdav["path"], "/local.zip");
        assert_eq!(value["devices"][0]["group"], "imported");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_local_password_keeps_cloud_password() {
        let dir = std::env::temp_dir().join(format!(
            "qshell_webdav_test_empty_{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let config_path = dir.join("config.json");
        fs::write(
            &config_path,
            r#"{"settings":{"webdav":{"url":"https://local.example/dav/","user":"alice","pwd":"","path":"/local.zip"}}}"#,
        )
        .unwrap();

        let imported = br#"{"settings":{"webdav":{"url":"https://cloud.example/dav/","user":"cloud","pwd":"cloud-secret","path":"/cloud.zip"}}}"#;
        let merged = preserve_webdav_settings(&config_path, imported).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&merged).unwrap();
        let webdav = &value["settings"]["webdav"];
        assert_eq!(webdav["url"], "https://local.example/dav/");
        assert_eq!(webdav["user"], "alice");
        assert_eq!(webdav["pwd"], "cloud-secret");
        assert_eq!(webdav["path"], "/local.zip");

        let _ = fs::remove_dir_all(&dir);
    }
}
