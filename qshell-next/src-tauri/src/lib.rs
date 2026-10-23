// 注意：本文件是 QShell 早期（0.99.x 之前）的宿主实现，已不再参与实际构建。
// 真正的宿主编译入口是 ../../QShell-WinUI3/src/lib.rs（含托盘、剪贴板、自启动、本地终端等）。
// 保留此文件仅用于参考历史差异，请勿在此处新增或修改功能。
mod ssh;
mod config;
mod probe;
mod sftp;
mod monitor;
mod backup;
mod traffic;
mod local_fs;
mod generic_terminal;

use ssh::{SshManager, start_ssh_session, write_ssh_session, resize_ssh_session, close_ssh_session};
use generic_terminal::{GenericStreamManager, start_telnet_session, start_serial_session, write_generic_session, resize_generic_session, close_generic_session};
use config::{load_hosts_config, save_hosts_config};
use probe::probe_latency;
use traffic::audit_host_traffic;
use sftp::{
    list_sftp_files, sftp_mkdir, sftp_remove, sftp_rename,
    sftp_chmod, sftp_stat, sftp_upload_file, sftp_download_file,
    sftp_home_dir, cancel_sftp_transfer
};
use monitor::fetch_system_metrics;
use local_fs::{list_local_dir, local_roots, local_known_folders, local_path_exists, read_local_image_data_url, local_image_thumbnail};
use backup::{
    export_local_backup, import_local_backup, test_webdav_connection,
    webdav_upload_backup, webdav_download_backup
};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;

#[tauri::command]
fn is_autostart_launch() -> bool {
    std::env::args().any(|arg| arg == "--autostart")
}

#[tauri::command]
fn get_autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|error| format!("读取开机自启状态失败: {}", error))
}

