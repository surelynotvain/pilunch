//! End-to-end tests of the agent loop against an in-process mock of the Messages API
//! (real HTTP + SSE over a local socket, Tauri's mock runtime for the app).

use super::*;
use crate::conversations::ToolStatus;
use serde_json::json;
use std::sync::{mpsc, Arc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

enum MockResp {
    Sse(String),
    Status(u16, String),
    /// Send this SSE prefix, then keep the connection open (for cancellation tests).
    Hang(String),
}

#[derive(Clone, Debug)]
struct Recorded {
    headers: String,
    body: Value,
}

fn sse(events: &[Value]) -> String {
    events.iter().map(|e| format!("event: {}\ndata: {}\n\n", e["type"].as_str().unwrap(), e)).collect()
}

fn text_turn(text: &str, stop: &str) -> Vec<Value> {
    vec![
        json!({"type":"message_start","message":{"id":"msg_x","model":"claude-opus-5-5","usage":{"input_tokens":20,"output_tokens":1}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":stop},"usage":{"output_tokens":7}}),
        json!({"type":"message_stop"}),
    ]
}

fn tool_turn(name: &str, input_json_parts: &[&str]) -> Vec<Value> {
    let mut v = vec![
        json!({"type":"message_start","message":{"id":"msg_1","model":"claude-opus-5-5","usage":{"input_tokens":100,"cache_read_input_tokens":50}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Plan it."}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-abc"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"On it."}}),
        json!({"type":"content_block_stop","index":1}),
        json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":name,"input":{}}}),
    ];
    for p in input_json_parts {
        v.push(json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":p}}));
    }
    v.extend([
        json!({"type":"content_block_stop","index":2}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":30}}),
        json!({"type":"message_stop"}),
    ]);
    v
}

async fn read_request(sock: &mut tokio::net::TcpStream) -> Option<Recorded> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let header_end = loop {
        let n = sock.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let headers = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
    let len: usize = headers
        .lines()
        .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
        .unwrap_or(0);
    while buf.len() < header_end + len {
        let n = sock.read(&mut tmp).await.ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let body = serde_json::from_slice(&buf[header_end..]).unwrap_or(Value::Null);
    Some(Recorded { headers, body })
}

/// Serves `responses` in order, one per request, and records every request.
async fn mock_api(responses: Vec<MockResp>) -> (String, Arc<Mutex<Vec<Recorded>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let rec = recorded.clone();
    let responses = Arc::new(Mutex::new(responses.into_iter()));
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let rec = rec.clone();
            let responses = responses.clone();
            tokio::spawn(async move {
                let Some(req) = read_request(&mut sock).await else { return };
                rec.lock().unwrap().push(req);
                let resp = responses.lock().unwrap().next();
                match resp {
                    Some(MockResp::Sse(body)) => {
                        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
                        let _ = sock.write_all(head.as_bytes()).await;
                        // Dribble the body out in small chunks to exercise the SSE parser.
                        for chunk in body.as_bytes().chunks(37) {
                            let _ = sock.write_all(chunk).await;
                            let _ = sock.flush().await;
                        }
                        let _ = sock.shutdown().await;
                    }
                    Some(MockResp::Status(code, body)) => {
                        let head = format!(
                            "HTTP/1.1 {code} Error\r\ncontent-type: application/json\r\nretry-after: 0\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = sock.write_all(head.as_bytes()).await;
                        let _ = sock.write_all(body.as_bytes()).await;
                        let _ = sock.shutdown().await;
                    }
                    Some(MockResp::Hang(prefix)) => {
                        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
                        let _ = sock.write_all(head.as_bytes()).await;
                        let _ = sock.write_all(prefix.as_bytes()).await;
                        let _ = sock.flush().await;
                        tokio::time::sleep(Duration::from_secs(60)).await;
                    }
                    None => {
                        let _ = sock.write_all(b"HTTP/1.1 500 Unexpected\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
                    }
                }
            });
        }
    });
    (addr, recorded)
}

struct Harness {
    app: tauri::App<tauri::test::MockRuntime>,
    conv_id: String,
    ws: tempfile::TempDir,
    _dirs: tempfile::TempDir,
}

fn harness(base_url: &str, mode: &str) -> Harness {
    let dirs = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    std::fs::write(ws.path().join("main.rs"), "fn main() {}\n").unwrap();
    let state = AppState::new(dirs.path().join("config"), dirs.path().join("data"));
    state.settings.set_api_key("test-key".into()).unwrap();
    state.settings.update(json!({ "baseUrl": base_url, "permissionMode": mode })).unwrap();
    let ws_root = Workspace::open(ws.path()).unwrap().root().display().to_string();
    let conv_id = state.conversations.create(Some(ws_root)).unwrap().meta.id;
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    Harness { app, conv_id, ws, _dirs: dirs }
}

/// Runs one user message. `on_approval` decides each approval request.
fn run_message(h: &Harness, text: &str, on_approval: impl Fn(&Value) -> (Decision, Option<String>) + Send + Sync + 'static) -> Vec<Value> {
    let (tx, rx) = mpsc::channel::<Value>();
    let handle = h.app.handle().clone();
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev2 = events.clone();
    let channel = Channel::new(move |body| {
        let v: Value = body.deserialize().unwrap();
        if v["type"] == "approvalRequest" {
            let (d, f) = on_approval(&v);
            handle.state::<AppState>().agent.respond(v["approvalId"].as_str().unwrap(), d, f).unwrap();
        }
        ev2.lock().unwrap().push(v.clone());
        let _ = tx.send(v);
        Ok(())
    });
    start(h.app.handle().clone(), SendRequest { conversation_id: h.conv_id.clone(), text: text.into(), attachments: vec![] }, channel).unwrap();
    loop {
        let v = rx.recv_timeout(Duration::from_secs(20)).expect("agent run timed out");
        if v["type"] == "done" {
            break;
        }
    }
    // The run removes itself from the registry right after sending `done`.
    std::thread::sleep(Duration::from_millis(50));
    let out = events.lock().unwrap().clone();
    out
}

fn types(events: &[Value]) -> Vec<String> {
    events.iter().map(|e| e["type"].as_str().unwrap().to_string()).collect()
}

#[test]
fn tool_loop_with_retry_approval_and_thinking_replay() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, recorded) = rt.block_on(mock_api(vec![
        MockResp::Status(529, r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#.into()),
        MockResp::Sse(sse(&tool_turn("write_file", &["{\"path\": \"hel", "lo.txt\", \"content\": \"hi\\n\"}"]))),
        MockResp::Sse(sse(&text_turn("Created hello.txt.", "end_turn"))),
    ]));
    let h = harness(&addr, "ask");
    let events = run_message(&h, "make hello.txt", |_| (Decision::Allow, None));
    let t = types(&events);

    // Event flow
    assert!(t.contains(&"retrying".to_string()), "{t:?}");
    assert!(t.contains(&"approvalRequest".to_string()));
    let approval = events.iter().find(|e| e["type"] == "approvalRequest").unwrap();
    assert_eq!(approval["kind"], "edit");
    assert!(approval["detail"].as_str().unwrap().contains("+hi"));
    let tool_input = events.iter().find(|e| e["type"] == "toolInput").unwrap();
    assert_eq!(tool_input["input"], json!({"path":"hello.txt","content":"hi\n"}));
    let last = events.last().unwrap();
    assert_eq!(last["type"], "done");
    assert_eq!(last["stopReason"], "end_turn");
    let streamed: String = events.iter().filter(|e| e["type"] == "delta").map(|e| e["text"].as_str().unwrap()).collect();
    assert!(streamed.contains("On it.") && streamed.contains("Created hello.txt."));

    // The file was written
    assert_eq!(std::fs::read_to_string(h.ws.path().join("hello.txt")).unwrap(), "hi\n");

    // History was persisted: user, assistant(thinking,text,tool_use), user(tool_result), assistant(text)
    let state = h.app.state::<AppState>();
    let conv = state.conversations.get(&h.conv_id).unwrap();
    let roles: Vec<_> = conv.messages.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, ["user", "assistant", "user", "assistant"]);
    assert_eq!(conv.meta.title, "make hello.txt");
    assert_eq!(conv.tool_ui["toolu_1"].status, ToolStatus::Done);
    assert_eq!(conv.tool_ui["toolu_1"].detail_kind.as_deref(), Some("diff"));
    assert_eq!(conv.usage.cache_read_tokens, 50);

    // What the API received
    let reqs = recorded.lock().unwrap().clone();
    assert_eq!(reqs.len(), 3);
    for r in &reqs {
        assert!(r.headers.contains("x-api-key: test-key"));
        assert!(r.headers.contains("anthropic-version: 2023-06-01"));
        assert_eq!(r.body["stream"], true);
        assert_eq!(r.body["model"], "claude-opus-5-5");
        // custom endpoint: no eager streaming / fallbacks
        assert!(r.body.get("fallbacks").is_none());
        assert!(r.body["tools"].as_array().unwrap().iter().all(|t| t.get("eager_input_streaming").is_none()));
    }
    let second = &reqs[2].body["messages"];
    assert_eq!(second[1]["content"][0], json!({"type":"thinking","thinking":"Plan it.","signature":"sig-abc"}));
    assert_eq!(second[2]["content"][0]["type"], "tool_result");
    assert_eq!(second[2]["content"][0]["tool_use_id"], "toolu_1");
    assert!(second[2]["content"][0].get("is_error").is_none());
    assert!(state.agent.running().is_empty());
}

#[test]
fn denied_command_feedback_reaches_the_model() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, recorded) = rt.block_on(mock_api(vec![
        MockResp::Sse(sse(&tool_turn("run_command", &["{\"command\": \"rm -rf build\"}"]))),
        MockResp::Sse(sse(&text_turn("OK, I won't.", "end_turn"))),
    ]));
    let h = harness(&addr, "acceptEdits");
    let events = run_message(&h, "clean up", |e| {
        assert_eq!(e["kind"], "command");
        assert_eq!(e["detail"], "rm -rf build");
        (Decision::Deny, Some("keep the build dir".into()))
    });
    assert_eq!(events.last().unwrap()["stopReason"], "end_turn");
    let reqs = recorded.lock().unwrap().clone();
    let result = &reqs[1].body["messages"][2]["content"][0];
    assert_eq!(result["is_error"], true);
    assert!(result["content"].as_str().unwrap().contains("Their feedback: keep the build dir"));
    let conv = h.app.state::<AppState>().conversations.get(&h.conv_id).unwrap();
    assert_eq!(conv.tool_ui["toolu_1"].status, ToolStatus::Denied);
}

#[test]
fn bypass_mode_runs_commands_and_plan_mode_hides_write_tools() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, recorded) = rt.block_on(mock_api(vec![
        MockResp::Sse(sse(&tool_turn("run_command", &["{\"command\": \"echo pilunch-ok\"}"]))),
        MockResp::Sse(sse(&text_turn("Done.", "end_turn"))),
        MockResp::Sse(sse(&text_turn("Here is the plan.", "end_turn"))),
    ]));
    let h = harness(&addr, "bypass");
    let events = run_message(&h, "run it", |_| panic!("bypass mode must not ask"));
    let output: String = events.iter().filter(|e| e["type"] == "toolOutput").map(|e| e["text"].as_str().unwrap()).collect();
    assert!(output.contains("pilunch-ok"));
    let reqs = recorded.lock().unwrap().clone();
    assert!(reqs[1].body["messages"][2]["content"][0]["content"].as_str().unwrap().starts_with("Exit code: 0"));

    h.app.state::<AppState>().settings.update(json!({"permissionMode": "plan"})).unwrap();
    run_message(&h, "plan something", |_| panic!("no approvals in plan mode"));
    let reqs = recorded.lock().unwrap().clone();
    let names: Vec<_> = reqs[2].body["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    assert!(names.contains(&"read_file".to_string()) && names.contains(&"git_diff".to_string()));
    assert!(!names.iter().any(|n| ["edit_file", "write_file", "run_command", "delete_path", "find_replace"].contains(&n.as_str())));
    assert!(reqs[2].body["system"][0]["text"].as_str().unwrap().contains("# Plan mode"));
    // second user message follows the previous assistant answer
    let msgs = reqs[2].body["messages"].as_array().unwrap();
    assert_eq!(msgs.last().unwrap()["role"], "user");
}

#[test]
fn cancel_mid_stream_keeps_partial_text() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let prefix = sse(&[
        json!({"type":"message_start","message":{"id":"m","model":"claude-opus-5-5","usage":{"input_tokens":5}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Partial answer"}}),
    ]);
    let (addr, _) = rt.block_on(mock_api(vec![MockResp::Hang(prefix)]));
    let h = harness(&addr, "ask");
    let handle = h.app.handle().clone();
    let conv_id = h.conv_id.clone();
    // Cancel as soon as text starts streaming.
    let (tx, rx) = mpsc::channel::<Value>();
    let channel = Channel::new(move |body| {
        let v: Value = body.deserialize().unwrap();
        if v["type"] == "delta" {
            handle.state::<AppState>().agent.cancel(&conv_id);
        }
        let _ = tx.send(v);
        Ok(())
    });
    start(h.app.handle().clone(), SendRequest { conversation_id: h.conv_id.clone(), text: "hi".into(), attachments: vec![] }, channel).unwrap();
    let t0 = std::time::Instant::now();
    let done = loop {
        let v = rx.recv_timeout(Duration::from_secs(10)).expect("timed out");
        if v["type"] == "done" {
            break v;
        }
    };
    assert_eq!(done["stopReason"], "cancelled");
    assert!(t0.elapsed() < Duration::from_secs(5));
    let conv = h.app.state::<AppState>().conversations.get(&h.conv_id).unwrap();
    assert_eq!(conv.messages.len(), 2);
    assert_eq!(conv.messages[1].content, vec![json!({"type":"text","text":"Partial answer"})]);
}

#[test]
fn refusal_discards_partial_output() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut ev = text_turn("partial", "refusal");
    ev[4] = json!({"type":"message_delta","delta":{"stop_reason":"refusal","stop_details":{"type":"refusal","category":"cyber","explanation":"Declined for safety."}}});
    let (addr, _) = rt.block_on(mock_api(vec![MockResp::Sse(sse(&ev))]));
    let h = harness(&addr, "ask");
    let events = run_message(&h, "do something", |_| unreachable!());
    let refusal = events.iter().find(|e| e["type"] == "refusal").unwrap();
    assert_eq!(refusal["message"], "Declined for safety.");
    assert_eq!(events.last().unwrap()["stopReason"], "refusal");
    let conv = h.app.state::<AppState>().conversations.get(&h.conv_id).unwrap();
    assert_eq!(conv.messages.len(), 1, "partial refused output is not kept");
}

#[test]
fn auth_error_is_reported_without_retry() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, recorded) = rt.block_on(mock_api(vec![MockResp::Status(
        401,
        r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#.into(),
    )]));
    let h = harness(&addr, "ask");
    let events = run_message(&h, "hi", |_| unreachable!());
    let err = events.iter().find(|e| e["type"] == "error").unwrap();
    assert!(err["message"].as_str().unwrap().contains("Invalid API key"));
    assert_eq!(recorded.lock().unwrap().len(), 1);
}

