//! Where a question's input tokens go, before anybody spends them.
//!
//! ```text
//! cargo run -p rusty-ai --example token_budget -- <project> [file-to-attach]
//! ```
//!
//! Every round of the agent loop sends the system prompt, every tool's
//! definition and the whole conversation so far — earlier questions, the
//! files attached to them and every tool answer included — so what a
//! question costs is those three, times the rounds it takes. This prints
//! each part's size as it would go on the wire: the prompt, each
//! definition, what each read-only tool answers about `project`, and then
//! what a short conversation costs round by round. Tokens are estimated —
//! four characters of ASCII to a token, one per character of anything
//! else — which is close enough to see which part is the bill.

use std::path::Path;

use rusty_ai::{Content, Message, SYSTEM_PROMPT, ToolContext, ToolRegistry, wire_history};
use serde_json::{Value, json};

fn tokens(text: &str) -> usize {
    let ascii = text.bytes().filter(u8::is_ascii).count();
    let other = text.chars().filter(|c| !c.is_ascii()).count();
    ascii.div_ceil(4) + other
}

/// A definition as both dialects send it: name, description, schema.
fn wire(def: &rusty_ai::ToolDef) -> String {
    json!({
        "name": def.name,
        "description": def.description,
        "input_schema": def.input_schema,
    })
    .to_string()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let project = args
        .next()
        .expect("usage: token_budget <project> [file-to-attach]");
    let attach = args.next();
    let root = Path::new(&project);

    let registry = ToolRegistry::workbench();
    let defs = registry.defs();

    println!("== sent with every round ==");
    println!("{:>7} tokens  system prompt", tokens(SYSTEM_PROMPT));
    let mut all_defs = 0;
    for def in &defs {
        let size = tokens(&wire(def));
        all_defs += size;
        println!(
            "{size:>7} tokens  tool `{}` (description {} tokens)",
            def.name,
            tokens(&def.description)
        );
    }
    println!("{all_defs:>7} tokens  all {} definitions", defs.len());
    let fixed = tokens(SYSTEM_PROMPT) + all_defs;
    println!("{fixed:>7} tokens  in every request before a word of the conversation");

    let ctx = ToolContext {
        root: Some(root),
        ..ToolContext::empty()
    };
    let calls: Vec<(&str, Value)> = vec![
        ("project_status", json!({})),
        ("toolchain_status", json!({})),
        ("chip_catalogue", json!({})),
        ("chip_catalogue", json!({ "chip": "esp32c3" })),
        ("list_files", json!({})),
        ("search_project", json!({ "query": "fn main" })),
        ("workspace_report", json!({})),
        (
            "math_sheet",
            json!({ "rows": ["q = euler(30°, 10°, 45°)", "acc = accel_at_rest(q)"] }),
        ),
    ];
    println!("\n== what each tool answers about {project} ==");
    let mut answers = Vec::new();
    for (name, input) in calls {
        let answer = match registry.call(name, &input, &ctx) {
            Ok(value) => value.to_string(),
            Err(error) => error.to_string(),
        };
        println!("{:>7} tokens  {name} {input}", tokens(&answer));
        answers.push((name, input, answer));
    }

    // What the drawer sends with the cursor at the top: the whole file when
    // it is short, its first lines when it is not.
    let attached = attach.map(|path| {
        let text = std::fs::read_to_string(root.join(&path)).expect("the file to attach");
        let whole = tokens(&text);
        (Content::attach(path, &text, None), whole)
    });
    if let Some((content, whole)) = &attached {
        let Content::Attachment { path, lines, .. } = content else {
            unreachable!("attach makes an attachment")
        };
        let prose = content.prose().unwrap_or_default();
        match lines {
            None => println!(
                "\n{:>7} tokens  the open file, {path}, whole",
                tokens(&prose)
            ),
            Some(lines) => println!(
                "\n{:>7} tokens  the open file, {path}: lines {}–{} of {} ({whole} tokens whole)",
                tokens(&prose),
                lines.first,
                lines.last,
                lines.total
            ),
        }
    }

    // Two short questions, each answered after one tool call: what the
    // drawer sends round by round, the way the loop builds it. Each request
    // as one text — the prompt, the definitions, the conversation as
    // `wire_history` has it — so what a provider's prefix cache could supply
    // is what it repeats from the start of the request before.
    println!("\n== two short questions, one tool call each ==");
    let definitions: String = defs.iter().map(wire).collect();
    let request = |history: &[Message]| {
        format!(
            "{SYSTEM_PROMPT}{definitions}{}",
            serde_json::to_string(&wire_history(history)).unwrap()
        )
    };
    let mut previous = String::new();
    let mut history: Vec<Message> = Vec::new();
    let (mut total, mut repeated) = (0, 0);
    let (_, _, status) = &answers[0];
    for (question, tool) in [
        ("这个项目用的什么芯片？", "project_status"),
        ("还缺什么工具吗？", "toolchain_status"),
    ] {
        let mut content = vec![Content::Text {
            text: question.into(),
        }];
        if let Some((attachment, _)) = &attached {
            content.push(attachment.clone());
        }
        history.push(Message {
            role: rusty_ai::Role::User,
            content,
        });
        let (first, first_repeats) = send(&mut previous, request(&history));
        let answer = answers
            .iter()
            .find(|(name, _, _)| *name == tool)
            .map_or(status.clone(), |(_, _, a)| a.clone());
        history.push(Message::assistant(vec![Content::ToolUse {
            id: "call".into(),
            name: tool.into(),
            input: json!({}),
        }]));
        history.push(Message::tool_results(vec![Content::ToolResult {
            id: "call".into(),
            content: answer,
            is_error: false,
        }]));
        let (second, second_repeats) = send(&mut previous, request(&history));
        history.push(Message::assistant(vec![Content::Text {
            text: "A short answer of about fifty words, which is what a simple question gets."
                .repeat(4),
        }]));
        total += first + second;
        repeated += first_repeats + second_repeats;
        println!(
            "{first:>7} + {second:>7} tokens  \"{question}\" (two rounds; {} repeat the request before)",
            first_repeats + second_repeats
        );
    }
    println!("{total:>7} tokens  of input for the two");
    println!(
        "{repeated:>7} tokens  of them repeat the start of the request before: what a prefix \
         cache supplies, at a tenth of the price on Anthropic and DeepSeek"
    );
}

/// `now`'s size, and the size of what it repeats from the start of
/// `previous`, which it then replaces.
fn send(previous: &mut String, now: String) -> (usize, usize) {
    let mut shared = previous
        .bytes()
        .zip(now.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !now.is_char_boundary(shared) {
        shared -= 1;
    }
    let cost = (tokens(&now), tokens(&now[..shared]));
    *previous = now;
    cost
}