#[tauri::command]
fn set_autostart_enabled(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    if enabled {
        manager
            .enable()
            .map_err(|error| format!("开启开机自启失败: {}", error))
    } else {
        manager
            .disable()
            .map_err(|error| format!("关闭开机自启失败: {}", error))
    }
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[tauri::command]
fn list_serial_ports() -> Vec<String> {
    serialport::available_ports()
        .map(|ports| {
            let mut list: Vec<String> = ports.into_iter().map(|p| p.port_name).collect();
            list.sort();
            list
        })
        .unwrap_or_default()
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn notify_tray(body: String) -> Result<(), String> {
    notify_rust::Notification::new()
        .app_id("QShell")
        .summary("QShell")
        .body(&body)
        .sound_name("Default")
        .show()
        .map(|_| ())
        .map_err(|error| format!("发送系统通知失败: {}", error))
}

#[cfg(windows)]
fn register_notification_app_id() {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok((key, _)) = hkcu.create_subkey(r"Software\Classes\AppUserModelId\QShell") {
        let _ = key.set_value("DisplayName", &"QShell");
        let _ = key.set_value("ShowInSettings", &1u32);
        if let Ok(exe) = std::env::current_exe() {
            let _ = key.set_value("IconUri", &exe.to_string_lossy().to_string());
        }
    }
}

#[cfg(not(windows))]
fn register_notification_app_id() {}

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show_item = MenuItem::with_id(app, "show", "显示主界面", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出 QShell", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .tooltip("QShell")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "quit" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.destroy();
                } else {
                    app.exit(0);
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

#[cfg(windows)]
#[repr(C)]
struct NcRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[cfg(windows)]
#[repr(C)]
struct NcCalcSizeParams {
    rgrc: [NcRect; 3],
    lppos: *mut std::ffi::c_void,
}

#[cfg(windows)]
unsafe extern "system" fn nc_border_subclass_proc(
    hwnd: isize,
    msg: u32,
    wparam: usize,
    lparam: isize,
    _id: usize,
    _data: usize,
) -> isize {
    #[link(name = "comctl32")]
    unsafe extern "system" {
        fn DefSubclassProc(
            hwnd: isize,
            msg: u32,
            wparam: usize,
            lparam: isize,
        ) -> isize;
    }

    const WM_NCCALCSIZE: u32 = 0x0083;
    if msg == WM_NCCALCSIZE && wparam != 0 && lparam != 0 {
        let params = &mut *(lparam as *mut NcCalcSizeParams);
        // 保留 1px 非客户区，让 DWM 有一个可绘制的窗口框来应用圆角。
        params.rgrc[0].left += 1;
        params.rgrc[0].top += 1;
        params.rgrc[0].right -= 1;
        params.rgrc[0].bottom -= 1;
        return 0;
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

#[cfg(windows)]
fn install_nc_border_subclass(window: &tauri::WebviewWindow) {
    #[link(name = "comctl32")]
    unsafe extern "system" {
        fn SetWindowSubclass(
            hwnd: isize,
            proc: Option<unsafe extern "system" fn(isize, u32, usize, isize, usize, usize) -> isize>,
            id: usize,
            data: usize,
        ) -> i32;
    }
    if let Ok(hwnd) = window.hwnd() {
        unsafe {
            let _ = SetWindowSubclass(
                hwnd.0 as isize,
                Some(nc_border_subclass_proc),
                1,
                0,
            );
        }
    }
}

#[cfg(windows)]
fn apply_windows_rounded_corners(window: &tauri::WebviewWindow) {
    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: isize,
            attribute: u32,
            value: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }

    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    // DWMWA_WINDOW_CORNER_PREFERENCE = 33, DWMWCP_ROUND = 2
    let preference: i32 = 2;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd.0 as isize,
            33,
            &preference as *const i32 as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }
}

#[cfg(not(windows))]
fn apply_windows_rounded_corners(_window: &tauri::WebviewWindow) {}

#[cfg(windows)]
fn clear_windows_rounded_corners(_window: &tauri::WebviewWindow) {
    // DWM 圆角不需要清除窗口区域。
}

#[cfg(not(windows))]
fn clear_windows_rounded_corners(_window: &tauri::WebviewWindow) {}

#[cfg(windows)]
fn handle_window_resize(window: &tauri::WebviewWindow) {
    apply_windows_rounded_corners(window);
}

#[cfg(not(windows))]
fn handle_window_resize(_window: &tauri::WebviewWindow) {}

#[tauri::command]
fn apply_window_rounding(window: tauri::WebviewWindow) {
    let _ = window.set_shadow(false);
    apply_windows_rounded_corners(&window);
}

fn install_crash_logging() {
    use std::io::Write;
    let log_path = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("QShell")
        .join("crash.log");
    let _ = std::fs::create_dir_all(log_path.parent().unwrap_or(&std::env::temp_dir()));
    std::panic::set_hook(Box::new(move |info| {
        let message = format!(
            "\n[{}] {}\n",
            chrono_like_timestamp(),
            info
        );
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let _ = file.write_all(message.as_bytes());
            let _ = file.flush();
        }
        eprintln!("{}", message);
    }));
}

#[cfg(windows)]
fn install_native_crash_handler() {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "dbghelp")]
    unsafe extern "system" {
        fn MiniDumpWriteDump(
            process: isize,
            process_id: u32,
            file: isize,
            dump_type: u32,
            exception_param: *mut std::ffi::c_void,
            user_stream_param: *mut std::ffi::c_void,
            callback_param: *mut std::ffi::c_void,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetCurrentProcessId() -> u32;
        fn SetUnhandledExceptionFilter(
            filter: Option<unsafe extern "system" fn(*mut std::ffi::c_void) -> i32>,
        ) -> *mut std::ffi::c_void;
    }

    unsafe extern "system" fn crash_filter(info: *mut std::ffi::c_void) -> i32 {
        let base = std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("QShell");
        let _ = std::fs::create_dir_all(&base);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let dump_path = base.join(format!("crash-{}.dmp", stamp));
        if let Ok(file) = std::fs::File::create(&dump_path) {
            unsafe {
                let _ = MiniDumpWriteDump(
                    GetCurrentProcess(),
                    GetCurrentProcessId(),
                    file.as_raw_handle() as isize,
                    2,
                    info,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
            }
        }
        let _ = std::fs::write(base.join("crash.flag"), dump_path.to_string_lossy().as_bytes());
        let _ = std::fs::write(
            base.join("crash-native.log"),
            format!("[{}] unhandled exception, dump: {}\n", chrono_like_timestamp(), dump_path.display()),
        );
        0
    }

    unsafe {
        let _ = SetUnhandledExceptionFilter(Some(crash_filter));
    }
}

#[cfg(not(windows))]
fn install_native_crash_handler() {}

#[cfg(windows)]
fn spawn_watchdog() {
    use std::process::Command;
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe)
            .arg("--watchdog")
            .arg(std::process::id().to_string())
            .spawn();
    }
}