#[test]
fn invalid_streamed_tool_json_is_returned_as_error() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, recorded) = rt.block_on(mock_api(vec![
        MockResp::Sse(sse(&tool_turn("write_file", &["{\"path\": \"a.txt\", \"content\": \"broken"]))),
        MockResp::Sse(sse(&text_turn("Retrying differently.", "end_turn"))),
    ]));
    let h = harness(&addr, "bypass");
    run_message(&h, "write", |_| unreachable!());
    let reqs = recorded.lock().unwrap().clone();
    let result = &reqs[1].body["messages"][2]["content"][0];
    assert_eq!(result["is_error"], true);
    assert!(result["content"].as_str().unwrap().contains("INVALID_JSON"));
    assert!(!h.ws.path().join("a.txt").exists());
}

fn oa_sse(chunks: &[Value]) -> String {
    let mut s: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    s.push_str("data: [DONE]\n\n");
    s
}

#[test]
fn local_openai_compatible_provider_runs_tools() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, recorded) = rt.block_on(mock_api(vec![
        MockResp::Sse(oa_sse(&[
            json!({"model":"qwen3-coder","choices":[{"delta":{"reasoning_content":"look at main"}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"main.rs\"}"}}]},"finish_reason":"tool_calls"}]}),
        ])),
        MockResp::Sse(oa_sse(&[json!({"choices":[{"delta":{"content":"It has a main fn."},"finish_reason":"stop"}],"usage":{"prompt_tokens":50,"completion_tokens":6}})])),
    ]));
    let h = harness("http://unused", "ask");
    h.app.state::<AppState>().settings.update(json!({"provider":"local","localBaseUrl": format!("{addr}/v1"),"localModel":"qwen3-coder"})).unwrap();
    let events = run_message(&h, "what is in main.rs?", |_| unreachable!("read tools never ask"));
    assert_eq!(events.last().unwrap()["stopReason"], "end_turn", "{events:?}");
    let reqs = recorded.lock().unwrap().clone();
    assert_eq!(reqs.len(), 2);
    assert!(reqs[0].headers.starts_with("post /v1/chat/completions"), "{}", reqs[0].headers);
    assert_eq!(reqs[0].body["model"], "qwen3-coder");
    assert_eq!(reqs[0].body["messages"][0]["role"], "system");
    assert!(reqs[0].body["tools"].as_array().unwrap().iter().all(|t| t["type"] == "function"));
    let second = reqs[1].body["messages"].as_array().unwrap().clone();
    let n = second.len();
    assert_eq!(second[n - 2]["tool_calls"][0]["id"], "call_a");
    assert_eq!(second[n - 1]["role"], "tool");
    assert!(second[n - 1]["content"].as_str().unwrap().contains("fn main()"));
    let conv = h.app.state::<AppState>().conversations.get(&h.conv_id).unwrap();
    assert_eq!(conv.tool_ui["call_a"].status, ToolStatus::Done);
    assert!(conv.api_messages().iter().all(|m| m["content"].as_array().unwrap().iter().all(|b| b["type"] != "thinking")));
}

