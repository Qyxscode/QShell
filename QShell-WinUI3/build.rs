fn main() {
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");
    println!("cargo:rerun-if-changed=app.manifest");

    // 显式嵌入 asInvoker manifest：避免 Windows 因缺少 manifest 而把程序当安装程序、要求管理员提权。
    let windows_attributes = tauri_build::WindowsAttributes::new()
        .app_manifest(include_str!("app.manifest"));

    tauri_build::try_build(
        tauri_build::Attributes::new().windows_attributes(windows_attributes),
    )
    .expect("failed to run tauri-build");
}
