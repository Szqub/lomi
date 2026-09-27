use serde_json::{json, Value};
use tauri::{AppHandle, Manager, Webview};

pub fn page(webview: &Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if webview.label() != "main"
        || !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
    {
        return;
    }
    let directory = std::env::var("LOMI_EXPLORER_CLIPBOARD_SMOKE_DIRECTORY").unwrap();
    let _ = webview.eval(include_str!("explorer-clipboard-smoke.js").replace(
        "SMOKE_DIRECTORY",
        &serde_json::to_string(&directory).unwrap(),
    ));
}

pub fn result(app: &AppHandle, stage: &str, data: Value) -> Result<Value, String> {
    let directory = std::path::PathBuf::from(
        std::env::var("LOMI_EXPLORER_CLIPBOARD_SMOKE_DIRECTORY")
            .map_err(|error| error.to_string())?,
    );
    match stage {
        "window-status" => {
            let window = app
                .get_webview_window("main")
                .ok_or("Main window is unavailable.")?;
            Ok(json!({
                "visible": window.is_visible().unwrap_or(false),
                "focused": window.is_focused().unwrap_or(false),
            }))
        }
        #[cfg(target_os = "macos")]
        "focus-window" => {
            let window = app
                .get_webview_window("main")
                .ok_or("Main window is unavailable.")?;
            app.run_on_main_thread(move || {
                let main = objc2::MainThreadMarker::new().unwrap();
                let application = objc2_app_kit::NSApplication::sharedApplication(main);
                unsafe {
                    let _: () = objc2::msg_send![&*application, activateIgnoringOtherApps: true];
                }
                let _ = window.set_focus();
            })
            .map_err(|error| error.to_string())?;
            Ok(Value::Null)
        }
        #[cfg(not(target_os = "macos"))]
        "focus-window" => Err("The Explorer clipboard smoke requires macOS.".into()),
        #[cfg(target_os = "macos")]
        "clipboard-menu" => {
            let action = data
                .get("action")
                .and_then(Value::as_str)
                .ok_or("Missing native Edit menu action")?;
            let index = match action {
                "cut" => 3isize,
                "copy" => 4isize,
                "paste" => 5isize,
                _ => return Err("Unknown native Edit menu action".into()),
            };
            app.run_on_main_thread(move || {
                let main = objc2::MainThreadMarker::new().unwrap();
                let application = objc2_app_kit::NSApplication::sharedApplication(main);
                unsafe {
                    let _: () = objc2::msg_send![&*application, activateIgnoringOtherApps: true];
                    let menu: *mut objc2::runtime::AnyObject =
                        objc2::msg_send![&*application, mainMenu];
                    let item: *mut objc2::runtime::AnyObject =
                        objc2::msg_send![menu, itemAtIndex: 1isize];
                    let submenu: *mut objc2::runtime::AnyObject = objc2::msg_send![item, submenu];
                    let _: () = objc2::msg_send![submenu, performActionForItemAtIndex: index];
                }
            })
            .map_err(|error| error.to_string())?;
            Ok(json!({ "action": action }))
        }
        #[cfg(not(target_os = "macos"))]
        "clipboard-menu" => Err("The Explorer clipboard smoke requires macOS.".into()),
        "clipboard-status" => {
            let output = std::process::Command::new(directory.join("clipboard"))
                .arg("status")
                .arg(&directory)
                .output()
                .map_err(|error| error.to_string())?;
            if !output.status.success() {
                return Err("Could not inspect the isolated clipboard marker.".into());
            }
            serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())
        }
        "passed" | "failed" => {
            std::fs::write(
                directory.join("result.json"),
                serde_json::to_vec_pretty(&json!({ "stage": stage, "data": data }))
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            app.exit(if stage == "passed" { 0 } else { 1 });
            Ok(Value::Null)
        }
        _ => Err("Unknown Explorer clipboard smoke stage.".into()),
    }
}
