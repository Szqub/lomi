use super::*;
const PROTOCOLS: [Protocol; 4] = [
    Protocol::Anthropic,
    Protocol::OpenAiChat,
    Protocol::OpenAiResponses,
    Protocol::Gemini,
];
fn conversation() -> Input {
    Input {
        system: vec!["Write safe Rust.".into()],
        messages: vec![
            Message {
                role: "user".into(),
                parts: vec![Part::Text("Read lib.rs".into())],
            },
            Message {
                role: "assistant".into(),
                parts: vec![
                    Part::Text("Reading.".into()),
                    Part::Call {
                        id: "call_42".into(),
                        name: "read_file".into(),
                        args: json!({"path":"lib.rs","line":2}),
                    },
                ],
            },
            Message {
                role: "user".into(),
                parts: vec![Part::Result {
                    id: "call_42".into(),
                    name: "read_file".into(),
                    text: "fn main() {}".into(),
                }],
            },
        ],
        tools: vec![
            json!({"name":"read_file","description":"Read a file","parameters":{"type":"object","properties":{"path":{"type":"string"}}},"strict":false}),
        ],
        max: Some(json!(2048)),
        temperature: Some(json!(0.2)),
        top_p: Some(json!(0.9)),
        stream: true,
        ..Input::default()
    }
}
#[test]
fn all_cross_pairs_preserve_function_conversation_and_generation_controls() {
    for native in PROTOCOLS {
        for upstream in PROTOCOLS {
            if native == upstream {
                continue;
            }
            let source =
                encode(&encode_request(native, conversation(), "source-model").unwrap()).unwrap();
            let converted = request(
                native,
                upstream,
                if native == Protocol::Gemini {
                    "/v1beta/models/source:streamGenerateContent?alt=sse"
                } else {
                    "/v1/messages"
                },
                &source,
                "target-model",
            )
            .unwrap();
            assert!(converted.stream);
            let decoded =
                decode_request(upstream, &parse(&converted.body, REQUEST_LIMIT).unwrap()).unwrap();
            assert_eq!(decoded.system, vec!["Write safe Rust."]);
            assert_eq!(decoded.max, Some(json!(2048)));
            assert_eq!(decoded.temperature, Some(json!(0.2)));
            assert_eq!(decoded.top_p, Some(json!(0.9)));
            let parts = decoded
                .messages
                .iter()
                .flat_map(|m| m.parts.iter())
                .collect::<Vec<_>>();
            assert!(parts.iter().any(|p|matches!(p,Part::Call{id,name,args} if id=="call_42"&&name=="read_file"&&args==&json!({"path":"lib.rs","line":2}))));
            assert!(parts.iter().any(
                |p| matches!(p,Part::Result{id,text,..} if id=="call_42"&&text=="fn main() {}")
            ));
            if upstream == Protocol::Gemini {
                assert!(converted.path.ends_with(":streamGenerateContent?alt=sse"));
            }
        }
    }
}
fn output() -> Output {
    Output {
        id: "response_7".into(),
        parts: vec![
            Part::Text("Reading.".into()),
            Part::Call {
                id: "call_42".into(),
                name: "read_file".into(),
                args: json!({"path":"lib.rs"}),
            },
        ],
        finish: "tool".into(),
        input: 15,
        output: 8,
        cached: 3,
        usage_present: true,
        ..Output::default()
    }
}
#[test]
fn all_cross_pairs_preserve_response_text_tool_id_usage_and_sse_lifecycle() {
    for native in PROTOCOLS {
        for upstream in PROTOCOLS {
            if native == upstream {
                continue;
            }
            let body =
                encode(&encode_response(upstream, &output(), "upstream-model").unwrap()).unwrap();
            let converted = response(native, upstream, &body, "native-model").unwrap();
            let decoded =
                decode_response(native, &parse(&converted, RESPONSE_LIMIT).unwrap()).unwrap();
            assert_eq!(decoded.id, "response_7");
            assert_eq!(decoded.finish, "tool");
            assert_eq!((decoded.input, decoded.output, decoded.cached), (15, 8, 3));
            assert!(
                matches!(&decoded.parts[1],Part::Call{id,name,args} if id=="call_42"&&name=="read_file"&&args==&json!({"path":"lib.rs"}))
            );
            let source_stream = stream::emit(upstream, &output(), "upstream-model").unwrap();
            let target_stream =
                stream_response(native, upstream, &source_stream, "native-model").unwrap();
            let replayed =
                stream_response(upstream, native, &target_stream, "upstream-model").unwrap();
            assert_eq!(source_stream, replayed);
            // Gemini can emit the complete response in one terminal SSE frame.
            // Removing that frame must produce an empty, rejected stream too.
            assert!(source_stream.ends_with(b"\n\n"));
            let last_boundary = source_stream[..source_stream.len() - 2]
                .windows(2)
                .rposition(|w| w == b"\n\n")
                .map(|position| position + 2)
                .unwrap_or(0);
            assert!(stream_response(
                native,
                upstream,
                &source_stream[..last_boundary],
                "native-model"
            )
            .is_err());
        }
    }
}
#[test]
fn duplicate_members_opaque_references_and_hosted_tools_are_rejected() {
    for body in [
        r#"{"model":"x","input":"hello","model":"y"}"#,
        r#"{"model":"x","input":"hello","previous_response_id":"private"}"#,
        r#"{"model":"x","input":[{"type":"item_reference","id":"private"}]}"#,
        r#"{"model":"x","input":[{"type":"reasoning","encrypted_content":"private"}]}"#,
        r#"{"model":"x","input":"hello","tools":[{"type":"web_search"}]}"#,
        r#"{"model":"x","input":"hello","tools":[{"type":"custom","name":"x","format":{"type":"grammar"}}]}"#,
    ] {
        let error = request(
            Protocol::OpenAiResponses,
            Protocol::Anthropic,
            "/v1/responses",
            body.as_bytes(),
            "target",
        )
        .err()
        .unwrap();
        assert!(!error.contains("private"));
    }
    assert!(parse(br#"{"outer":{"same":1,"same":2}}"#, REQUEST_LIMIT).is_err());
}
#[test]
fn cross_protocol_counting_never_constructs_generation_request() {
    for path in ["/v1/messages/count_tokens", "/v1beta/models/x:countTokens"] {
        assert!(
            request(Protocol::Anthropic, Protocol::OpenAiChat, path, b"{}", "x")
                .err()
                .unwrap()
                .contains("token counting")
        );
    }
}
#[test]
fn inline_images_survive_cross_protocol_requests() {
    for native in PROTOCOLS {
        for upstream in PROTOCOLS {
            if native == upstream {
                continue;
            }
            let mut n = conversation();
            n.messages[0].parts.push(Part::Image {
                mime: "image/png".into(),
                data: "aGVsbG8=".into(),
            });
            let source = encode(&encode_request(native, n, "source").unwrap()).unwrap();
            let converted = request(native, upstream, "/v1/messages", &source, "target").unwrap();
            let n =
                decode_request(upstream, &parse(&converted.body, REQUEST_LIMIT).unwrap()).unwrap();
            assert!(
                matches!(&n.messages[0].parts[1],Part::Image{mime,data} if mime=="image/png"&&data=="aGVsbG8=")
            );
        }
    }
}
#[test]
fn errors_after_terminal_events_and_malformed_tool_arguments_are_rejected() {
    let mut s = stream::emit(Protocol::OpenAiChat, &output(), "x").unwrap();
    s.extend_from_slice(b"data: {\"error\":{\"message\":\"private\"}}\n\n");
    assert!(stream_response(Protocol::Anthropic, Protocol::OpenAiChat, &s, "x").is_err());
    let body = json!({"id":"x","choices":[{"message":{"content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{\"x\":1,\"x\":2}"}}]},"finish_reason":"tool_calls"}]});
    assert!(response(
        Protocol::Anthropic,
        Protocol::OpenAiChat,
        &encode(&body).unwrap(),
        "x"
    )
    .is_err());
}
#[test]
fn older_gemini_calls_without_ids_pair_only_unambiguous_pending_names() {
    let body = json!({"contents":[{"role":"model","parts":[{"functionCall":{"name":"read_file","args":{"path":"a"}}}]},{"role":"user","parts":[{"functionResponse":{"name":"read_file","response":{"content":"a"}}}]},{"role":"model","parts":[{"functionCall":{"name":"read_file","args":{"path":"b"}}}]},{"role":"user","parts":[{"functionResponse":{"name":"read_file","response":{"content":"b"}}}]}]});
    let converted = request(
        Protocol::Gemini,
        Protocol::OpenAiChat,
        "/v1beta/models/x:generateContent",
        &encode(&body).unwrap(),
        "x",
    )
    .unwrap();
    let value = parse(&converted.body, REQUEST_LIMIT).unwrap();
    assert_eq!(value["messages"][0]["tool_calls"][0]["id"], "lomi_call_0");
    assert_eq!(value["messages"][1]["tool_call_id"], "lomi_call_0");
    assert_eq!(value["messages"][2]["tool_calls"][0]["id"], "lomi_call_1");
    assert_eq!(value["messages"][3]["tool_call_id"], "lomi_call_1");
    let ambiguous = json!({"contents":[{"role":"model","parts":[{"functionCall":{"name":"read_file","args":{}}},{"functionCall":{"name":"read_file","args":{}}}]}]});
    assert!(request(
        Protocol::Gemini,
        Protocol::OpenAiChat,
        "/v1beta/models/x:generateContent",
        &encode(&ambiguous).unwrap(),
        "x"
    )
    .is_err());
    let signed = json!({"contents":[{"role":"model","parts":[{"text":"private","thoughtSignature":"private"}]}]});
    assert!(request(
        Protocol::Gemini,
        Protocol::OpenAiChat,
        "/v1beta/models/x:generateContent",
        &encode(&signed).unwrap(),
        "x"
    )
    .is_err());
}
#[test]
fn fragmented_chat_tool_arguments_are_joined_before_native_tool_use() {
    let fixture = concat!(
        "data: {\"id\":\"chat_1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"call_9\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chat_1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"lib.rs\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: {\"id\":\"chat_1\",\"object\":\"chat.completion.chunk\",\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":4,\"total_tokens\":14}}\n\n",
        "data: [DONE]\n\n"
    );
    let converted = stream_response(
        Protocol::Anthropic,
        Protocol::OpenAiChat,
        fixture.as_bytes(),
        "native",
    )
    .unwrap();
    let text = String::from_utf8(converted).unwrap();
    assert!(text.contains("event: content_block_start"));
    assert!(text.contains("\"id\":\"call_9\""));
    assert!(text.contains("\\\"path\\\":\\\"lib.rs\\\""));
    assert!(text.ends_with("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"));
    let truncated = fixture.strip_suffix("data: [DONE]\n\n").unwrap();
    assert!(stream_response(
        Protocol::Anthropic,
        Protocol::OpenAiChat,
        truncated.as_bytes(),
        "native"
    )
    .is_err());
}
#[test]
fn responses_terminal_snapshot_must_match_streamed_content() {
    let source = stream::emit(Protocol::OpenAiResponses, &output(), "x").unwrap();
    let text = String::from_utf8(source).unwrap();
    let mismatched = text.replace("\"delta\":\"Reading.\"", "\"delta\":\"Changed.\"");
    assert_ne!(mismatched, text);
    assert!(stream_response(
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        mismatched.as_bytes(),
        "x"
    )
    .is_err());
}
#[test]
fn absent_provider_usage_is_not_reported_as_zero_tokens() {
    let body=br#"{"id":"x","choices":[{"message":{"role":"assistant","content":"Hello"},"finish_reason":"stop"}]}"#;
    let converted = response(Protocol::Gemini, Protocol::OpenAiChat, body, "x").unwrap();
    assert!(parse(&converted, RESPONSE_LIMIT)
        .unwrap()
        .get("usageMetadata")
        .is_none());
    let converted = response(Protocol::OpenAiResponses, Protocol::OpenAiChat, body, "x").unwrap();
    assert!(parse(&converted, RESPONSE_LIMIT).unwrap()["usage"].is_null());
    assert!(response(Protocol::Anthropic, Protocol::OpenAiChat, body, "x").is_err());
}
// Source: Codex 0.160.0 core/src/client.rs build_responses_request and the
// gateway-owned pinned catalog. Required native options remain in this fixture.
fn native_codex_request_fixture() -> Value {
    let catalog: Value =
        serde_json::from_str(include_str!("../codex-models-0.160.0.json")).unwrap();
    let model = catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["slug"] == "gpt-6.1-sol")
        .unwrap();
    assert_eq!(model["use_responses_lite"], true);
    assert_eq!(model["default_reasoning_summary"], "none");
    assert!(model["apply_patch_tool_type"].is_null());
    json!({"model":model["slug"],"instructions":"Read a Rust file and describe the change.",
        "input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"Read lib.rs"}]}],
        "tools":[{"type":"function","name":"update_plan","description":"Update the work plan","parameters":{"type":"object","properties":{"plan":{"type":"array","items":{"type":"object","properties":{"step":{"type":"string"},"status":{"type":"string","enum":["pending","in_progress","completed"]}},"required":["step","status"],"additionalProperties":false}}},"required":["plan"],"additionalProperties":false},"strict":false}],
        "tool_choice":"auto","parallel_tool_calls":false,"reasoning":{"effort":model["default_reasoning_level"],"context":"all_turns"},
        "include":["reasoning.encrypted_content"],"text":{"verbosity":model["default_verbosity"]},"prompt_cache_key":"owned-thread-fixture","client_metadata":{},"stream":true,"store":false})
}
#[test]
fn admitted_codex_defaults_fail_before_constructing_cross_protocol_dispatch() {
    let native = native_codex_request_fixture();
    for target in [Protocol::OpenAiChat, Protocol::Anthropic, Protocol::Gemini] {
        let error = request(
            Protocol::OpenAiResponses,
            target,
            "/v1/responses",
            &encode(&native).unwrap(),
            "gpt-6.1-sol",
        )
        .err()
        .unwrap();
        assert!(error.contains("encrypted reasoning"));
        assert!(!error.contains("owned-thread-fixture"));
        assert!(!native_cli_supports(
            crate::cli_catalog::TitleCli::Codex,
            Protocol::OpenAiResponses,
            target
        ));
    }
    assert!(native_cli_supports(
        crate::cli_catalog::TitleCli::Codex,
        Protocol::OpenAiResponses,
        Protocol::OpenAiResponses
    ));
    let mut no_include = native;
    no_include["include"] = json!([]);
    assert!(request(
        Protocol::OpenAiResponses,
        Protocol::OpenAiChat,
        "/v1/responses",
        &encode(&no_include).unwrap(),
        "gpt-6.1-sol"
    )
    .err()
    .unwrap()
    .contains("reasoning continuation context"));
}
#[test]
fn public_openai_subset_preserves_reasoning_text_format_cache_metadata_and_tier() {
    // Explicitly qualify a public subset; this does not claim native Codex's
    // mandatory encrypted/all-turns request is equivalent to Chat.
    let mut source = native_codex_request_fixture();
    source["include"] = json!([]);
    source["reasoning"]
        .as_object_mut()
        .unwrap()
        .remove("context");
    source["text"]["format"] = json!({"type":"json_schema","name":"result","description":"Return a result","schema":{"type":"object","properties":{"summary":{"type":"string"}},"required":["summary"],"additionalProperties":false},"strict":true});
    source["metadata"] = json!({"purpose":"coding-fixture"});
    source["prompt_cache_retention"] = json!("in_memory");
    source["service_tier"] = json!("default");
    let converted = request(
        Protocol::OpenAiResponses,
        Protocol::OpenAiChat,
        "/v1/responses",
        &encode(&source).unwrap(),
        "gpt-6.1-sol",
    )
    .unwrap();
    let chat = parse(&converted.body, REQUEST_LIMIT).unwrap();
    assert_eq!(chat["reasoning_effort"], source["reasoning"]["effort"]);
    assert_eq!(chat["verbosity"], source["text"]["verbosity"]);
    assert_eq!(
        chat["response_format"]["json_schema"]["schema"],
        source["text"]["format"]["schema"]
    );
    assert_eq!(chat["response_format"]["json_schema"]["strict"], true);
    assert_eq!(chat["metadata"], source["metadata"]);
    assert_eq!(chat["prompt_cache_key"], source["prompt_cache_key"]);
    assert_eq!(
        chat["prompt_cache_retention"],
        source["prompt_cache_retention"]
    );
    assert_eq!(chat["service_tier"], source["service_tier"]);
    let restored = request(
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        "/v1/chat/completions",
        &converted.body,
        "gpt-6.1-sol",
    )
    .unwrap();
    let responses = parse(&restored.body, REQUEST_LIMIT).unwrap();
    assert_eq!(
        responses["reasoning"]["effort"],
        source["reasoning"]["effort"]
    );
    assert_eq!(responses["text"], source["text"]);
    assert_eq!(responses["metadata"], source["metadata"]);
    assert_eq!(responses["prompt_cache_key"], source["prompt_cache_key"]);
    assert!(request(
        Protocol::OpenAiResponses,
        Protocol::Anthropic,
        "/v1/responses",
        &encode(&source).unwrap(),
        "gpt-6.1-sol"
    )
    .err()
    .unwrap()
    .contains("OpenAI reasoning effort"));
}
#[test]
fn native_claude_cache_and_user_metadata_are_not_silently_erased() {
    let cached = json!({"model":"claude-sonnet","system":[{"type":"text","text":"Code safely","cache_control":{"type":"ephemeral","ttl":"1h"}}],"messages":[{"role":"user","content":"Read lib.rs"}],"max_tokens":1024,"metadata":{"user_id":"private-user"}});
    for target in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::Gemini,
    ] {
        let error = request(
            Protocol::Anthropic,
            target,
            "/v1/messages",
            &encode(&cached).unwrap(),
            "claude-sonnet",
        )
        .err()
        .unwrap();
        assert!(error.contains("cache breakpoints"));
        assert!(!error.contains("private-user"));
    }
    let mut metadata_only = cached;
    metadata_only["system"] = json!("Code safely");
    assert!(request(
        Protocol::Anthropic,
        Protocol::OpenAiChat,
        "/v1/messages",
        &encode(&metadata_only).unwrap(),
        "claude-sonnet"
    )
    .err()
    .unwrap()
    .contains("user metadata semantics"));
    let custom = json!({"model":"gpt-6.1-sol","input":"Apply this patch","tools":[{"type":"custom","name":"apply_patch","format":{"type":"grammar","syntax":"lark","definition":"private-grammar"}}]});
    let error = request(
        Protocol::OpenAiResponses,
        Protocol::OpenAiChat,
        "/v1/responses",
        &encode(&custom).unwrap(),
        "gpt-6.1-sol",
    )
    .err()
    .unwrap();
    assert!(error.contains("custom tool input and grammar"));
    assert!(!error.contains("private-grammar"));
}
