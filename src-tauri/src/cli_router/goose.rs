//! Source-qualified Goose 1.53.0 (aaif-goose/goose commit 76da81c).
//! Fresh OpenAI API text turns only; this is not a sandbox for arbitrary Goose
//! configurations. Shared system configuration is refused before CLI startup.

use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) const VERSION: &str = "1.53.0";
const CONFIG: &[u8] = b"CONTEXT_FILE_NAMES: []\n";
const SYSTEM_PROMPT: &str = "You are a text-only assistant in Lomi. Answer using only the supplied conversation. Tools, filesystem access, shell commands, extensions and project context are unavailable. Do not claim to inspect or modify project files. Ask the user for missing information.";
const ENVIRONMENT: &[(&str, &str)] = &[
    ("GOOSE_DISABLE_KEYRING", "1"),
    ("GOOSE_MODE", "chat"),
    ("GOOSE_DISABLE_SESSION_NAMING", "true"),
    ("GOOSE_TOOLSHIM", "false"),
    ("GOOSE_AUTO_COMPACT_THRESHOLD", "0"),
    ("GOOSE_STATE_MACHINE", "0"),
    ("OPENAI_HOST", "https://api.openai.com"),
    ("OPENAI_BASE_PATH", "v1/chat/completions"),
];

pub(crate) fn home(parent: &Path) -> Result<PathBuf, String> {
    let root = parent.join(format!("goose-{}", super::new_id()?));
    fs::create_dir(&root).map_err(|_| "Cannot create fresh Goose attempt storage.")?;
    crate::chat::storage::private(&root, true)?;
    let config = root.join("config");
    fs::create_dir(&config).map_err(|_| "Cannot isolate Goose configuration.")?;
    crate::chat::storage::private(&config, true)?;
    crate::chat::storage::atomic(&config.join("config.yaml"), CONFIG)?;
    crate::chat::storage::atomic(&config.join("secrets.yaml"), b"{}\n")?;
    crate::chat::storage::atomic(&root.join("system-prompt.txt"), SYSTEM_PROMPT.as_bytes())?;
    // Paths::path_root also confines plugin/agent roots. Cwd and HOME are this
    // same fresh directory, with no project/global plugins, recipes or hints.
    root.canonicalize()
        .map_err(|_| "Cannot resolve private Goose attempt storage.".into())
}

pub(crate) fn environment(command: &mut Command, root: &Path) {
    command.env("HOME", root).env("GOOSE_PATH_ROOT", root);
    for (name, value) in ENVIRONMENT {
        command.env(name, value);
    }
    command.env(
        "GOOSE_SYSTEM_PROMPT_FILE_PATH",
        root.join("system-prompt.txt"),
    );
}

pub(crate) fn model(model: Option<&str>) -> Result<&str, String> {
    // Explicit CLI model selection wins in session/builder.rs. Reject effort
    // suffix syntax and surrounding whitespace rather than normalizing an ID.
    model.filter(|id| {
        !id.is_empty() && id.len() <= 200
            && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            && !id.rsplit('-').next().is_some_and(|suffix| {
                matches!(suffix, "none" | "low" | "medium" | "high" | "xhigh")
            })
    }).ok_or_else(|| "Goose requires an exact OpenAI model ID without aliases, whitespace or effort suffixes. No request was sent.".into())
}

pub(crate) fn arguments(command: &mut Command, root: &Path, model: &str) {
    environment(command, root);
    command.args([
        "run",
        "--no-profile",
        "--no-session",
        "--quiet",
        "--output-format",
        "stream-json",
        "--provider",
        "openai",
        "--model",
        model,
        "--instructions",
        "-",
        "--max-turns",
        "1",
    ]);
}

pub(crate) fn version_matches(output: &str) -> bool {
    output.trim() == format!("goose {VERSION}")
}

pub(crate) fn clean_exit(exited: bool, eof: bool, input_written: bool, success: bool) -> bool {
    exited && eof && input_written && success
}

#[cfg(unix)]
fn trusted_directory(uid: u32, mode: u32, directory: bool) -> bool {
    uid == 0 && mode & 0o022 == 0 && directory
}

