use std::path::PathBuf;

fn state_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("QShell").join("session_state.json")
}

#[tauri::command]
pub fn save_session_state(json: String) -> Result<(), String> {
    let path = state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    std::fs::write(&path, json).map_err(|e| format!("保存会话状态失败: {}", e))
}

#[tauri::command]
pub fn load_session_state() -> Result<Option<String>, String> {
    let path = state_path();
    if !path.exists() {
        return Ok(None);
    }
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|e| format!("读取会话状态失败: {}", e))
}

#[tauri::command]
pub fn clear_session_state() -> Result<(), String> {
    let path = state_path();
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("清除会话状态失败: {}", e))?;
    }
    Ok(())
}