#[test]
fn max_thinking_level_forces_more_reasoning() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let short = |r: &str, answer: &str| {
        MockResp::Sse(oa_sse(&[
            json!({"model":"qwen3","choices":[{"delta":{"reasoning_content":r}}]}),
            json!({"choices":[{"delta":{"content":answer},"finish_reason":"stop"}]}),
        ]))
    };
    let long = "x".repeat(4 * 2_100);
    let (addr, recorded) = rt.block_on(mock_api(vec![short("first idea", "too quick"), short(&long, "Final answer.")]));
    let h = harness("http://unused", "ask");
    h.app
        .state::<AppState>()
        .settings
        .update(json!({"provider":"local","localBaseUrl": format!("{addr}/v1"),"localModel":"qwen3","thinkingLevel":"xhigh"}))
        .unwrap();
    let events = run_message(&h, "think hard", |_| unreachable!());
    assert_eq!(events.last().unwrap()["stopReason"], "end_turn", "{events:?}");
    let reqs = recorded.lock().unwrap().clone();
    assert_eq!(reqs.len(), 2, "one forced extra round");
    assert_eq!(reqs[0].body["reasoning_effort"], "high");
    let nudge = reqs[1].body["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().to_string();
    assert!(nudge.contains("first idea") && nudge.contains("Wait"), "{nudge}");
    let conv = h.app.state::<AppState>().conversations.get(&h.conv_id).unwrap();
    let last = conv.messages.last().unwrap();
    assert_eq!(last.content[0]["type"], "thinking");
    assert!(last.content[0]["thinking"].as_str().unwrap().starts_with("first idea"));
    assert_eq!(last.content[1]["text"], "Final answer.");
    // the discarded first answer is not in the history
    assert!(!serde_json::to_string(&conv.messages).unwrap().contains("too quick"));
    assert!(events.iter().any(|e| e["type"] == "notice" && e["message"].as_str().unwrap().contains("Thinking longer")));
}

