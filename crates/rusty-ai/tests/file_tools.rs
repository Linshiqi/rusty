//! The file tools hand the model text it will repeat as fact, so these check
//! the payloads: line numbers a person would recognise, caps that announce
//! themselves, and refusals for what the Files panel would never show.

use std::{fs, path::Path};

use rusty_ai::{ToolContext, ToolRegistry};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A small project with a chapter, a source file, build output and a dot
/// directory — the last two so the tests can assert they stay invisible.
fn project() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let write = |rel: &str, body: &str| {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    };
    write(
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    );
    write(".gitignore", "target/\n");
    write("src/main.rs", "fn main() {\n    println!(\"hello\");\n}\n");
    write(
        "book/src/02-feedback-pid.md",
        "# Feedback\n\n## Exercise 2.2\n\nTake I = 1 and Kp = 9. Find Kd.\n",
    );
    write("target/debug/demo.d", "build output nobody should read\n");
    write(
        ".cargo/config.toml",
        "[build]\ntarget = \"riscv32imc-unknown-none-elf\"\n",
    );
    dir
}

fn ctx(root: &Path) -> ToolContext<'_> {
    ToolContext {
        workspace: None,
        root: Some(root),
        firmware: None,
        catalog: None,
    }
}

fn call(name: &str, args: Value, root: &Path) -> Value {
    ToolRegistry::workbench()
        .call(name, &args, &ctx(root))
        .unwrap_or_else(|e| panic!("{name} failed: {e}"))
}

/// A chapter comes back numbered from 1, whole when it fits, and a window
/// asked for by line lands on those lines.
#[test]
fn read_file_numbers_lines_the_way_an_editor_does() {
    let dir = project();
    let whole = call(
        "read_file",
        json!({ "path": "book/src/02-feedback-pid.md" }),
        dir.path(),
    );
    assert_eq!(whole["total_lines"], 5);
    assert_eq!(whole["truncated"], false);
    let text = whole["text"].as_str().unwrap();
    assert!(text.contains("    1  # Feedback"), "{text}");
    assert!(text.contains("    3  ## Exercise 2.2"), "{text}");

    let window = call(
        "read_file",
        json!({ "path": "book/src/02-feedback-pid.md", "start_line": 3, "end_line": 3 }),
        dir.path(),
    );
    assert_eq!(window["text"], "    3  ## Exercise 2.2");
    assert_eq!(
        window["truncated"], true,
        "line 3 of 5 is not the whole file"
    );
}

/// A path that climbs out of the project, and a file that is not text, are
/// refused by name rather than answered with something else.
#[test]
fn read_file_refuses_what_the_files_panel_would_not_show() {
    let dir = project();
    let registry = ToolRegistry::workbench();
    let outside = registry
        .call(
            "read_file",
            &json!({ "path": "../secrets.txt" }),
            &ctx(dir.path()),
        )
        .unwrap_err()
        .to_string();
    assert!(outside.contains("read_file"), "{outside}");

    fs::write(dir.path().join("blob.bin"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
    let binary = registry
        .call(
            "read_file",
            &json!({ "path": "blob.bin" }),
            &ctx(dir.path()),
        )
        .unwrap_err()
        .to_string();
    assert!(binary.contains("not a text file"), "{binary}");
}

/// A search finds the chapter by a phrase in it, reports 1-based lines, and
/// never looks inside build output or a dot directory.
#[test]
fn search_project_finds_prose_and_skips_build_output() {
    let dir = project();
    let hits = call("search_project", json!({ "query": "Kp = 9" }), dir.path());
    let first = &hits["hits"][0];
    assert_eq!(first["path"], "book/src/02-feedback-pid.md");
    assert_eq!(first["line"], 5, "1-based, as the editor's gutter counts");
    assert_eq!(hits["files"], 1);

    let none = call(
        "search_project",
        json!({ "query": "nobody should read" }),
        dir.path(),
    );
    assert_eq!(
        none["hits"].as_array().unwrap().len(),
        0,
        "target/ is ignored"
    );
    let dot = call(
        "search_project",
        json!({ "query": "riscv32imc" }),
        dir.path(),
    );
    assert_eq!(
        dot["hits"].as_array().unwrap().len(),
        0,
        ".cargo/ is a dot directory"
    );
}

/// The listing is the tree the panel shows, optionally under one directory.
#[test]
fn list_files_shows_the_tree_and_narrows_to_a_directory() {
    let dir = project();
    let all = call("list_files", json!({}), dir.path());
    let files: Vec<&str> = all["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(files.contains(&"Cargo.toml"), "{files:?}");
    assert!(files.contains(&"book/src/02-feedback-pid.md"), "{files:?}");
    assert!(
        !files
            .iter()
            .any(|f| f.starts_with("target/") || f.starts_with(".cargo/")),
        "{files:?}"
    );

    let book = call("list_files", json!({ "dir": "book" }), dir.path());
    assert_eq!(book["total"], 1);
    assert_eq!(book["files"][0], "book/src/02-feedback-pid.md");
}
