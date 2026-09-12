//! A question, a file tool in the middle, an answer — against a loopback
//! server speaking the OpenAI dialect.
//!
//! `tool_registry.rs` and `file_tools.rs` prove each tool answers when
//! called. This proves the answer reaches the model: the file tools go out
//! with the first request, the call the model makes runs against the open
//! project, and what it returned goes back as the tool message the second
//! request carries. It is the check that cannot be made from a desk with no
//! route to a provider — the machine this was written on had none the day the
//! tools landed — and it runs on every CI runner.

use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};

use rusty_ai::{
    AgentEvent, Assistant, Content, Http, Message, ProviderConfig, ProviderKind, Role, ToolContext,
    config,
};
use serde_json::{Value, json};

/// One streamed answer in the OpenAI dialect: some text and, optionally, one
/// tool call with its arguments.
fn answer(text: &str, call: Option<(&str, &str, &Value)>) -> String {
    let mut body = String::new();
    body.push_str(&format!(
        "data: {}\n\n",
        json!({ "choices": [{ "delta": { "content": text } }] })
    ));
    let finish = match call {
        Some((id, name, args)) => {
            body.push_str(&format!(
                "data: {}\n\n",
                json!({ "choices": [{ "delta": { "tool_calls": [{
                    "index": 0, "id": id,
                    "function": { "name": name, "arguments": "" }
                }] } }] })
            ));
            body.push_str(&format!(
                "data: {}\n\n",
                json!({ "choices": [{ "delta": { "tool_calls": [{
                    "index": 0,
                    "function": { "arguments": args.to_string() }
                }] } }] })
            ));
            "tool_calls"
        }
        None => "stop",
    };
    body.push_str(&format!(
        "data: {}\n\n",
        json!({ "choices": [{ "delta": {}, "finish_reason": finish }] })
    ));
    body.push_str("data: [DONE]\n\n");
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len(),
    )
}

/// A server on a loopback port: one canned response per connection, in
/// order. Returns the base URL and every request body it was sent, so the
/// test can read what the model would have read.
fn serve(responses: Vec<String>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&bodies);
    thread::spawn(move || {
        for response in responses {
            let Ok((mut socket, _)) = listener.accept() else {
                return;
            };
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 4096];
            // The head, then exactly the body it announces: answering a
            // request still being written is a reset on the client's side.
            let head_end = loop {
                match socket.read(&mut chunk) {
                    Ok(0) | Err(_) => break None,
                    Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                }
                if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break Some(at + 4);
                }
            };
            let Some(head_end) = head_end else { return };
            let head = String::from_utf8_lossy(&bytes[..head_end]).into_owned();
            let length: usize = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().ok())
                        .flatten()
                })
                .unwrap_or(0);
            while bytes.len() < head_end + length {
                match socket.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                }
            }
            seen.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&bytes[head_end..]).into_owned());
            let _ = socket.write_all(response.as_bytes());
        }
    });
    (base, bodies)
}

const CHAPTER: &str = "book/src/02-feedback-pid.md";
const CHAPTER_TEXT: &str = "# Feedback\n\n## Exercise 2.2\n\nTake I = 1 and Kp = 9. Find Kd.\n";
const OPEN_FILE: &str = "pub fn kd() -> f32 { 0.0 }\n";

#[tokio::test]
async fn a_question_about_a_chapter_reads_the_chapter_before_answering() {
    let project = tempfile::tempdir().unwrap();
    let chapter = project.path().join(CHAPTER);
    fs::create_dir_all(chapter.parent().unwrap()).unwrap();
    fs::write(&chapter, CHAPTER_TEXT).unwrap();
    fs::create_dir_all(project.path().join("src")).unwrap();
    fs::write(project.path().join("src/lib.rs"), OPEN_FILE).unwrap();

    let read = json!({ "path": CHAPTER });
    let (base, bodies) = serve(vec![
        answer("Let me read it.", Some(("call_1", "read_file", &read))),
        answer("Exercise 2.2 asks for Kd given I = 1 and Kp = 9.", None),
    ]);
    let config = ProviderConfig {
        profile: "loop".into(),
        kind: ProviderKind::OpenAiCompatible,
        base_url: base,
        model: "m1".into(),
        max_tokens: 64,
        temperature: None,
        supports_tools: true,
    };
    let assistant = Assistant::new(config::build(&config, None, &Http::default()).unwrap());
    let ctx = ToolContext {
        workspace: None,
        root: Some(project.path()),
        firmware: None,
        catalog: None,
    };

    // The user has another file open; it rides along with the question.
    let mut history = vec![Message {
        role: Role::User,
        content: vec![
            Content::Text {
                text: "What does exercise 2.2 ask?".into(),
            },
            Content::Attachment {
                path: "src/lib.rs".into(),
                text: OPEN_FILE.into(),
            },
        ],
    }];
    let mut events = Vec::new();
    assistant
        .ask(&ctx, &mut history, &mut |event| events.push(event))
        .await
        .expect("the loop completes");

    let bodies = bodies.lock().unwrap();
    assert_eq!(
        bodies.len(),
        2,
        "one round trip to ask, one to answer with the file in hand"
    );

    // The first request: the file tools are offered, and the open file is
    // in the user's message, named.
    let first: Value = serde_json::from_str(&bodies[0]).unwrap();
    let offered: Vec<&str> = first["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    for tool in ["read_file", "search_project", "list_files"] {
        assert!(offered.contains(&tool), "{offered:?}");
    }
    let user = first["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "user")
        .expect("the user's message");
    let asked = user["content"].as_str().unwrap();
    assert!(asked.contains("What does exercise 2.2 ask?"), "{asked}");
    assert!(asked.contains("`src/lib.rs`"), "{asked}");
    assert!(asked.contains("pub fn kd()"), "{asked}");

    // The second request: the chapter, read from the project, went back to
    // the model as the result of the call it made.
    let second: Value = serde_json::from_str(&bodies[1]).unwrap();
    let tool_message = second["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .expect("a tool message");
    assert_eq!(tool_message["tool_call_id"], "call_1");
    let result: Value = serde_json::from_str(tool_message["content"].as_str().unwrap())
        .expect("the tool's JSON, verbatim");
    assert_eq!(result["path"], CHAPTER);
    assert_eq!(result["total_lines"], 5);
    assert_eq!(result["truncated"], false);
    assert!(
        result["text"]
            .as_str()
            .unwrap()
            .contains("    5  Take I = 1 and Kp = 9. Find Kd."),
        "{}",
        result["text"]
    );

    // What the window is told, and what the conversation keeps.
    assert!(events.iter().any(
        |e| matches!(e, AgentEvent::ToolStarted { name, input, .. } if name == "read_file" && input["path"] == CHAPTER)
    ));
    assert!(events.iter().any(
        |e| matches!(e, AgentEvent::ToolFinished { name, ok: true, .. } if name == "read_file")
    ));
    let answer = history.last().unwrap();
    assert_eq!(answer.role, Role::Assistant);
    assert!(answer.text().contains("Exercise 2.2 asks for Kd"));
}
