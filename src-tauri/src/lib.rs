// Tauri core logic — shared between desktop and mobile

use tauri::command;

/// A simple greet command that returns a greeting message.
/// This is called from the JavaScript frontend via `invoke("greet", { name: "..." })`.
#[command]
fn greet(name: String) -> String {
    format!("你好，{}！欢迎使用 Tauri 🎉", name)
}

/// The app entry point.
///
/// On desktop, this is called from `main.rs`.
/// On Android, the `#[cfg_attr(mobile, tauri::mobile_entry_point)]` attribute
/// generates the necessary JNI bindings so Android can launch the Rust backend.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("启动应用时出错");
}
