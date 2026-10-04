//! Codex 0.160.0 app-server: private, subscription-authenticated text only.
//! Source contract: openai/codex tag rust-v0.160.0. No native thread resume
//! across accounts: callers reconstruct a logical text context for each attempt.
#[path = "codex_coding.rs"]
mod coding;
pub(crate) use coding::{coding_home, CodingProtocol, SavedThread, ToolRequest};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) const VERSION: &str = "0.160.0";
pub(crate) const MAX_OUTPUT: usize = 1024 * 1024;
const CATALOG: &str = include_str!("codex-models-0.160.0.json");
const MODELS: &[&str] = &[
    "gpt-6-astra",
    "gpt-6.1-sol",
    "gpt-6-sol",
    "gpt-6-luna",
    "gpt-5.6-sol",
    "gpt-5.5",
];
const FEATURES: &[&str] = &[
    "shell_tool",
    "shell_snapshot",
    "shell_snapshot_v2",
    "unified_exec",
    "unified_exec_tty",
    "shell_zsh_fork",
    "unified_exec_zsh_fork",
    "view_image",
    "multi_agent",
    "multi_agent_v2",
    "agent_message_board",
    "multi_agent_mode",
    "enable_fanout",
    "apps",
    "psp",
    "enable_mcp_apps",
    "plugins",
    "remote_plugin",
    "tool_suggest",
    "recommended_plugins",
    "hooks",
    "plugin_hooks",
    "browser_use",
    "browser_use_external",
    "computer_use",
    "image_generation",
    "memories",
    "external_agent_memory_import",
    "goals",
    "token_budget",
    "context_management",
    "current_time_reminder",
    "sleep_tool",
    "send_message_to_user_async",
    "send_async_message",
    "deferred_executor",
    "executor_capability_discovery",
    "code_mode",
    "code_mode_only",
    "code_mode_prewarm",
    "code_mode_host",
    "code_mode_buffered_exec",
    "js_repl",
    "js_repl_tools_only",
    "reasoning_effort_override",
    "step_model_switching",
    "daemon_auto_start",
    "remote_control",
    "realtime_conversation",
    "apply_patch_freeform",
    "search_tool",
    "standalone_web_search",
    "request_permissions_tool",
    "request_rule",
    "tool_search",
    "skill_mcp_dependency_install",
    "skill_search",
    "skill_env_var_dependency_prompt",
    "workspace_dependencies",
    "auth_elicitation",
    "tool_call_mcp_elicitation",
    "default_mode_request_user_input",
    "artifact",
    "worktrees",
    "unbounded_connection_retries",
    "network_proxy",
    "respect_system_proxy",
    "system_proxy_fallback",
    "in_app_local_automation",
    "in_app_updates",
    "external_migration",
];
// Exact embedded defaults at openai/codex rust-v0.160.0,
// codex-rs/config/defaults.toml. These lower-precedence defaults are admitted
// only when attributed to the owned pinned executable; our layer overrides
// credential persistence, docs and history before any inference.
fn packaged_defaults() -> Value {
    json!({"include_permissions_instructions":true,"include_apps_instructions":true,"include_collaboration_mode_instructions":true,"include_environment_context":true,"cli_auth_credentials_store":"file","mcp_oauth_credentials_store":"auto","project_doc_max_bytes":32768,"project_doc_fallback_filenames":[],"background_terminal_max_timeout":300000,"file_opener":"vscode","hide_agent_reasoning":false,"chatgpt_base_url":"https://chatgpt.com/backend-api/","project_root_markers":[".git"],"history":{"persistence":"save-all"}})
}
/// Resolve the exact native executable without evaluating an npm JavaScript
/// wrapper. Wrapper bytes are identical in rust-v0.160.0 and the published
/// @openai/codex@0.160.0 tarball (registry.npmjs.org). Native version admission
/// remains the caller's mandatory next gate, before spawn or credentials.
pub(crate) fn resolve_native(program: &Path) -> Result<PathBuf, String> {
    let program = program
        .canonicalize()
        .map_err(|_| "Cannot resolve the selected Codex executable.")?;
    if native_file(&program)? {
        return Ok(program);
    }
    let metadata = fs::metadata(&program).map_err(|_| "Cannot inspect the Codex wrapper.")?;
    if metadata.len() > 65536 {
        return Err("This Codex wrapper is not the pinned source wrapper.".into());
    }
    let bytes = fs::read(&program).map_err(|_| "Cannot inspect the Codex wrapper.")?;
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if digest != "61b0194f3bb6534439c8d26a3ed57d0805f84b884588b761795323eeb92fcf70"
        || program.file_name().and_then(|name| name.to_str()) != Some("codex.js")
        || program
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some("bin")
    {
        return Err("This Codex wrapper is not the pinned source wrapper.".into());
    }
    let root = program
        .parent()
        .and_then(Path::parent)
        .ok_or("Cannot establish the Codex package installation.")?;
    let package = package_metadata(&root.join("package.json"))?;
    if package["name"] != "@openai/codex"
        || package["version"] != VERSION
        || package["bin"]["codex"] != "bin/codex.js"
    {
        return Err("This Codex package is not the pinned installation.".into());
    }
    let (platform, arch, target) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("darwin", "arm64", "aarch64-apple-darwin"),
        ("macos", "x86_64") => ("darwin", "x64", "x86_64-apple-darwin"),
        ("linux", "aarch64") => ("linux", "arm64", "aarch64-unknown-linux-musl"),
        ("linux", "x86_64") => ("linux", "x64", "x86_64-unknown-linux-musl"),
        _ => return Err("Managed Codex execution is unavailable on this platform.".into()),
    };
    let name = format!("codex-{platform}-{arch}");
    let version = format!("{VERSION}-{platform}-{arch}");
    if package["optionalDependencies"][format!("@openai/{name}")]
        != format!("npm:@openai/codex@{version}")
    {
        return Err("The Codex native dependency does not match the pinned package.".into());
    }
    // Node createRequire searches these node_modules roots from the package's
    // bin directory upward. Resolve the first existing exact optional package;
    // do not fall through a present but invalid installation to another binary.
    for ancestor in program.parent().unwrap().ancestors() {
        if ancestor.file_name().and_then(|name| name.to_str()) == Some("node_modules") {
            continue;
        }
        let dependency = ancestor.join("node_modules").join("@openai").join(&name);
        match fs::symlink_metadata(&dependency) {
            Ok(_) => {
                let dependency = dependency
                    .canonicalize()
                    .map_err(|_| "Cannot resolve the Codex native dependency.")?;
                let metadata = package_metadata(&dependency.join("package.json"))?;
                if metadata["name"] != "@openai/codex"
                    || metadata["version"] != version
                    || metadata["os"] != json!([platform])
                    || metadata["cpu"] != json!([arch])
                {
                    return Err(
                        "The Codex native dependency is not the pinned platform package.".into(),
                    );
                }
                return native_candidate(
                    &dependency.join("vendor").join(target).join("bin/codex"),
                    &dependency,
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err("Cannot establish the Codex native dependency ownership.".into()),
        }
    }
    // Exact legacy vendor fallback in the pinned wrapper.
    native_candidate(&root.join("vendor").join(target).join("bin/codex"), root)
}
fn package_metadata(path: &Path) -> Result<Value, String> {
    if fs::metadata(path)
        .map_err(|_| "Cannot inspect Codex package ownership.")?
        .len()
        > 65536
    {
        return Err("Codex package metadata exceeds the supported bound.".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(|_| "Cannot inspect Codex package ownership.")?)
        .map_err(|_| "Cannot inspect Codex package ownership.".into())
}
fn native_candidate(path: &Path, installation: &Path) -> Result<PathBuf, String> {
    let path = path
        .canonicalize()
        .map_err(|_| "The pinned Codex native dependency is unavailable.")?;
    if !path.starts_with(installation) {
        return Err("The Codex native dependency escapes its pinned installation.".into());
    }
    if !native_file(&path)? {
        return Err("The Codex dependency is not a native executable.".into());
    }
    Ok(path)
}
fn native_file(path: &Path) -> Result<bool, String> {
    let metadata = fs::metadata(path).map_err(|_| "Cannot inspect the native Codex executable.")?;
    if !metadata.is_file() {
        return Err("The Codex executable is not a regular file.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o111 == 0
            || metadata.mode() & 0o022 != 0
            || ![0, unsafe { libc::geteuid() }].contains(&metadata.uid())
        {
            return Err("The Codex executable ownership is not trusted.".into());
        }
    }
    let mut header = [0u8; 4];
    fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|_| "Cannot inspect the native Codex executable.")?;
    Ok(match std::env::consts::OS {
        "linux" => header == [0x7f, b'E', b'L', b'F'],
        "macos" => [
            [0xfe, 0xed, 0xfa, 0xce],
            [0xce, 0xfa, 0xed, 0xfe],
            [0xfe, 0xed, 0xfa, 0xcf],
            [0xcf, 0xfa, 0xed, 0xfe],
            [0xca, 0xfe, 0xba, 0xbe],
            [0xbe, 0xba, 0xfe, 0xca],
            [0xca, 0xfe, 0xba, 0xbf],
            [0xbf, 0xba, 0xfe, 0xca],
        ]
        .contains(&header),
        _ => false,
    })
}
pub(crate) fn model(value: Option<&str>) -> Result<&str, String> {
    value
        .filter(|id| MODELS.contains(id))
        .ok_or_else(|| "Choose an exact supported Codex text model. No request was sent.".into())
}
pub(crate) fn effort<'a>(id: &str, value: Option<&'a str>) -> Result<&'a str, String> {
    model(Some(id))?;
    let selected =
        value.ok_or("Choose an explicit Codex reasoning effort. No request was sent.")?;
    let catalog: Value =
        serde_json::from_str(CATALOG).map_err(|_| "Codex catalog is unavailable.")?;
    let entry = catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["slug"] == id)
        .ok_or("Codex model is absent from the pinned catalog.")?;
    if !entry["supported_reasoning_levels"]
        .as_array()
        .is_some_and(|levels| levels.iter().any(|level| level["effort"] == selected))
    {
        return Err("This exact Codex model does not support the requested reasoning effort. No request was sent.".into());
    }
    Ok(selected)
}
/// Choices come from the same owned catalog and admission functions as dispatch.
pub(crate) fn model_choices() -> Vec<(String, Vec<String>)> {
    let Ok(catalog) = serde_json::from_str::<Value>(CATALOG) else {
        return Vec::new();
    };
    let Some(entries) = catalog["models"].as_array() else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let id = entry["slug"].as_str()?;
            model(Some(id)).ok()?;
            let levels = entry["supported_reasoning_levels"].as_array()?;
            let choices = levels
                .iter()
                .filter_map(|level| {
                    let selected = level["effort"].as_str()?;
                    effort(id, Some(selected)).ok().map(str::to_owned)
                })
                .collect();
            Some((id.to_owned(), choices))
        })
        .collect()
}

