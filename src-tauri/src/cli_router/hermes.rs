//! Hermes Agent 0.21.5 / stable v2026.9.24, f97608f178d1ffeca59860195ab7da295f7c8e5f.
//! Native Anthropic API text candidate, never OAuth or account failover. Source:
//! NousResearch/hermes-agent at this commit, hermes_cli/{main,stream_json,
//! cli_single_query,plugins,config_defaults}, agent/{conversation_loop,
//! turn_tool_validation,models_dev,anthropic_adapter}, model_tools.py, toolsets.py.
//! Admission requires an unmodified installed Python distribution before even
//! its version probe. The owner injects only the selected ANTHROPIC_API_KEY.
//! No real client/account has been exercised; this is not an OS sandbox.

use super::{
    runtime::{exit_pending, OwnedChild},
    transport::Outcome,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

pub(crate) const VERSION: &str = "0.21.5";
const MODEL: &str = "claude-sonnet-4-6";
const MAX_OUTPUT: usize = 1024 * 1024;
const MAX_CONTEXT: usize = 60 * 1024;
const PROMPT_PREFIX: &str = "Lomi text-only conversation. Answer using only the supplied conversation. Tools, files and external context are unavailable.\n\n";
const TIMEOUT: Duration = Duration::from_secs(300);
fn failure() -> String {
    "Hermes did not confirm its owned text-only contract. Review saved partial text before explicitly continuing.".into()
}
pub(crate) fn model(value: Option<&str>) -> Result<&str, String> {
    value.filter(|id| *id == MODEL).ok_or_else(|| {
        "Choose the exact admitted Hermes Anthropic text model. No request was sent.".into()
    })
}
pub(crate) fn supported_models() -> &'static [&'static str] {
    &[MODEL]
}
pub(crate) fn admit_api_key(value: &str) -> Result<(), String> {
    if value.trim().is_empty()
        || value.starts_with("sk-ant-oat01-")
        || value.chars().any(char::is_control)
    {
        return Err("Hermes requires a native Anthropic API key; OAuth credentials are unavailable. No request was sent.".into());
    }
    Ok(())
}
fn settings() -> Value {
    json!({
        "_config_version":46,
        "security":{"allow_lazy_installs":false},
        "model":{"provider":"anthropic","default":MODEL,"base_url":"https://api.anthropic.com","api_mode":"anthropic_messages","context_length":1000000},
        "fallback_providers":[], "providers":{},
        "agent":{"max_turns":1,"run_budget_seconds":300,"api_max_retries":1,"auto_recovery_cycles":0,"disabled_toolsets":["setup"],"reasoning_effort":"none","tool_use_enforcement":false},
        "compression":{"enabled":false},
        "auxiliary":{"title_generation":{"enabled":false,"model_upgrade_enabled":false},"background_review":{"enabled":false}},
        "memory":{"memory_enabled":false,"user_profile_enabled":false,"provider":""},
        "curator":{"enabled":false,"consolidate":false},
        "plugins":{"enabled":[],"disabled":[]},"mcp_servers":{},"hooks":{},"hooks_auto_accept":false,"outbound_webhooks":{},
        "model_catalog":{"enabled":false},"updates":{"check":false},
        "display":{"streaming":true,"final_response_markdown":"raw"},
        "model_overrides":{"anthropic":{(MODEL):{"context_window":1000000,"supports_tools":false,"supports_vision":false,"supports_reasoning":false}}}
    })
}
fn metadata() -> Value {
    json!({"anthropic":{"id":"anthropic","name":"Anthropic","models":{(MODEL):{
        "id":MODEL,"name":"Claude Sonnet 4.6","limit":{"context":1000000,"output":128000},
        "reasoning":false,"tool_call":false,"attachment":false,"temperature":true,
        "modalities":{"input":["text"],"output":["text"]}
    }}}})
}
pub(crate) fn home(parent: &Path, id: &str) -> Result<PathBuf, String> {
    model(Some(id))?;
    let root = parent.join(format!("hermes-{}", super::new_id()?));
    fs::create_dir(&root).map_err(|_| failure())?;
    crate::chat::storage::private(&root, true)?;
    let root = root.canonicalize().map_err(|_| failure())?;
    for name in [
        ".hermes",
        "config",
        "data",
        "state",
        "cache",
        "tmp",
        "bundled-skills",
        "optional-skills",
        "optional-mcps",
        "pycache",
        "empty-bin",
    ] {
        let path = root.join(name);
        fs::create_dir(&path).map_err(|_| failure())?;
        crate::chat::storage::private(&path, true)?;
    }
    // JSON is a strict subset of YAML. Never copy an existing profile or role.
    crate::chat::storage::atomic(
        &root.join(".hermes/config.yaml"),
        &serde_json::to_vec(&settings()).map_err(|_| failure())?,
    )?;
    crate::chat::storage::atomic(
        &root.join(".hermes/models_dev_cache.json"),
        &serde_json::to_vec(&metadata()).map_err(|_| failure())?,
    )?;
    Ok(root)
}
pub(crate) fn environment(command: &mut Command, root: &Path, _id: &str) {
    // Absolute native venv entrypoint/shebang admission makes inherited PATH
    // unnecessary. Python, provider, proxy, shell and loader overrides are absent.
    command
        .env_clear()
        .current_dir(root)
        .env("PATH", root.join("empty-bin"))
        .env("HOME", root)
        .env("USERPROFILE", root)
        .env("HERMES_REAL_HOME", root)
        .env("HERMES_HOME", root.join(".hermes"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("TMPDIR", root.join("tmp"))
        .env("TMP", root.join("tmp"))
        .env("TEMP", root.join("tmp"))
        .env("HERMES_DISABLE_LAZY_INSTALLS", "1")
        .env("HERMES_SAFE_MODE", "1")
        .env("HERMES_IGNORE_RULES", "1")
        .env("HERMES_QUIET", "1")
        .env("HERMES_GUEST_ONBOARDING", "0")
        .env("HERMES_BUNDLED_SKILLS", root.join("bundled-skills"))
        .env("HERMES_OPTIONAL_SKILLS", root.join("optional-skills"))
        .env("HERMES_OPTIONAL_MCPS", root.join("optional-mcps"))
        .env("PYTHONPYCACHEPREFIX", root.join("pycache"))
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONUTF8", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("LANG", "en_US.UTF-8");
}
pub(crate) fn version_matches(output: &str) -> bool {
    let first = output.lines().next().unwrap_or_default();
    let base = "Hermes Agent v0.21.5 (2026.9.24)";
    first == base
        || first == format!("{base} · upstream f97608f")
        || first == format!("{base} · upstream f97608f178d1ffeca59860195ab7da295f7c8e5f")
}

fn absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err("Hermes refuses installation environment overrides. No request was sent.".into()),
    }
}
fn trusted(path: &Path, directory: bool) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| failure())?;
    if metadata.file_type().is_symlink()
        || metadata.is_dir() != directory
        || (!directory && !metadata.is_file())
    {
        return Err(failure());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if ![0, unsafe { libc::geteuid() }].contains(&metadata.uid())
            || metadata.mode() & 0o022 != 0
        {
            return Err(failure());
        }
    }
    Ok(())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn python_tree(
    root: &Path,
    base: &Path,
    entries: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    trusted(root, true)?;
    for entry in fs::read_dir(root).map_err(|_| failure())? {
        let path = entry.map_err(|_| failure())?.path();
        let meta = fs::symlink_metadata(&path).map_err(|_| failure())?;
        if meta.file_type().is_symlink() {
            return Err(failure());
        }
        if meta.is_dir() {
            if path.file_name().is_some_and(|name| name == "__pycache__") {
                continue;
            }
            python_tree(&path, base, entries)?;
        } else if path
            .extension()
            .is_some_and(|ext| matches!(ext.to_str(), Some("pyc" | "pyo" | "so" | "pyd" | "dylib")))
        {
            // Sourceless bytecode and extension modules can win import lookup
            // over source whose digest otherwise matches the release.
            return Err(failure());
        } else if path.extension().is_some_and(|ext| ext == "py") {
            trusted(&path, false)?;
            if meta.len() > 2 * MAX_OUTPUT as u64 || entries.len() > 2048 {
                return Err(failure());
            }
            let relative = path
                .strip_prefix(base)
                .map_err(|_| failure())?
                .to_str()
                .ok_or_else(failure)?
                .replace('\\', "/");
            entries.insert(relative, fs::read(&path).map_err(|_| failure())?);
        }
    }
    Ok(())
}
fn tree_hash(entries: BTreeMap<String, Vec<u8>>) -> String {
    let mut digest = Sha256::new();
    for (path, bytes) in entries {
        digest.update((path.len() as u64).to_be_bytes());
        digest.update(path.as_bytes());
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    format!("{digest:x}", digest = digest.finalize())
}

fn reject_startup_modules(directory: &Path) -> Result<(), String> {
    trusted(directory, true)?;
    for entry in fs::read_dir(directory).map_err(|_| failure())? {
        let path = entry.map_err(|_| failure())?.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(failure)?;
        if ["sitecustomize", "usercustomize"]
            .iter()
            .any(|prefix| name == *prefix || name.starts_with(&format!("{prefix}.")))
        {
            return Err(failure());
        }
    }
    Ok(())
}

#[derive(Clone)]
struct Layout {
    launcher: PathBuf,
    root: PathBuf,
    python: PathBuf,
    site: PathBuf,
    stdlib: PathBuf,
}
fn source_layout(program: &Path) -> Result<Layout, String> {
    trusted(program, false)?;
    for ancestor in program.parent().ok_or_else(failure)?.ancestors() {
        trusted(ancestor, true)?;
    }
    if fs::metadata(program).map_err(|_| failure())?.len() > 8192 {
        return Err(failure());
    }
    let script = fs::read_to_string(program).map_err(|_| failure())?;
    let lines = script.lines().collect::<Vec<_>>();
    // scripts/install.sh2157: validate paths as data, never evaluate the shim.
    if lines.len() != 4
        || lines[..3]
            != [
                "#!/usr/bin/env bash",
                "unset PYTHONPATH",
                "unset PYTHONHOME",
            ]
    {
        return Err(failure());
    }
    let command = lines[3]
        .strip_prefix("exec \"")
        .and_then(|line| line.strip_suffix("\" \"$@\""))
        .ok_or_else(failure)?;
    let (python, entry) = command.split_once("\" \"").ok_or_else(failure)?;
    if [python, entry]
        .iter()
        .any(|path| path.contains(['"', '$', '`', '\\']) || path.chars().any(char::is_control))
    {
        return Err(failure());
    }
    let entry = Path::new(entry);
    let root = entry
        .parent()
        .ok_or_else(failure)?
        .canonicalize()
        .map_err(|_| failure())?;
    let python = PathBuf::from(python);
    if !python.is_absolute()
        || !entry.is_absolute()
        || entry != root.join("hermes")
        || python != root.join("venv/bin/python")
    {
        return Err(failure());
    }
    for ancestor in root.ancestors() {
        trusted(ancestor, true)?;
    }
    for name in [
        ".env",
        ".op.env",
        ".update-incomplete",
        ".lazy-refresh-incomplete",
        ".update-restore-claim",
    ] {
        absent(&root.join(name))?;
    }
    // Both early recovery and main recovery must remain incapable of repairing
    // a foreign installation or starting an installer before configuration.
    match fs::symlink_metadata(root.join(".git")) {
        Ok(_) => trusted(&root.join(".git"), true)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(failure()),
    }
    for name in ["hermes-update-pull", "hermes-update-pull.claim"] {
        absent(&root.join(".git").join(name))?;
    }
    trusted(&root.join("hermes"), false)?;
    if hash(&fs::read(root.join("hermes")).map_err(|_| failure())?) != ENTRYPOINT_HASH {
        return Err(failure());
    }
    trusted(&root.join("venv"), true)?;
    trusted(&root.join("venv/bin"), true)?;
    let native = python.canonicalize().map_err(|_| failure())?;
    trusted(&native, false)?;
    for ancestor in native.parent().ok_or_else(failure)?.ancestors() {
        trusted(ancestor, true)?;
    }
    trusted(&root.join("venv/pyvenv.cfg"), false)?;
    let cfg = fs::read_to_string(root.join("venv/pyvenv.cfg")).map_err(|_| failure())?;
    let field = |name: &str| {
        cfg.lines().find_map(|line| {
            line.split_once('=')
                .and_then(|(key, value)| (key.trim() == name).then_some(value.trim()))
        })
    };
    if field("include-system-site-packages") != Some("false") {
        return Err(failure());
    }
    let version = field("version").ok_or_else(failure)?;
    let mut numbers = version.split('.');
    if numbers.next() != Some("3") {
        return Err(failure());
    }
    let minor = numbers
        .next()
        .filter(|minor| ["11", "12", "13"].contains(minor))
        .ok_or_else(failure)?;
    let site = root
        .join("venv/lib")
        .join(format!("python3.{minor}/site-packages"));
    let site = site.canonicalize().map_err(|_| failure())?;
    if !site.starts_with(root.join("venv")) {
        return Err(failure());
    }
    trusted(&site, true)?;
    // Framework Python binaries live below Resources/Python.app; ordinary
    // CPython binaries live in bin. Use the closest native-prefix stdlib.
    let stdlib = native
        .ancestors()
        .skip(1)
        .take(10)
        .map(|ancestor| ancestor.join("lib").join(format!("python3.{minor}")))
        .find(|path| path.join("os.py").is_file())
        .ok_or_else(failure)?;
    let stdlib = stdlib.canonicalize().map_err(|_| failure())?;
    for ancestor in stdlib.ancestors() {
        trusted(ancestor, true)?;
    }
    let base = stdlib.parent().and_then(Path::parent).ok_or_else(failure)?;
    reject_startup_modules(&stdlib)?;
    reject_startup_modules(&stdlib.join("lib-dynload"))?;
    absent(&base.join("lib").join(format!("python3{minor}.zip")))?;
    for directory in [&root, &site, &root.join("venv/bin")] {
        reject_startup_modules(directory)?;
        for entry in fs::read_dir(directory).map_err(|_| failure())? {
            let path = entry.map_err(|_| failure())?.path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(failure)?;
            if PINNED_FILES.iter().any(|(file, _)| {
                let stem = file.strip_suffix(".py").unwrap();
                name == stem || name.starts_with(&format!("{stem}.")) && name != *file
            }) {
                return Err(failure());
            }
            if PINNED_TREES
                .iter()
                .any(|(package, _)| name.starts_with(&format!("{package}.")))
            {
                return Err(failure());
            }
        }
    }
    let sdk = site.join("anthropic-0.87.0.dist-info/METADATA");
    trusted(&sdk,false).map_err(|_|"Install the pinned Anthropic SDK 0.87.0 before using Hermes. Lomi will not install dependencies.".to_string())?;
    if fs::metadata(&sdk).map_err(|_| failure())?.len() > MAX_OUTPUT as u64 {
        return Err(failure());
    }
    let sdk = fs::read_to_string(sdk).map_err(|_| failure())?;
    if !sdk.lines().any(|line| line == "Name: anthropic")
        || !sdk.lines().any(|line| line == "Version: 0.87.0")
    {
        return Err(failure());
    }
    for (directory, expected) in PINNED_TREES {
        let mut entries = BTreeMap::new();
        python_tree(&root.join(directory), &root, &mut entries)?;
        if tree_hash(entries) != *expected {
            return Err("Hermes installed source differs from the admitted stable release. No request was sent.".into());
        }
    }
    for (name, expected) in PINNED_FILES {
        let path = root.join(name);
        trusted(&path, false)?;
        if hash(&fs::read(path).map_err(|_| failure())?) != *expected {
            return Err(failure());
        }
    }
    Ok(Layout {
        launcher: program.to_owned(),
        root,
        python,
        site,
        stdlib,
    })
}
fn bootstrap(layout: &Layout) -> Result<String, String> {
    let paths = [
        layout.stdlib.clone(),
        layout.stdlib.join("lib-dynload"),
        layout.site.clone(),
    ];
    let paths = paths
        .iter()
        .map(|path| path.to_str().ok_or_else(failure))
        .collect::<Result<Vec<_>, _>>()?;
    let paths = serde_json::to_string(&paths).map_err(|_| failure())?;
    let entry = serde_json::to_string(&layout.root.join("hermes").to_str().ok_or_else(failure)?)
        .map_err(|_| failure())?;
    let root =
        serde_json::to_string(&layout.root.to_str().ok_or_else(failure)?).map_err(|_| failure())?;
    let mut names = PINNED_TREES
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>();
    names.extend(
        PINNED_FILES
            .iter()
            .map(|(name, _)| name.strip_suffix(".py").unwrap()),
    );
    let names = serde_json::to_string(&names).map_err(|_| failure())?;
    // Pinned main may add its checkout to sys.path. This finder still restricts
    // every top-level resolution to pinned modules or the approved runtime
    // paths, so an added checkout json.py/requests.py/package cannot shadow it.
    Ok(format!("import sys\nfrom importlib.machinery import PathFinder\n_lomi_paths = {paths}\n_lomi_names = frozenset({names})\nclass _LomiPinnedImports:\n    def find_spec(self, fullname, path=None, target=None):\n        if path is None:\n            roots = [{root}] if fullname in _lomi_names else _lomi_paths\n            spec = PathFinder.find_spec(fullname, roots, target)\n            if spec is not None:\n                return spec\n            raise ModuleNotFoundError(fullname)\n        return PathFinder.find_spec(fullname, path, target)\nsys.meta_path.append(_LomiPinnedImports())\nsys.meta_path.remove(PathFinder)\nsys.path[:] = _lomi_paths\nsys.argv[:] = [{entry}, *sys.argv[1:]]\nfrom hermes_cli.main import main\nmain()\n"))
}
pub(crate) struct Controller {
    root: PathBuf,
    model: String,
    prompt: String,
    layout: Option<Layout>,
}
impl Controller {
    pub(crate) fn new(root: &Path, id: &str, prompt: &str) -> Result<Self, String> {
        model(Some(id))?;
        if prompt.trim().is_empty()
            || prompt.len().saturating_add(PROMPT_PREFIX.len()) > MAX_CONTEXT
            || prompt.contains('\0')
        {
            return Err("Hermes text context must fit within 60 KiB. No request was sent.".into());
        }
        Ok(Self {
            root: root.to_owned(),
            model: id.into(),
            prompt: format!("{PROMPT_PREFIX}{prompt}"),
            layout: None,
        })
    }
    pub(crate) fn bind_executable(&mut self, program: &Path) -> Result<(), String> {
        let layout = source_layout(program)?;
        crate::chat::storage::atomic(
            &self.root.join("hermes-bootstrap.py"),
            bootstrap(&layout)?.as_bytes(),
        )?;
        self.layout = Some(layout);
        self.ready()
    }
    pub(crate) fn program(&self) -> Result<&Path, String> {
        Ok(&self.layout.as_ref().ok_or_else(failure)?.python)
    }
    pub(crate) fn configure(&self, command: &mut Command) -> Result<(), String> {
        if command.get_program() != self.program()?.as_os_str() {
            return Err(failure());
        }
        command
            .args(["-I", "-S", "-B", "-X", "utf8", "-X"])
            .arg(format!(
                "pycache_prefix={}",
                self.root.join("pycache").display()
            ))
            .arg(self.root.join("hermes-bootstrap.py"));
        Ok(())
    }
    pub(crate) fn ready(&self) -> Result<(), String> {
        let prior = self.layout.as_ref().ok_or_else(failure)?;
        let layout = source_layout(&prior.launcher)?;
        if layout.root != prior.root
            || layout.site != prior.site
            || layout.python != prior.python
            || layout.stdlib != prior.stdlib
        {
            return Err(failure());
        }
        let path = self.root.join("hermes-bootstrap.py");
        trusted(&path, false)?;
        if fs::read(&path).map_err(|_| failure())? != bootstrap(&layout)?.as_bytes() {
            return Err(failure());
        }
        let cache = self.root.join("pycache");
        trusted(&cache, true)?;
        if fs::read_dir(&cache)
            .map_err(|_| failure())?
            .next()
            .is_some()
        {
            return Err(failure());
        }
        let path = self.root.join(".hermes/models_dev_cache.json");
        trusted(&path, false)?;
        let modified = fs::metadata(&path)
            .and_then(|m| m.modified())
            .map_err(|_| failure())?;
        if SystemTime::now()
            .duration_since(modified)
            .map_err(|_| failure())?
            > Duration::from_secs(60 * 60)
        {
            return Err(failure());
        }
        if serde_json::from_slice::<Value>(&fs::read(path).map_err(|_| failure())?)
            .map_err(|_| failure())?
            != metadata()
        {
            return Err(failure());
        }
        let config = self.root.join(".hermes/config.yaml");
        trusted(&config, false)?;
        if serde_json::from_slice::<Value>(&fs::read(config).map_err(|_| failure())?)
            .map_err(|_| failure())?
            != settings()
        {
            return Err(failure());
        }
        for name in [
            ".env",
            ".op.env",
            "profile.yaml",
            "auth.json",
            ".anthropic_oauth.json",
        ] {
            absent(&self.root.join(".hermes").join(name))?;
        }
        Ok(())
    }
}
pub(crate) fn arguments(command: &mut Command, root: &Path, id: &str, _controller: &Controller) {
    command
        .args([
            "chat",
            "--query-file",
            "-",
            "--format",
            "stream-json",
            "--in",
        ])
        .arg(root)
        .args([
            "--provider",
            "anthropic",
            "-m",
            id,
            "--reasoning",
            "none",
            "--toolsets",
            "setup",
            "--ignore-rules",
            "--max-turns",
            "1",
            "--run-budget",
            "300",
        ]);
}

#[derive(Default)]
struct Progress {
    session: Option<String>,
    output: String,
    terminal: bool,
    complete: bool,
    valid: bool,
    effects: bool,
}
fn closed(event: &Value, fields: &[&str]) -> bool {
    event
        .as_object()
        .is_some_and(|map| map.keys().all(|key| fields.contains(&key.as_str())))
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
impl Progress {
    fn event(&mut self, event: &Value, id: &str) -> Result<(), String> {
        if !self.valid || self.terminal || event["timestamp"].as_u64().is_none() {
            self.valid = false;
            return Err(failure());
        }
        let result = (|| {
            match event["type"].as_str() {
                Some("system")
                    if self.session.is_none()
                        && closed(
                            event,
                            &["type", "subtype", "model", "session_id", "timestamp"],
                        )
                        && event["subtype"] == "init"
                        && event["model"] == id =>
                {
                    let session = event["session_id"]
                        .as_str()
                        .filter(|id| identifier(id))
                        .ok_or_else(failure)?;
                    self.session = Some(session.into());
                }
                Some("text")
                    if self.session.is_some() && closed(event, &["type", "text", "timestamp"]) =>
                {
                    let text = event["text"].as_str().ok_or_else(failure)?;
                    if self.output.len().saturating_add(text.len()) > MAX_OUTPUT {
                        return Err(failure());
                    }
                    self.output.push_str(text);
                }
                Some("result")
                    if self.session.is_some()
                        && closed(
                            event,
                            &[
                                "type",
                                "session_id",
                                "exit_code",
                                "text",
                                "tokens",
                                "duration_ms",
                                "timestamp",
                                "error",
                            ],
                        ) =>
                {
                    if event["session_id"].as_str() != self.session.as_deref()
                        || event["exit_code"].as_i64().is_none()
                        || event["duration_ms"].as_u64().is_none()
                    {
                        return Err(failure());
                    }
                    let tokens = &event["tokens"];
                    if !closed(
                        tokens,
                        &["input", "output", "total", "cache_read", "cache_write"],
                    ) || ["input", "output", "total", "cache_read", "cache_write"]
                        .iter()
                        .any(|key| tokens[key].as_u64().is_none())
                    {
                        return Err(failure());
                    }
                    let text = event["text"].as_str().ok_or_else(failure)?;
                    if text.len() > MAX_OUTPUT || !text.starts_with(&self.output) {
                        return Err(failure());
                    }
                    // Preserve streamed bytes; only an authoritative suffix may
                    // be appended. A correction leaves the retained prefix uncertain.
                    self.output.push_str(&text[self.output.len()..]);
                    self.terminal = true;
                    self.complete = event["exit_code"] == 0
                        && event.get("error").is_none()
                        && !text.trim().is_empty();
                }
                Some("tool_use" | "tool_result") => {
                    self.effects = true;
                    return Err(failure());
                }
                _ => return Err(failure()),
            }
            Ok(())
        })();
        if result.is_err() {
            self.valid = false;
        }
        result
    }
}

pub(crate) fn drive(
    mut child: OwnedChild,
    controller: Controller,
    cancelled: &Arc<AtomicBool>,
    mut checkpoint: impl FnMut(&str) -> Result<(), String>,
) -> Result<Outcome, String> {
    let readiness = controller.ready();
    if let Err(error) = readiness {
        let _ = child.stop_and_wait();
        drop(child);
        return Err(error);
    }
    let stdout = child.stdout.take().ok_or_else(failure)?;
    let stderr = child.stderr.take().ok_or_else(failure)?;
    let mut stdin = child.stdin.take().ok_or_else(failure)?;
    let prompt = controller.prompt;
    let writer = thread::Builder::new()
        .name("hermes-input".into())
        .spawn(move || {
            let result = stdin.write_all(prompt.as_bytes());
            drop(stdin);
            result
        });
    let writer = match writer {
        Ok(writer) => writer,
        Err(_) => {
            let _ = child.stop_and_wait();
            drop(child);
            return Err(failure());
        }
    };
    let (sender, receiver) = mpsc::sync_channel(32);
    let reader =
        thread::Builder::new()
            .name("hermes-output".into())
            .spawn(move || -> Result<(), ()> {
                let mut source = BufReader::new(stdout);
                let mut total = 0usize;
                loop {
                    let mut line = Vec::new();
                    let size = (&mut source)
                        .take((2 * MAX_OUTPUT + 8192) as u64)
                        .read_until(b'\n', &mut line)
                        .map_err(|_| ())?;
                    if size == 0 {
                        return Ok(());
                    }
                    total = total.saturating_add(size);
                    if total > 4 * MAX_OUTPUT || line.last() != Some(&b'\n') {
                        let _ = sender.send(Err(()));
                        return Err(());
                    }
                    let event = serde_json::from_slice::<Value>(&line).map_err(|_| ());
                    if sender.send(event).is_err() {
                        return Err(());
                    }
                }
            });
    let reader = match reader {
        Ok(reader) => reader,
        Err(_) => {
            let _ = child.stop_and_wait();
            drop(child);
            drop(receiver);
            let _ = writer.join();
            return Err(failure());
        }
    };
    let error_reader =
        thread::Builder::new()
            .name("hermes-errors".into())
            .spawn(move || -> Result<(), ()> {
                let mut stream = stderr;
                let mut total = 0usize;
                let mut bytes = [0u8; 4096];
                loop {
                    let size = stream.read(&mut bytes).map_err(|_| ())?;
                    if size == 0 {
                        return Ok(());
                    }
                    total = total.saturating_add(size);
                    if total > MAX_OUTPUT {
                        return Err(());
                    }
                }
            });
    let error_reader = match error_reader {
        Ok(error_reader) => error_reader,
        Err(_) => {
            let _ = child.stop_and_wait();
            drop(child);
            drop(receiver);
            let _ = reader.join();
            let _ = writer.join();
            return Err(failure());
        }
    };
    let mut progress = Progress {
        valid: true,
        ..Progress::default()
    };
    let mut disconnected = false;
    let mut exited = false;
    let start = Instant::now();
    let execution = (|| -> Result<(), String> {
        loop {
            if cancelled.load(Ordering::SeqCst) || start.elapsed() > TIMEOUT || !progress.valid {
                break;
            }
            if disconnected {
                thread::sleep(Duration::from_millis(25));
            } else {
                match receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(Ok(event)) => {
                        let _ = progress.event(&event, &controller.model);
                        checkpoint(&progress.output)?;
                    }
                    Ok(Err(())) => progress.valid = false,
                    Err(mpsc::RecvTimeoutError::Disconnected) => disconnected = true,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
            if !exited {
                exited = exit_pending(&child)?;
                if exited {
                    child.stop_group();
                }
            }
            if exited && disconnected {
                break;
            }
        }
        Ok(())
    })();
    let status = child.stop_and_wait();
    drop(child);
    drop(receiver);
    let read_ok = reader.join().is_ok_and(|result| result.is_ok());
    let error_ok = error_reader.join().is_ok_and(|result| result.is_ok());
    let wrote = writer.join().is_ok_and(|result| result.is_ok());
    let tail = checkpoint(&progress.output);
    execution?;
    tail?;
    let drained = status.as_ref().is_ok_and(|status| status.success())
        && exited
        && disconnected
        && read_ok
        && error_ok
        && wrote;
    let stopped = cancelled.load(Ordering::SeqCst);
    let valid = progress.valid && read_ok && error_ok && wrote;
    Ok(Outcome {
        completed: valid && drained && !stopped && progress.complete,
        exhausted: false,
        auth_failed: false,
        effects: progress.effects,
        stopped,
        drained,
        valid,
        output: progress.output,
    })
}

// SHA256 inventories of the pinned distribution Python packages. Hash input is
// sorted UTF-8 relative path length/path, then byte length/content, big-endian u64.
const PINNED_TREES: &[(&str, &str)] = &[
    (
        "agent",
        "7badcbd6c315b49a93305adc94e31028d4c75a167244b2a7a13b8f3566888b30",
    ),
    (
        "tools",
        "41ef29c36a14ddb12a6819e48d215989e4b5073253129a2a36d01dced014e510",
    ),
    (
        "hermes_cli",
        "3d8dfb256a42d874179b9658f67233ccb0d30668df87f8abae252b7e3a1eed2a",
    ),
    (
        "gateway",
        "a98816f57e34c426def7a20060796a835ffddf1d8628df33c90400d32aaba07c",
    ),
    (
        "tui_gateway",
        "adedac29321cde6476cbfbbe421a8638c81414f6a70c59d880824e01dee68ba7",
    ),
    (
        "cron",
        "95ebf957a860dca903ce327dee23ed50354dd69ef6af1565dfa4187c93cdeee2",
    ),
    (
        "acp_adapter",
        "30f3f85800e3cc1f29233613ca524b73009b23c99475c9ebd8eaa9baf0d5591f",
    ),
    (
        "plugins",
        "94bd5d2efccbe13ba7d6b827902c84a671f132a7da3d848b9d57177a4720da91",
    ),
    (
        "providers",
        "ba676cb5b8af2dd3ec57911596415ebceac063ceef342d34d7091447fad7f97a",
    ),
    (
        "hermes_platform",
        "6a13cd63f75a397c1f27a4ccd969e8f23c6926a4e838850a0859edeb490db383",
    ),
];
const PINNED_FILES: &[(&str, &str)] = &[
    (
        "batch_runner.py",
        "37ed0192c695b572d9de081c024abd9911be75b4749dd460f5c7d735b64e048a",
    ),
    (
        "cli.py",
        "267c7566b53c079b9f9c69794231d4f8e5907939f6167a2f96e4cff671c18b25",
    ),
    (
        "hermes_bootstrap.py",
        "7e46edcde82897d5dbd95e7ae8a601b9b1f13009f98d8a07d19116c85d7f4d2a",
    ),
    (
        "hermes_constants.py",
        "3dd95a77b1a4956280f146e3b065789ca5d82921f69e6e2cae06d49a13f82405",
    ),
    (
        "hermes_constants_scratch.py",
        "9dca8ebcc435e8d90ce65de9396c9dc8b9025e48a1d1881b767797c52efc6dd1",
    ),
    (
        "hermes_logging.py",
        "19deccd2c8a71cf0c9cf94f9cc44021a5fb9e8f4dc29df4da30ea7605b9ab84a",
    ),
    (
        "hermes_startup_watchdog.py",
        "6f8aa7e3425f35589caaeac32ec71393b01c04a0c7b840f1b485b9fb8fc9a983",
    ),
    (
        "hermes_state.py",
        "134fb1af988fd49407433cf54afa74f2188c93021fc4576bcf2c7174e787c2eb",
    ),
    (
        "hermes_state_common.py",
        "e66bdefc2bb58ab1e750a5040245fb95b3176a277773f35c2d2bbb3ef789070c",
    ),
    (
        "hermes_state_compression.py",
        "c920bb587aa3e96778e71bad2082831b37f76f73ddc76272e4cc79c1fd7571e8",
    ),
    (
        "hermes_state_dbfile.py",
        "317c4e5cbc7469259331f19e0a90f9081b729bbf70e97e81e7670ea6e630f7df",
    ),
    (
        "hermes_state_errors.py",
        "340226604dbf8c73416fe5075bac105c9e4175fa2a30badcd5b3207f7169cd67",
    ),
    (
        "hermes_state_fts.py",
        "e64c445d8662a58ffae83c326f1d3205f25c945594ab40d67017e39cea145a71",
    ),
    (
        "hermes_state_gateway.py",
        "8719a569f381a82c6cc4dbc1d8b1164b9eb164e209b6eccb19c7eb6852510c7c",
    ),
    (
        "hermes_state_guard.py",
        "4119cd9839a5028d9527c741f263d4011b3851994db5f8cf46d869885c32c626",
    ),
    (
        "hermes_state_health.py",
        "71dff34c7e2e29d1800a49b1a5a5e72d95dc0c14188c3ffd23036c67693617a8",
    ),
    (
        "hermes_state_holders.py",
        "4fef9cc809f7d90a6b641a6f22146d64c3d4ea5e17fe49ae8067355c1eb3503b",
    ),
    (
        "hermes_state_ids.py",
        "c2b4531690fedd0f4d37c3f6c44f5478b4e87c1883b102c22a86340f3a4bdeb1",
    ),
    (
        "hermes_state_lockguard.py",
        "dbcffa9bdb9c3d235d2165ca8485c8f4a7b09869b219c2b38729942e022bebd6",
    ),
    (
        "hermes_state_lockowners.py",
        "2ec2c3476a292b874bf45828d63aa5d631709e644fc948d316c96666e2f501b9",
    ),
    (
        "hermes_state_maintenance.py",
        "fa38baad474f3d535e05bb9317d328fd116a33d8e4d0feb6b137ebb468d9911d",
    ),
    (
        "hermes_state_messages.py",
        "f17ecce68d2d24384297c05b568fb9e1634982f1616a7f2791ba0c9eb1a1a8cc",
    ),
    (
        "hermes_state_portability.py",
        "40f4e911bde9a672839a494d79a33c6e5e48dc4b356da31ace2a7e60e324f551",
    ),
    (
        "hermes_state_profile_repair.py",
        "02c5580949ab95fcdd42b69290be76bdaf83c16895fc52f42eb3156506695e75",
    ),
    (
        "hermes_state_readpool.py",
        "191aed84a6bad0102b8b5f829f74a21d32027c7a26f7a9bd91ca86c359e623b6",
    ),
    (
        "hermes_state_registry.py",
        "fe2fb3a6a2051964b4c21cdf61cdaec8389ce72fe66f81e615ab5b091df7f3bb",
    ),
    (
        "hermes_state_repair.py",
        "7af22ad9aa7cf3823e768177c4d56d1827b53f7253d0549e0bd7e3195ff0b14e",
    ),
    (
        "hermes_state_rewind.py",
        "c147473e80de3d615a3a70c31798539338af3cd4d9405a0b9049bfd24830540b",
    ),
    (
        "hermes_state_schema.py",
        "8712aa97c222cdc2f1ec5ab8d2dadd01756117c43e1597dcfbe43c0645e30ef0",
    ),
    (
        "hermes_state_search.py",
        "9228e1e297037dae4f7d41d30e09bdd27f0d651c6d2343fcc55a9f16a4ed4d9b",
    ),
    (
        "hermes_state_sessions.py",
        "873da411c4bee265fbf40270ecdc314b94cb219204fa85e0a944d413616eb819",
    ),
    (
        "hermes_state_telegram.py",
        "31e916e99a2c40fe3d4d7cc8838f89567f3d9d0c035a07877279df8f56b49a8b",
    ),
    (
        "hermes_state_timeline.py",
        "ad605786c9ef97da056dbe88e2302d033207ac099cb6cc99b8efcaded38c7329",
    ),
    (
        "hermes_state_titles.py",
        "df8b44c7e8576cf5abb4da3a6dbff102b59fcb5e943ebb79acb9c0bcdfb11804",
    ),
    (
        "hermes_state_usage.py",
        "dfff8f361338bfa3d84e56b410f86ac75735294b20ede742e01cd6a711ad89b0",
    ),
    (
        "hermes_state_user_copy.py",
        "d15365c9dc94d516d90c49f8205e108b403c6d2aa4eec63cfdf100bf0f39ae55",
    ),
    (
        "hermes_state_wal.py",
        "da67d22fe73076a27489664481bead92342f62c3fa1d38336229eb89a70cf819",
    ),
    (
        "hermes_time.py",
        "df02f281fd9ec46f6e763a4692f3e176dd71076de3dabb87886e895a0fe0d175",
    ),
    (
        "mcp_serve.py",
        "a7330ff77d756a378c9860a3e32e0f80c48c2d5ea865216846d661596b2102c0",
    ),
    (
        "mini_swe_runner.py",
        "e4aa2604ff0c380beb6ee983be72955bbed5468ac1ccd148351076ed879c42a0",
    ),
    (
        "model_tools.py",
        "5d5a947d84f31f1ba4ef5267e28154b819e8f957a0b378739696f1ac305e1509",
    ),
    (
        "registration_lifecycle.py",
        "98efbd89a8b76534f56c123ddb597c5b8e8f9ca0e840d7493582e2eeb74acb47",
    ),
    (
        "run_agent.py",
        "244da863d3c21591a3b5326dc14c2962d4e31131dda52df628502cd9fcbfea33",
    ),
    (
        "toolset_distributions.py",
        "91e073501606c7845107d746b31b894d09d062f0351c9d3d9ed3c45c46c23f94",
    ),
    (
        "toolsets.py",
        "48ba8bea0b9bcd5821747f480055a224639fd83565d9b14d6134bbe7d433aa43",
    ),
    (
        "trajectory_compressor.py",
        "149b09c5aa4f55b627ad1a539e51c5c3ff16f00b2e65c479fd3826186e203fe0",
    ),
    (
        "utils.py",
        "15c10bc1a0499368f76f3092c15c2e4960bc2faa7f23b54bd1d8502190d8ed41",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    fn fresh_progress() -> Progress {
        Progress {
            valid: true,
            ..Progress::default()
        }
    }
    fn init() -> Value {
        json!({"type":"system","subtype":"init","model":MODEL,"session_id":"20260924_session","timestamp":1})
    }
    fn final_text(text: &str) -> Value {
        json!({"type":"result","session_id":"20260924_session","exit_code":0,"text":text,"tokens":{"input":1,"output":1,"total":2,"cache_read":0,"cache_write":0},"duration_ms":1,"timestamp":2})
    }
    #[test]
    fn definitive_text_preserves_deltas_and_allows_only_authoritative_tail() {
        let mut progress = fresh_progress();
        progress.event(&init(), MODEL).unwrap();
        progress
            .event(
                &json!({"type":"text","text":"Answer ","timestamp":1}),
                MODEL,
            )
            .unwrap();
        progress
            .event(&final_text("Answer complete"), MODEL)
            .unwrap();
        assert!(progress.complete && progress.valid && progress.terminal);
        assert_eq!(progress.output, "Answer complete");
        assert!(progress
            .event(&final_text("Answer complete"), MODEL)
            .is_err());
        assert!(!progress.valid);
    }
    #[test]
    fn corrections_and_wrong_session_never_replace_uncertain_text() {
        for event in [final_text("different"), {
            let mut event = final_text("prefix answer");
            event["session_id"] = json!("other");
            event
        }] {
            let mut progress = fresh_progress();
            progress.event(&init(), MODEL).unwrap();
            progress
                .event(&json!({"type":"text","text":"prefix","timestamp":1}), MODEL)
                .unwrap();
            assert!(progress.event(&event, MODEL).is_err());
            assert_eq!(progress.output, "prefix");
            assert!(!progress.complete && !progress.valid);
        }
    }
    #[test]
    fn tools_fail_closed_and_failed_result_does_not_prove_quota() {
        let mut progress = fresh_progress();
        progress.event(&init(), MODEL).unwrap();
        assert!(progress
            .event(
                &json!({"type":"tool_use","name":"terminal","timestamp":1}),
                MODEL
            )
            .is_err());
        assert!(progress.effects && !progress.valid);
        let mut progress = fresh_progress();
        progress.event(&init(), MODEL).unwrap();
        let mut event = final_text("partial");
        event["exit_code"] = json!(1);
        event["error"] = json!("provider rejected request");
        progress.event(&event, MODEL).unwrap();
        assert!(progress.terminal && !progress.complete);
        assert_eq!(progress.output, "partial");
        assert!(admit_api_key("sk-ant-oat01-oauth").is_err());
        assert!(admit_api_key("native-api-key-fixture").is_ok());
    }
    #[test]
    fn path_shaped_user_context_never_becomes_a_file_drop_query() {
        let exact = "a".repeat(MAX_CONTEXT - PROMPT_PREFIX.len());
        let controller = Controller::new(Path::new("/private/attempt"), MODEL, &exact).unwrap();
        assert_eq!(controller.prompt.len(), MAX_CONTEXT);
        assert!(
            Controller::new(Path::new("/private/attempt"), MODEL, &format!("{exact}a")).is_err()
        );
        for prompt in [
            "/etc/passwd",
            "~/secrets",
            "./image.png",
            "../private",
            "file:///tmp/image",
            "\"/tmp/image.png\"",
        ] {
            let controller = Controller::new(Path::new("/private/attempt"), MODEL, prompt).unwrap();
            assert!(controller.prompt.starts_with(PROMPT_PREFIX));
            assert!(controller.prompt.ends_with(prompt));
        }
        assert!(Controller::new(
            Path::new("/private/attempt"),
            MODEL,
            &"a".repeat(MAX_CONTEXT)
        )
        .is_err());
    }
    #[test]
    fn owned_bootstrap_preserves_public_argv_without_editable_site_startup() {
        let layout = Layout {
            launcher: "/launcher/hermes".into(),
            root: "/checkout".into(),
            python: "/checkout/venv/bin/python".into(),
            site: "/checkout/venv/lib/python3.13/site-packages".into(),
            stdlib: "/python/lib/python3.13".into(),
        };
        let mut controller = Controller::new(Path::new("/private/attempt"), MODEL, "text").unwrap();
        controller.layout = Some(layout.clone());
        let source = bootstrap(&layout).unwrap();
        assert!(source.contains("sys.argv[:] = [\"/checkout/hermes\", *sys.argv[1:]]"));
        assert!(source.contains("from hermes_cli.main import main\nmain()"));
        assert!(!source.contains("import site"));
        assert!(source.contains("sys.meta_path.remove(PathFinder)"));
        assert!(
            source.contains("roots = [\"/checkout\"] if fullname in _lomi_names else _lomi_paths")
        );
        assert!(source.contains("return PathFinder.find_spec(fullname, path, target)"));
        assert!(source.contains("_lomi_paths = [\"/python/lib/python3.13\",\"/python/lib/python3.13/lib-dynload\",\"/checkout/venv/lib/python3.13/site-packages\"]"));
        let mut command = Command::new(&layout.python);
        controller.configure(&mut command).unwrap();
        command.arg("--version");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "-I",
                "-S",
                "-B",
                "-X",
                "utf8",
                "-X",
                "pycache_prefix=/private/attempt/pycache",
                "/private/attempt/hermes-bootstrap.py",
                "--version"
            ]
            .map(std::ffi::OsStr::new)
        );
    }
    #[test]
    fn startup_modules_reject_packages_bytecode_and_native_variants() {
        for name in [
            "sitecustomize",
            "sitecustomize.pyc",
            "sitecustomize.cpython-314-darwin.so",
            "usercustomize.pyd",
        ] {
            let root = tempfile::tempdir().unwrap();
            if name == "sitecustomize" {
                fs::create_dir(root.path().join(name)).unwrap();
            } else {
                fs::write(root.path().join(name), b"untrusted startup code").unwrap();
            }
            assert!(reject_startup_modules(root.path()).is_err());
        }
    }
    #[test]
    fn isolation_keeps_owned_config_and_empty_setup_role() {
        let config = settings();
        assert_eq!(config["fallback_providers"], json!([]));
        assert_eq!(config["agent"]["disabled_toolsets"], json!(["setup"]));
        assert_eq!(config["updates"]["check"], false);
        assert_eq!(config["security"]["allow_lazy_installs"], false);
        let mut command = Command::new("hermes");
        environment(&mut command, Path::new("/private/attempt"), MODEL);
        let env = command.get_envs().collect::<Vec<_>>();
        assert!(env
            .iter()
            .any(|(key, value)| *key == "HERMES_SAFE_MODE"
                && value.is_some_and(|value| value == "1")));
        assert!(!env
            .iter()
            .any(|(key, _)| *key == "HERMES_IGNORE_USER_CONFIG"));
        assert!(!env.iter().any(|(key, _)| *key == "ANTHROPIC_TOKEN"));
        assert!(env
            .iter()
            .any(|(key, value)| *key == "HERMES_DISABLE_LAZY_INSTALLS"
                && value.is_some_and(|value| value == "1")));
        assert!(env.iter().any(|(key, value)| *key == "PATH"
            && value.is_some_and(|value| value == "/private/attempt/empty-bin")));
    }
}

const ENTRYPOINT_HASH: &str = "6e1adae1e73ce67121d4ec380a5b66b8fb84f02dd8004b3a9988c39660139417";
