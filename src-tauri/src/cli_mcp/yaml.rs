use super::{owned, Registration};
use serde_json::{json, Value};

fn document(source: Option<&str>) -> Result<Value, String> {
    let options = serde_saphyr::options! {
        duplicate_keys: serde_saphyr::DuplicateKeyPolicy::Error,
        merge_keys: serde_saphyr::MergeKeyPolicy::Error,
        reject_unsupported_tags: true,
        strict_booleans: true,
        with_snippet: false,
        budget: serde_saphyr::budget! {
            max_depth: 64,
            max_nodes: 50_000,
            max_events: 100_000,
            max_total_scalar_bytes: 1024 * 1024,
            max_aliases: 0,
            max_anchors: 0,
        },
    };
    let value: Value = serde_saphyr::from_str_with_options(source.unwrap_or("{}"), options)
        .map_err(|_| "CLI YAML is invalid or uses unsupported aliases, tags, or merge keys. The file was left intact.")?;
    if !value.is_object() {
        return Err("CLI YAML configuration must be a mapping.".into());
    }
    Ok(value)
}

fn entry(doc: &Value) -> Result<Option<&Value>, String> {
    let Some(servers) = doc.get("mcp_servers") else {
        return Ok(None);
    };
    Ok(servers
        .as_object()
        .ok_or("CLI MCP settings must be a mapping.")?
        .get("lomi"))
}

pub(super) fn configured(
    source: Option<&str>,
    expected: Option<&Registration>,
) -> Result<bool, String> {
    let doc = document(source)?;
    let Some(entry) = entry(&doc)? else {
        return Ok(false);
    };
    if !entry.is_object() {
        return Err("Lomi MCP settings must be a mapping.".into());
    }
    let Some(expected) = expected else {
        return Ok(false);
    };
    Ok(entry["command"] == expected.command
        && entry["args"] == json!(expected.args)
        && entry.get("url").is_none()
        && entry.get("uri").is_none()
        && entry["disabled"] != true
        && entry["enabled"] != false)
}

pub(super) fn updated(source: Option<&str>, expected: &Registration) -> Result<String, String> {
    let mut doc = document(source)?;
    let existing = entry(&doc)?;
    if existing.is_some_and(|entry| {
        !owned(
            entry.get("command").and_then(Value::as_str),
            entry
                .get("args")
                .and_then(Value::as_array)
                .and_then(|args| args.iter().map(Value::as_str).collect()),
            expected,
        )
    }) {
        return Err("A different MCP server is already named lomi. Rename that entry before installing Lomi MCP.".into());
    }
    let mut value = existing.cloned().unwrap_or_else(|| json!({}));
    let server = value
        .as_object_mut()
        .ok_or("Lomi MCP settings must be a mapping.")?;
    server.insert("command".into(), json!(expected.command));
    server.insert("args".into(), json!(expected.args));
    server.remove("url");
    server.remove("uri");
    if server.contains_key("disabled") {
        server.insert("disabled".into(), json!(false));
    }
    if server.contains_key("enabled") {
        server.insert("enabled".into(), json!(true));
    }
    doc.as_object_mut()
        .unwrap()
        .entry("mcp_servers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("CLI MCP settings must be a mapping.")?
        .insert("lomi".into(), value);
    serde_saphyr::to_string(&doc).map_err(|_| "Cannot serialize CLI YAML configuration.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cli_catalog::TitleCli, cli_config};

    #[test]
    fn hermes_preserves_settings_and_servers_and_does_not_reinstall() {
        let registration = Registration {
            command: "/Applications/Lomi.app/Contents/MacOS/lomi".into(),
            args: vec![
                "--mcp".into(),
                "--discovery-file".into(),
                "/private/lomi/discovery.json".into(),
            ],
        };
        let source =
            "# retained in backup\nmodel: test\nmcp_servers:\n  other:\n    command: keep\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, source).unwrap();
        super::super::enable(
            TitleCli::Hermes,
            &path,
            cli_config::revision(Some(source)).as_deref(),
            &registration,
        )
        .unwrap();
        let output = std::fs::read_to_string(&path).unwrap();
        assert!(configured(Some(&output), Some(&registration)).unwrap());
        let parsed = document(Some(&output)).unwrap();
        let previous = document(Some(source)).unwrap();
        assert_eq!(parsed["model"], previous["model"]);
        assert_eq!(
            parsed["mcp_servers"]["other"],
            previous["mcp_servers"]["other"]
        );
        super::super::enable(
            TitleCli::Hermes,
            &path,
            cli_config::revision(Some(&output)).as_deref(),
            &registration,
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), output);
        let backup = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| entry.file_name().to_string_lossy().contains("lomi-backup"))
            .unwrap();
        assert_eq!(std::fs::read_to_string(backup.path()).unwrap(), source);
    }

    #[test]
    fn yaml_preserves_unsupported_or_conflicting_documents() {
        let registration = Registration {
            command: "/bin/lomi".into(),
            args: vec!["--mcp".into()],
        };
        for source in [
            "mcp_servers:\n  lomi: {}",
            "mcp_servers: []",
            "model: a\nmodel: b",
            "model: &a [one]\nother: *a",
            "model: !include private.yaml",
            "---\nmodel: a\n---\nmodel: b",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.yaml");
            std::fs::write(&path, source).unwrap();
            assert!(
                super::super::enable(
                    TitleCli::Hermes,
                    &path,
                    cli_config::revision(Some(source)).as_deref(),
                    &registration,
                )
                .is_err(),
                "{source}"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        }
    }
}
