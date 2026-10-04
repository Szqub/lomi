//! Options with documented equivalents in both public OpenAI wire formats.
//! Native reasoning continuation, explicit cache breakpoints and custom-tool
//! grammars have no equivalent in the shared text/function-tool subset.
//! OpenAI reference: https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create
use super::*;

#[derive(Default)]
pub(super) struct Options {
    effort: Option<Value>,
    verbosity: Option<Value>,
    format: Option<Value>,
    metadata: Option<Value>,
    cache_key: Option<Value>,
    cache_retention: Option<Value>,
    service_tier: Option<Value>,
}
fn error(feature: &'static str) -> String {
    format!(
        "Gateway protocol conversion cannot preserve {feature}; use a matching native protocol."
    )
}
fn value(v: &Value, key: &str) -> Option<Value> {
    v.get(key).filter(|value| !value.is_null()).cloned()
}
fn text(v: &Value, max: usize) -> Result<(), String> {
    let s = v.as_str().ok_or_else(invalid)?;
    if s.is_empty() || s.len() > max || s.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(())
}
fn text_format(v: &Value, chat: bool) -> Result<Value, String> {
    let kind = v["type"].as_str().ok_or_else(invalid)?;
    match kind {
        "text" | "json_object" => {
            fields(v, &["type"])?;
            Ok(v.clone())
        }
        "json_schema" => {
            let schema = if chat {
                fields(v, &["type", "json_schema"])?;
                &v["json_schema"]
            } else {
                v
            };
            fields(
                schema,
                if chat {
                    &["name", "description", "schema", "strict"]
                } else {
                    &["type", "name", "description", "schema", "strict"]
                },
            )?;
            text(&schema["name"], 64)?;
            if !schema["schema"].is_object()
                || schema.get("description").is_some_and(|x| !x.is_string())
                || schema.get("strict").is_some_and(|x| !x.is_boolean())
            {
                return Err(invalid());
            }
            let mut result = schema.clone();
            result["type"] = json!("json_schema");
            Ok(result)
        }
        _ => Err(error("the requested output text format")),
    }
}
fn has_cache_control(v: &Value) -> bool {
    v.get("cache_control").is_some()
        || v["system"]
            .as_array()
            .is_some_and(|blocks| blocks.iter().any(|b| b.get("cache_control").is_some()))
        || v["tools"]
            .as_array()
            .is_some_and(|tools| tools.iter().any(|tool| tool.get("cache_control").is_some()))
        || v["messages"].as_array().is_some_and(|messages| {
            messages.iter().any(|message| {
                message["content"]
                    .as_array()
                    .is_some_and(|blocks| blocks.iter().any(|b| b.get("cache_control").is_some()))
            })
        })
}
pub(super) fn decode(p: Protocol, v: &Value) -> Result<Options, String> {
    let mut n = Options::default();
    if p == Protocol::Anthropic {
        if has_cache_control(v) {
            return Err(error("explicit Anthropic cache breakpoints and lifetime"));
        }
        if let Some(metadata) = v.get("metadata").filter(|x| !x.is_null()) {
            if !metadata.is_object() || !metadata.as_object().ok_or_else(invalid)?.is_empty() {
                return Err(error("Anthropic user metadata semantics"));
            }
        }
        return Ok(n);
    }
    if !matches!(p, Protocol::OpenAiChat | Protocol::OpenAiResponses) {
        return Ok(n);
    }
    if v["tools"]
        .as_array()
        .is_some_and(|tools| tools.iter().any(|tool| tool["type"] == "custom"))
    {
        return Err(error("custom tool input and grammar"));
    }
    if v.get("store")
        .is_some_and(|store| !store.is_null() && store != false)
    {
        return Err(error("stored-response retrieval semantics"));
    }
    if p == Protocol::OpenAiResponses {
        if let Some(include) = v.get("include").filter(|v| !v.is_null()) {
            if !array(include)?.is_empty() {
                return Err(error("requested encrypted reasoning or output enrichment"));
            }
        }
        if let Some(reasoning) = v.get("reasoning").filter(|v| !v.is_null()) {
            fields(reasoning, &["effort", "summary", "context"])?;
            if reasoning.get("context").is_some_and(|x| !x.is_null()) {
                return Err(error("Responses reasoning continuation context"));
            }
            if reasoning.get("summary").is_some_and(|x| !x.is_null()) {
                return Err(error("Responses reasoning summaries"));
            }
            n.effort = value(reasoning, "effort");
        }
        if let Some(config) = v.get("text").filter(|v| !v.is_null()) {
            fields(config, &["verbosity", "format"])?;
            n.verbosity = value(config, "verbosity");
            if let Some(format) = value(config, "format") {
                n.format = Some(text_format(&format, false)?);
            }
        }
        if let Some(metadata) = v.get("client_metadata").filter(|v| !v.is_null()) {
            if !metadata.is_object() || !metadata.as_object().ok_or_else(invalid)?.is_empty() {
                return Err(error("native client transport metadata"));
            }
        }
        if v.get("access_programs").is_some_and(|v| !v.is_null()) {
            return Err(error("native access program context"));
        }
        if v.get("stream_options").is_some_and(|v| !v.is_null()) {
            return Err(error("Responses reasoning stream delivery options"));
        }
    } else {
        n.effort = value(v, "reasoning_effort");
        n.verbosity = value(v, "verbosity");
        if let Some(format) = value(v, "response_format") {
            n.format = Some(text_format(&format, true)?);
        }
    }
    if let Some(effort) = &n.effort {
        text(effort, 32)?;
        if !matches!(
            effort.as_str(),
            Some("none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra")
        ) {
            return Err(invalid());
        }
    }
    if let Some(verbosity) = &n.verbosity {
        if !matches!(verbosity.as_str(), Some("low" | "medium" | "high")) {
            return Err(invalid());
        }
    }
    n.metadata = value(v, "metadata");
    if let Some(metadata) = &n.metadata {
        let map = metadata.as_object().ok_or_else(invalid)?;
        if map.len() > 16
            || map.iter().any(|(key, value)| {
                key.chars().count() > 64 || value.as_str().is_none_or(|s| s.chars().count() > 512)
            })
        {
            return Err(invalid());
        }
    }
    n.cache_key = value(v, "prompt_cache_key");
    if let Some(key) = &n.cache_key {
        text(key, 1024)?;
    }
    n.cache_retention = value(v, "prompt_cache_retention");
    if n.cache_retention
        .as_ref()
        .is_some_and(|v| !matches!(v.as_str(), Some("in_memory" | "24h")))
    {
        return Err(invalid());
    }
    n.service_tier = value(v, "service_tier");
    if n.service_tier.as_ref().is_some_and(|v| {
        !matches!(
            v.as_str(),
            Some("auto" | "default" | "flex" | "priority" | "scale")
        )
    }) {
        return Err(invalid());
    }
    Ok(n)
}
pub(super) fn encode(p: Protocol, n: &Options, v: &mut Value) -> Result<(), String> {
    if !matches!(p, Protocol::OpenAiChat | Protocol::OpenAiResponses) {
        if n.effort.is_some() {
            return Err(error("OpenAI reasoning effort"));
        }
        if n.verbosity.is_some() || n.format.is_some() {
            return Err(error("OpenAI text verbosity and format"));
        }
        if n.metadata.is_some() {
            return Err(error("OpenAI object metadata"));
        }
        if n.cache_key.is_some() || n.cache_retention.is_some() {
            return Err(error("OpenAI cache key and retention"));
        }
        if n.service_tier.is_some() {
            return Err(error("OpenAI service tier"));
        }
        return Ok(());
    }
    if let Some(effort) = &n.effort {
        if p == Protocol::OpenAiChat {
            v["reasoning_effort"] = effort.clone();
        } else {
            v["reasoning"] = json!({"effort":effort});
        }
    }
    if let Some(verbosity) = &n.verbosity {
        if p == Protocol::OpenAiChat {
            v["verbosity"] = verbosity.clone();
        } else {
            v["text"]["verbosity"] = verbosity.clone();
        }
    }
    if let Some(format) = &n.format {
        if p == Protocol::OpenAiChat {
            if format["type"] == "json_schema" {
                let mut schema = format.clone();
                schema.as_object_mut().ok_or_else(invalid)?.remove("type");
                v["response_format"] = json!({"type":"json_schema","json_schema":schema});
            } else {
                v["response_format"] = format.clone();
            }
        } else {
            v["text"]["format"] = format.clone();
        }
    }
    for (key, value) in [
        ("metadata", &n.metadata),
        ("prompt_cache_key", &n.cache_key),
        ("prompt_cache_retention", &n.cache_retention),
        ("service_tier", &n.service_tier),
    ] {
        if let Some(value) = value {
            v[key] = value.clone();
        }
    }
    Ok(())
}