#[cfg(unix)]
fn trusted_resolved_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    // Canonicalization admits Darwin's root-owned /etc -> private/etc link.
    // Every resolved ancestor must still be a root-owned non-writable directory.
    let resolved = path
        .canonicalize()
        .map_err(|_| "Cannot resolve Goose system configuration ancestors.")?;
    for ancestor in resolved.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)
            .map_err(|_| "Cannot inspect Goose system configuration ancestors.")?;
        if !trusted_directory(metadata.uid(), metadata.mode(), metadata.is_dir()) {
            return Err("Goose system configuration ancestors must be trusted root-owned directories without group or world write access.".into());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn admit_system_config_at(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    // symlink_metadata sees dangling links too. Never read the config contents.
    match fs::symlink_metadata(path) {
        Ok(_) => return Err(
            "Managed Goose refuses any shared /etc/goose/config.yaml entry. No request was sent."
                .into(),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err("Cannot establish that Goose shared system configuration is absent.".into())
        }
    }
    for ancestor in path
        .parent()
        .ok_or("Invalid Goose system configuration path.")?
        .ancestors()
    {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    if metadata.uid() != 0 {
                        return Err(
                            "Goose system configuration ancestor links must be root-owned.".into(),
                        );
                    }
                } else if !trusted_directory(metadata.uid(), metadata.mode(), metadata.is_dir()) {
                    return Err("Goose system configuration ancestors must be trusted root-owned directories without group or world write access.".into());
                }
                trusted_resolved_directory(ancestor)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot inspect Goose system configuration ancestors.".into()),
        }
    }
    Ok(())
}

