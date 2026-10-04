use super::*;
use std::collections::BTreeMap;

fn frames(body: &[u8]) -> Result<Vec<(String, Value)>, String> {
    if body.len() > RESPONSE_LIMIT {
        return Err(invalid());
    }
    let text = std::str::from_utf8(body)
        .map_err(|_| invalid())?
        .replace("\r\n", "\n");
    if !text.ends_with("\n\n") {
        return Err(invalid());
    }
    let mut frames = Vec::new();
    for frame in text.split("\n\n") {
        let mut name = String::new();
        let mut data = Vec::new();
        for line in frame.lines() {
            if line.starts_with(':') || line.is_empty() {
                continue;
            }
            if let Some(s) = line.strip_prefix("event:") {
                if !name.is_empty() {
                    return Err(invalid());
                }
                name = s.trim_start().into();
            } else if let Some(s) = line.strip_prefix("data:") {
                data.push(s.strip_prefix(' ').unwrap_or(s));
            } else if !line.starts_with("id:") && !line.starts_with("retry:") {
                return Err(invalid());
            }
        }
        if data.is_empty() {
            continue;
        }
        let data = data.join("\n");
        let value = if data == "[DONE]" {
            json!("[DONE]")
        } else {
            parse(data.as_bytes(), RESPONSE_LIMIT)?
        };
        if value.get("error").is_some_and(|x| !x.is_null())
            || name == "error"
            || value["type"] == "error"
        {
            return Err(invalid());
        }
        if !name.is_empty() && value.get("type").is_some_and(|t| t != name.as_str()) {
            return Err(invalid());
        }
        frames.push((name, value));
    }
    if frames.is_empty() {
        return Err(invalid());
    }
    Ok(frames)
}
pub(super) fn convert(
    native: Protocol,
    upstream: Protocol,
    body: &[u8],
    model: &str,
) -> Result<Vec<u8>, String> {
    let frames = frames(body)?;
    let output = match upstream {
        Protocol::Anthropic => anthropic(frames)?,
        Protocol::OpenAiChat => chat(frames)?,
        Protocol::OpenAiResponses => responses(frames)?,
        Protocol::Gemini => gemini(frames)?,
    };
    emit(native, &output, model)
}
fn anthropic(frames: Vec<(String, Value)>) -> Result<Output, String> {
    let mut message: Option<Value> = None;
    let mut blocks: BTreeMap<usize, (Value, String, bool)> = BTreeMap::new();
    let mut terminal = false;
    let mut finish = false;
    for (_, v) in frames {
        if terminal {
            return Err(invalid());
        }
        match v["type"].as_str() {
            Some("ping") => {}
            Some("message_start") => {
                if message.is_some() {
                    return Err(invalid());
                }
                let m = v["message"].clone();
                if !array(&m["content"])?.is_empty() || !m["usage"].is_object() {
                    return Err(invalid());
                }
                message = Some(m);
            }
            Some("content_block_start") => {
                if message.is_none() || finish {
                    return Err(invalid());
                }
                let i = v["index"].as_u64().ok_or_else(invalid)? as usize;
                if i != blocks.len() {
                    return Err(invalid());
                }
                let b = v["content_block"].clone();
                if !matches!(b["type"].as_str(), Some("text" | "tool_use")) {
                    return Err(unsupported());
                }
                blocks.insert(i, (b, String::new(), false));
            }
            Some("content_block_delta") => {
                let i = v["index"].as_u64().ok_or_else(invalid)? as usize;
                let (b, args, closed) = blocks.get_mut(&i).ok_or_else(invalid)?;
                if *closed {
                    return Err(invalid());
                }
                match v["delta"]["type"].as_str() {
                    Some("text_delta") if b["type"] == "text" => {
                        let mut text = string(&b["text"])?;
                        text.push_str(&string(&v["delta"]["text"])?);
                        b["text"] = json!(text);
                    }
                    Some("input_json_delta") if b["type"] == "tool_use" => {
                        args.push_str(&string(&v["delta"]["partial_json"])?)
                    }
                    _ => return Err(unsupported()),
                }
            }
            Some("content_block_stop") => {
                let i = v["index"].as_u64().ok_or_else(invalid)? as usize;
                let (b, args, closed) = blocks.get_mut(&i).ok_or_else(invalid)?;
                if *closed {
                    return Err(invalid());
                }
                if b["type"] == "tool_use" && !args.is_empty() {
                    b["input"] = arguments(&json!(args))?;
                }
                *closed = true;
            }
            Some("message_delta") => {
                if blocks.values().any(|(_, _, closed)| !*closed) {
                    return Err(invalid());
                }
                let m = message.as_mut().ok_or_else(invalid)?;
                if !v["delta"]["stop_reason"].is_null() {
                    m["stop_reason"] = v["delta"]["stop_reason"].clone();
                    finish = true;
                }
                if let Some(usage) = v["usage"].as_object() {
                    for (k, value) in usage {
                        m["usage"][k] = value.clone();
                    }
                }
            }
            Some("message_stop") => {
                if !finish || message.is_none() || blocks.values().any(|(_, _, closed)| !*closed) {
                    return Err(invalid());
                }
                terminal = true;
            }
            _ => return Err(unsupported()),
        }
    }
    if !terminal {
        return Err(invalid());
    }
    let mut m = message.ok_or_else(invalid)?;
    m["content"] = json!(blocks.into_values().map(|(b, _, _)| b).collect::<Vec<_>>());
    decode_response(Protocol::Anthropic, &m)
}
fn chat(frames: Vec<(String, Value)>) -> Result<Output, String> {
    let mut id = None;
    let mut text = String::new();
    let mut calls: BTreeMap<usize, Value> = BTreeMap::new();
    let mut reason = None;
    let mut usage = Value::Null;
    let mut terminal = false;
    for (_, v) in frames {
        if terminal {
            return Err(invalid());
        }
        if v == "[DONE]" {
            if reason.is_none() {
                return Err(invalid());
            }
            terminal = true;
            continue;
        }
        if v["object"] != "chat.completion.chunk" {
            return Err(invalid());
        }
        let current = string(&v["id"])?;
        if id.as_ref().is_some_and(|id| id != &current) {
            return Err(invalid());
        }
        id = Some(current);
        if !v["usage"].is_null() {
            usage = v["usage"].clone();
        }
        let choices = array(&v["choices"])?;
        if choices.is_empty() {
            continue;
        }
        if choices.len() != 1 || choices[0]["index"] != 0 || reason.is_some() {
            return Err(invalid());
        }
        let c = &choices[0];
        let d = &c["delta"];
        fields(d, &["role", "content", "tool_calls"])?;
        if let Some(s) = d.get("content").filter(|x| !x.is_null()) {
            text.push_str(&string(s)?);
        }
        if let Some(tool_calls) = d.get("tool_calls") {
            for call in array(tool_calls)? {
                fields(call, &["index", "id", "type", "function"])?;
                let i = call["index"].as_u64().ok_or_else(invalid)? as usize;
                let entry = calls.entry(i).or_insert_with(
                    || json!({"id":"","type":"function","function":{"name":"","arguments":""}}),
                );
                if let Some(t) = call.get("type") {
                    if t != "function" {
                        return Err(unsupported());
                    }
                }
                for key in ["id", "name", "arguments"] {
                    let fragment = if key == "id" {
                        call.get(key)
                    } else {
                        call["function"].get(key)
                    };
                    if let Some(f) = fragment {
                        let dst = if key == "id" {
                            &mut entry["id"]
                        } else {
                            &mut entry["function"][key]
                        };
                        let mut s = string(dst)?;
                        s.push_str(&string(f)?);
                        *dst = json!(s);
                    }
                }
            }
        }
        if !c["finish_reason"].is_null() {
            reason = Some(string(&c["finish_reason"])?);
        }
    }
    if !terminal {
        return Err(invalid());
    }
    for (i, call) in calls.values().enumerate() {
        if !calls.contains_key(&i)
            || string(&call["id"])?.is_empty()
            || string(&call["function"]["name"])?.is_empty()
        {
            return Err(invalid());
        }
        arguments(&call["function"]["arguments"])?;
    }
    let mut message = json!({"content":text});
    if !calls.is_empty() {
        message["tool_calls"] = json!(calls.into_values().collect::<Vec<_>>());
    }
    decode_response(
        Protocol::OpenAiChat,
        &json!({"id":id.ok_or_else(invalid)?,"choices":[{"message":message,"finish_reason":reason.ok_or_else(invalid)?}],"usage":usage}),
    )
}
fn responses(frames: Vec<(String, Value)>) -> Result<Output, String> {
    let mut created = false;
    let mut terminal = None;
    let mut last_sequence = None;
    let mut id = None;
    let mut items: BTreeMap<usize, (Value, bool)> = BTreeMap::new();
    let mut parts: BTreeMap<(usize, usize), bool> = BTreeMap::new();
    let mut args_done = BTreeMap::new();
    for (_, v) in frames {
        if terminal.is_some() {
            return Err(invalid());
        }
        let seq = v["sequence_number"].as_u64().ok_or_else(invalid)?;
        if last_sequence.is_some_and(|last| seq <= last) {
            return Err(invalid());
        }
        last_sequence = Some(seq);
        let kind = v["type"].as_str().ok_or_else(invalid)?;
        if kind == "response.created" {
            if created {
                return Err(invalid());
            }
            created = true;
            id = Some(string(&v["response"]["id"])?);
            continue;
        }
        if !created {
            return Err(invalid());
        }
        if matches!(kind, "response.completed" | "response.incomplete") {
            if id.as_ref() != Some(&string(&v["response"]["id"])?)
                || items.values().any(|(_, closed)| !*closed)
                || parts.values().any(|closed| !*closed)
                || (kind == "response.completed" && v["response"]["status"] != "completed")
                || (kind == "response.incomplete" && v["response"]["status"] != "incomplete")
            {
                return Err(invalid());
            }
            let output = array(&v["response"]["output"])?;
            if output.len() != items.len()
                || output
                    .iter()
                    .enumerate()
                    .any(|(i, item)| items.get(&i).is_none_or(|(seen, _)| seen != item))
            {
                return Err(invalid());
            }
            terminal = Some(decode_response(Protocol::OpenAiResponses, &v["response"])?);
            continue;
        }
        if kind == "response.in_progress" {
            continue;
        }
        let i = v["output_index"].as_u64().ok_or_else(invalid)? as usize;
        if kind == "response.output_item.added" {
            if i != items.len()
                || !matches!(
                    v["item"]["type"].as_str(),
                    Some("message" | "function_call")
                )
            {
                return Err(unsupported());
            }
            let item = v["item"].clone();
            string(&item["id"])?;
            if item["type"] == "message" && !array(&item["content"])?.is_empty() {
                return Err(invalid());
            }
            if item["type"] == "function_call" && item["arguments"] != "" {
                return Err(invalid());
            }
            items.insert(i, (item, false));
            continue;
        }
        let (item, closed) = items.get_mut(&i).ok_or_else(invalid)?;
        if *closed || v.get("item_id").is_some_and(|value| value != &item["id"]) {
            return Err(invalid());
        }
        match kind {
            "response.function_call_arguments.delta" => {
                if item["type"] != "function_call" || args_done.contains_key(&i) {
                    return Err(invalid());
                }
                let mut args = string(&item["arguments"])?;
                args.push_str(&string(&v["delta"])?);
                item["arguments"] = json!(args);
            }
            "response.function_call_arguments.done" => {
                if item["type"] != "function_call"
                    || args_done.insert(i, true).is_some()
                    || item["arguments"] != v["arguments"]
                {
                    return Err(invalid());
                }
                arguments(&item["arguments"])?;
            }
            "response.content_part.added" => {
                let j = v["content_index"].as_u64().ok_or_else(invalid)? as usize;
                if item["type"] != "message"
                    || v["part"]["type"] != "output_text"
                    || j != array(&item["content"])?.len()
                    || v["part"]["text"] != ""
                {
                    return Err(unsupported());
                }
                item["content"]
                    .as_array_mut()
                    .ok_or_else(invalid)?
                    .push(v["part"].clone());
                parts.insert((i, j), false);
            }
            "response.output_text.delta"
            | "response.output_text.done"
            | "response.content_part.done" => {
                let j = v["content_index"].as_u64().ok_or_else(invalid)? as usize;
                let closed = parts.get_mut(&(i, j)).ok_or_else(invalid)?;
                if *closed {
                    return Err(invalid());
                }
                let part = item["content"]
                    .as_array_mut()
                    .and_then(|a| a.get_mut(j))
                    .ok_or_else(invalid)?;
                if kind == "response.output_text.delta" {
                    let mut text = string(&part["text"])?;
                    text.push_str(&string(&v["delta"])?);
                    part["text"] = json!(text);
                } else if kind == "response.output_text.done" {
                    if part["text"] != v["text"] {
                        return Err(invalid());
                    }
                } else {
                    if part != &v["part"] {
                        return Err(invalid());
                    }
                    *closed = true;
                }
            }
            "response.output_item.done" => {
                if item["type"] == "function_call" && !args_done.contains_key(&i)
                    || parts
                        .iter()
                        .any(|((index, _), closed)| *index == i && !*closed)
                {
                    return Err(invalid());
                }
                item["status"] = v["item"]["status"].clone();
                if item != &v["item"] {
                    return Err(invalid());
                }
                *closed = true;
            }
            _ => return Err(unsupported()),
        }
    }
    terminal.ok_or_else(invalid)
}
fn gemini(frames: Vec<(String, Value)>) -> Result<Output, String> {
    let mut parts = Vec::new();
    let mut usage = Value::Null;
    let mut reason = None;
    let mut id = None;
    for (_, v) in frames {
        if reason.is_some() {
            return Err(invalid());
        }
        if let Some(current) = v.get("responseId") {
            let current = string(current)?;
            if id.as_ref().is_some_and(|id| id != &current) {
                return Err(invalid());
            }
            id = Some(current);
        }
        if !v["usageMetadata"].is_null() {
            usage = v["usageMetadata"].clone();
        }
        let choices = array(&v["candidates"])?;
        if choices.len() != 1 {
            return Err(invalid());
        }
        let c = &choices[0];
        if let Some(p) = c["content"].get("parts") {
            parts.extend(array(p)?.iter().cloned());
        }
        if !c["finishReason"].is_null() {
            reason = Some(string(&c["finishReason"])?);
        }
    }
    decode_response(
        Protocol::Gemini,
        &json!({"responseId":id.unwrap_or_else(||"lomi-gateway-response".into()),"candidates":[{"content":{"parts":parts},"finishReason":reason.ok_or_else(invalid)?}],"usageMetadata":usage}),
    )
}
fn event(out: &mut Vec<u8>, name: Option<&str>, v: &Value) -> Result<(), String> {
    if let Some(name) = name {
        out.extend_from_slice(b"event: ");
        out.extend_from_slice(name.as_bytes());
        out.push(b'\n');
    }
    out.extend_from_slice(b"data: ");
    out.extend_from_slice(&encode(v)?);
    out.extend_from_slice(b"\n\n");
    if out.len() > RESPONSE_LIMIT {
        return Err(invalid());
    }
    Ok(())
}
fn response_event(
    out: &mut Vec<u8>,
    seq: &mut u64,
    name: &str,
    mut v: Value,
) -> Result<(), String> {
    v["type"] = json!(name);
    v["sequence_number"] = json!(*seq);
    *seq += 1;
    event(out, Some(name), &v)
}
pub(super) fn emit(p: Protocol, n: &Output, model: &str) -> Result<Vec<u8>, String> {
    let final_value = encode_response(p, n, model)?;
    let mut out = Vec::new();
    match p {
        Protocol::Gemini => event(&mut out, None, &final_value)?,
        Protocol::OpenAiChat => {
            let chunk = |delta: Value, finish: Value| json!({"id":n.id,"object":"chat.completion.chunk","created":0,"model":model,"choices":[{"index":0,"delta":delta,"finish_reason":finish}]});
            event(
                &mut out,
                None,
                &chunk(json!({"role":"assistant","content":""}), Value::Null),
            )?;
            let mut index = 0;
            for part in &n.parts {
                let d = match part {
                    Part::Text(text) => json!({"content":text}),
                    Part::Call { .. } => {
                        let mut call = encode_part(p, part, true)?;
                        call["index"] = json!(index);
                        index += 1;
                        json!({"tool_calls":[call]})
                    }
                    _ => return Err(unsupported()),
                };
                event(&mut out, None, &chunk(d, Value::Null))?;
            }
            event(
                &mut out,
                None,
                &chunk(
                    json!({}),
                    final_value["choices"][0]["finish_reason"].clone(),
                ),
            )?;
            if n.usage_present {
                event(
                    &mut out,
                    None,
                    &json!({"id":n.id,"object":"chat.completion.chunk","created":0,"model":model,"choices":[],"usage":final_value["usage"]}),
                )?;
            }
            out.extend_from_slice(b"data: [DONE]\n\n");
        }
        Protocol::Anthropic => {
            let mut start = final_value.clone();
            start["content"] = json!([]);
            start["stop_reason"] = Value::Null;
            start["usage"]["output_tokens"] = json!(0);
            event(
                &mut out,
                Some("message_start"),
                &json!({"type":"message_start","message":start}),
            )?;
            for (i, part) in n.parts.iter().enumerate() {
                let mut block = encode_part(p, part, true)?;
                let delta = match part {
                    Part::Text(text) => {
                        block["text"] = json!("");
                        json!({"type":"text_delta","text":text})
                    }
                    Part::Call { args, .. } => {
                        block["input"] = json!({});
                        json!({"type":"input_json_delta","partial_json":serde_json::to_string(args).map_err(|_|invalid())?})
                    }
                    _ => return Err(unsupported()),
                };
                event(
                    &mut out,
                    Some("content_block_start"),
                    &json!({"type":"content_block_start","index":i,"content_block":block}),
                )?;
                event(
                    &mut out,
                    Some("content_block_delta"),
                    &json!({"type":"content_block_delta","index":i,"delta":delta}),
                )?;
                event(
                    &mut out,
                    Some("content_block_stop"),
                    &json!({"type":"content_block_stop","index":i}),
                )?;
            }
            event(
                &mut out,
                Some("message_delta"),
                &json!({"type":"message_delta","delta":{"stop_reason":final_value["stop_reason"],"stop_sequence":null},"usage":final_value["usage"]}),
            )?;
            event(
                &mut out,
                Some("message_stop"),
                &json!({"type":"message_stop"}),
            )?;
        }
        Protocol::OpenAiResponses => {
            let mut seq = 0;
            let mut start = final_value.clone();
            start["status"] = json!("in_progress");
            start["output"] = json!([]);
            start["usage"] = Value::Null;
            start["incomplete_details"] = Value::Null;
            response_event(
                &mut out,
                &mut seq,
                "response.created",
                json!({"response":start}),
            )?;
            response_event(
                &mut out,
                &mut seq,
                "response.in_progress",
                json!({"response":start}),
            )?;
            for (i, item) in array(&final_value["output"])?.iter().enumerate() {
                let mut added = item.clone();
                added["status"] = json!("in_progress");
                if item["type"] == "message" {
                    added["content"] = json!([]);
                } else {
                    added["arguments"] = json!("");
                }
                response_event(
                    &mut out,
                    &mut seq,
                    "response.output_item.added",
                    json!({"output_index":i,"item":added}),
                )?;
                if item["type"] == "function_call" {
                    response_event(
                        &mut out,
                        &mut seq,
                        "response.function_call_arguments.delta",
                        json!({"item_id":item["id"],"output_index":i,"delta":item["arguments"]}),
                    )?;
                    response_event(
                        &mut out,
                        &mut seq,
                        "response.function_call_arguments.done",
                        json!({"item_id":item["id"],"output_index":i,"arguments":item["arguments"]}),
                    )?;
                } else {
                    for (j, part) in array(&item["content"])?.iter().enumerate() {
                        if part["type"] != "output_text" {
                            return Err(unsupported());
                        }
                        let mut added = part.clone();
                        added["text"] = json!("");
                        response_event(
                            &mut out,
                            &mut seq,
                            "response.content_part.added",
                            json!({"item_id":item["id"],"output_index":i,"content_index":j,"part":added}),
                        )?;
                        response_event(
                            &mut out,
                            &mut seq,
                            "response.output_text.delta",
                            json!({"item_id":item["id"],"output_index":i,"content_index":j,"delta":part["text"]}),
                        )?;
                        response_event(
                            &mut out,
                            &mut seq,
                            "response.output_text.done",
                            json!({"item_id":item["id"],"output_index":i,"content_index":j,"text":part["text"]}),
                        )?;
                        response_event(
                            &mut out,
                            &mut seq,
                            "response.content_part.done",
                            json!({"item_id":item["id"],"output_index":i,"content_index":j,"part":part}),
                        )?;
                    }
                }
                response_event(
                    &mut out,
                    &mut seq,
                    "response.output_item.done",
                    json!({"output_index":i,"item":item}),
                )?;
            }
            response_event(
                &mut out,
                &mut seq,
                if final_value["status"] == "incomplete" {
                    "response.incomplete"
                } else {
                    "response.completed"
                },
                json!({"response":final_value}),
            )?;
        }
    }
    if out.len() > RESPONSE_LIMIT {
        return Err(invalid());
    }
    Ok(out)
}
