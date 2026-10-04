//! Bounded, stateless conversion of the shared text/function-tool wire subset.
//! Same-protocol traffic belongs to the byte-preserving gateway path.
use super::gateway_profiles::Protocol;
use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

#[path = "gateway_transform/options.rs"]
mod options;
#[path = "gateway_transform/stream.rs"]
mod stream;
#[cfg(test)]
#[path = "gateway_transform/tests.rs"]
mod tests;

const REQUEST_LIMIT: usize = 16 * 1024 * 1024;
const RESPONSE_LIMIT: usize = 64 * 1024 * 1024;
fn invalid() -> String {
    "Gateway protocol conversion rejected malformed or unsupported data.".into()
}
fn unsupported() -> String {
    "Gateway protocol conversion does not support this request feature.".into()
}

// serde_json::Value otherwise silently accepts duplicate object members.
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Unique;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("unique JSON members")
            }
            fn visit_bool<E: de::Error>(self, x: bool) -> Result<Unique, E> {
                Ok(Unique(json!(x)))
            }
            fn visit_i64<E: de::Error>(self, x: i64) -> Result<Unique, E> {
                Ok(Unique(json!(x)))
            }
            fn visit_u64<E: de::Error>(self, x: u64) -> Result<Unique, E> {
                Ok(Unique(json!(x)))
            }
            fn visit_f64<E: de::Error>(self, x: f64) -> Result<Unique, E> {
                Ok(Unique(json!(x)))
            }
            fn visit_str<E: de::Error>(self, x: &str) -> Result<Unique, E> {
                Ok(Unique(json!(x)))
            }
            fn visit_string<E: de::Error>(self, x: String) -> Result<Unique, E> {
                Ok(Unique(json!(x)))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Unique, A::Error> {
                let mut v = Vec::new();
                while let Some(x) = a.next_element::<Unique>()? {
                    v.push(x.0);
                }
                Ok(Unique(Value::Array(v)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Unique, A::Error> {
                let mut v = Map::new();
                while let Some((k, x)) = a.next_entry::<String, Unique>()? {
                    if v.insert(k, x.0).is_some() {
                        return Err(de::Error::custom("duplicate JSON member"));
                    }
                }
                Ok(Unique(Value::Object(v)))
            }
        }
        d.deserialize_any(V)
    }
}
fn parse(body: &[u8], limit: usize) -> Result<Value, String> {
    if body.len() > limit {
        return Err(invalid());
    }
    serde_json::from_slice::<Unique>(body)
        .map(|v| v.0)
        .map_err(|_| invalid())
}
fn string(v: &Value) -> Result<String, String> {
    v.as_str().map(str::to_owned).ok_or_else(invalid)
}
fn array(v: &Value) -> Result<&Vec<Value>, String> {
    v.as_array().ok_or_else(invalid)
}
fn fields(v: &Value, allowed: &[&str]) -> Result<(), String> {
    if v.as_object()
        .ok_or_else(invalid)?
        .keys()
        .any(|k| !allowed.contains(&k.as_str()))
    {
        return Err(unsupported());
    }
    Ok(())
}
fn encode(v: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(v).map_err(|_| invalid())
}
fn arguments(v: &Value) -> Result<Value, String> {
    let x = if let Some(s) = v.as_str() {
        parse(s.as_bytes(), REQUEST_LIMIT)?
    } else {
        v.clone()
    };
    if !x.is_object() {
        return Err(invalid());
    }
    Ok(x)
}
#[derive(Clone, Debug)]
enum Part {
    Text(String),
    Image {
        mime: String,
        data: String,
    },
    Call {
        id: String,
        name: String,
        args: Value,
    },
    Result {
        id: String,
        name: String,
        text: String,
    },
}
#[derive(Clone, Debug)]
struct Message {
    role: String,
    parts: Vec<Part>,
}
#[derive(Default)]
struct Input {
    system: Vec<String>,
    messages: Vec<Message>,
    tools: Vec<Value>,
    max: Option<Value>,
    temperature: Option<Value>,
    top_p: Option<Value>,
    top_k: Option<Value>,
    stop: Option<Value>,
    choice: Option<Value>,
    parallel: Option<bool>,
    stream: bool,
    options: options::Options,
}
#[derive(Default)]
struct Output {
    id: String,
    parts: Vec<Part>,
    finish: String,
    input: u64,
    output: u64,
    cached: u64,
    cache_write: u64,
    reasoning: u64,
    usage_present: bool,
}

pub(super) struct Request {
    pub path: String,
    pub body: Vec<u8>,
    pub stream: bool,
}
pub(super) fn supports(native: Protocol, upstream: Protocol) -> bool {
    native != upstream
}
/// Native Codex always requests encrypted reasoning enrichment and may require
/// all-turns reasoning context. Its admitted profile therefore keeps Responses.
/// Other native families still undergo request-level subset validation.
pub(super) fn native_cli_supports(
    cli: crate::cli_catalog::TitleCli,
    native: Protocol,
    upstream: Protocol,
) -> bool {
    native == upstream || (cli != crate::cli_catalog::TitleCli::Codex && supports(native, upstream))
}
pub(super) fn request(
    native: Protocol,
    upstream: Protocol,
    path: &str,
    body: &[u8],
    model: &str,
) -> Result<Request, String> {
    if path.contains("count_tokens") || path.contains("countTokens") {
        return Err(
            "Cross-protocol token counting is unsupported; use a matching provider protocol."
                .into(),
        );
    }
    if native == upstream {
        return Err(unsupported());
    }
    let value = parse(body, REQUEST_LIMIT)?;
    let mut input = decode_request(native, &value)?;
    if native == Protocol::Gemini {
        input.stream = path.contains(":streamGenerateContent");
    }
    let stream = input.stream;
    let body = encode(&encode_request(upstream, input, model)?)?;
    if body.len() > REQUEST_LIMIT {
        return Err(invalid());
    }
    let path = match upstream {
        Protocol::Anthropic => "/v1/messages".into(),
        Protocol::OpenAiChat => "/v1/chat/completions".into(),
        Protocol::OpenAiResponses => "/v1/responses".into(),
        Protocol::Gemini => {
            if model.is_empty()
                || !model
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            {
                return Err(unsupported());
            }
            format!(
                "/v1beta/models/{model}:{}",
                if stream {
                    "streamGenerateContent?alt=sse"
                } else {
                    "generateContent"
                }
            )
        }
    };
    Ok(Request { path, body, stream })
}
pub(super) fn response(
    native: Protocol,
    upstream: Protocol,
    body: &[u8],
    model: &str,
) -> Result<Vec<u8>, String> {
    if native == upstream {
        return Err(unsupported());
    }
    encode(&encode_response(
        native,
        &decode_response(upstream, &parse(body, RESPONSE_LIMIT)?)?,
        model,
    )?)
}
pub(super) fn stream_response(
    native: Protocol,
    upstream: Protocol,
    body: &[u8],
    model: &str,
) -> Result<Vec<u8>, String> {
    if native == upstream {
        return Err(unsupported());
    }
    stream::convert(native, upstream, body, model)
}

fn text_parts(v: &Value) -> Result<Vec<String>, String> {
    if let Some(s) = v.as_str() {
        return Ok(vec![s.into()]);
    }
    array(v)?
        .iter()
        .map(|x| {
            fields(x, &["type", "text"])?;
            if x["type"] != "text" {
                return Err(unsupported());
            }
            string(&x["text"])
        })
        .collect()
}
fn inline_image(url: &str) -> Result<Part, String> {
    let (header, data) = url.split_once(',').ok_or_else(unsupported)?;
    let mime = header
        .strip_prefix("data:")
        .and_then(|x| x.strip_suffix(";base64"))
        .ok_or_else(unsupported)?;
    if !matches!(
        mime,
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    ) || data.is_empty()
    {
        return Err(unsupported());
    }
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| invalid())?;
    Ok(Part::Image {
        mime: mime.into(),
        data: data.into(),
    })
}
fn decode_parts(
    p: Protocol,
    v: &Value,
    names: &mut HashMap<String, String>,
) -> Result<Vec<Part>, String> {
    if v.is_null() {
        return Ok(vec![]);
    }
    if let Some(s) = v.as_str() {
        return Ok(vec![Part::Text(s.into())]);
    }
    let mut parts = Vec::new();
    for x in array(v)? {
        match p {
            Protocol::Anthropic => match x["type"].as_str().ok_or_else(invalid)? {
                "text" => {
                    fields(x, &["type", "text"])?;
                    parts.push(Part::Text(string(&x["text"])?));
                }
                "image" => {
                    fields(x, &["type", "source"])?;
                    fields(&x["source"], &["type", "media_type", "data"])?;
                    if x["source"]["type"] != "base64" {
                        return Err(unsupported());
                    }
                    parts.push(inline_image(&format!(
                        "data:{};base64,{}",
                        string(&x["source"]["media_type"])?,
                        string(&x["source"]["data"])?
                    ))?);
                }
                "tool_use" => {
                    fields(x, &["type", "id", "name", "input"])?;
                    let id = string(&x["id"])?;
                    let name = string(&x["name"])?;
                    names.insert(id.clone(), name.clone());
                    parts.push(Part::Call {
                        id,
                        name,
                        args: arguments(&x["input"])?,
                    });
                }
                "tool_result" => {
                    fields(x, &["type", "tool_use_id", "content", "is_error"])?;
                    if x["is_error"] == true {
                        return Err(unsupported());
                    }
                    let id = string(&x["tool_use_id"])?;
                    let text = text_parts(&x["content"])?.join("");
                    parts.push(Part::Result {
                        name: names.get(&id).cloned().ok_or_else(invalid)?,
                        id,
                        text,
                    });
                }
                _ => return Err(unsupported()),
            },
            Protocol::OpenAiChat | Protocol::OpenAiResponses => {
                match x["type"].as_str().ok_or_else(invalid)? {
                    "text" | "input_text" | "output_text" => {
                        fields(x, &["type", "text", "annotations", "logprobs"])?;
                        if x.get("annotations")
                            .is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty()))
                        {
                            return Err(unsupported());
                        }
                        if x.get("logprobs").is_some_and(|v| {
                            !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty())
                        }) {
                            return Err(unsupported());
                        }
                        parts.push(Part::Text(string(&x["text"])?));
                    }
                    "image_url" => {
                        fields(x, &["type", "image_url"])?;
                        fields(&x["image_url"], &["url", "detail"])?;
                        if !x["image_url"]["detail"].is_null() && x["image_url"]["detail"] != "auto"
                        {
                            return Err(unsupported());
                        }
                        parts.push(inline_image(&string(&x["image_url"]["url"])?)?);
                    }
                    "input_image" => {
                        fields(x, &["type", "image_url", "detail"])?;
                        if !x["detail"].is_null() && x["detail"] != "auto" {
                            return Err(unsupported());
                        }
                        parts.push(inline_image(&string(&x["image_url"])?)?);
                    }
                    _ => return Err(unsupported()),
                }
            }
            Protocol::Gemini => {
                fields(
                    x,
                    &[
                        "text",
                        "inlineData",
                        "inline_data",
                        "functionCall",
                        "functionResponse",
                    ],
                )?;
                if let Some(t) = x.get("text") {
                    parts.push(Part::Text(string(t)?));
                } else if let Some(i) = x.get("inlineData").or_else(|| x.get("inline_data")) {
                    fields(i, &["mimeType", "mime_type", "data"])?;
                    let mime = i
                        .get("mimeType")
                        .or_else(|| i.get("mime_type"))
                        .ok_or_else(invalid)?;
                    parts.push(inline_image(&format!(
                        "data:{};base64,{}",
                        string(mime)?,
                        string(&i["data"])?
                    ))?);
                } else if let Some(c) = x.get("functionCall") {
                    fields(c, &["id", "name", "args"])?;
                    let name = string(&c["name"])?;
                    let id = if let Some(id) = c.get("id") {
                        string(id)?
                    } else {
                        // Older Gemini tool calls identify results by name only.
                        // Multiple pending calls with that name cannot be paired safely.
                        if names.values().any(|pending| pending == &name) {
                            return Err(unsupported());
                        }
                        format!("lomi_call_{}", names.len())
                    };
                    if id.is_empty()
                        || name.is_empty()
                        || names.insert(id.clone(), name.clone()).is_some()
                    {
                        return Err(invalid());
                    }
                    parts.push(Part::Call {
                        id,
                        name,
                        args: arguments(c.get("args").unwrap_or(&json!({})))?,
                    });
                } else if let Some(r) = x.get("functionResponse") {
                    fields(r, &["id", "name", "response"])?;
                    let name = string(&r["name"])?;
                    let id = if let Some(id) = r.get("id") {
                        string(id)?
                    } else {
                        let matching = names
                            .iter()
                            .filter(|(_, pending)| *pending == &name)
                            .map(|(id, _)| id.clone())
                            .collect::<Vec<_>>();
                        if matching.len() != 1 {
                            return Err(unsupported());
                        }
                        matching[0].clone()
                    };
                    if names.get(&id) != Some(&name) {
                        return Err(invalid());
                    }
                    names.insert(id.clone(), String::new());
                    if !r["response"].is_object() {
                        return Err(invalid());
                    }
                    let text = if r["response"].as_object().is_some_and(|o| o.len() == 1) {
                        r["response"]["output"].as_str().map(str::to_owned)
                    } else {
                        None
                    }
                    .unwrap_or(serde_json::to_string(&r["response"]).map_err(|_| invalid())?);
                    parts.push(Part::Result { id, name, text });
                } else {
                    return Err(unsupported());
                }
            }
        }
    }
    Ok(parts)
}
fn decode_request(p: Protocol, v: &Value) -> Result<Input, String> {
    let mut n = Input {
        options: options::decode(p, v)?,
        ..Input::default()
    };
    let mut names = HashMap::new();
    let messages = match p {
        Protocol::Anthropic => {
            fields(
                v,
                &[
                    "model",
                    "messages",
                    "system",
                    "metadata",
                    "max_tokens",
                    "temperature",
                    "top_p",
                    "top_k",
                    "stop_sequences",
                    "tools",
                    "tool_choice",
                    "stream",
                ],
            )?;
            if let Some(s) = v.get("system") {
                n.system = text_parts(s)?;
            }
            n.max = v.get("max_tokens").cloned();
            n.stop = v.get("stop_sequences").cloned();
            n.top_k = v.get("top_k").cloned();
            &v["messages"]
        }
        Protocol::OpenAiChat => {
            fields(
                v,
                &[
                    "model",
                    "messages",
                    "max_tokens",
                    "max_completion_tokens",
                    "temperature",
                    "top_p",
                    "stop",
                    "tools",
                    "tool_choice",
                    "parallel_tool_calls",
                    "stream",
                    "stream_options",
                    "n",
                    "store",
                    "reasoning_effort",
                    "verbosity",
                    "response_format",
                    "metadata",
                    "prompt_cache_key",
                    "prompt_cache_retention",
                    "service_tier",
                ],
            )?;
            if v.get("n").is_some_and(|x| x != 1) {
                return Err(unsupported());
            }
            if let Some(s) = v.get("stream_options") {
                fields(s, &["include_usage"])?;
            }
            if v.get("max_tokens").is_some() && v.get("max_completion_tokens").is_some() {
                return Err(unsupported());
            }
            n.max = v
                .get("max_completion_tokens")
                .or_else(|| v.get("max_tokens"))
                .cloned();
            n.stop = v.get("stop").cloned();
            &v["messages"]
        }
        Protocol::OpenAiResponses => {
            fields(
                v,
                &[
                    "model",
                    "input",
                    "instructions",
                    "max_output_tokens",
                    "temperature",
                    "top_p",
                    "tools",
                    "tool_choice",
                    "parallel_tool_calls",
                    "stream",
                    "store",
                    "reasoning",
                    "include",
                    "text",
                    "metadata",
                    "prompt_cache_key",
                    "prompt_cache_retention",
                    "service_tier",
                    "client_metadata",
                    "access_programs",
                    "stream_options",
                ],
            )?;
            if v["store"] == true {
                return Err(unsupported());
            }
            if let Some(s) = v.get("instructions") {
                n.system.push(string(s)?);
            }
            n.max = v.get("max_output_tokens").cloned();
            &v["input"]
        }
        Protocol::Gemini => {
            fields(
                v,
                &[
                    "contents",
                    "systemInstruction",
                    "generationConfig",
                    "tools",
                    "toolConfig",
                ],
            )?;
            if let Some(s) = v.get("systemInstruction") {
                fields(s, &["role", "parts"])?;
                n.system = decode_parts(p, &s["parts"], &mut names)?
                    .into_iter()
                    .map(|x| match x {
                        Part::Text(s) => Ok(s),
                        _ => Err(unsupported()),
                    })
                    .collect::<Result<_, _>>()?;
            }
            if let Some(c) = v.get("generationConfig") {
                fields(
                    c,
                    &[
                        "maxOutputTokens",
                        "temperature",
                        "topP",
                        "topK",
                        "stopSequences",
                        "candidateCount",
                    ],
                )?;
                if c.get("candidateCount").is_some_and(|x| x != 1) {
                    return Err(unsupported());
                }
                n.max = c.get("maxOutputTokens").cloned();
                n.temperature = c.get("temperature").cloned();
                n.top_p = c.get("topP").cloned();
                n.top_k = c.get("topK").cloned();
                n.stop = c.get("stopSequences").cloned();
            }
            &v["contents"]
        }
    };
    if p != Protocol::Gemini {
        n.temperature = v.get("temperature").cloned();
        n.top_p = v.get("top_p").cloned();
        n.stream = v
            .get("stream")
            .map(|x| x.as_bool().ok_or_else(invalid))
            .transpose()?
            .unwrap_or(false);
        n.parallel = v
            .get("parallel_tool_calls")
            .map(|x| x.as_bool().ok_or_else(invalid))
            .transpose()?;
    }
    if let Some(s) = messages.as_str() {
        n.messages.push(Message {
            role: "user".into(),
            parts: vec![Part::Text(s.into())],
        });
    } else {
        for m in array(messages)? {
            if p == Protocol::OpenAiResponses && m["type"] == "function_call" {
                fields(m, &["type", "id", "call_id", "name", "arguments", "status"])?;
                let id = string(&m["call_id"])?;
                let name = string(&m["name"])?;
                names.insert(id.clone(), name.clone());
                n.messages.push(Message {
                    role: "assistant".into(),
                    parts: vec![Part::Call {
                        id,
                        name,
                        args: arguments(&m["arguments"])?,
                    }],
                });
                continue;
            }
            if p == Protocol::OpenAiResponses && m["type"] == "function_call_output" {
                fields(m, &["type", "call_id", "output"])?;
                let id = string(&m["call_id"])?;
                n.messages.push(Message {
                    role: "user".into(),
                    parts: vec![Part::Result {
                        name: names.get(&id).cloned().ok_or_else(invalid)?,
                        id,
                        text: string(&m["output"])?,
                    }],
                });
                continue;
            }
            fields(
                m,
                if p == Protocol::Gemini {
                    &["role", "parts"]
                } else {
                    &[
                        "role",
                        "content",
                        "type",
                        "tool_calls",
                        "tool_call_id",
                        "name",
                    ]
                },
            )?;
            if m.get("type").is_some_and(|x| x != "message") {
                return Err(unsupported());
            }
            let mut role = string(&m["role"])?;
            if role == "model" {
                role = "assistant".into();
            }
            if role == "system" || role == "developer" {
                n.system.extend(text_parts(&m["content"])?);
                continue;
            }
            if role == "tool" {
                let id = string(&m["tool_call_id"])?;
                n.messages.push(Message {
                    role: "user".into(),
                    parts: vec![Part::Result {
                        name: names.get(&id).cloned().ok_or_else(invalid)?,
                        id,
                        text: string(&m["content"])?,
                    }],
                });
                continue;
            }
            if role != "user" && role != "assistant" {
                return Err(unsupported());
            }
            let mut parts = decode_parts(
                p,
                if p == Protocol::Gemini {
                    &m["parts"]
                } else {
                    &m["content"]
                },
                &mut names,
            )?;
            if let Some(calls) = m.get("tool_calls") {
                for c in array(calls)? {
                    fields(c, &["id", "type", "function"])?;
                    if c["type"] != "function" {
                        return Err(unsupported());
                    }
                    fields(&c["function"], &["name", "arguments"])?;
                    let id = string(&c["id"])?;
                    let name = string(&c["function"]["name"])?;
                    names.insert(id.clone(), name.clone());
                    parts.push(Part::Call {
                        id,
                        name,
                        args: arguments(&c["function"]["arguments"])?,
                    });
                }
            }
            n.messages.push(Message { role, parts });
        }
    }
    if let Some(tools) = v.get("tools") {
        for t in array(tools)? {
            if p == Protocol::Gemini {
                fields(t, &["functionDeclarations"])?;
                for f in array(&t["functionDeclarations"])? {
                    fields(
                        f,
                        &["name", "description", "parameters", "parametersJsonSchema"],
                    )?;
                    if f.get("parameters").is_some() && f.get("parametersJsonSchema").is_some() {
                        return Err(unsupported());
                    }
                    n.tools.push(json!({"name":string(&f["name"])?,"description":f.get("description").cloned().unwrap_or(json!("")),"parameters":f.get("parametersJsonSchema").or_else(||f.get("parameters")).cloned().unwrap_or(json!({"type":"object","properties":{}}))}));
                }
                continue;
            }
            let f = if p == Protocol::OpenAiChat {
                fields(t, &["type", "function"])?;
                if t["type"] != "function" {
                    return Err(unsupported());
                }
                &t["function"]
            } else {
                t
            };
            fields(
                f,
                if p == Protocol::Anthropic {
                    &["name", "description", "input_schema", "strict"]
                } else {
                    &["type", "name", "description", "parameters", "strict"]
                },
            )?;
            if p == Protocol::OpenAiResponses && f["type"] != "function" {
                return Err(unsupported());
            }
            n.tools.push(json!({"name":string(&f["name"])?,"description":f.get("description").cloned().unwrap_or(json!("")),"parameters":f.get(if p==Protocol::Anthropic {"input_schema"}else{"parameters"}).cloned().unwrap_or(json!({"type":"object","properties":{}})),"strict":f.get("strict").cloned().unwrap_or(json!(false))}));
        }
    }
    n.choice = decode_choice(p, v, &mut n.parallel)?;
    for x in [&n.max, &n.temperature, &n.top_p, &n.top_k]
        .into_iter()
        .flatten()
    {
        if !x.is_number() {
            return Err(invalid());
        }
    }
    Ok(n)
}
fn decode_choice(
    p: Protocol,
    v: &Value,
    parallel: &mut Option<bool>,
) -> Result<Option<Value>, String> {
    let c = if p == Protocol::Gemini {
        let Some(c) = v.get("toolConfig") else {
            return Ok(None);
        };
        fields(c, &["functionCallingConfig"])?;
        let c = &c["functionCallingConfig"];
        fields(c, &["mode", "allowedFunctionNames"])?;
        let mode = string(&c["mode"])?;
        if mode != "ANY"
            && c.get("allowedFunctionNames")
                .is_some_and(|names| !names.as_array().is_some_and(Vec::is_empty))
        {
            return Err(unsupported());
        }
        return Ok(Some(match mode.as_str() {
            "AUTO" => json!("auto"),
            "NONE" => json!("none"),
            "ANY" => {
                if let Some(names) = c.get("allowedFunctionNames") {
                    let names = array(names)?;
                    if names.len() != 1 {
                        return Err(unsupported());
                    }
                    json!({"name":string(&names[0])?})
                } else {
                    json!("required")
                }
            }
            _ => return Err(unsupported()),
        }));
    } else {
        let Some(c) = v.get("tool_choice") else {
            return Ok(None);
        };
        c
    };
    if let Some(s) = c.as_str() {
        return match s {
            "auto" | "none" | "required" => Ok(Some(json!(s))),
            _ => Err(unsupported()),
        };
    }
    if p == Protocol::Anthropic {
        fields(c, &["type", "name", "disable_parallel_tool_use"])?;
        *parallel = c
            .get("disable_parallel_tool_use")
            .map(|x| x.as_bool().map(|b| !b).ok_or_else(invalid))
            .transpose()?;
        return Ok(Some(match c["type"].as_str() {
            Some("auto") => json!("auto"),
            Some("none") => json!("none"),
            Some("any") => json!("required"),
            Some("tool") => json!({"name":string(&c["name"])?}),
            _ => return Err(unsupported()),
        }));
    }
    fields(c, &["type", "function", "name"])?;
    if c["type"] != "function" {
        return Err(unsupported());
    }
    let name = if p == Protocol::OpenAiChat {
        fields(&c["function"], &["name"])?;
        string(&c["function"]["name"])?
    } else {
        string(&c["name"])?
    };
    Ok(Some(json!({"name":name})))
}
fn encode_part(p: Protocol, part: &Part, assistant: bool) -> Result<Value, String> {
    Ok(match (p, part) {
        (Protocol::Anthropic, Part::Text(text)) => json!({"type":"text","text":text}),
        (Protocol::Anthropic, Part::Image { mime, data }) => {
            json!({"type":"image","source":{"type":"base64","media_type":mime,"data":data}})
        }
        (Protocol::Anthropic, Part::Call { id, name, args }) => {
            json!({"type":"tool_use","id":id,"name":name,"input":args})
        }
        (Protocol::Anthropic, Part::Result { id, text, .. }) => {
            json!({"type":"tool_result","tool_use_id":id,"content":text})
        }
        (Protocol::OpenAiChat, Part::Text(text)) => json!({"type":"text","text":text}),
        (Protocol::OpenAiChat, Part::Image { mime, data }) => {
            json!({"type":"image_url","image_url":{"url":format!("data:{mime};base64,{data}")}})
        }
        (Protocol::OpenAiChat, Part::Call { id, name, args }) => {
            json!({"id":id,"type":"function","function":{"name":name,"arguments":serde_json::to_string(args).map_err(|_|invalid())?}})
        }
        (Protocol::OpenAiResponses, Part::Text(text)) => {
            if assistant {
                json!({"type":"output_text","text":text,"annotations":[]})
            } else {
                json!({"type":"input_text","text":text})
            }
        }
        (Protocol::OpenAiResponses, Part::Image { mime, data }) => {
            json!({"type":"input_image","image_url":format!("data:{mime};base64,{data}")})
        }
        (Protocol::OpenAiResponses, Part::Call { id, name, args }) => {
            json!({"type":"function_call","call_id":id,"name":name,"arguments":serde_json::to_string(args).map_err(|_|invalid())?})
        }
        (Protocol::OpenAiResponses, Part::Result { id, text, .. }) => {
            json!({"type":"function_call_output","call_id":id,"output":text})
        }
        (Protocol::Gemini, Part::Text(text)) => json!({"text":text}),
        (Protocol::Gemini, Part::Image { mime, data }) => {
            if mime == "image/gif" {
                return Err(unsupported());
            }
            json!({"inlineData":{"mimeType":mime,"data":data}})
        }
        (Protocol::Gemini, Part::Call { id, name, args }) => {
            json!({"functionCall":{"id":id,"name":name,"args":args}})
        }
        (Protocol::Gemini, Part::Result { id, name, text }) => {
            json!({"functionResponse":{"id":id,"name":name,"response":{"output":text}}})
        }
        _ => return Err(unsupported()),
    })
}
fn encode_messages(p: Protocol, messages: &[Message]) -> Result<Vec<Value>, String> {
    let mut result: Vec<Value> = Vec::new();
    for m in messages {
        if p == Protocol::OpenAiResponses {
            let mut content = Vec::new();
            // Flush message parts around calls/results to retain ordering.
            for part in &m.parts {
                if matches!(part, Part::Call { .. } | Part::Result { .. }) {
                    if !content.is_empty() {
                        result.push(json!({"role":m.role,"content":std::mem::take(&mut content)}));
                    }
                    result.push(encode_part(p, part, m.role == "assistant")?);
                } else {
                    content.push(encode_part(p, part, m.role == "assistant")?);
                }
            }
            if !content.is_empty() {
                result.push(json!({"role":m.role,"content":content}));
            }
        } else if p == Protocol::OpenAiChat {
            let mut content = Vec::new();
            let mut calls = Vec::new();
            for part in &m.parts {
                match part {
                    Part::Call { .. } => calls.push(encode_part(p, part, true)?),
                    Part::Result { id, text, .. } => {
                        if !content.is_empty() || !calls.is_empty() {
                            let mut prior =
                                json!({"role":m.role,"content":std::mem::take(&mut content)});
                            if !calls.is_empty() {
                                prior["tool_calls"] = json!(std::mem::take(&mut calls));
                            }
                            result.push(prior);
                        }
                        result.push(json!({"role":"tool","tool_call_id":id,"content":text}));
                    }
                    _ => content.push(encode_part(p, part, m.role == "assistant")?),
                }
            }
            if !content.is_empty() || !calls.is_empty() {
                let mut v = json!({"role":m.role,"content":if content.is_empty(){Value::Null}else{json!(content)}});
                if !calls.is_empty() {
                    v["tool_calls"] = json!(calls);
                }
                result.push(v);
            }
        } else {
            let content = m
                .parts
                .iter()
                .map(|x| encode_part(p, x, m.role == "assistant"))
                .collect::<Result<Vec<_>, _>>()?;
            if p == Protocol::Gemini {
                result.push(
                    json!({"role":if m.role=="assistant"{"model"}else{"user"},"parts":content}),
                );
            } else {
                if let Some(last) = result.last_mut().filter(|v| v["role"] == m.role) {
                    last["content"]
                        .as_array_mut()
                        .ok_or_else(invalid)?
                        .extend(content);
                } else {
                    result.push(json!({"role":m.role,"content":content}));
                }
            }
        }
    }
    Ok(result)
}
fn encode_request(p: Protocol, n: Input, model: &str) -> Result<Value, String> {
    if n.top_k.is_some() && matches!(p, Protocol::OpenAiChat | Protocol::OpenAiResponses)
        || n.stop.is_some() && p == Protocol::OpenAiResponses
        || n.parallel == Some(false) && p == Protocol::Gemini
    {
        return Err(unsupported());
    }
    let messages = encode_messages(p, &n.messages)?;
    let mut v = match p {
        Protocol::Anthropic => {
            json!({"model":model,"messages":messages,"max_tokens":n.max.clone().unwrap_or(json!(4096)),"stream":n.stream})
        }
        Protocol::OpenAiChat => json!({"model":model,"messages":messages,"stream":n.stream}),
        Protocol::OpenAiResponses => {
            json!({"model":model,"input":messages,"stream":n.stream,"store":false})
        }
        Protocol::Gemini => json!({"contents":messages}),
    };
    if !n.system.is_empty() {
        let text = n.system.join("\n\n");
        match p {
            Protocol::Anthropic => v["system"] = json!(text),
            Protocol::OpenAiResponses => v["instructions"] = json!(text),
            Protocol::OpenAiChat => v["messages"]
                .as_array_mut()
                .ok_or_else(invalid)?
                .insert(0, json!({"role":"system","content":text})),
            Protocol::Gemini => v["systemInstruction"] = json!({"parts":[{"text":text}]}),
        }
    }
    let mut config = json!({});
    for (value, normal, gemini) in [
        (
            &n.max,
            match p {
                Protocol::OpenAiResponses => "max_output_tokens",
                Protocol::OpenAiChat => "max_completion_tokens",
                _ => "max_tokens",
            },
            "maxOutputTokens",
        ),
        (&n.temperature, "temperature", "temperature"),
        (&n.top_p, "top_p", "topP"),
        (&n.top_k, "top_k", "topK"),
    ] {
        if let Some(value) = value {
            if p == Protocol::Gemini {
                config[gemini] = value.clone();
            } else {
                v[normal] = value.clone();
            }
        }
    }
    if let Some(stop) = n.stop {
        let stop = if stop.is_string() {
            json!([stop])
        } else {
            if array(&stop)?.iter().any(|x| !x.is_string()) {
                return Err(invalid());
            }
            stop
        };
        if p == Protocol::Gemini {
            config["stopSequences"] = stop;
        } else {
            v[if p == Protocol::Anthropic {
                "stop_sequences"
            } else {
                "stop"
            }] = stop;
        }
    }
    if p == Protocol::Gemini && !config.as_object().ok_or_else(invalid)?.is_empty() {
        v["generationConfig"] = config;
    }
    if !n.tools.is_empty() {
        let mut tools = Vec::new();
        for t in &n.tools {
            if p == Protocol::Gemini && t["strict"] == true {
                return Err(unsupported());
            }
            let mut f = json!({"name":t["name"],"description":t["description"],"parameters":t["parameters"]});
            match p {
                Protocol::Anthropic => {
                    f.as_object_mut().ok_or_else(invalid)?.remove("parameters");
                    f["input_schema"] = t["parameters"].clone();
                    if t["strict"] == true {
                        f["strict"] = json!(true);
                    }
                }
                Protocol::OpenAiChat => {
                    f["strict"] = t["strict"].clone();
                    f = json!({"type":"function","function":f});
                }
                Protocol::OpenAiResponses => {
                    f["type"] = json!("function");
                    f["strict"] = t["strict"].clone();
                }
                Protocol::Gemini => {
                    f.as_object_mut().ok_or_else(invalid)?.remove("parameters");
                    f["parametersJsonSchema"] = t["parameters"].clone();
                }
            }
            tools.push(f);
        }
        v["tools"] = if p == Protocol::Gemini {
            json!([{"functionDeclarations":tools}])
        } else {
            json!(tools)
        };
    }
    if let Some(c) = n.choice {
        let name = c.get("name");
        let mode = c.as_str().unwrap_or("named");
        match p {
            Protocol::Anthropic => {
                let mut c = if let Some(name) = name {
                    json!({"type":"tool","name":name})
                } else {
                    json!({"type":if mode=="required"{"any"}else{mode}})
                };
                if let Some(parallel) = n.parallel {
                    c["disable_parallel_tool_use"] = json!(!parallel);
                }
                v["tool_choice"] = c;
            }
            Protocol::OpenAiChat => {
                v["tool_choice"] = if let Some(name) = name {
                    json!({"type":"function","function":{"name":name}})
                } else {
                    c
                }
            }
            Protocol::OpenAiResponses => {
                v["tool_choice"] = if let Some(name) = name {
                    json!({"type":"function","name":name})
                } else {
                    c
                }
            }
            Protocol::Gemini => {
                let mut c = json!({"mode":match mode{"auto"=>"AUTO","none"=>"NONE",_=>"ANY"}});
                if let Some(name) = name {
                    c["allowedFunctionNames"] = json!([name]);
                }
                v["toolConfig"] = json!({"functionCallingConfig":c});
            }
        }
    } else if p == Protocol::Anthropic {
        if let Some(parallel) = n.parallel {
            v["tool_choice"] = json!({"type":"auto","disable_parallel_tool_use":!parallel});
        }
    }
    if matches!(p, Protocol::OpenAiChat | Protocol::OpenAiResponses) {
        if let Some(parallel) = n.parallel {
            v["parallel_tool_calls"] = json!(parallel);
        }
    }
    if p == Protocol::OpenAiChat && n.stream {
        v["stream_options"] = json!({"include_usage":true});
    }
    options::encode(p, &n.options, &mut v)?;
    Ok(v)
}
fn token(v: &Value) -> Result<u64, String> {
    if v.is_null() {
        Ok(0)
    } else {
        v.as_u64().ok_or_else(invalid)
    }
}
fn finish(p: Protocol, s: &str, tools: bool) -> Result<String, String> {
    Ok(match (p, s) {
        (Protocol::Anthropic, "end_turn" | "stop_sequence")
        | (Protocol::OpenAiChat, "stop")
        | (Protocol::OpenAiResponses, "completed")
        | (Protocol::Gemini, "STOP") => {
            if tools {
                "tool"
            } else {
                "stop"
            }
        }
        (Protocol::Anthropic, "tool_use") | (Protocol::OpenAiChat, "tool_calls") => "tool",
        (Protocol::Anthropic, "max_tokens")
        | (Protocol::OpenAiChat, "length")
        | (Protocol::OpenAiResponses, "max_output_tokens")
        | (Protocol::Gemini, "MAX_TOKENS") => "length",
        (Protocol::Anthropic, "refusal")
        | (Protocol::OpenAiChat, "content_filter")
        | (Protocol::OpenAiResponses, "content_filter")
        | (
            Protocol::Gemini,
            "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII",
        ) => "filter",
        _ => return Err(unsupported()),
    }
    .into())
}
fn decode_response(p: Protocol, v: &Value) -> Result<Output, String> {
    if v.get("error").is_some_and(|x| !x.is_null()) {
        return Err(invalid());
    }
    let mut n = Output::default();
    let mut names = HashMap::new();
    let (usage, reason) = match p {
        Protocol::Anthropic => {
            n.id = string(&v["id"])?;
            n.parts = decode_parts(p, &v["content"], &mut names)?;
            (&v["usage"], string(&v["stop_reason"])?)
        }
        Protocol::OpenAiChat => {
            n.id = string(&v["id"])?;
            let choices = array(&v["choices"])?;
            if choices.len() != 1 {
                return Err(unsupported());
            }
            let c = &choices[0];
            fields(
                &c["message"],
                &["role", "content", "tool_calls", "refusal", "annotations"],
            )?;
            if c["message"].get("refusal").is_some_and(|x| !x.is_null())
                || c["message"]
                    .get("annotations")
                    .is_some_and(|x| x.as_array().is_none_or(|a| !a.is_empty()))
            {
                return Err(unsupported());
            }
            n.parts = decode_parts(p, &c["message"]["content"], &mut names)?;
            if let Some(calls) = c["message"].get("tool_calls") {
                for c in array(calls)? {
                    if c["type"] != "function" {
                        return Err(unsupported());
                    }
                    n.parts.push(Part::Call {
                        id: string(&c["id"])?,
                        name: string(&c["function"]["name"])?,
                        args: arguments(&c["function"]["arguments"])?,
                    });
                }
            }
            (&v["usage"], string(&c["finish_reason"])?)
        }
        Protocol::OpenAiResponses => {
            n.id = string(&v["id"])?;
            for item in array(&v["output"])? {
                match item["type"].as_str() {
                    Some("message") => {
                        n.parts
                            .extend(decode_parts(p, &item["content"], &mut names)?)
                    }
                    Some("function_call") => n.parts.push(Part::Call {
                        id: string(&item["call_id"])?,
                        name: string(&item["name"])?,
                        args: arguments(&item["arguments"])?,
                    }),
                    _ => return Err(unsupported()),
                }
            }
            let reason = if v["status"] == "incomplete" {
                string(&v["incomplete_details"]["reason"])?
            } else {
                string(&v["status"])?
            };
            (&v["usage"], reason)
        }
        Protocol::Gemini => {
            n.id = v
                .get("responseId")
                .map(string)
                .transpose()?
                .unwrap_or_else(|| "lomi-gateway-response".into());
            let choices = array(&v["candidates"])?;
            if choices.len() != 1 {
                return Err(unsupported());
            }
            let c = &choices[0];
            n.parts = decode_parts(p, &c["content"]["parts"], &mut names)?;
            (&v["usageMetadata"], string(&c["finishReason"])?)
        }
    };
    n.usage_present = !usage.is_null();
    if n.usage_present && !usage.is_object() {
        return Err(invalid());
    }
    if n.usage_present {
        let (input_key, output_key) = match p {
            Protocol::Anthropic | Protocol::OpenAiResponses => ("input_tokens", "output_tokens"),
            Protocol::OpenAiChat => ("prompt_tokens", "completion_tokens"),
            Protocol::Gemini => ("promptTokenCount", "candidatesTokenCount"),
        };
        if usage[input_key].as_u64().is_none() || usage[output_key].as_u64().is_none() {
            return Err(invalid());
        }
    }
    n.finish = finish(
        p,
        &reason,
        n.parts.iter().any(|p| matches!(p, Part::Call { .. })),
    )?;
    match p {
        Protocol::Anthropic => {
            n.input = token(&usage["input_tokens"])?;
            n.output = token(&usage["output_tokens"])?;
            n.cached = token(&usage["cache_read_input_tokens"])?;
            n.cache_write = token(&usage["cache_creation_input_tokens"])?;
        }
        Protocol::OpenAiChat => {
            n.input = token(&usage["prompt_tokens"])?;
            n.output = token(&usage["completion_tokens"])?;
            n.cached = token(&usage["prompt_tokens_details"]["cached_tokens"])?;
            n.reasoning = token(&usage["completion_tokens_details"]["reasoning_tokens"])?;
        }
        Protocol::OpenAiResponses => {
            n.input = token(&usage["input_tokens"])?;
            n.output = token(&usage["output_tokens"])?;
            n.cached = token(&usage["input_tokens_details"]["cached_tokens"])?;
            n.reasoning = token(&usage["output_tokens_details"]["reasoning_tokens"])?;
        }
        Protocol::Gemini => {
            n.input = token(&usage["promptTokenCount"])?;
            n.output = token(&usage["candidatesTokenCount"])?;
            n.cached = token(&usage["cachedContentTokenCount"])?;
            n.reasoning = token(&usage["thoughtsTokenCount"])?;
        }
    }
    if p == Protocol::Anthropic {
        n.input = n
            .input
            .checked_add(n.cached)
            .and_then(|x| x.checked_add(n.cache_write))
            .ok_or_else(invalid)?;
    }
    if p == Protocol::Gemini {
        n.output = n.output.checked_add(n.reasoning).ok_or_else(invalid)?;
    }
    Ok(n)
}
fn encode_response(p: Protocol, n: &Output, model: &str) -> Result<Value, String> {
    if n.cache_write > 0 && p != Protocol::Anthropic || n.reasoning > 0 && p == Protocol::Anthropic
    {
        return Err(unsupported());
    }
    if p == Protocol::OpenAiChat
        && n.parts
            .iter()
            .any(|x| !matches!(x, Part::Text(_) | Part::Call { .. }))
    {
        return Err(unsupported());
    }
    let total = n.input.checked_add(n.output).ok_or_else(invalid)?;
    let input = if p == Protocol::Anthropic {
        n.input
            .checked_sub(n.cached)
            .and_then(|x| x.checked_sub(n.cache_write))
            .ok_or_else(invalid)?
    } else {
        n.input
    };
    let mut value = match p {
        Protocol::Anthropic => {
            json!({"id":n.id,"type":"message","role":"assistant","model":model,"content":n.parts.iter().map(|x|encode_part(p,x,true)).collect::<Result<Vec<_>,_>>()?,"stop_reason":match n.finish.as_str(){"tool"=>"tool_use","length"=>"max_tokens","filter"=>"refusal",_=>"end_turn"},"stop_sequence":null,"usage":{"input_tokens":input,"output_tokens":n.output,"cache_read_input_tokens":n.cached,"cache_creation_input_tokens":n.cache_write}})
        }
        Protocol::OpenAiChat => {
            let mut message = json!({"role":"assistant","content":n.parts.iter().filter_map(|x|if let Part::Text(t)=x{Some(t.as_str())}else{None}).collect::<String>()});
            let calls = n
                .parts
                .iter()
                .filter(|x| matches!(x, Part::Call { .. }))
                .map(|x| encode_part(p, x, true))
                .collect::<Result<Vec<_>, _>>()?;
            if !calls.is_empty() {
                message["tool_calls"] = json!(calls);
            }
            json!({"id":n.id,"object":"chat.completion","created":0,"model":model,"choices":[{"index":0,"message":message,"finish_reason":match n.finish.as_str(){"tool"=>"tool_calls","length"=>"length","filter"=>"content_filter",_=>"stop"}}],"usage":{"prompt_tokens":n.input,"completion_tokens":n.output,"total_tokens":total,"prompt_tokens_details":{"cached_tokens":n.cached},"completion_tokens_details":{"reasoning_tokens":n.reasoning}}})
        }
        Protocol::OpenAiResponses => {
            let mut output = Vec::new();
            let mut text = Vec::new();
            for (i, part) in n.parts.iter().enumerate() {
                if matches!(part, Part::Call { .. }) {
                    if !text.is_empty() {
                        output.push(json!({"type":"message","id":format!("msg_{}_{}",n.id,i),"status":"completed","role":"assistant","content":std::mem::take(&mut text)}));
                    }
                    let mut call = encode_part(p, part, true)?;
                    call["id"] = json!(format!("fc_{}_{}", n.id, i));
                    call["status"] = json!("completed");
                    output.push(call);
                } else {
                    text.push(encode_part(p, part, true)?);
                }
            }
            if !text.is_empty() {
                output.push(json!({"type":"message","id":format!("msg_{}",n.id),"status":"completed","role":"assistant","content":text}));
            }
            json!({"id":n.id,"object":"response","created_at":0,"model":model,"status":if n.finish=="length"||n.finish=="filter"{"incomplete"}else{"completed"},"error":null,"incomplete_details":if n.finish=="length"{json!({"reason":"max_output_tokens"})}else if n.finish=="filter"{json!({"reason":"content_filter"})}else{Value::Null},"output":output,"usage":{"input_tokens":n.input,"output_tokens":n.output,"total_tokens":total,"input_tokens_details":{"cached_tokens":n.cached},"output_tokens_details":{"reasoning_tokens":n.reasoning}}})
        }
        Protocol::Gemini => {
            json!({"responseId":n.id,"modelVersion":model,"candidates":[{"index":0,"content":{"role":"model","parts":n.parts.iter().map(|x|encode_part(p,x,true)).collect::<Result<Vec<_>,_>>()?},"finishReason":match n.finish.as_str(){"length"=>"MAX_TOKENS","filter"=>"SAFETY",_=>"STOP"}}],"usageMetadata":{"promptTokenCount":n.input,"candidatesTokenCount":n.output.checked_sub(n.reasoning).ok_or_else(invalid)?,"totalTokenCount":total,"cachedContentTokenCount":n.cached,"thoughtsTokenCount":n.reasoning}})
        }
    };
    if !n.usage_present {
        match p {
            Protocol::Anthropic => return Err(unsupported()),
            Protocol::OpenAiResponses => value["usage"] = Value::Null,
            Protocol::OpenAiChat => {
                value.as_object_mut().ok_or_else(invalid)?.remove("usage");
            }
            Protocol::Gemini => {
                value
                    .as_object_mut()
                    .ok_or_else(invalid)?
                    .remove("usageMetadata");
            }
        }
    }
    Ok(value)
}