pub(crate) fn admit_system_config() -> Result<(), String> {
    #[cfg(unix)]
    {
        admit_system_config_at(Path::new("/etc/goose/config.yaml"))
    }
    #[cfg(not(unix))]
    {
        Err("Goose system configuration admission is unqualified on this platform.".into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rejection {
    Malformed,
    Failed,
    Effect,
}

pub(crate) struct Progress {
    model: String,
    text_seen: bool,
    terminal: bool,
}

impl Progress {
    pub(crate) fn new(model: &str) -> Self {
        Self {
            model: model.into(),
            text_seen: false,
            terminal: false,
        }
    }

    pub(crate) fn event(&mut self, value: &Value) -> Result<(Option<String>, bool), Rejection> {
        if self.terminal {
            return Err(Rejection::Malformed);
        }
        match value["type"].as_str() {
            Some("message") => {
                if !value
                    .as_object()
                    .is_some_and(|event| event.len() == 2 && event.contains_key("message"))
                {
                    return Err(Rejection::Malformed);
                }
                let message = &value["message"];
                let metadata = &message["metadata"];
                if message["role"] != "assistant"
                    || !message["created"].is_i64()
                    || !message["id"].as_str().is_some_and(|id| !id.is_empty())
                    || metadata["inference"]["provider"] != "openai"
                    || metadata["inference"]["requestedModel"].as_str() != Some(self.model.as_str())
                    || metadata["userVisible"] != true
                    || metadata["agentVisible"] != true
                {
                    return Err(Rejection::Malformed);
                }
                for flag in ["outputTokenLimitReached", "steer", "turnContext"] {
                    if metadata.get(flag).is_some_and(|value| value != false) {
                        return Err(Rejection::Failed);
                    }
                }
                if metadata
                    .pointer("/usage/isCompaction")
                    .is_some_and(|value| value != false)
                {
                    return Err(Rejection::Failed);
                }
                let blocks = message["content"].as_array().ok_or(Rejection::Malformed)?;
                if blocks.is_empty() {
                    return Err(Rejection::Malformed);
                }
                let mut text = String::new();
                for block in blocks {
                    match block["type"].as_str() {
                        Some("text") => {
                            let chunk = block["text"].as_str().ok_or(Rejection::Malformed)?;
                            if text.len().saturating_add(chunk.len()) > super::runtime::MAX_OUTPUT {
                                return Err(Rejection::Malformed);
                            }
                            text.push_str(chunk);
                        }
                        Some("thinking")
                            if block["thinking"].is_string() && block["signature"].is_string() => {}
                        Some(
                            "toolRequest"
                            | "toolResponse"
                            | "toolConfirmationRequest"
                            | "actionRequired",
                        ) => return Err(Rejection::Effect),
                        Some("error" | "systemNotification") => return Err(Rejection::Failed),
                        _ => return Err(Rejection::Malformed),
                    }
                }
                self.text_seen |= !text.trim().is_empty();
                Ok(((!text.is_empty()).then_some(text), false))
            }
            Some("complete") => {
                if !self.text_seen {
                    return Err(Rejection::Failed);
                }
                let event = value.as_object().ok_or(Rejection::Malformed)?;
                if !event.contains_key("total_tokens")
                    || event.iter().any(|(key, value)| match key.as_str() {
                        "type" => false,
                        "total_tokens"
                        | "input_tokens"
                        | "output_tokens"
                        | "cache_read_input_tokens"
                        | "cache_write_input_tokens" => {
                            !value.is_null() && value.as_u64().is_none()
                        }
                        "cost_usd" => {
                            !value.is_null()
                                && !value
                                    .as_f64()
                                    .is_some_and(|cost| cost.is_finite() && cost >= 0.0)
                        }
                        _ => true,
                    })
                {
                    return Err(Rejection::Malformed);
                }
                self.terminal = true;
                Ok((None, true))
            }
            Some("notification") => Err(Rejection::Effect),
            Some("error") => Err(Rejection::Failed),
            _ => Err(Rejection::Malformed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chunk(text: &str) -> Value {
        json!({"type":"message","message":{"id":"chunk","role":"assistant","created":1,
            "metadata":{"userVisible":true,"agentVisible":true,
                "inference":{"provider":"openai","requestedModel":"gpt-4.1"}},
            "content":[{"type":"text","text":text}]}})
    }

    #[test]
    fn chunks_concatenate_and_exactly_one_complete_is_required() {
        let mut progress = Progress::new("gpt-4.1");
        assert_eq!(
            progress.event(&chunk("Hello ")).unwrap(),
            (Some("Hello ".into()), false)
        );
        assert_eq!(
            progress.event(&chunk("world")).unwrap(),
            (Some("world".into()), false)
        );
        assert!(!progress.terminal);
        assert_eq!(
            progress
                .event(&json!({"type":"complete","total_tokens":null}))
                .unwrap(),
            (None, true)
        );
        assert_eq!(
            progress.event(&json!({"type":"complete"})),
            Err(Rejection::Malformed)
        );
        assert_eq!(progress.event(&chunk("late")), Err(Rejection::Malformed));
    }

    #[test]
    fn every_chunk_requires_exact_inference_provenance() {
        for pointer in [
            "/message/metadata/inference/provider",
            "/message/metadata/inference/requestedModel",
        ] {
            let mut value = chunk("answer");
            *value.pointer_mut(pointer).unwrap() = json!("wrong");
            assert_eq!(
                Progress::new("gpt-4.1").event(&value),
                Err(Rejection::Malformed)
            );
        }
        let mut synthetic = chunk("Maximum number of turns reached.");
        synthetic["message"]["metadata"]
            .as_object_mut()
            .unwrap()
            .remove("inference");
        assert_eq!(
            Progress::new("gpt-4.1").event(&synthetic),
            Err(Rejection::Malformed)
        );
    }

    #[test]
    fn diagnostics_effects_and_nontext_never_become_an_answer() {
        for (kind, rejection) in [
            ("toolRequest", Rejection::Effect),
            ("toolResponse", Rejection::Effect),
            ("toolConfirmationRequest", Rejection::Effect),
            ("actionRequired", Rejection::Effect),
            ("systemNotification", Rejection::Failed),
            ("error", Rejection::Failed),
            ("image", Rejection::Malformed),
            ("document", Rejection::Malformed),
            ("unknown", Rejection::Malformed),
        ] {
            let mut value = chunk("answer");
            value["message"]["content"][0]["type"] = json!(kind);
            assert_eq!(Progress::new("gpt-4.1").event(&value), Err(rejection));
        }
        let mut limited = chunk("partial");
        limited["message"]["metadata"]["outputTokenLimitReached"] = json!(true);
        assert_eq!(
            Progress::new("gpt-4.1").event(&limited),
            Err(Rejection::Failed)
        );
        for kind in ["error", "notification", "unknown"] {
            assert!(Progress::new("gpt-4.1")
                .event(&json!({"type":kind}))
                .is_err());
        }
        let mut thinking = chunk("");
        thinking["message"]["content"] =
            json!([{"type":"thinking","thinking":"reason","signature":""}]);
        let mut progress = Progress::new("gpt-4.1");
        assert!(progress.event(&thinking).is_ok());
        assert_eq!(
            progress.event(&json!({"type":"complete"})),
            Err(Rejection::Failed)
        );
    }

    #[test]
    fn exact_arguments_and_nonsecret_environment_are_owned() {
        let root = Path::new("/private/goose-attempt");
        let mut command = Command::new("goose");
        command.env_clear();
        arguments(&mut command, root, model(Some("gpt-4.1")).unwrap());
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "run",
                "--no-profile",
                "--no-session",
                "--quiet",
                "--output-format",
                "stream-json",
                "--provider",
                "openai",
                "--model",
                "gpt-4.1",
                "--instructions",
                "-",
                "--max-turns",
                "1"
            ]
        );
        for (key, expected) in ENVIRONMENT {
            assert!(
                command
                    .get_envs()
                    .any(|(name, value)| name == *key
                        && value == Some(std::ffi::OsStr::new(expected)))
            );
        }
        assert!(command
            .get_envs()
            .any(|(name, value)| name == "HOME" && value == Some(root.as_os_str())));
        assert!(command.get_envs().all(|(name, _)| name != "OPENAI_API_KEY"));
        assert_eq!(CONFIG, b"CONTEXT_FILE_NAMES: []\n");
        for invalid in [
            "",
            " gpt-4.1",
            "gpt-4.1:high",
            "gpt-4.1@high",
            "gpt 4.1",
            "gpt-5-high",
            "gpt-5-none",
        ] {
            assert!(model(Some(invalid)).is_err());
        }
        assert!(version_matches("goose 1.53.0\n"));
        for wrong in [
            "1.53.0",
            "other 1.53.0",
            "goose 1.53.1",
            "goose 1.53.0 extra",
        ] {
            assert!(!version_matches(wrong));
        }
    }

    #[test]
    fn completion_requires_clean_eof_clean_exit_and_written_input() {
        assert!(clean_exit(true, true, true, true));
        assert!(!clean_exit(false, true, true, true));
        assert!(!clean_exit(true, false, true, true));
        assert!(!clean_exit(true, true, false, true));
        assert!(!clean_exit(true, true, true, false));
        for bad in [
            json!({"type":"complete"}),
            json!({"type":"complete","total_tokens":null,"status":"error"}),
            json!({"type":"complete","total_tokens":-1}),
            json!({"type":"complete","total_tokens":null,"cost_usd":-0.5}),
        ] {
            let mut progress = Progress::new("gpt-4.1");
            progress.event(&chunk("answer")).unwrap();
            assert_eq!(progress.event(&bad), Err(Rejection::Malformed));
        }
    }

    #[cfg(unix)]
    #[test]
    fn shared_entries_including_dangling_links_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("config.yaml");
        fs::write(&file, b"{}").unwrap();
        assert!(admit_system_config_at(&file).is_err());
        fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(temp.path().join("missing"), &file).unwrap();
        assert!(admit_system_config_at(&file).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn resolved_ancestor_trust_requires_root_and_no_shared_write() {
        assert!(trusted_directory(0, 0o40755, true));
        assert!(trusted_directory(0, 0o40700, true));
        for (uid, mode, directory) in [
            (501, 0o40755, true),
            (0, 0o40775, true),
            (0, 0o40757, true),
            (0, 0o100644, false),
        ] {
            assert!(!trusted_directory(uid, mode, directory));
        }
    }
}
