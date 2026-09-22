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

  // The pin sets the two mocked chips actually have. Without them the
  // devkit draws rails and nothing else — `kit_rows` derives the header
  // from the die — and every wire to a GPIO is reported as reaching a pin
  // that is not there, which is a correct complaint about a mock that had
  // not said what the part is.
  const ESP32_GPIO = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
    19, 20, 21, 22, 23, 25, 26, 27, 32, 33, 34, 35, 36, 37, 38, 39];
  const C3_GPIO = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21];

  // The three symbols the mock board is drawn from, in KiCad's own units:
  // millimetres, y up, `at` the connection point and `angle` pointing from
  // it *into* the body. Enough of the real thing that the sheet's geometry,
  // its nets and the solver all behave as they do in the app — a resistor
  // whose pins were not 2.54 apart would snap to a different grid here than
  // there, which is the kind of difference that makes a mock lie.
  const pin = (number, name, x, angle) => ({
    number, name, kind: "passive", at: [x, 0], length: 2.54, angle,
  });
  const MOCK_SYMBOLS = [
    {
      library: "Device", name: "R", reference: "R", value: "R",
      pins: [pin("1", "~", -3.81, 270), pin("2", "~", 3.81, 90)],
      graphics: [
        { kind: "rectangle", start: [-1.016, 2.54], end: [1.016, -2.54], width: 0.254, fill: "none" },
      ],
    },
    // A lamp and a button, the two parts every first board has — and what
    // the tangle below is made of.
    {
      library: "Device", name: "LED", reference: "D", value: "LED",
      pins: [pin("1", "K", -3.81, 0), pin("2", "A", 3.81, 180)],
      graphics: [
        { kind: "polyline", points: [[-1.27, -1.27], [-1.27, 1.27]], width: 0.254, fill: "none" },
      ],
    },
    {
      library: "Device", name: "SW_Push", reference: "SW", value: "SW_Push",
      pins: [pin("1", "1", -3.81, 0), pin("2", "2", 3.81, 180)],
      graphics: [
        { kind: "polyline", points: [[-1.27, 0], [1.27, 0]], width: 0.254, fill: "none" },
      ],
    },
    {
      library: "rusty", name: "GND", reference: "#PWR", value: "GND",
      pins: [pin("1", "GND", 0, 90)],
      graphics: [
        { kind: "polyline", points: [[-1.27, -2.54], [1.27, -2.54]], width: 0.254, fill: "none" },
      ],
    },
    {
      library: "rusty", name: "Supply", reference: "#PWR", value: "3V3",
      pins: [pin("1", "VCC", 0, 270)],
      graphics: [
        { kind: "polyline", points: [[-1.27, 2.54], [0, 3.81], [1.27, 2.54]], width: 0.254, fill: "none" },
      ],
    },
    // The three parts whose face is drawn from the protocol rather than
    // from a level on a pin: bytes on the bus, bytes on a wire, and a duty
    // with a frequency beside it. Without them the only way to exercise any
    // of that here is to guess at the app.
    {
      library: "rusty", name: "Display", reference: "DS", value: "Display",
      pins: [pin("1", "SDA", -7.62, 0), pin("2", "SCL", -7.62, 0),
             pin("3", "VCC", 0, 270), pin("4", "GND", 0, 90)],
      graphics: [
        { kind: "rectangle", start: [-5.08, 3.81], end: [5.08, -3.81], width: 0.254, fill: "background" },
      ],
    },
    {
      library: "rusty", name: "Strip", reference: "D", value: "WS2812",
      pins: [pin("1", "DIN", -10.16, 0), pin("2", "VCC", 0, 270),
             pin("3", "GND", 0, 90), pin("4", "DOUT", 10.16, 180)],
      graphics: [
        { kind: "rectangle", start: [-7.62, 2.54], end: [7.62, -2.54], width: 0.254, fill: "background" },
      ],
    },
    {
      library: "rusty", name: "Servo", reference: "M", value: "SG90",
      pins: [pin("1", "SIG", -7.62, 0), pin("2", "VCC", 0, 270),
             pin("3", "GND", 0, 90)],
      graphics: [
        { kind: "rectangle", start: [-5.08, 3.81], end: [5.08, -3.81], width: 0.254, fill: "background" },
      ],
    },
    // A lens that mixes three duties, so a colour set by PWM can be watched:
    // its pins where the shipped library puts them.
    {
      library: "rusty", name: "RGB_LED", reference: "D", value: "RGB",
      pins: [
        { number: "1", name: "R", kind: "passive", at: [-7.62, 2.54], length: 2.54, angle: 0 },
        { number: "2", name: "G", kind: "passive", at: [-7.62, 0], length: 2.54, angle: 0 },
        { number: "3", name: "B", kind: "passive", at: [-7.62, -2.54], length: 2.54, angle: 0 },
        { number: "4", name: "COM", kind: "passive", at: [7.62, 0], length: 2.54, angle: 180 },
      ],
      graphics: [
        { kind: "rectangle", start: [-5.08, 3.81], end: [5.08, -3.81], width: 0.254, fill: "background" },
      ],
    },
  ];

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
    // A test module, so the gutter's run arrows and its fold chevrons both
    // have something to point at. Without one the two newest decorations in
    // the margin cannot be exercised here at all.
    "#[cfg(test)]",
    "mod tests {",
    "    use super::*;",
    "",
    "    #[test]",
    "    fn a_radio_frobnicates() {",
    "        assert_eq!(Radio::new().frobnicate(), 42);",
    "    }",
    "",
    "    #[test]",
    "    fn a_radio_starts_with_no_flags() {",
    "        assert_eq!(Radio::new().flags(), 0);",
    "    }",
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
    paint: 1,
    readOnly: false,
  });

  const ITEMS = [
    { label: "frobnicate", kind: "method", detail: "fn frobnicate(&self) -> u32", insert: "frobnicate()", edit: null },
    { label: "free_heap", kind: "method", detail: "fn free_heap(&self) -> usize", insert: "free_heap()", edit: null },
    { label: "flags", kind: "method", detail: "fn flags(&self) -> u8", insert: "flags()", edit: null },
    { label: "new", kind: "assoc fn", detail: "fn new() -> Self", insert: "new()", edit: null },
  ];

  const ROOT = "E:\\mock\\firmware";
  const projectOf = (playground) => ({
    root: playground ? `C:\\mock\\rusty-data\\playground\\${playground}` : ROOT,
    chip: playground || "esp32c3", chipSource: "target triple", runtime: null,
    configuredTarget: playground === "esp32" ? "xtensa-esp32-none-elf" : "riscv32imc-unknown-none-elf",
    configuredToolchain: null, frameworks: ["esp-hal"], usesDefmt: false, usesEmbassy: false,
    evidence: [], problems: [], playground,
  });
  // The playground's own board, as its template draws it: an LED behind a
  // resistor on the chip's LED pin, and a button to ground on GPIO4.
  const playgroundBoard = (chip) => {
    const led = chip === "esp32" ? "GPIO2" : "GPIO0";
    return {
      chip, kitX: chip === "esp32" ? 160 : 460, kitY: 96,
      parts: [
        { reference: "R1", symbol: "Device:R", value: "220", x: chip === "esp32" ? 440 : 312, y: chip === "esp32" ? 224 : 176, rot: 90 },
        { reference: "D1", symbol: "Device:LED", value: "red", x: chip === "esp32" ? 536 : 216, y: chip === "esp32" ? 224 : 176, props: { vf: "2.0" } },
        { reference: "SW1", symbol: "Device:SW_Push", value: "", x: chip === "esp32" ? 440 : 312, y: chip === "esp32" ? 304 : 256 },
      ],
      // Both devkits have a ground on each side, which only its number
      // names: the C3's parts sit to its left (row 13), the ESP32's to its
      // right (row 17).
      wires: chip === "esp32" ? [
        { from: { part: "U1", pin: led }, to: { part: "R1", pin: "1" }, bends: [] },
        { from: { part: "R1", pin: "2" }, to: { part: "D1", pin: "A" }, bends: [] },
        { from: { part: "D1", pin: "K" }, to: { part: "U1", pin: "17" }, bends: [] },
        { from: { part: "U1", pin: "GPIO4" }, to: { part: "SW1", pin: "1" }, bends: [] },
        { from: { part: "SW1", pin: "2" }, to: { part: "U1", pin: "17" }, bends: [] },
      ] : [
        { from: { part: "U1", pin: led }, to: { part: "R1", pin: "1" }, bends: [] },
        { from: { part: "R1", pin: "2" }, to: { part: "D1", pin: "A" }, bends: [] },
        { from: { part: "D1", pin: "K" }, to: { part: "U1", pin: "13" }, bends: [] },
        { from: { part: "U1", pin: "GPIO4" }, to: { part: "SW1", pin: "2" }, bends: [] },
        { from: { part: "SW1", pin: "1" }, to: { part: "U1", pin: "13" }, bends: [] },
      ],
      symbols: MOCK_SYMBOLS,
    };
  };
  // Tree paths are project-relative and /-separated, exactly as the real
  // tree.rs builds them — the paths are identities the reveal flow compares.
  const MAIN = "src/main.rs";
  const tool = (name, purpose, installed, required, install) => ({
    name, purpose, version: installed ? `${name} 1.0.0` : null,
    path: installed ? `C:/Users/mock/.cargo/bin/${name}.exe` : null,
    installCommand: install, installable: name !== "rustup", required,
  });
  const TOOLCHAIN = {
    status: {
      toolchains: [{ name: "stable", isDefault: true, isEsp: false }],
      installedTargets: ["x86_64-pc-windows-msvc"],
      tools: [
        tool("rustup", "Manages Rust toolchains and targets", true, true, "https://rustup.rs"),
        tool("espflash", "Flashes and monitors over USB serial", false, true, "cargo install espflash --locked"),
        tool("rust-analyzer", "Completion, diagnostics and navigation", true, true, "rustup component add rust-analyzer"),
        tool("probe-rs", "Flashes and debugs through a JTAG/SWD probe", false, false, "cargo install probe-rs-tools --locked"),
        tool("qemu-system-riscv32", "Runs the firmware without a board", false, false, "downloaded by rusty"),
      ],
      hasEspToolchain: false,
    },
    requiredTarget: "riscv32imc-unknown-none-elf",
    requiredTargetInstalled: false,
    needsEspToolchain: false,
    problems: [],
  };

  window.__mock = {
    locale: null, toolchain: TOOLCHAIN, installs: [], completes: [], changes: [], calls: [],
    signatures: [], saved: {}, searches: [], trees: [], traces: [], created: [], sent: [], params: {},
    runs: [], buildFails: false, playground: null, resets: 0, simRuns: [],
    // One board and a port that is not one: Flash picks the board by itself.
    ports: [
      { name: "COM3", bridge: "CP210x", boards: ["ESP32-C3-DevKitM-1"], likelyBoard: true, usb: null },
      { name: "COM1", bridge: null, boards: [], likelyBoard: false, usb: null },
    ],
  };

  // Every construct the renderer claims to handle, so the preview can be
  // looked at rather than reasoned about.
  const MD = [
    "# rusty",
    "",
    "An embedded Rust workbench. **ESP32 first**, STM32 next.",
    "",
    "## Crates",
    "",
    "| Crate | Does |",
    "|---|---|",
    "| `rusty-core` | Cargo workspace analysis |",
    "| `rusty-embed` | Chips, boards, flashing |",
    "",
    "### Getting started",
    "",
    "1. Install the toolchain",
    "2. Open a project",
    "   - the chip is detected",
    "   - the target is checked",
    "3. Press Run",
    "",
    "> A tool you cannot read a file in is a dashboard about work you do",
    "> somewhere else.",
    "",
    "```bash",
    "cargo test --workspace",
    "```",
    "",
    "See [the docs](https://example.test/docs) and ~~the old ones~~.",
    "",
    "![a badge](https://example.test/badge.svg)",
    "",
    "---",
    "",
    "#### Notes",
    "",
    "- [x] markdown renders",
    "- [ ] everything else",
  ].join("\n");

  const handlers = {
    // `mock.norecents` in localStorage starts the window on the welcome
    // screen, which a launch that reopens the last project never shows.
    recent_projects: () => (localStorage.getItem("mock.norecents") ? [] : [ROOT]),
    // Stored across a reload, because that is the whole mechanism: choosing a
    // language saves it and reloads into it. Held in memory this reads as a
    // setting the app ignores, which is what it looked like the first time.
    display_locale: () => localStorage.getItem("mock.locale"),
    set_display_locale: (a) => {
      if (a.tag) localStorage.setItem("mock.locale", a.tag);
      else localStorage.removeItem("mock.locale");
      return null;
    },
    storage_location: () => ({ path: "C:\\mock\\rusty-data", isDefault: true, envOverride: false }),
    open_project: () => {
      window.__mock.playground = null;
      window.__mock.opened = true;
      return { project: projectOf(null), workspace: null, workspaceError: "mock: no cargo here" };
    },
    // A playground is a project the backend keeps per chip; the one thing
    // that marks it is `playground`, which lays the window out code beside
    // board. `reset` forgets what was saved, as the real one rewrites it.
    open_playground: (a) => {
      window.__mock.playground = a.chip;
      window.__mock.opened = true;
      return { project: projectOf(a.chip) };
    },
    reset_playground: () => { window.__mock.saved = {}; window.__mock.savedBoard = null; window.__mock.resets += 1; return null; },
    keep_playground: (a) => { window.__mock.kept = a; return a.dest; },
    project_status: () => projectOf(window.__mock.playground),
    // Nothing held before anything is opened, as the backend answers at a
    // fresh launch — or the welcome screen could never be reached here.
    project_path: () => (localStorage.getItem("mock.norecents") && !window.__mock.opened
      ? null : projectOf(window.__mock.playground).root),
    file_tree: (a) => {
      window.__mock.trees.push(a);
      return [
      { name: "src", path: "src", isDir: true, children: [
        { name: "main.rs", path: MAIN, isDir: false, children: [] },
      ]},
      { name: "Cargo.toml", path: "Cargo.toml", isDir: false, children: [] },
      { name: "README.md", path: "README.md", isDir: false, children: [] },
      ];
    },
    // Stateful, as the disk is: what save wrote is what open reads back.
    // Without this, format-on-save looks broken in the mock — the re-read
    // "restores" pre-format text no real backend would still have.
    open_file: (a) => {
      let fallback = RS;
      if (a.path.endsWith("Cargo.toml")) fallback = TOML;
      if (a.path.endsWith(".md")) fallback = MD;
      return docOf(a.path, window.__mock.saved[a.path] || fallback);
    },
    // Every repaint answered whole, which is always a correct answer.
    repaint_text: (a) => ({ version: 1, from: 0, lines: docOf(a.path || MAIN, a.text).lines }),
    // A picture for any path, base64 as the real command answers it: one
    // small SVG carrying the path it stands for, so a page's figures and
    // the image view can be driven without a project on disk.
    read_blob: (a) => btoa(
      '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="160">' +
      '<rect width="320" height="160" rx="12" fill="#2a3a4a"/>' +
      '<text x="160" y="86" text-anchor="middle" fill="#e0b080" font-size="14" font-family="monospace">' +
      a.path + '</text></svg>'
    ),
    save_file: (a) => { window.__mock.saved[a.path] = a.text; return null; },
    // The real command is a long-lived stream; resolving would read as "the
    // server exited" and flip Ready back to Off.
    // The connectivity check's amber outcome: reached, but the model named is
    // not one the endpoint lists. The verdict is worded by the frontend.
    ai_check_provider: (a) => ({ verdict: "reachable", model: a.config.model, modelsListed: 3, modelListed: false }),
    pin_report: () => ({ chip: "esp32c3", pins: [], source: null, note: null, unknown: [] }),
    ai_cancel: () => null,
    lsp_start: (a) => { window.__mock.lspChannel = a.onEvent; a.onEvent.send({ event: "ready" }); return new Promise(() => {}); },
    // Also long-lived. The channel is kept so a change can be injected by
    // hand — `__mock.watchChannel.send({changed: ["src/main.rs"], tree: false})`
    // is how the follow-the-disk path is exercised without a disk.
    watch_project: (a) => { window.__mock.watchChannel = a.onChange; return new Promise(() => {}); },
    // The environment check's queue runs one of these per missing tool and
    // chains on the exit code, so a resolved promise is the contract here —
    // unlike the streams above, this one *must* end. `__mock.installs`
    // records the order, which is the thing worth asserting.
    // `__mock.installDelay` (ms) holds each install open, so the progress a
    // page draws while one runs can be looked at rather than raced.
    install_sim_tool: (a) => new Promise((resolve) => {
      window.__mock.installs.push(a.name);
      a.onLine.send({ stream: "stdout", text: `$ installing ${a.name}`, level: null });
      setTimeout(() => {
        // Mark it present, so the re-probe afterwards reflects the install
        // and the screen empties the way it would against a real machine.
        const found = window.__mock.toolchain.status.tools.find((t) => t.name === a.name);
        if (found) found.path = `C:/Users/mock/.cargo/bin/${a.name}.exe`;
        resolve(0);
      }, window.__mock.installDelay || 0);
    }),
    lsp_open: () => null,
    lsp_saved: () => null,
    lsp_close: () => null,
    lsp_change: (a) => { window.__mock.changes.push(a); return null; },
    // Edits carry the range as of *this* request, which is what the real
    // server does and what the stale-range bug depended on: ask while two
    // characters are typed and the range covers two, however many more
    // arrive before the item is accepted.
    lsp_complete: (a) => {
      window.__mock.completes.push(a);
      const start = a.col - 2;
      const items = ITEMS.map((i, index) => ({ ...i, index, edit: { startLine: a.line, startCol: start, endLine: a.line, endCol: a.col, newText: i.insert } }));
      return { items, incomplete: false, reply: 1 };
    },
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
    // Navigation answers nothing in the browser: an empty list is what a
    // server still loading says, and the finder shows it as such.
    lsp_references: () => [],
    lsp_implementations: () => [],
    lsp_type_definition: () => [],
    lsp_highlights: () => [],
    lsp_document_symbols: () => [],
    lsp_workspace_symbols: () => [],
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
    // Stateful like the disk is: a replace that reported a count without
    // changing what the next search finds would look right in the mock and
    // wrong in the app.
    replace_in_project: (outer) => {
      // One struct, matching the command's single `args` parameter — a flat
      // payload here would deserialise to nothing and the reply would be an
      // error about a missing field rather than about the shape.
      const a = outer.args;
      if (!a || a.drafts === undefined) {
        return { changed: [], replaced: 0, skipped: [], error: "mock: args.drafts missing" };
      }
      const skipped = a.drafts.map((path) => ({ path, reason: "unsaved" }));
      const changed = ["src/main.rs", "Cargo.toml"].filter((p) => !a.drafts.includes(p));
      window.__mock.replaced = a;
      return { changed, replaced: changed.length * 2, skipped, error: null };
    },
    // The sheet as it was last saved, like `.rusty/sim.toml` is: Run saves
    // the board and plans again, and a plan that answered with the example
    // every time put a resistor changed to 1k straight back to 220. The
    // symbols come from the library on the way back, as the planner's do —
    // what is saved is the file, and the file carries none.
    plan_simulation: () => window.__mock.playground ? ({
      supported: true, reason: null, missing: [],
      steps: [{ program: "cargo", args: ["build"], display: "cargo build --release", rationale: "builds it" }],
      board: (window.__mock.savedBoard && window.__mock.savedBoard.chip === window.__mock.playground)
        ? { ...window.__mock.savedBoard, symbols: MOCK_SYMBOLS } : playgroundBoard(window.__mock.playground),
      library: MOCK_SYMBOLS,
      parts: [],
      debug: { gdbCommand: "echo mock-gdb", elf: "target/x/playground", port: 1234 },
      debugTool: null,
    }) : ({
      supported: true, reason: null, missing: [],
      steps: [{ program: "cargo", args: ["build"], display: "cargo build --release", rationale: "builds it" }],
      // A sheet with a *circuit* on it, not an empty one. This used to be
      // the first board's `leds`/`buttons` shape, which the wire model left
      // behind two formats ago — serde ignored every key of it and the
      // panel opened on a bare devkit, so nothing in the sim panel could be
      // driven here at all. A divider is the smallest board that exercises
      // the whole of it: two resistors, both rails, a tap on a GPIO, and an
      // answer anybody can check (3.3 × 10/30 = 1.1 V).
      board: {
        chip: "esp32c3", kitX: 460, kitY: 40,
        parts: [
          { reference: "R1", symbol: "Device:R", value: "20k", x: 200, y: 120 },
          { reference: "R2", symbol: "Device:R", value: "10k", x: 200, y: 240 },
          { reference: "PWR1", symbol: "rusty:Supply", value: "3V3", x: 200, y: 40 },
          { reference: "GND1", symbol: "rusty:GND", value: "GND", x: 200, y: 330 },
          // A screen that says which controller is behind its glass, which
          // is what makes its writes readable as a picture; a strip on the
          // pin RMT reports; and a servo on the pin LEDC does.
          {
            reference: "DS1", symbol: "rusty:Display", value: "Display",
            x: 640, y: 120, props: { addr: "3c", panel: "ssd1306" },
          },
          { reference: "D1", symbol: "rusty:Strip", value: "WS2812 x8", x: 640, y: 260 },
          { reference: "M1", symbol: "rusty:Servo", value: "SG90", x: 640, y: 360 },
          // Four parts dropped in one square inch and wired to pins in the
          // opposite order — the tangle the layout work is about, kept here
          // so `Tidy` can be driven against something that needs it. It is
          // what an import from another editor's canvas looks like.
          { reference: "D2", symbol: "Device:LED", value: "red", x: 300, y: 150 },
          { reference: "D3", symbol: "Device:LED", value: "green", x: 312, y: 158 },
          { reference: "SW1", symbol: "Device:SW_Push", value: "", x: 324, y: 166 },
          { reference: "SW2", symbol: "Device:SW_Push", value: "", x: 336, y: 174 },
        ],
        wires: [
          { from: { part: "DS1", pin: "SDA" }, to: { part: "U1", pin: "GPIO5" }, bends: [] },
          { from: { part: "DS1", pin: "SCL" }, to: { part: "U1", pin: "GPIO6" }, bends: [] },
          { from: { part: "D1", pin: "DIN" }, to: { part: "U1", pin: "GPIO8" }, bends: [] },
          { from: { part: "M1", pin: "SIG" }, to: { part: "U1", pin: "GPIO7" }, bends: [] },
          { from: { part: "D2", pin: "K" }, to: { part: "U1", pin: "GPIO21" }, bends: [] },
          { from: { part: "D3", pin: "K" }, to: { part: "U1", pin: "GPIO19" }, bends: [] },
          { from: { part: "SW1", pin: "1" }, to: { part: "U1", pin: "GPIO18" }, bends: [] },
          { from: { part: "SW2", pin: "1" }, to: { part: "U1", pin: "GPIO10" }, bends: [] },
          { from: { part: "PWR1", pin: "VCC" }, to: { part: "R1", pin: "1" }, bends: [] },
          { from: { part: "R1", pin: "2" }, to: { part: "R2", pin: "1" }, bends: [] },
          { from: { part: "R2", pin: "1" }, to: { part: "U1", pin: "GPIO4" }, bends: [] },
          { from: { part: "R2", pin: "2" }, to: { part: "GND1", pin: "GND" }, bends: [] },
        ],
        symbols: MOCK_SYMBOLS,
      },
      library: MOCK_SYMBOLS,
      parts: [],
      debug: { gdbCommand: "echo mock-gdb", elf: "target/x/blinky", port: 1234 },
      debugTool: null,
    }),
    save_sim_board: (a) => { window.__mock.savedBoard = a.board; return null; },
    run_simulation: (a) => {
      const m = window.__mock;
      m.simChannel = a.onLine;
      // What the playground's saved code was, as each run found it: the
      // check that Run and Restart write the editor first.
      m.simRuns.push({ debug: a.debug, saved: { ...m.saved } });
      // Debug runs freeze the boot and say so; the frontend's hook on that
      // line is what starts the in-app debugger.
      if (a.debug) setTimeout(() => a.onLine.send({ stream: "stdout", text: "[rusty:debug] frozen at reset", level: null }), 40);
      // A playground's firmware says hello and blinks its LED, so the board
      // beside the code has something to show. With `__mock.breathe` set it
      // breathes the LED instead, through the LED controller's report — the
      // line rusty's QEMU writes for LEDC, carrier and all — so the glow a
      // duty draws can be watched without an emulator.
      if (m.playground && !a.debug) {
        const pin = m.playground === "esp32" ? 2 : 0;
        let on = false;
        let step = 0;
        a.onLine.send({ stream: "stdout", text: "Hello from the playground!", level: null });
        m.simTimer = setInterval(() => {
          if (m.breathe) {
            step = (step + 1) % 60;
            const duty = (1 - Math.cos((step / 60) * 2 * Math.PI)) / 2;
            a.onLine.send({ stream: "stdout", text: `[rusty:pwm@${step * 50000}] ${pin}=${duty.toFixed(4)}@24000.0`, level: null });
            return;
          }
          on = !on;
          a.onLine.send({ stream: "stdout", text: `[rusty:gpio] ${pin}=${on ? 1 : 0}`, level: null });
        }, m.breathe ? 50 : 400);
      }
      // QEMU runs until something stops it, so this resolves only when
      // something does — the Stop button, or the debugger going away.
      return new Promise((resolve) => { m.simResolve = resolve; });
    },
    // One cargo-style warning so the Output panel's location links can be
    // exercised: the ` --> path:line:col` must render as a click-to-open.
    run_command: (a) => new Promise((resolve) => {
      const send = (text) => a.onLine.send({ stream: "stderr", text, level: null });
      // `rustup target add` is the one command whose effect the next probe
      // has to see: without it the environment check installs everything,
      // re-probes, and still lists the target — which is the mock lying
      // about a flow that works.
      if (a.program === "rustup" && a.args[0] === "target") {
        window.__mock.toolchain.status.installedTargets.push(a.args[2]);
        window.__mock.toolchain.requiredTargetInstalled = true;
        send(`info: installing component for ${a.args[2]}`);
        resolve(0);
        return;
      }
      send("warning: unused variable: `state`");
      send("  --> src\\bin\\main.rs:62:33");
      resolve(0);
    }),
    save_sim_trace: (a) => { window.__mock.traces.push(a.text); return "E:\mock\firmware\target\rusty-sim\trace.vcd"; },
    // A board on the end of a port rusty holds open: it announces its
    // tunables, streams telemetry, and — the part that matters — answers a
    // write with what it took. A mock that swallowed writes would make the
    // sliders look correct while proving nothing about the round trip.
    serial_link: (a) => {
      const m = window.__mock;
      m.linkChannel = a.onLine;
      m.params = { kp: 2, setpoint: 50 };
      const say = (text) => a.onLine.send({ stream: "stdout", text, level: null });
      say("[rusty:param] kp=2 0..20");
      say("[rusty:param] setpoint=50 0..100");
      let at = 0;
      let measured = 0;
      m.linkTimer = setInterval(() => {
        at += 20000;
        measured += (m.params.setpoint - measured) * 0.08 * m.params.kp;
        say(`[rusty:tel@${at}] setpoint=${m.params.setpoint},measured=${measured.toFixed(2)}`);
      }, 40);
      return new Promise((resolve) => { m.linkResolve = resolve; });
    },
    sim_send: (a) => {
      const m = window.__mock;
      m.sent.push(a.text);
      // The firmware's half of the contract: clamp, then re-announce.
      const set = /^S([A-Za-z_][\w]*)=(-?[\d.]+)$/.exec(a.text || "");
      if (set && m.linkChannel && set[1] in m.params) {
        const bounds = set[1] === "kp" ? [0, 20] : [0, 100];
        const took = Math.min(bounds[1], Math.max(bounds[0], parseFloat(set[2])));
        m.params[set[1]] = took;
        m.linkChannel.send({
          stream: "stdout", level: null,
          text: `[rusty:param] ${set[1]}=${took} ${bounds[0]}..${bounds[1]}`,
        });
      }
      return null;
    },
    stop_flash: () => {
      const m = window.__mock;
      // The emulator is in the same slot as everything else that runs.
      if (m.simTimer) { clearInterval(m.simTimer); m.simTimer = null; }
      if (m.simResolve) { const r = m.simResolve; m.simResolve = null; setTimeout(() => r(null), 30); }
      if (m.linkTimer) { clearInterval(m.linkTimer); m.linkTimer = null; }
      if (m.linkResolve) { m.linkResolve(null); m.linkResolve = null; }
      if (m.flashTimer) { clearInterval(m.flashTimer); m.flashTimer = null; }
      if (m.flashResolve) { m.flashResolve(null); m.flashResolve = null; }
      return null;
    },
    // Every planned command — a build, a flash, a monitor — the way the tools
    // talk: cargo's `Compiling` and its own summary line, espflash's
    // `Flashing has completed!`, then a board printing until something stops
    // it. `__mock.buildFails` makes the build fail with cargo's own counts;
    // `__mock.runs` records what ran, so a driven test can assert the order.
    run_flash: (a) => new Promise((resolve) => {
      const m = window.__mock;
      m.runs.push(a.plan.display);
      const send = (text) => a.onLine.send({ stream: "stderr", text, level: null });
      const script = (lines, then) => {
        let at = 0;
        const tick = () => {
          if (at < lines.length) { send(lines[at++]); setTimeout(tick, 250); } else { then(); }
        };
        tick();
      };
      const watch = () => {
        let n = 0;
        m.flashResolve = resolve;
        m.flashTimer = setInterval(() => send(`hello from the board ${++n}`), 400);
      };
      if (a.plan.program === "cargo") {
        const compiling = ["   Compiling esp-hal v1.1.2", "   Compiling blinky v0.1.0 (E:/mock/blinky)"];
        if (m.buildFails) {
          script([...compiling,
            "error[E0425]: cannot find value `led` in this scope",
            "  --> src/bin/main.rs:21:9",
            "warning: `blinky` (bin \"blinky\") generated 1 warning",
            "error: could not compile `blinky` (bin \"blinky\") due to 1 previous error; 1 warning emitted",
          ], () => resolve(101));
        } else {
          script([...compiling,
            "warning: `blinky` (bin \"blinky\") generated 2 warnings",
            "    Finished `release` profile [optimized] target(s) in 1.52s",
          ], () => resolve(0));
        }
      } else if (a.plan.display.startsWith("espflash monitor")) {
        script(["Commands:", "    CTRL+R    Reset chip", "    CTRL+C    Exit"], watch);
      } else {
        script([
          "[2026-09-21T08:00:00Z INFO ] Serial port: 'COM3'",
          "Chip type:         esp32c3 (revision v0.4)",
          "[00:00:01] [========================================]      13/13      0x10000",
          "Flashing has completed!",
        ], () => (a.plan.display.includes("--monitor") ? watch() : resolve(0)));
      }
    }),
    memory_report: (a) => ({
      elfPath: a.elfPath, chip: "esp32c3", sections: [], crates: [], unattributedBytes: 0,
      totals: { flashBytes: 87342, ramBytes: 20612, ramCapacity: 327680 },
    }),
    editor_view: () => ({ inlayHints: true, minimap: true, stickyScroll: true, indentGuides: true }),
    set_editor_view: () => null,
    auto_save_enabled: () => false,
    set_auto_save: () => null,
    // The real shape, and a machine with holes in it — the empty stub here
    // never matched `ToolchainReport` at all, so the Toolchain panel and the
    // environment check both failed to decode it and showed nothing.
    // `__mock.toolchain` is swappable, so a ready machine can be tested too.
    toolchain_report: () => window.__mock.toolchain,
    // Swappable, like the toolchain: one board is the auto-picked case, two
    // are a question, none is the empty picker.
    serial_ports: () => window.__mock.ports,
    debug_probes: () => [],
    firmware_list: () => [{
      path: "target/xtensa-esp32-none-elf/release/blinky", name: "blinky",
      profile: "release", target: "xtensa-esp32-none-elf", bytes: 1234567,
      modified: 1765600000, matchesConfiguredTarget: true,
    }],
    // The planner's shape for the device and action asked about, and its
    // warning when the port's board is not the project's chip.
    plan_flash: (a) => {
      const port = a.transport.port;
      const elf = a.firmware ? ` ${a.firmware}` : "";
      const display = a.action === "monitor"
        ? `espflash monitor --chip esp32c3 --port ${port}${a.firmware ? " --elf" + elf : ""}`
        : `espflash flash --chip esp32c3 --port ${port}${a.action === "flashAndMonitor" ? " --monitor" : ""}${elf}`;
      const boards = (window.__mock.ports.find((p) => p.name === port) || { boards: [] }).boards;
      const warning = boards.includes("ESP32 DevKit")
        ? "This project builds for esp32c3, but the device on this port looks like esp32."
        : undefined;
      return { program: "espflash", args: [], display, rationale: "", warning };
    },
    chip_catalogue: () => [
      { id: "esp32", name: "ESP32", vendor: "espressif", arch: "xtensa", cores: 2, sramBytes: 520000, flashBytes: null, bareMetalTarget: "xtensa-esp32-none-elf", stdTarget: null, toolchain: "espXtensa", flashers: [], probeRsTarget: null, radios: [], gpio: ESP32_GPIO },
      { id: "esp32c3", name: "ESP32-C3", vendor: "espressif", arch: "riscV", cores: 1, sramBytes: 400000, flashBytes: null, bareMetalTarget: "riscv32imc-unknown-none-elf", stdTarget: null, toolchain: "stock", flashers: [], probeRsTarget: null, radios: [], gpio: C3_GPIO },
      { id: "esp32s3", name: "ESP32-S3", vendor: "espressif", arch: "xtensa", cores: 2, sramBytes: 512000, flashBytes: null, bareMetalTarget: "xtensa-esp32s3-none-elf", stdTarget: null, toolchain: "espXtensa", flashers: [], probeRsTarget: null, radios: [] },
    ],
    board_catalogue: () => [
      { id: "esp32-devkitc", name: "ESP32 DevKit", chip: "esp32", flashBytes: 4194304, psramBytes: null, usb: [], flashBaud: null, pins: [], source: "builtin" },
      { id: "esp32c3-devkitm-1", name: "ESP32-C3-DevKitM-1", chip: "esp32c3", flashBytes: 4194304, psramBytes: null, usb: [], flashBaud: null, pins: [], source: "builtin" },
    ],
    catalog_problems: () => [],
    wizard_options: () => [],
    ai_presets: () => [],
    ai_tools: () => [],
    window_minimize: () => null,
    window_toggle_maximize: () => null,
    terminal_close: () => null,
    window_set_zoom: (a) => { document.documentElement.style.zoom = a.factor; return null; },
    terminal_shells: () => [
      { label: "rusty bash (built-in)", value: "auto" },
      { label: "PowerShell 7", value: "pwsh.exe" },
      { label: "Git Bash", value: "bash.exe" },
    ],
    // A debug session, shaped like a real one: two frames that both have
    // source (clicking the outer one must navigate), and one local whose
    // value is a HAL handle's entire type structure — the row that has to
    // stay readable.
    debug_start: (a) => {
      const stopped = {
        running: false, attached: true, reason: "breakpoint", frame: 0,
        // Both lines exist in the mock's own main.rs, or the reveal clamps to
        // the end of the file and every navigation assertion reads the same
        // number whatever it was asked for.
        stack: [
          { level: 0, function: "blinky::__xtensa_lx_rt_main", file: "src/bin/main.rs", line: 39, address: "0x400d1a2c" },
          { level: 1, function: "blinky::__xtensa_lx_rt_main_trampoline", file: "src/bin/main.rs", line: 11, address: "0x40080f10" },
        ],
        variables: [
          { name: "tick", value: "42", kind: "u32", handle: null, children: 0 },
          { name: "pot", value: "128", kind: "u8", handle: null, children: 0 },
          {
            name: "rx", kind: "esp_hal::uart::UartRx<esp_hal::Blocking>", handle: null, children: 0,
            value: "{uart: esp_hal::uart::AnyUart (esp_hal::uart::any::Inner::Uart0(esp_hal::peripherals::UART0 {_marker: core::marker::PhantomData<*const ()>})), phantom: core::marker::PhantomData<esp_hal::Blocking>, guard: esp_hal::system::PeripheralGuard {peripheral: esp_hal::system::Peripheral::Uart0}}",
          },
        ],
        breakpoints: [{ number: 1, file: "src/bin/main.rs", line: 39, verified: true, reason: null, enabled: true }],
        memory: [],
        error: null, exited: null,
      };
      a.onState.send({ ...stopped, running: true, stack: [], variables: [] });
      setTimeout(() => a.onState.send(stopped), 60);
      window.__mock.debugStarted = a;
      window.__mock.stopped = stopped;
      return new Promise(() => {});
    },
    // Both of these push a fresh state in the real session — gdb answers
    // `-break-insert` and `-stack-select-frame` by relisting frames and
    // variables. Without that here, the panel looks correct in the mock
    // while every such update drags the editor back to frame 0 in the app.
    debug_breakpoint: (a) => {
      (window.__mock.breakpoints = window.__mock.breakpoints || []).push(a);
      window.__mock.debugStarted?.onState.send(window.__mock.stopped);
      return null;
    },
    debug_control: (a) => { (window.__mock.control = window.__mock.control || []).push(a.action); return null; },
    debug_frame: (a) => {
      window.__mock.frame = a.level;
      window.__mock.debugStarted?.onState.send({ ...window.__mock.stopped, frame: a.level });
      return null;
    },
    // Stopping the debugger ends the run it started, as the backend now does:
    // the sim stream resolves, which is what puts the Play button back. A mock
    // that only killed gdb modelled the orphaned-QEMU bug rather than the fix.
    // A chip switch, with the shape the popover has to lay out: a few files
    // and the notes, which are long — they are what decides the width.
    plan_migration: (a) => ({
      from: "esp32", to: a.chip,
      files: [
        { path: ".cargo/config.toml", edits: [{ before: "xtensa-esp32-none-elf", after: "riscv32imc-unknown-none-elf" }, { before: "esp32", after: a.chip }] },
        { path: "rust-toolchain.toml", edits: [{ before: "channel = \"esp\"", after: "channel = \"stable\"" }] },
        { path: "Cargo.toml", edits: [{ before: "esp32", after: a.chip }] },
      ],
      notes: [
        "Pins and peripherals in your source are not touched. ESP32 and ESP32-C3 do not have the same GPIOs, and only your code knows what each one should become — build after switching and the compiler names every site.",
        "This also changes architecture, Xtensa to RISC-V: anything written in assembly, and any interrupt or critical-section code that assumes one of them, needs reading.",
      ],
      blocker: null,
    }),
    apply_migration: () => [".cargo/config.toml", "rust-toolchain.toml", "Cargo.toml"],
    debug_stop: () => {
      window.__mock.simResolve?.(0);
      window.__mock.simResolve = null;
      return null;
    },
    check_update: () => ({
      current: "0.1.0", latest: "0.2.0", newer: true,
      url: "https://github.com/Linshiqi/rusty/releases/tag/v0.2.0", note: null,
    }),
    open_url: () => null,
    terminal_shell_info: () => ({
      active: window.__mock.shellPref === "system" ? "pwsh.exe" : "rusty's built-in shell",
      preference: window.__mock.shellPref || null,
    }),
    set_terminal_shell: (a) => { window.__mock.shellPref = a.value; return null; },
    keybinds: () => window.__mock.keybinds || {},
    set_keybind: (a) => {
      const m = (window.__mock.keybinds = window.__mock.keybinds || {});
      if (a.chord) m[a.id] = a.chord; else delete m[a.id];
      return null;
    },
    // Stateful like the file is: turning Vim on and reloading must come back
    // on, which is the whole reason the switch is not localStorage.
    vim_enabled: () => window.__mock.vim === true,
    set_vim: (a) => { window.__mock.vim = a.enabled; return null; },
    create_entry: (a) => { window.__mock.created.push(a); return null; },
    open_editor_window: (a) => { window.__mock.detached = a.path; return null; },
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
    // `confirm` is what the app has where a browser has `window.confirm`; the
    // mock answers with the real one so a discard can be exercised here.
    // A folder picked when a test sets `__mock.pickFolder`; cancelled otherwise.
    dialog: { open: () => Promise.resolve(window.__mock.pickFolder ?? null), confirm: (m) => Promise.resolve(window.confirm(m)) },
  };
})();