#[cfg(not(windows))]
fn spawn_watchdog() {}

#[cfg(windows)]
fn run_watchdog(parent_pid: u32) {
    use std::process::Command;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> isize;
        fn WaitForSingleObject(handle: isize, milliseconds: u32) -> u32;
        fn GetExitCodeProcess(handle: isize, code: *mut u32) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const INFINITE: u32 = 0xFFFF_FFFF;
    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, parent_pid) };
    if handle == 0 {
        return;
    }
    let mut code: u32 = 0;
    unsafe {
        let _ = WaitForSingleObject(handle, INFINITE);
        let _ = GetExitCodeProcess(handle, &mut code);
        let _ = CloseHandle(handle);
    }
    if code == 0 {
        return;
    }
    let base = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("QShell");
    let flag = base.join("crash.flag");
    if !flag.exists() {
        return;
    }
    let _ = std::fs::remove_file(flag);
    std::thread::sleep(std::time::Duration::from_millis(900));
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe).arg("--autostart").spawn();
    }
}

#[cfg(not(windows))]
fn run_watchdog(_parent_pid: u32) {}

fn chrono_like_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", now.as_secs(), now.subsec_millis())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|v| v == "--watchdog").unwrap_or(false) {
        if let Some(pid) = args.get(2).and_then(|v| v.parse::<u32>().ok()) {
            run_watchdog(pid);
        }
        return;
    }

    install_crash_logging();
    install_native_crash_handler();
    spawn_watchdog();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .manage(SshManager::new())
        .manage(GenericStreamManager::new())
        .setup(|app| {
            register_notification_app_id();
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_shadow(false);
                clear_windows_rounded_corners(&window);
            }
            if let Err(error) = setup_tray(app) {
                eprintln!("系统托盘初始化失败: {}", error);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Resized(_) = event {
                if let Some(webview) = window.app_handle().get_webview_window(window.label()) {
                    handle_window_resize(&webview);
                }
            }
            if let tauri::WindowEvent::Destroyed = event {
                if window.label() == "main" {
                    window.app_handle().exit(0);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            start_ssh_session,
            write_ssh_session,
            resize_ssh_session,
            close_ssh_session,
            load_hosts_config,
            save_hosts_config,
            probe_latency,
            list_sftp_files,
            sftp_home_dir,
            sftp_mkdir,
            sftp_remove,
            sftp_rename,
            sftp_chmod,
            sftp_stat,
            sftp_upload_file,
            sftp_download_file,
            cancel_sftp_transfer,
            apply_window_rounding,
            list_local_dir,
            local_roots,
            local_known_folders,
            local_path_exists,
            read_local_image_data_url,
            local_image_thumbnail,
            fetch_system_metrics,
            audit_host_traffic,
            quit_app,
            list_serial_ports,
            start_telnet_session,
            start_serial_session,
            write_generic_session,
            resize_generic_session,
            close_generic_session,
            notify_tray,
            is_autostart_launch,
            get_autostart_enabled,
            set_autostart_enabled,
            export_local_backup,
            import_local_backup,
            test_webdav_connection,
            webdav_upload_backup,
            webdav_download_backup
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
