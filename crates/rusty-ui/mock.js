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
  const MAIN = ROOT + "\\src\\main.rs";
  window.__mock = { completes: [], changes: [], calls: [] };

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
    file_tree: () => [
      { name: "src", path: ROOT + "\\src", isDir: true, children: [
        { name: "main.rs", path: MAIN, isDir: false, children: [] },
      ]},
      { name: "Cargo.toml", path: ROOT + "\\Cargo.toml", isDir: false, children: [] },
    ],
    open_file: (a) => docOf(a.path, RS),
    highlight_text: (a) => docOf(a.path || MAIN, a.text).lines,
    save_file: () => null,
    // The real command is a long-lived stream; resolving would read as "the
    // server exited" and flip Ready back to Off.
    lsp_start: (a) => { a.onEvent.send({ event: "ready" }); return new Promise(() => {}); },
    lsp_open: () => null,
    lsp_saved: () => null,
    lsp_change: (a) => { window.__mock.changes.push(a); return null; },
    lsp_complete: (a) => { window.__mock.completes.push(a); return ITEMS; },
    lsp_hover: () => null,
    lsp_definition: () => null,
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
