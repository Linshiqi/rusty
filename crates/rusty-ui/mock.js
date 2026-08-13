// Dev-only IPC mock so `trunk serve` can exercise flows that need the backend.
// Inert inside the real app; must be removed from index.html before commit.
(function () {
  if (window.__TAURI_INTERNALS__ || window.__TAURI__) return;

  class Channel {
    constructor() { this._handler = null; }
    set onmessage(fn) { this._handler = fn; if (this._queued) { const q = this._queued; this._queued = null; q.forEach((m) => fn(m)); } }
    get onmessage() { return this._handler; }
    send(msg) { if (this._handler) this._handler(msg); else (this._queued = this._queued || []).push(msg); }
  }

  const RS = [
    "#![no_std]",
    "#![no_main]",
    "// docs: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description-more-and-more-padding-to-make-it-really-really-long>",
    "fn helper_3() -> u32 { 3 }",
    "fn helper_4() -> u32 { 4 }",
    "fn helper_5() -> u32 { 5 }",
    "fn helper_6() -> u32 { 6 }",
    "fn helper_7() -> u32 { 7 }",
    "fn helper_8() -> u32 { 8 }",
    "fn helper_9() -> u32 { 9 }",
    "fn helper_10() -> u32 { 10 }",
    "fn helper_11() -> u32 { 11 }",
    "fn helper_12() -> u32 { 12 }",
    "fn helper_13() -> u32 { 13 }",
    "fn helper_14() -> u32 { 14 }",
    "fn helper_15() -> u32 { 15 }",
    "fn helper_16() -> u32 { 16 }",
    "fn helper_17() -> u32 { 17 }",
    "fn helper_18() -> u32 { 18 }",
    "fn helper_19() -> u32 { 19 }",
    "fn helper_20() -> u32 { 20 }",
    "fn helper_21() -> u32 { 21 }",
    "fn helper_22() -> u32 { 22 }",
    "fn helper_23() -> u32 { 23 }",
    "fn helper_24() -> u32 { 24 }",
    "fn helper_25() -> u32 { 25 }",
    "fn helper_26() -> u32 { 26 }",
    "fn helper_27() -> u32 { 27 }",
    "fn helper_28() -> u32 { 28 }",
    "fn helper_29() -> u32 { 29 }",
    "fn helper_30() -> u32 { 30 }",
    "fn helper_31() -> u32 { 31 }",
    "fn helper_32() -> u32 { 32 }",
    "fn helper_33() -> u32 { 33 }",
    "fn helper_34() -> u32 { 34 }",
    "fn helper_35() -> u32 { 35 }",
    "fn helper_36() -> u32 { 36 }",
    "fn helper_37() -> u32 { 37 }",
    "fn helper_38() -> u32 { 38 }",
    "fn helper_39() -> u32 { 39 }",
    "fn helper_40() -> u32 { 40 }",
    "fn helper_41() -> u32 { 41 }",
    "fn helper_42() -> u32 { 42 }",
    "fn helper_43() -> u32 { 43 }",
    "fn helper_44() -> u32 { 44 }",
    "fn helper_45() -> u32 { 45 }",
    "fn main() {",
    "    let radio = Radio::new();",
    "    radio",
    "}",
    "",
    "struct Radio;",
    "",
    "impl Radio {",
    "    fn new() -> Self { Radio }",
    "    fn frobnicate(&self) -> u32 { 42 }",
    "    fn free_heap(&self) -> usize { 0 }",
    "    fn flags(&self) -> u8 { 0 }",
    "}",
    "",
  ].join("\n");

  const TOML = ['[package]', 'name = "firmware"', 'version = "0.1.0"', ''].join("\n");
  const plain = (text) => ({ spans: text.length ? [{ text, token: "plain" }] : [] });
  const docOf = (path, text) => ({
    path,
    lines: text.split("\n").map(plain),
    text,
    language: "rust",
    binary: false,
    truncated: false,
    readOnly: false,
  });

  const ITEMS = [
    { label: "frobnicate", kind: "method", detail: "fn frobnicate(&self) -> u32", insert: "frobnicate()", edit: null },
    { label: "free_heap", kind: "method", detail: "fn free_heap(&self) -> usize", insert: "free_heap()", edit: null },
    { label: "flags", kind: "method", detail: "fn flags(&self) -> u8", insert: "flags()", edit: null },
    { label: "new", kind: "assoc fn", detail: "fn new() -> Self", insert: "new()", edit: null },
  ];

  const ROOT = "E:\\mock\\firmware";
  // Tree paths are project-relative and /-separated, exactly as the real
  // tree.rs builds them — the paths are identities the reveal flow compares.
  const MAIN = "src/main.rs";
  window.__mock = { completes: [], changes: [], calls: [], signatures: [], saved: {}, searches: [], trees: [], traces: [] };

  const handlers = {
    recent_projects: () => [ROOT],
    storage_location: () => ({ path: "C:\\mock\\rusty-data", isDefault: true, envOverride: false }),
    open_project: () => ({
      project: {
        root: ROOT, chip: "esp32c3", chipSource: "target triple", runtime: null,
        configuredTarget: "riscv32imc-unknown-none-elf", configuredToolchain: null,
        frameworks: ["esp-hal"], usesDefmt: false, usesEmbassy: false, evidence: [], problems: [],
      },
      workspace: null,
      workspaceError: "mock: no cargo here",
    }),
    project_status: () => handlers.open_project().project,
    project_path: () => ROOT,
    file_tree: (a) => {
      window.__mock.trees.push(a);
      return [
      { name: "src", path: "src", isDir: true, children: [
        { name: "main.rs", path: MAIN, isDir: false, children: [] },
      ]},
      { name: "Cargo.toml", path: "Cargo.toml", isDir: false, children: [] },
      ];
    },
    // Stateful, as the disk is: what save wrote is what open reads back.
    // Without this, format-on-save looks broken in the mock — the re-read
    // "restores" pre-format text no real backend would still have.
    open_file: (a) => {
      const fallback = a.path.endsWith("Cargo.toml") ? TOML : RS;
      return docOf(a.path, window.__mock.saved[a.path] || fallback);
    },
    highlight_text: (a) => docOf(a.path || MAIN, a.text).lines,
    save_file: (a) => { window.__mock.saved[a.path] = a.text; return null; },
    // The real command is a long-lived stream; resolving would read as "the
    // server exited" and flip Ready back to Off.
    lsp_start: (a) => { window.__mock.lspChannel = a.onEvent; a.onEvent.send({ event: "ready" }); return new Promise(() => {}); },
    lsp_open: () => null,
    lsp_saved: () => null,
    lsp_change: (a) => { window.__mock.changes.push(a); return null; },
    lsp_complete: (a) => { window.__mock.completes.push(a); return ITEMS; },
    lsp_hover: (a) => ({
      text: "```rust\npub struct Radio {\n    gain: u32,\n}\n```\n---\nA struct providing radio control. See `Radio::new()`.",
      range: { startLine: a.line, startCol: 4, endLine: a.line, endCol: 9 },
    }),
    lsp_definition: () => null,
    lsp_code_actions: (a) => [{
      title: "Import `std::collections::HashMap`",
      kind: "quickfix",
      edits: [{ range: { startLine: 0, startCol: 0, endLine: 0, endCol: 0 }, newText: ["use std::collections::HashMap;", "", ""].join("\n") }],
    }],
    lsp_semantic: () => [
      { line: 47, startCol: 8, length: 5, kind: "variable" },
      { line: 51, startCol: 7, length: 5, kind: "struct" },
    ],
    lsp_signature: (a) => { window.__mock.signatures.push(a); return {
      label: "fn mix(&self, gain: u32, bias: i32) -> u32",
      paramStart: 25, paramEnd: 34, doc: "Blends the two inputs.",
    }; },
    format_text: (a) => ({ text: a.text + "// formatted\n", changed: true }),
    search_project: (a) => {
      window.__mock.searches.push(a);
      const all = [
        { path: "src/main.rs", line: 1, col: 8, text: "    let radio = Radio::new();", spanStart: 8, spanEnd: 13 },
        { path: "src/main.rs", line: 5, col: 7, text: "struct Radio;", spanStart: 7, spanEnd: 12 },
        { path: "Cargo.toml", line: 0, col: 0, text: "radio = \"0.1\"", spanStart: 0, spanEnd: 5 },
      ];
      if (a.regex === undefined) return { hits: [], files: 0, truncated: false, error: "mock: regex arg missing" };
      const hits = a.include && a.include.includes(".rs")
        ? all.filter((h) => h.path.endsWith(".rs"))
        : all;
      return { hits, files: new Set(hits.map((h) => h.path)).size, truncated: false, error: null };
    },
    plan_simulation: () => ({
      supported: true, reason: null, missing: [],
      steps: [{ program: "cargo", args: ["build"], display: "cargo build --release", rationale: "builds it" }],
      board: {
        chip: "esp32", kitX: 460, kitY: 40,
        leds: [{ pin: 26, color: "green", label: "GPIO26", x: 60, y: 40, routes: [] }],
        buttons: [], rgbs: [], sevens: [], displays: [], pots: [],
      },
      parts: [],
      debug: { gdbCommand: "echo mock-gdb" },
      debugTool: null,
    }),
    save_sim_board: (a) => { window.__mock.savedBoard = a.board; return null; },
    run_simulation: (a) => { window.__mock.simChannel = a.onLine; return new Promise(() => {}); },
    // One cargo-style warning so the Output panel's location links can be
    // exercised: the ` --> path:line:col` must render as a click-to-open.
    run_command: (a) => new Promise((resolve) => {
      const send = (text) => a.onLine.send({ stream: "stderr", text, level: null });
      send("warning: unused variable: `state`");
      send("  --> src\\bin\\main.rs:62:33");
      resolve(0);
    }),
    save_sim_trace: (a) => { window.__mock.traces.push(a.text); return "E:\mock\firmware\target\rusty-sim\trace.vcd"; },
    toolchain_report: () => ({ tools: [], targets: [], problems: [] }),
    firmware_list: () => [],
    chip_catalogue: () => [],
    board_catalogue: () => [],
    catalog_problems: () => [],
    wizard_options: () => [],
    ai_presets: () => [],
    ai_tools: () => [],
    window_is_maximized: () => false,
    window_minimize: () => null,
    window_toggle_maximize: () => null,
    terminal_close: () => null,
  };

  window.__TAURI__ = {
    core: {
      Channel,
      invoke: (cmd, args) => {
        window.__mock.calls.push(cmd);
        const handler = handlers[cmd];
        if (!handler) {
          console.warn("[mock] unhandled:", cmd, args);
          return Promise.reject({ message: "mock: no handler for " + cmd });
        }
        try { return Promise.resolve(handler(args || {})); }
        catch (error) { return Promise.reject({ message: String(error) }); }
      },
    },
    event: { listen: () => Promise.resolve(() => {}) },
    dialog: { open: () => Promise.resolve(null) },
  };
})();
