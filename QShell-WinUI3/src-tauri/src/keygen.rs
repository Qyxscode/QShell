use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Serialize, Clone)]
pub struct SshKeyInfo {
    pub private_path: String,
    pub public_path: String,
    pub public_key: String,
    pub fingerprint: String,
    pub algorithm: String,
}

/// 定位 ssh-keygen：优先 PATH，其次 Windows 自带 / Git for Windows。
fn keygen_binary() -> PathBuf {
    if Command::new("ssh-keygen").arg("-?").output().is_ok() {
        return PathBuf::from("ssh-keygen");
    }
    for candidate in [
        r"C:\Windows\System32\OpenSSH\ssh-keygen.exe",
        r"C:\Program Files\Git\usr\bin\ssh-keygen.exe",
        r"C:\Program Files\OpenSSH\ssh-keygen.exe",
    ] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from("ssh-keygen")
}

#[cfg(windows)]
fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
fn hide_console(_cmd: &mut Command) {}

fn default_ssh_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join(".ssh")
}

/// 只允许纯文件名：拒绝路径分隔符与 `..`，避免写到目标目录之外。
fn sanitize_file_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("文件名不能为空".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("文件名不能包含路径分隔符或 ..".to_string());
    }
    let invalid = ['<', '>', ':', '"', '|', '?', '*'];
    if name.chars().any(|c| invalid.contains(&c) || (c as u32) < 0x20) {
        return Err("文件名包含非法字符".to_string());
    }
    Ok(name.to_string())
}

fn read_public_key(public_path: &Path) -> Result<String, String> {
    std::fs::read_to_string(public_path)
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("读取公钥失败: {}", e))
}

fn fingerprint_of(binary: &Path, public_path: &Path) -> String {
    let mut cmd = Command::new(binary);
    cmd.args(["-l", "-f"]).arg(public_path);
    hide_console(&mut cmd);
    match cmd.output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => String::new(),
    }
}

/// 生成 SSH 密钥对（OpenSSH 格式）。返回私钥/公钥路径、公钥内容与指纹。
#[tauri::command]
pub async fn generate_ssh_key(
    kind: String,
    directory: Option<String>,
    file_name: String,
    passphrase: Option<String>,
    comment: Option<String>,
) -> Result<SshKeyInfo, String> {
    tokio::task::spawn_blocking(move || {
        let binary = keygen_binary();
        let file_name = sanitize_file_name(&file_name)?;

        let dir = match directory.as_deref() {
            Some(d) if !d.trim().is_empty() => PathBuf::from(d.trim()),
            _ => default_ssh_dir(),
        };
        if !dir.exists() {
            std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败: {}", e))?;
        }
        if !dir.is_dir() {
            return Err("所选路径不是目录".to_string());
        }

        let private_path = dir.join(&file_name);
        let public_path = dir.join(format!("{}.pub", file_name));
        if private_path.exists() {
            return Err(format!("{} 已存在，请换一个文件名", private_path.display()));
        }

        let mut args: Vec<String> = vec![
            "-q".into(),
            "-f".into(),
            private_path.to_string_lossy().into_owned(),
        ];
        let algorithm = match kind.as_str() {
            "ed25519" => {
                args.extend(["-t".into(), "ed25519".into()]);
                "ED25519"
            }
            "rsa2048" => {
                args.extend(["-t".into(), "rsa".into(), "-b".into(), "2048".into()]);
                "RSA"
            }
            "rsa4096" => {
                args.extend(["-t".into(), "rsa".into(), "-b".into(), "4096".into()]);
                "RSA"
            }
            "ecdsa" => {
                args.extend(["-t".into(), "ecdsa".into(), "-b".into(), "256".into()]);
                "ECDSA"
            }
            other => return Err(format!("不支持的密钥类型: {}", other)),
        };
        args.extend(["-N".into(), passphrase.unwrap_or_default()]);
        if let Some(comment) = comment {
            if !comment.trim().is_empty() {
                args.extend(["-C".into(), comment.trim().to_string()]);
            }
        }

        let mut cmd = Command::new(&binary);
        cmd.args(&args);
        hide_console(&mut cmd);
        let output = cmd
            .output()
            .map_err(|e| format!("无法运行 ssh-keygen（请确认系统已安装 OpenSSH 客户端）: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return Err(format!(
                "生成密钥失败: {}",
                if !stderr.is_empty() { stderr } else { stdout }
            ));
        }

        let public_key = read_public_key(&public_path)?;
        let fingerprint = fingerprint_of(&binary, &public_path);
        Ok(SshKeyInfo {
            private_path: private_path.to_string_lossy().to_string(),
            public_path: public_path.to_string_lossy().to_string(),
            public_key,
            fingerprint,
            algorithm: algorithm.to_string(),
        })
    })
    .await
    .map_err(|e| format!("密钥生成任务失败: {}", e))?
}

/// 读取已有密钥的公钥与指纹（用于导入预览）。传入私钥或 .pub 均可。
#[tauri::command]
pub async fn inspect_ssh_key(path: String) -> Result<SshKeyInfo, String> {
    tokio::task::spawn_blocking(move || {
        let binary = keygen_binary();
        let p = PathBuf::from(&path);
        let public_path = if p.extension().map(|e| e == "pub").unwrap_or(false) {
            p.clone()
        } else {
            PathBuf::from(format!("{}.pub", path))
        };
        if !public_path.is_file() {
            return Err("未找到对应的 .pub 公钥文件".to_string());
        }
        let public_key = read_public_key(&public_path)?;
        let fingerprint = fingerprint_of(&binary, &public_path);
        let algorithm = fingerprint
            .rsplit('(')
            .next()
            .and_then(|s| s.split(')').next())
            .unwrap_or("")
            .to_string();
        Ok(SshKeyInfo {
            private_path: path,
            public_path: public_path.to_string_lossy().to_string(),
            public_key,
            fingerprint,
            algorithm,
        })
    })
    .await
    .map_err(|e| format!("读取密钥信息任务失败: {}", e))?
}
