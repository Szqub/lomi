use tauri::Manager;

pub fn page(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if webview.label() == "main"
        && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
    {
        let directory = std::env::var("LOMI_USAGE_SMOKE_DIRECTORY").unwrap();
        let script = include_str!("agent-usage-smoke.js")
            .replace(
                "SMOKE_DIRECTORY",
                &serde_json::to_string(&directory).unwrap(),
            )
            .replace(
                "SMOKE_LIVE_HOME",
                &serde_json::to_string(&std::env::var("LOMI_USAGE_SMOKE_LIVE_HOME").ok()).unwrap(),
            );
        let script = script.replace(
            "SMOKE_LIVE_MODE",
            &serde_json::to_string(&std::env::var("LOMI_USAGE_SMOKE_LIVE_MODE").ok()).unwrap(),
        );
        let script = script.replace(
            "SMOKE_LIVE_KIMI_HOME",
            &serde_json::to_string(&std::env::var("LOMI_USAGE_SMOKE_LIVE_KIMI_HOME").ok()).unwrap(),
        );
        let _ = webview.eval(script);
    }
}

pub fn result(
    app: &tauri::AppHandle,
    stage: &str,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    match stage {
        "usage-settings" => {
            let script = format!(
                r#"(async()=>{{const invoke=window.__TAURI_INTERNALS__.invoke;const data={};let denied=false;try{{await invoke('inspect_cli_usage',{{targets:[],force:false}});}}catch{{denied=true;}}const report=denied?data:{{...data,offlineSmoke:'failed',settingsDenied:false}};await invoke('plugin_smoke_result',{{stage:denied?'passed':'failed',data:report}});}})();"#,
                serde_json::to_string(&data).unwrap(),
            );
            app.get_webview("settings")
                .ok_or("Missing settings webview")?
                .eval(script)
                .map_err(|error| error.to_string())?;
        }
        "passed" | "failed" => {
            let directory =
                std::env::var("LOMI_USAGE_SMOKE_DIRECTORY").map_err(|error| error.to_string())?;
            std::fs::write(
                std::path::Path::new(&directory).join("result.json"),
                serde_json::to_vec_pretty(&serde_json::json!({"stage":stage,"data":data})).unwrap(),
            )
            .map_err(|error| error.to_string())?;
            app.state::<crate::terminal::Terminals>().stop_all();
            app.exit(if stage == "passed" { 0 } else { 1 });
        }
        _ => return Err("Unknown agent usage smoke stage.".into()),
    }
    Ok(serde_json::Value::Null)
}