#[test]
fn rejected_reasoning_params_are_dropped_and_retried() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, recorded) = rt.block_on(mock_api(vec![
        MockResp::Status(400, r#"{"error":{"message":"Unrecognized request argument supplied: reasoning_effort"}}"#.into()),
        MockResp::Sse(oa_sse(&[json!({"model":"m","choices":[{"delta":{"content":"hi"},"finish_reason":"stop"}]})])),
    ]));
    let h = harness("http://unused", "ask");
    h.app.state::<AppState>().settings.update(json!({"provider":"local","localBaseUrl": format!("{addr}/v1"),"localModel":"m","thinkingLevel":"high"})).unwrap();
    let events = run_message(&h, "hello", |_| unreachable!());
    assert_eq!(events.last().unwrap()["stopReason"], "end_turn", "{events:?}");
    let reqs = recorded.lock().unwrap().clone();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].body["reasoning_effort"], "high");
    assert!(reqs[1].body.get("reasoning_effort").is_none());
    assert!(reqs[1].body.get("chat_template_kwargs").is_none());
}

#[test]
fn runs_record_usage_and_save_traces() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (addr, _rec) = rt.block_on(mock_api(vec![MockResp::Sse(oa_sse(&[
        json!({"model":"qwen3","choices":[{"delta":{"reasoning_content":"hmm"}}]}),
        json!({"choices":[{"delta":{"content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":3}}),
    ]))]));
    let h = harness("http://unused", "ask");
    let state = h.app.state::<AppState>();
    state.settings.update(json!({"provider":"local","localBaseUrl": format!("{addr}/v1"),"localModel":"qwen3","saveTraces":true,"tracesScope":"local"})).unwrap();
    run_message(&h, "hi", |_| unreachable!());
    let usage = state.records.usage_entries();
    assert_eq!(usage.len(), 1);
    assert_eq!((usage[0].provider.as_str(), usage[0].model.as_str(), usage[0].input_tokens), ("local", "qwen3", 12));
    let trace: Value = serde_json::from_slice(&std::fs::read(state.records.traces_dir().join(format!("{}.json", h.conv_id))).unwrap()).unwrap();
    assert_eq!(trace["provider"], "local");
    let msgs = trace["messages"].as_array().unwrap();
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs.last().unwrap()["reasoning_content"], "hmm");
    assert_eq!(msgs.last().unwrap()["content"], "hello");
}