pub(crate) fn version_matches(output: &str) -> bool {
    output.trim() == format!("codex-cli {VERSION}")
}
fn restrictions(root: &Path, id: &str) -> Value {
    let features: serde_json::Map<String, Value> = FEATURES
        .iter()
        .map(|key| ((*key).into(), json!(false)))
        .collect();
    json!({"model":id,"model_provider":"openai","model_catalog_json":root.join("models.json"),"cli_auth_credentials_store":"ephemeral","approval_policy":"never","approvals_reviewer":"user","sandbox_mode":"read-only","web_search":"disabled","notify":[],"project_doc_max_bytes":0,"project_root_markers":[],"project_doc_fallback_filenames":[],"check_for_update_on_startup":false,"analytics":{"enabled":false},"feedback":{"enabled":false},"history":{"persistence":"none"},"tools":{"update_plan":{"enabled":false},"experimental_request_user_input":{"enabled":false}},"skills":{"include_instructions":false,"bundled":{"enabled":false}},"cloud":{"skills":{"enabled":false}},"memories":{"generate_memories":false,"use_memories":false,"dedicated_tools":false},"features":features})
}
/// Check before launching, because system layers may cause effects during startup.
pub(super) fn ambient_policy() -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // /etc may be the platform's root-owned /private/etc alias. Resolve it,
        // then require every real parent to be root-owned and non-writable.
        let parent = Path::new("/etc")
            .canonicalize()
            .map_err(|_| "Cannot establish trusted Codex system policy parents.")?;
        for ancestor in parent.ancestors() {
            let meta = fs::symlink_metadata(ancestor)
                .map_err(|_| "Cannot establish trusted Codex system policy parents.")?;
            if !meta.is_dir() || meta.uid() != 0 || meta.mode() & 0o022 != 0 {
                return Err("Codex system policy parents are not trusted.".into());
            }
        }
        match fs::symlink_metadata(parent.join("codex")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => {
                return Err(
                    "Managed Codex text turns require an absent /etc/codex namespace.".into(),
                )
            }
        }
    }
    #[cfg(target_os = "macos")]
    forced_preferences()?;
    Ok(())
}
#[cfg(target_os = "macos")]
fn forced_preferences() -> Result<(), String> {
    use std::ffi::{c_char, c_void};
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(a: *const c_void, s: *const c_char, e: u32) -> *const c_void;
        fn CFPreferencesAppValueIsForced(key: *const c_void, application: *const c_void) -> u8;
        fn CFRelease(value: *const c_void);
    }
    unsafe {
        let domain =
            CFStringCreateWithCString(std::ptr::null(), c"com.openai.codex".as_ptr(), 0x08000100);
        if domain.is_null() {
            return Err("Cannot inspect managed Codex preferences.".into());
        }
        let mut forced = false;
        for key in [c"config_toml_base64", c"requirements_toml_base64"] {
            let key = CFStringCreateWithCString(std::ptr::null(), key.as_ptr(), 0x08000100);
            if key.is_null() {
                CFRelease(domain);
                return Err("Cannot inspect managed Codex preferences.".into());
            }
            forced |= CFPreferencesAppValueIsForced(key, domain) != 0;
            CFRelease(key);
        }
        CFRelease(domain);
        if forced {
            return Err("Forced Codex preferences prevent isolated text execution.".into());
        }
    }
    Ok(())
}
pub(crate) fn home(parent: &Path, id: &str) -> Result<PathBuf, String> {
    model(Some(id))?;
    ambient_policy()?;
    let root = parent.join(format!("codex-{}", super::new_id()?));
    fs::create_dir(&root).map_err(|_| "Cannot create fresh Codex attempt storage.")?;
    crate::chat::storage::private(&root, true)?;
    let root = root
        .canonicalize()
        .map_err(|_| "Cannot resolve private Codex attempt storage.")?;
    crate::chat::storage::atomic(&root.join("models.json"), CATALOG.as_bytes())?;
    // Each top-level JSON value is valid TOML inline syntax except JSON objects.
    // Explicit recursive serializer below produces TOML inline tables safely.
    let config = restrictions(&root, id)
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| format!("{key} = {}\n", toml_value(value)))
        .collect::<String>();
    crate::chat::storage::atomic(&root.join("config.toml"), config.as_bytes())?;
    Ok(root)
}
fn toml_value(value: &Value) -> String {
    match value {
        Value::Object(map) => format!(
            "{{ {} }}",
            map.iter()
                .map(|(key, value)| format!(
                    "{} = {}",
                    serde_json::to_string(key).unwrap(),
                    toml_value(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Array(values) => format!(
            "[{}]",
            values.iter().map(toml_value).collect::<Vec<_>>().join(", ")
        ),
        _ => value.to_string(),
    }
}
pub(crate) fn environment(command: &mut Command, root: &Path) {
    command
        .current_dir(root)
        .env("HOME", root)
        .env("CODEX_HOME", root)
        .env("TMPDIR", root)
        .env("XDG_CONFIG_HOME", root)
        .env("XDG_DATA_HOME", root)
        .env("XDG_CACHE_HOME", root);
}
pub(crate) fn arguments(command: &mut Command, root: &Path, _id: &str) {
    environment(command, root);
    command.args(["app-server", "--listen", "stdio://"]);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rejection {
    Malformed,
    Failed,
    Effect,
    Auth,
    Policy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Initialize,
    LoginReady,
    Login,
    Quota,
    Config,
    Requirements,
    Models,
    Thread,
    Turn,
    Running,
    Terminal,
}
struct TextItem {
    text: String,
    phase: Option<String>,
    completed: bool,
}
pub(crate) struct Protocol {
    phase: Phase,
    root: PathBuf,
    executable: Option<PathBuf>,
    model: String,
    effort: String,
    prompt: String,
    thread: Option<String>,
    turn: Option<String>,
    account: Option<String>,
    items: HashMap<String, TextItem>,
    item_order: Vec<String>,
    delta: Option<String>,
    pub(crate) output: String,
    pub(crate) completed: bool,
    pub(crate) terminal: bool,
    pub(crate) exhausted: bool,
    pub(crate) effect: bool,
    pub(crate) rejection: Option<Rejection>,
}
fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"id":id,"method":method,"params":params})
}
impl Protocol {
    pub(crate) fn new(
        id: &str,
        effort: Option<&str>,
        root: &Path,
        prompt: &str,
    ) -> Result<Self, String> {
        model(Some(id))?;
        let effort = self::effort(id, effort)?;
        Ok(Self {
            phase: Phase::Initialize,
            root: root.into(),
            executable: None,
            model: id.into(),
            effort: effort.into(),
            prompt: prompt.into(),
            thread: None,
            turn: None,
            account: None,
            items: HashMap::new(),
            item_order: Vec::new(),
            delta: None,
            output: String::new(),
            completed: false,
            terminal: false,
            exhausted: false,
            effect: false,
            rejection: None,
        })
    }
    /// Correlate the packaged-default layer with the exact native child binary.
    pub(crate) fn bind_executable(&mut self, path: &Path) -> Result<(), String> {
        if self.phase != Phase::Initialize || self.executable.is_some() {
            return Err("Codex executable ownership is already fixed.".into());
        }
        let path = path
            .canonicalize()
            .map_err(|_| "Cannot resolve the owned Codex executable.")?;
        if !path.is_absolute() {
            return Err("Codex executable ownership is unavailable.".into());
        }
        self.executable = Some(path);
        Ok(())
    }
    pub(crate) fn initialize(&self) -> Value {
        request(
            1,
            "initialize",
            json!({"clientInfo":{"name":"lomi","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}),
        )
    }
    pub(crate) fn needs_login(&self) -> bool {
        self.phase == Phase::LoginReady
    }
    pub(crate) fn login(
        &mut self,
        access_token: &str,
        account_id: &str,
    ) -> Result<Value, Rejection> {
        if !self.needs_login() || access_token.is_empty() || account_id.is_empty() {
            return Err(Rejection::Auth);
        }
        self.phase = Phase::Login;
        self.account = Some(account_id.into());
        Ok(request(
            2,
            "account/login/start",
            json!({"type":"chatgptAuthTokens","accessToken":access_token,"chatgptAccountId":account_id,"chatgptPlanType":null}),
        ))
    }
    pub(crate) fn take_delta(&mut self) -> Option<String> {
        self.delta.take()
    }
    pub(crate) fn interrupt(&self) -> Option<Value> {
        Some(request(
            9,
            "turn/interrupt",
            json!({"threadId":self.thread.as_ref()?,"turnId":self.turn.as_ref()?}),
        ))
    }
    pub(crate) fn consume(&mut self, event: &Value) -> Result<Vec<Value>, Rejection> {
        let result = self.accept(event);
        if let Err(error) = result {
            self.rejection = Some(error);
            self.completed = false;
            if error == Rejection::Effect {
                self.effect = true;
            }
        }
        result
    }
    fn accept(&mut self, event: &Value) -> Result<Vec<Value>, Rejection> {
        if (self.rejection.is_some()
            && !(self.terminal && self.rejection == Some(Rejection::Failed)))
            || !event.is_object()
        {
            return Err(Rejection::Malformed);
        }
        if event.get("method").is_some() && event.get("id").is_some() {
            return Err(Rejection::Auth);
        }
        if let Some(id) = event.get("id").and_then(Value::as_u64) {
            if id == 9
                && matches!(self.phase, Phase::Running | Phase::Terminal)
                && event.get("result").is_some()
                && event.get("error").is_none()
            {
                return Ok(vec![]);
            }
            let expected = match self.phase {
                Phase::Initialize => 1,
                Phase::Login => 2,
                Phase::Quota => 8,
                Phase::Config => 3,
                Phase::Requirements => 4,
                Phase::Models => 5,
                Phase::Thread => 6,
                Phase::Turn => 7,
                _ => return Err(Rejection::Malformed),
            };
            if id != expected || event.get("error").is_some() {
                return Err(Rejection::Failed);
            }
            let result = event.get("result").ok_or(Rejection::Malformed)?;
            return self.response(result);
        }
        let method = event["method"].as_str().ok_or(Rejection::Malformed)?;
        let p = &event["params"];
        if !p.is_object() {
            return Err(Rejection::Malformed);
        }
        if self.terminal {
            match method {
                "thread/status/changed"
                | "thread/tokenUsage/updated"
                | "account/rateLimits/updated" => {}
                "account/updated" => {
                    if p["authMode"] != "chatgptAuthTokens" {
                        return Err(Rejection::Auth);
                    }
                    return Ok(vec![]);
                }
                "turn/completed" => return Err(Rejection::Malformed),
                _ => return Err(Rejection::Effect),
            }
        }
        match method {
            "account/updated"
                if matches!(
                    self.phase,
                    Phase::Login
                        | Phase::Quota
                        | Phase::Config
                        | Phase::Requirements
                        | Phase::Models
                ) =>
            {
                if p["authMode"] != "chatgptAuthTokens" {
                    return Err(Rejection::Auth);
                }
            }
            "account/login/completed"
                if matches!(
                    self.phase,
                    Phase::Login
                        | Phase::Quota
                        | Phase::Config
                        | Phase::Requirements
                        | Phase::Models
                ) =>
            {
                if !p["loginId"].is_null() || p["success"] != true || !p["error"].is_null() {
                    return Err(Rejection::Auth);
                }
            }
            "thread/started" if matches!(self.phase, Phase::Thread | Phase::Turn) => {
                let id = p["thread"]["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or(Rejection::Malformed)?;
                if self.thread.as_deref().is_some_and(|old| old != id) {
                    return Err(Rejection::Malformed);
                }
                self.thread = Some(id.into());
            }
            "thread/settings/updated" if matches!(self.phase, Phase::Turn | Phase::Running) => {
                if self.thread.is_none() || p["threadId"].as_str() != self.thread.as_deref() {
                    return Err(Rejection::Malformed);
                }
                self.settings(&p["threadSettings"])?;
            }
            "turn/started" if matches!(self.phase, Phase::Turn | Phase::Running) => {
                self.correlate(p, true)?;
                let id = p["turn"]["id"].as_str().ok_or(Rejection::Malformed)?;
                if self.turn.as_deref().is_some_and(|old| old != id) {
                    return Err(Rejection::Malformed);
                }
                self.turn = Some(id.into());
            }
            "item/started" | "item/completed" => {
                self.correlate(p, false)?;
                self.item(&p["item"], method == "item/completed")?;
            }
            "item/agentMessage/delta" => {
                self.correlate(p, false)?;
                let id = p["itemId"].as_str().ok_or(Rejection::Malformed)?;
                let delta = p["delta"].as_str().ok_or(Rejection::Malformed)?;
                let item = self.items.get_mut(id).ok_or(Rejection::Malformed)?;
                if item.completed || item.text.len() + delta.len() > MAX_OUTPUT {
                    return Err(Rejection::Malformed);
                }
                item.text.push_str(delta);
                self.reconcile_output()?;
            }
            "item/reasoning/summaryTextDelta"
            | "item/reasoning/textDelta"
            | "item/reasoning/summaryPartAdded"
            | "thread/tokenUsage/updated" => {
                self.correlate(p, false)?;
            }
            "turn/completed" => {
                self.correlate(p, true)?;
                let turn = &p["turn"];
                if turn["id"].as_str() != self.turn.as_deref() {
                    return Err(Rejection::Malformed);
                }
                let summary = turn["items"].as_array().ok_or(Rejection::Malformed)?;
                if turn["status"] == "completed" {
                    // The pinned server emits only its last meaningful message,
                    // with itemsView=summary, not the full text item history.
                    if summary.len() != 1 || turn["itemsView"] != "summary" {
                        return Err(Rejection::Malformed);
                    }
                    let item = &summary[0];
                    let last = self
                        .item_order
                        .iter()
                        .rev()
                        .find(|id| {
                            self.items
                                .get(*id)
                                .is_some_and(|item| !item.text.trim().is_empty())
                        })
                        .ok_or(Rejection::Failed)?;
                    if item["type"] != "agentMessage" || item["id"].as_str() != Some(last.as_str())
                    {
                        return Err(Rejection::Malformed);
                    }
                    let known = self.items.get(last).ok_or(Rejection::Malformed)?;
                    if !known.completed
                        || known.phase.as_deref() == Some("commentary")
                        || item["text"].as_str() != Some(known.text.as_str())
                        || item["phase"].as_str() != known.phase.as_deref()
                    {
                        return Err(Rejection::Malformed);
                    }
                    self.item(item, true)?;
                    if self.items.values().any(|item| !item.completed) {
                        return Err(Rejection::Malformed);
                    }
                } else {
                    // Failed/interrupted turns do not assert a canonical final.
                    // Any reported items still must be text-only and reconcile.
                    for item in summary {
                        self.item(item, true)?;
                    }
                }
                self.terminal = true;
                self.phase = Phase::Terminal;
                match turn["status"].as_str() {
                    Some("completed") if turn["error"].is_null() => {
                        if self.output.trim().is_empty() {
                            return Err(Rejection::Failed);
                        }
                        self.reconcile_output()?;
                        self.completed = true;
                    }
                    Some("failed") => {
                        self.exhausted = turn["error"]["codexErrorInfo"] == "usageLimitExceeded";
                        return Err(Rejection::Failed);
                    }
                    Some("interrupted") => return Err(Rejection::Failed),
                    _ => return Err(Rejection::Malformed),
                }
            }
            "error" => {
                self.correlate(p, false)?;
                if p["willRetry"] != false && p["willRetry"] != true {
                    return Err(Rejection::Malformed);
                }
            }
            "thread/status/changed" => {
                if p["threadId"].as_str() != self.thread.as_deref() || self.thread.is_none() {
                    return Err(Rejection::Malformed);
                }
                match p["status"]["type"].as_str() {
                    Some("idle" | "notLoaded") => {}
                    Some("systemError")
                        if !self.terminal
                            || (!self.completed && self.rejection == Some(Rejection::Failed)) => {}
                    Some("active")
                        if !self.terminal && empty_array(&p["status"]["activeFlags"]) => {}
                    _ => return Err(Rejection::Effect),
                }
            }
            "account/rateLimits/updated" => {}
            _ => return Err(Rejection::Effect),
        }
        Ok(vec![])
    }
    fn response(&mut self, result: &Value) -> Result<Vec<Value>, Rejection> {
        match self.phase {
            Phase::Initialize => {
                if !result.is_object() {
                    return Err(Rejection::Malformed);
                }
                self.phase = Phase::LoginReady;
                Ok(vec![json!({"method":"initialized"})])
            }
            Phase::Login => {
                if result["type"] != "chatgptAuthTokens" {
                    return Err(Rejection::Auth);
                }
                self.phase = Phase::Quota;
                Ok(vec![request(
                    8,
                    "account/rateLimits/read",
                    json!({"supportsLunaReserve":false,"excludeResetCreditDetails":true}),
                )])
            }
            Phase::Quota => {
                if result["accountId"].as_str() != self.account.as_deref() {
                    return Err(Rejection::Auth);
                }
                if result["ordinaryUsageAllowed"] != true {
                    return Err(Rejection::Policy);
                }
                self.phase = Phase::Config;
                Ok(vec![request(
                    3,
                    "config/read",
                    json!({"includeLayers":true,"cwd":self.root}),
                )])
            }
            Phase::Config => {
                let expected = restrictions(&self.root, &self.model);
                // The flattened effective config includes the supplied raw fields.
                // Reject absent, changed or overridden restrictions, then check all
                // active layers. Only pinned embedded defaults, the expected empty
                // Unix system layer and our exact owned user layer are admitted.
                let mut effective_expected = expected.clone();
                // Config.tools exposes web search only; validate other tool
                // switches against the exact raw owned layer below.
                effective_expected.as_object_mut().unwrap().remove("tools");
                if !contains(&result["config"], &effective_expected) {
                    return Err(Rejection::Policy);
                }
                let layers = result["layers"].as_array().ok_or(Rejection::Policy)?;
                let mut owned = false;
                let mut packaged = false;
                let mut system = false;
                let executable = self.executable.as_ref().ok_or(Rejection::Policy)?;
                for layer in layers {
                    if layer
                        .get("disabledReason")
                        .is_some_and(|value| !value.is_null())
                    {
                        continue;
                    }
                    if layer["name"]["type"] == "user"
                        && layer["name"]["file"].as_str() == self.root.join("config.toml").to_str()
                    {
                        if owned || layer["config"] != expected {
                            return Err(Rejection::Policy);
                        }
                        owned = true;
                    } else if layer["name"]["type"] == "packagedDefaults" {
                        if packaged
                            || layer["name"]["file"].as_str() != executable.to_str()
                            || layer["config"] != packaged_defaults()
                        {
                            return Err(Rejection::Policy);
                        }
                        packaged = true;
                    } else if layer["name"]["type"] == "system" {
                        // The pinned Unix loader always reports this empty layer.
                        // home() fenced the whole absent namespace under trusted
                        // root-owned parents before the process was launched.
                        if system
                            || layer["name"]["file"] != "/etc/codex/config.toml"
                            || !layer["config"]
                                .as_object()
                                .is_some_and(serde_json::Map::is_empty)
                        {
                            return Err(Rejection::Policy);
                        }
                        system = true;
                    } else if layer["name"]["type"] == "sessionFlags"
                        && layer["config"]
                            .as_object()
                            .is_some_and(serde_json::Map::is_empty)
                    {
                    } else {
                        return Err(Rejection::Policy);
                    }
                }
                if !owned || !packaged || !system {
                    return Err(Rejection::Policy);
                }
                self.phase = Phase::Requirements;
                Ok(vec![request(4, "configRequirements/read", json!({}))])
            }
            Phase::Requirements => {
                if result.get("requirements") != Some(&Value::Null) {
                    return Err(Rejection::Policy);
                }
                self.phase = Phase::Models;
                Ok(vec![request(5, "model/list", json!({}))])
            }
            Phase::Models => {
                // Never guess model aliases or accept an incomplete page.
                if result.get("nextCursor") != Some(&Value::Null) {
                    return Err(Rejection::Policy);
                }
                let models = result["data"].as_array().ok_or(Rejection::Malformed)?;
                let matching = models
                    .iter()
                    .filter(|m| m["model"] == self.model)
                    .collect::<Vec<_>>();
                if matching.len() != 1 {
                    return Err(Rejection::Policy);
                }
                let entry = matching[0];
                if entry["hidden"] != false
                    || entry["multiAgentVersion"] != "disabled"
                    || !entry["supportedReasoningEfforts"]
                        .as_array()
                        .is_some_and(|levels| {
                            levels
                                .iter()
                                .any(|level| level["reasoningEffort"] == self.effort)
                        })
                {
                    return Err(Rejection::Policy);
                }
                self.phase = Phase::Thread;
                Ok(vec![request(
                    6,
                    "thread/start",
                    json!({"model":self.model,"modelProvider":"openai","allowProviderModelFallback":false,"cwd":self.root,"runtimeWorkspaceRoots":[],"environments":[],"selectedCapabilityRoots":[],"dynamicTools":[],"experimentalRawEvents":false,"config":{"model_reasoning_effort":self.effort},"approvalPolicy":"never","approvalsReviewer":"user","sandbox":"read-only","ephemeral":true}),
                )])
            }
            Phase::Thread => {
                if result["model"] != self.model
                    || result["modelProvider"] != "openai"
                    || result["reasoningEffort"] != self.effort
                    || result["cwd"].as_str() != self.root.to_str()
                    || result["approvalPolicy"] != "never"
                    || result["approvalsReviewer"] != "user"
                    || result["sandbox"]["type"] != "readOnly"
                    || result["sandbox"]["networkAccess"] != false
                    || !empty_array(&result["runtimeWorkspaceRoots"])
                    || !empty_array(&result["instructionSources"])
                {
                    return Err(Rejection::Policy);
                }
                let thread = result["thread"]["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or(Rejection::Malformed)?;
                if self.thread.as_deref().is_some_and(|old| old != thread) {
                    return Err(Rejection::Malformed);
                }
                self.thread = Some(thread.into());
                self.phase = Phase::Turn;
                Ok(vec![request(
                    7,
                    "turn/start",
                    json!({"threadId":thread,"model":self.model,"effort":self.effort,"environments":[],"runtimeWorkspaceRoots":[],"input":[{"type":"text","text":self.prompt,"text_elements":[]}]}),
                )])
            }
            Phase::Turn => {
                let id = result["turn"]["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or(Rejection::Malformed)?;
                if self.turn.as_deref().is_some_and(|old| old != id) {
                    return Err(Rejection::Malformed);
                }
                self.turn = Some(id.into());
                self.phase = Phase::Running;
                Ok(vec![])
            }
            _ => Err(Rejection::Malformed),
        }
    }
    fn settings(&self, settings: &Value) -> Result<(), Rejection> {
        if settings["cwd"].as_str() != self.root.to_str()
            || settings["model"] != self.model
            || settings["modelProvider"] != "openai"
            || settings["effort"] != self.effort
            || settings["approvalPolicy"] != "never"
            || settings["approvalsReviewer"] != "user"
            || settings["sandboxPolicy"]["type"] != "readOnly"
            || settings["sandboxPolicy"]["networkAccess"] != false
            || !settings["serviceTier"].is_null()
            || settings["multiAgentMode"] != "explicitRequestOnly"
            || settings["collaborationMode"]["mode"] != "default"
            || settings["collaborationMode"]["settings"]["model"] != self.model
            || settings["collaborationMode"]["settings"]["reasoning_effort"] != self.effort
            || !settings["collaborationMode"]["settings"]["developer_instructions"].is_null()
        {
            return Err(Rejection::Policy);
        }
        Ok(())
    }
    fn correlate(&self, p: &Value, nested_turn: bool) -> Result<(), Rejection> {
        if !matches!(self.phase, Phase::Turn | Phase::Running | Phase::Terminal)
            || p["threadId"].as_str() != self.thread.as_deref()
        {
            return Err(Rejection::Malformed);
        }
        let id = if nested_turn {
            p["turn"]["id"].as_str()
        } else {
            p["turnId"].as_str()
        };
        if !nested_turn && self.turn.is_none() {
            return Err(Rejection::Malformed);
        }
        if self.turn.as_deref().is_some_and(|old| id != Some(old)) {
            return Err(Rejection::Malformed);
        }
        Ok(())
    }
    /// Keep the durable aggregate in native item order, including commentary.
    /// Boundaries are emitted once as text; final reconciliation may only append.
    fn reconcile_output(&mut self) -> Result<(), Rejection> {
        let aggregate = self
            .item_order
            .iter()
            .filter_map(|id| self.items.get(id))
            .filter(|item| !item.text.is_empty())
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        if aggregate.len() > MAX_OUTPUT || !aggregate.starts_with(&self.output) {
            return Err(Rejection::Malformed);
        }
        let tail = &aggregate[self.output.len()..];
        if !tail.is_empty() {
            self.delta.get_or_insert_with(String::new).push_str(tail);
        }
        self.output = aggregate;
        Ok(())
    }
    fn item(&mut self, item: &Value, complete: bool) -> Result<(), Rejection> {
        let id = item["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or(Rejection::Malformed)?;
        match item["type"].as_str() {
            Some("agentMessage") => {
                if !item["memoryCitation"].is_null()
                    || !item["questions"].is_null()
                    || !item["delivery"].is_null()
                {
                    return Err(Rejection::Effect);
                }
                let text = item["text"].as_str().ok_or(Rejection::Malformed)?;
                if text.len() > MAX_OUTPUT {
                    return Err(Rejection::Malformed);
                }
                let phase = match item["phase"].as_str() {
                    None if item["phase"].is_null() => None,
                    Some("commentary") => Some("commentary".to_string()),
                    Some("final_answer") => Some("final_answer".to_string()),
                    _ => return Err(Rejection::Malformed),
                };
                if let Some(known) = self.items.get_mut(id) {
                    if !complete
                        || !text.starts_with(&known.text)
                        || (known.completed && text != known.text)
                        || (known.phase.is_some() && known.phase != phase)
                    {
                        return Err(Rejection::Malformed);
                    }
                    known.text = text.into();
                    known.phase = phase;
                    known.completed = true;
                } else {
                    if complete || self.item_order.len() >= 4096 {
                        return Err(Rejection::Malformed);
                    }
                    self.item_order.push(id.into());
                    self.items.insert(
                        id.into(),
                        TextItem {
                            text: text.into(),
                            phase,
                            completed: false,
                        },
                    );
                }
                self.reconcile_output()?;
            }
            Some("reasoning") => {}
            Some("userMessage") => {
                let content = item["content"].as_array().ok_or(Rejection::Malformed)?;
                if content.len() != 1
                    || content[0]["type"] != "text"
                    || content[0]["text"] != self.prompt
                {
                    return Err(Rejection::Effect);
                }
            }
            _ => return Err(Rejection::Effect),
        }
        Ok(())
    }
}
fn empty_array(value: &Value) -> bool {
    value.as_array().is_some_and(Vec::is_empty)
}
fn contains(actual: &Value, expected: &Value) -> bool {
    match expected {
        Value::Object(map) => map.iter().all(|(key, value)| {
            actual
                .get(key)
                .is_some_and(|actual| contains(actual, value))
        }),
        _ => actual == expected,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn running() -> Protocol {
        let mut p = Protocol::new(
            "gpt-6.1-sol",
            Some("high"),
            Path::new("/private/codex"),
            "hello",
        )
        .unwrap();
        p.phase = Phase::Running;
        p.thread = Some("thread".into());
        p.turn = Some("turn".into());
        p
    }
    fn terminal(status: &str, error: Value) -> Value {
        json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"turn","status":status,"error":error,"items":[]}}})
    }
    #[test]
    fn authoritative_completion_confirms_deltas_and_emits_only_final_tail() {
        let mut p = running();
        let mut event = json!({"method":"item/started","params":{"threadId":"thread","turnId":"turn","item":{"id":"answer","type":"agentMessage","text":""}}});
        p.consume(&event).unwrap();
        p.consume(&json!({"method":"item/agentMessage/delta","params":{"threadId":"thread","turnId":"turn","itemId":"answer","delta":"provisional"}})).unwrap();
        assert_eq!(p.take_delta(), Some("provisional".into()));
        event["method"] = json!("item/completed");
        event["params"]["item"]["text"] = json!("provisional final");
        p.consume(&event).unwrap();
        assert_eq!(p.take_delta(), Some(" final".into()));
        let mut final_event = terminal("completed", Value::Null);
        final_event["params"]["turn"]["itemsView"] = json!("summary");
        final_event["params"]["turn"]["items"] = json!([event["params"]["item"].clone()]);
        p.consume(&final_event).unwrap();
        assert_eq!(p.take_delta(), None);
        p.consume(&json!({"method":"thread/status/changed","params":{"threadId":"thread","status":{"type":"idle"}}})).unwrap();
        assert!(p.completed && p.terminal);
        assert_eq!(p.output, "provisional final");
        assert!(!p.exhausted);
        assert!(p.consume(&terminal("completed", Value::Null)).is_err());
    }
    #[test]
    fn commentary_and_final_messages_preserve_the_durable_aggregate() {
        let mut p = running();
        let mut emitted = String::new();
        for (id, phase, provisional, final_text) in [
            ("comment", "commentary", "Working", "Working on it."),
            ("answer", "final_answer", "The", "The answer."),
            ("space", "final_answer", "  ", "  "),
        ] {
            p.consume(&json!({"method":"item/started","params":{"threadId":"thread","turnId":"turn","item":{"id":id,"type":"agentMessage","text":"","phase":phase}}})).unwrap();
            if let Some(delta) = p.take_delta() {
                emitted.push_str(&delta);
            }
            p.consume(&json!({"method":"item/agentMessage/delta","params":{"threadId":"thread","turnId":"turn","itemId":id,"delta":provisional}})).unwrap();
            if let Some(delta) = p.take_delta() {
                emitted.push_str(&delta);
            }
            p.consume(&json!({"method":"item/completed","params":{"threadId":"thread","turnId":"turn","item":{"id":id,"type":"agentMessage","text":final_text,"phase":phase}}})).unwrap();
            if let Some(delta) = p.take_delta() {
                emitted.push_str(&delta);
            }
        }
        let mut event = terminal("completed", Value::Null);
        event["params"]["turn"]["itemsView"] = json!("summary");
        event["params"]["turn"]["items"] = json!([{"id":"answer","type":"agentMessage","text":"The answer.","phase":"final_answer"}]);
        p.consume(&event).unwrap();
        assert!(p.completed);
        assert_eq!(p.output, "Working on it.\n\nThe answer.\n\n  ");
        assert_eq!(emitted, p.output);
        assert_eq!(p.take_delta(), None);
    }
    #[test]
    fn final_summary_must_confirm_last_meaningful_completed_message() {
        for (phase, summary_text) in [
            ("commentary", "commentary only"),
            ("final_answer", "changed final"),
        ] {
            let mut p = running();
            let start = json!({"method":"item/started","params":{"threadId":"thread","turnId":"turn","item":{"id":"answer","type":"agentMessage","text":"","phase":phase}}});
            p.consume(&start).unwrap();
            p.consume(&json!({"method":"item/completed","params":{"threadId":"thread","turnId":"turn","item":{"id":"answer","type":"agentMessage","text":"commentary only","phase":phase}}})).unwrap();
            let partial = p.output.clone();
            let mut event = terminal("completed", Value::Null);
            event["params"]["turn"]["itemsView"] = json!("summary");
            event["params"]["turn"]["items"] =
                json!([{"id":"answer","type":"agentMessage","text":summary_text,"phase":phase}]);
            assert_eq!(p.consume(&event), Err(Rejection::Malformed));
            assert_eq!(p.output, partial);
            assert!(!p.completed);
        }
    }
    #[cfg(unix)]
    #[test]
    fn arbitrary_script_wrappers_are_never_evaluated() {
        use std::os::unix::fs::PermissionsExt;
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("codex.js");
        fs::write(
            &path,
            "#!/usr/bin/env node\nthrow new Error('must never run');\n",
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(resolve_native(&path).is_err());
    }
    #[test]
    fn mismatched_canonical_text_retains_uncertain_partial() {
        let mut p = running();
        p.output = "partial".into();
        p.items.insert(
            "answer".into(),
            TextItem {
                text: "partial".into(),
                phase: None,
                completed: false,
            },
        );
        p.item_order.push("answer".into());
        assert_eq!(p.consume(&json!({"method":"item/completed","params":{"threadId":"thread","turnId":"turn","item":{"id":"answer","type":"agentMessage","text":"replacement"}}})),Err(Rejection::Malformed));
        assert_eq!(p.output, "partial");
        assert!(!p.completed);
    }
    #[test]
    fn startup_gates_require_owned_policy_and_authenticated_quota() {
        let mut p = running();
        p.phase = Phase::Quota;
        p.account = Some("account".into());
        assert_eq!(
            p.consume(&json!({"id":8,"result":{"accountId":"other","ordinaryUsageAllowed":true}})),
            Err(Rejection::Auth)
        );
        let mut p = running();
        p.phase = Phase::Config;
        p.executable = Some(PathBuf::from("/native/codex"));
        let config = restrictions(&p.root, &p.model);
        let response = json!({"id":3,"result":{"config":config,"layers":[
            {"name":{"type":"packagedDefaults","file":"/native/codex"},"config":packaged_defaults()},
            {"name":{"type":"system","file":"/etc/codex/config.toml"},"config":{}},
            {"name":{"type":"user","file":p.root.join("config.toml")},"config":config}
        ]}});
        assert_eq!(
            p.consume(&response).unwrap()[0]["method"],
            "configRequirements/read"
        );
        let mut p = running();
        p.phase = Phase::Config;
        p.executable = Some(PathBuf::from("/native/codex"));
        let mut response = response;
        response["result"]["layers"][0]["name"]["type"] = json!("mdm");
        assert_eq!(p.consume(&response), Err(Rejection::Policy));
    }
    fn settings_event() -> Value {
        json!({"method":"thread/settings/updated","params":{"threadId":"thread","threadSettings":{
            "cwd":"/private/codex","model":"gpt-6.1-sol","modelProvider":"openai","effort":"high",
            "approvalPolicy":"never","approvalsReviewer":"user","sandboxPolicy":{"type":"readOnly","networkAccess":false},
            "serviceTier":null,"multiAgentMode":"explicitRequestOnly",
            "collaborationMode":{"mode":"default","settings":{"model":"gpt-6.1-sol","reasoning_effort":"high","developer_instructions":null}}
        }}})
    }
    #[test]
    fn exact_settings_notifications_do_not_change_the_requested_contract() {
        let event = settings_event();
        running().consume(&event).unwrap();
        let mut p = running();
        p.phase = Phase::Turn;
        p.turn = None;
        p.consume(&event).unwrap();
        for (pointer, bad) in [
            ("/params/threadSettings/model", json!("gpt-6-sol")),
            ("/params/threadSettings/effort", json!("low")),
            ("/params/threadSettings/cwd", json!("/workspace")),
            ("/params/threadSettings/approvalPolicy", json!("on-request")),
            (
                "/params/threadSettings/sandboxPolicy/networkAccess",
                json!(true),
            ),
            (
                "/params/threadSettings/collaborationMode/mode",
                json!("plan"),
            ),
        ] {
            let mut bad_event = event.clone();
            *bad_event.pointer_mut(pointer).unwrap() = bad;
            assert_eq!(
                running().consume(&bad_event),
                Err(Rejection::Policy),
                "accepted {pointer}"
            );
        }
        let mut bad_event = event;
        bad_event["params"]["threadId"] = json!("other");
        assert_eq!(running().consume(&bad_event), Err(Rejection::Malformed));
    }
    #[test]
    fn pending_system_error_does_not_classify_exhaustion_until_typed_terminal() {
        let mut p = running();
        let status = json!({"method":"thread/status/changed","params":{"threadId":"thread","status":{"type":"systemError"}}});
        p.consume(&status).unwrap();
        assert!(!p.terminal);
        assert!(!p.exhausted);
        assert_eq!(
            p.consume(&terminal(
                "failed",
                json!({"codexErrorInfo":"usageLimitExceeded"})
            )),
            Err(Rejection::Failed)
        );
        p.consume(&status).unwrap();
        assert!(p.terminal && p.exhausted);
        assert_eq!(p.rejection, Some(Rejection::Failed));
    }
    #[test]
    fn foreign_packaged_defaults_and_nonempty_system_policy_are_rejected() {
        for (name, config) in [
            (
                json!({"type":"packagedDefaults","file":"/other/codex"}),
                packaged_defaults(),
            ),
            (
                json!({"type":"packagedDefaults","file":"/native/codex"}),
                json!({"notify":["effect"]}),
            ),
            (
                json!({"type":"system","file":"/etc/codex/config.toml"}),
                json!({"model_provider":"other"}),
            ),
        ] {
            let mut p = running();
            p.phase = Phase::Config;
            p.executable = Some(PathBuf::from("/native/codex"));
            let expected = restrictions(&p.root, &p.model);
            let response = json!({"id":3,"result":{"config":expected,"layers":[{"name":name,"config":config}]}});
            assert_eq!(p.consume(&response), Err(Rejection::Policy));
        }
    }
    #[test]
    fn catalog_is_owned_and_all_tool_capabilities_are_removed() {
        let catalog: Value = serde_json::from_str(CATALOG).unwrap();
        assert_eq!(catalog["models"].as_array().unwrap().len(), MODELS.len());
        for m in catalog["models"].as_array().unwrap() {
            assert!(MODELS.contains(&m["slug"].as_str().unwrap()));
            assert_eq!(m["shell_type"], "disabled");
            assert_eq!(m["tool_mode"], "direct");
            assert_eq!(m["multi_agent_version"], "disabled");
            assert_eq!(m["supports_search_tool"], false);
            assert!(m["apply_patch_tool_type"].is_null());
            assert!(empty_array(&m["experimental_supported_tools"]));
        }
    }
    #[test]
    fn terminal_exhaustion_is_typed_and_retains_partial_text() {
        for (kind, exhausted) in [
            ("usageLimitExceeded", true),
            ("rateLimitExceeded", false),
            ("sessionBudgetExceeded", false),
        ] {
            let mut p = running();
            p.output = "partial".into();
            assert_eq!(
                p.consume(&terminal("failed", json!({"codexErrorInfo":kind}))),
                Err(Rejection::Failed)
            );
            assert!(p.terminal);
            assert_eq!(p.exhausted, exhausted);
            assert_eq!(p.output, "partial");
            assert!(!p.completed);
            p.consume(&json!({"method":"thread/status/changed","params":{"threadId":"thread","status":{"type":"idle"}}})).unwrap();
            assert_eq!(p.exhausted, exhausted);
            assert_eq!(p.rejection, Some(Rejection::Failed));
            assert_eq!(
                p.consume(&terminal("failed", json!({"codexErrorInfo":kind}))),
                Err(Rejection::Malformed)
            );
        }
    }
    #[test]
    fn effects_wrong_correlation_and_refresh_are_rejected() {
        for kind in [
            "commandExecution",
            "fileChange",
            "mcpToolCall",
            "hookPrompt",
            "imageGeneration",
            "collabAgentToolCall",
            "webSearch",
            "plan",
        ] {
            let event = json!({"method":"item/started","params":{"threadId":"thread","turnId":"turn","item":{"id":"item","type":kind}}});
            assert_eq!(running().consume(&event), Err(Rejection::Effect));
        }
        let refresh = json!({"id":90,"method":"account/chatgptAuthTokens/refresh","params":{}});
        assert_eq!(running().consume(&refresh), Err(Rejection::Auth));
        let mut event = terminal("completed", Value::Null);
        event["params"]["threadId"] = json!("other");
        assert_eq!(running().consume(&event), Err(Rejection::Malformed));
    }
    #[test]
    fn model_and_login_never_silently_fallback() {
        assert!(model(Some("gpt-6")).is_err());
        assert!(effort("gpt-6.1-sol", None).is_err());
        assert!(Protocol::new(
            "gpt-6.1-sol",
            Some("invented"),
            Path::new("/private/codex"),
            "hello"
        )
        .is_err());
        let mut p = running();
        assert!(p.login("fixture", "account").is_err());
        assert!(version_matches("codex-cli 0.160.0\n"));
        assert!(!version_matches("codex-cli 0.159.0"));
    }
    #[test]
    fn effective_restrictions_are_fail_closed() {
        let root = Path::new("/private/codex");
        let expected = restrictions(root, "gpt-6.1-sol");
        assert!(contains(&expected, &expected));
        let mut actual = expected.clone();
        actual["features"]["shell_tool"] = json!(true);
        assert!(!contains(&actual, &expected));
        actual = expected.clone();
        actual.as_object_mut().unwrap().remove("skills");
        assert!(!contains(&actual, &expected));
        let mut command = Command::new("unused-fixture");
        command.env_clear();
        arguments(&mut command, root, "gpt-6.1-sol");
        assert_eq!(command.get_current_dir(), Some(root));
        assert_eq!(
            command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["app-server", "--listen", "stdio://"]
        );
    }
}
