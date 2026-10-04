//! Native process ownership is qualified on Unix only.
use serde_json::Value;
use tauri::Window;
fn unavailable(window: &Window) -> Result<Value, String> {
    crate::files::main_window(window)?;
    Err("Native account supervision is unavailable on this platform. Use the CLI's ordinary terminal.".into())
}
#[tauri::command]
pub(crate) fn cli_profile_refresh_native(
    window: Window,
    profile_id: String,
) -> Result<Value, String> {
    let _ = profile_id;
    unavailable(&window)
}
#[tauri::command]
pub(crate) fn cli_profile_native_report(
    window: Window,
    profile_id: String,
) -> Result<Option<Value>, String> {
    let _ = profile_id;
    unavailable(&window).map(Some)
}
#[tauri::command]
pub(crate) fn cli_native_permissions(window: Window, run_id: String) -> Result<Value, String> {
    let _ = run_id;
    unavailable(&window)
}
#[tauri::command]
pub(crate) fn cli_native_permission_reply(window: Window, request: Value) -> Result<Value, String> {
    let _ = request;
    unavailable(&window)
}
#[tauri::command]
pub(crate) fn cli_native_handoff_preview(window: Window, request: Value) -> Result<Value, String> {
    let _ = request;
    unavailable(&window)
}
#[tauri::command]
pub(crate) fn cli_native_handoff_apply(window: Window, request: Value) -> Result<Value, String> {
    let _ = request;
    unavailable(&window)
}