#[test]
fn agent_saves_and_loads_skills_and_runs_custom_tools() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let save = r#"{"name":"greet-steps","description":"How to greet","content":"1. Say hi"}"#;
    let (addr, recorded) = rt.block_on(mock_api(vec![
        MockResp::Sse(sse(&tool_turn("skill_save", &[save]))),
        MockResp::Sse(sse(&tool_turn("hello_tool", &[r#"{"who":"Ada"}"#]))),
        MockResp::Sse(sse(&text_turn("All done.", "end_turn"))),
    ]));
    let h = harness(&addr, "ask");
    let state = h.app.state::<AppState>();
    let cfg = state.settings.config_dir().to_path_buf();
    std::fs::create_dir_all(cfg.join("tools")).unwrap();
    std::fs::write(cfg.join("tools/hello.json"), r#"{"name":"hello_tool","description":"Greets","parameters":{"type":"object","properties":{"who":{"type":"string"}}},"command":"echo hello {{who}} $PILUNCH_ARG_WHO"}"#).unwrap();
    let kinds = Arc::new(Mutex::new(Vec::new()));
    let k2 = kinds.clone();
    let events = run_message(&h, "learn and greet", move |a| {
        k2.lock().unwrap().push((a["kind"].as_str().unwrap().to_string(), a["detail"].as_str().unwrap().to_string()));
        (Decision::Allow, None)
    });
    assert_eq!(events.last().unwrap()["stopReason"], "end_turn", "{events:?}");
    let kinds = kinds.lock().unwrap().clone();
    assert_eq!(kinds[0].0, "edit");
    assert!(kinds[0].1.contains("+1. Say hi"), "{}", kinds[0].1);
    assert_eq!(kinds[1], ("command".to_string(), "echo hello 'Ada' $PILUNCH_ARG_WHO".to_string()));
    let skill = std::fs::read_to_string(cfg.join("skills/greet-steps/SKILL.md")).unwrap();
    assert!(skill.starts_with("---\nname: greet-steps\ndescription: How to greet\n---"));
    let reqs = recorded.lock().unwrap().clone();
    let names: Vec<&str> = reqs[0].body["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"skill_load") && names.contains(&"hello_tool") && names.contains(&"browser"), "{names:?}");
    assert!(!names.contains(&"computer"), "computer use is off by default");
    let last = reqs[2].body["messages"].as_array().unwrap().last().unwrap().clone();
    assert!(last["content"][0]["content"].as_str().unwrap().contains("hello Ada Ada"), "{last}");

    // the next run lists the new skill in the system prompt
    let (addr2, rec2) = rt.block_on(mock_api(vec![MockResp::Sse(sse(&text_turn("ok", "end_turn")))]));
    state.settings.update(json!({ "baseUrl": addr2 })).unwrap();
    run_message(&h, "again", |_| unreachable!());
    let sys = rec2.lock().unwrap()[0].body["system"].to_string();
    assert!(sys.contains("- greet-steps: How to greet"), "{sys}");
}
