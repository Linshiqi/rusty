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
  window.__mock = { completes: [], changes: [], calls: [], signatures: [], saved: {}, searches: [], trees: [], traces: [], created: [], sent: [], params: {} };

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
    // Edits carry the range as of *this* request, which is what the real
    // server does and what the stale-range bug depended on: ask while two
    // characters are typed and the range covers two, however many more
    // arrive before the item is accepted.
    lsp_complete: (a) => {
      window.__mock.completes.push(a);
      const start = a.col - 2;
      return ITEMS.map((i) => ({ ...i, edit: { startLine: a.line, startCol: start, endLine: a.line, endCol: a.col, newText: i.insert } }));
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
      debug: { gdbCommand: "echo mock-gdb", elf: "target/x/blinky", port: 1234 },
      debugTool: null,
    }),
    save_sim_board: (a) => { window.__mock.savedBoard = a.board; return null; },
    run_simulation: (a) => {
      window.__mock.simChannel = a.onLine;
      // Debug runs freeze the boot and say so; the frontend's hook on that
      // line is what starts the in-app debugger.
      if (a.debug) setTimeout(() => a.onLine.send({ stream: "stdout", text: "[rusty:debug] frozen at reset", level: null }), 40);
      // QEMU runs until something stops it, so this resolves only when
      // something does — the Stop button, or the debugger going away.
      return new Promise((resolve) => { window.__mock.simResolve = resolve; });
    },
    // One cargo-style warning so the Output panel's location links can be
    // exercised: the ` --> path:line:col` must render as a click-to-open.
    run_command: (a) => new Promise((resolve) => {
      const send = (text) => a.onLine.send({ stream: "stderr", text, level: null });
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
      if (m.linkTimer) { clearInterval(m.linkTimer); m.linkTimer = null; }
      if (m.linkResolve) { m.linkResolve(null); m.linkResolve = null; }
      return null;
    },
    toolchain_report: () => ({ tools: [], targets: [], problems: [] }),
    serial_ports: () => [{ name: "COM3", bridge: "CP210x", boards: ["ESP32 DevKit"], likelyBoard: true, usb: null }],
    debug_probes: () => [],
    firmware_list: () => [{
      path: "target/xtensa-esp32-none-elf/release/blinky", name: "blinky",
      profile: "release", target: "xtensa-esp32-none-elf", bytes: 1234567,
      modified: 1765600000, matchesConfiguredTarget: true,
    }],
    plan_flash: (a) => ({
      program: "espflash", args: ["flash", "--monitor"],
      display: a.action === "monitor" ? "espflash monitor --port COM3"
        : "espflash flash --monitor target/xtensa-esp32-none-elf/release/blinky",
      rationale: "mock: espflash speaks the ROM bootloader on this transport",
    }),
    chip_catalogue: () => [
      { id: "esp32", name: "ESP32", vendor: "espressif", arch: "xtensa", cores: 2, sramBytes: 520000, flashBytes: null, bareMetalTarget: "xtensa-esp32-none-elf", stdTarget: null, toolchain: "espXtensa", flashers: [], probeRsTarget: null, radios: [] },
      { id: "esp32c3", name: "ESP32-C3", vendor: "espressif", arch: "riscV", cores: 1, sramBytes: 400000, flashBytes: null, bareMetalTarget: "riscv32imc-unknown-none-elf", stdTarget: null, toolchain: "stock", flashers: [], probeRsTarget: null, radios: [] },
      { id: "esp32s3", name: "ESP32-S3", vendor: "espressif", arch: "xtensa", cores: 2, sramBytes: 512000, flashBytes: null, bareMetalTarget: "xtensa-esp32s3-none-elf", stdTarget: null, toolchain: "espXtensa", flashers: [], probeRsTarget: null, radios: [] },
    ],
    board_catalogue: () => [],
    catalog_problems: () => [],
    wizard_options: () => [],
    ai_presets: () => [],
    ai_tools: () => [],
    window_is_maximized: () => false,
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
    dialog: { open: () => Promise.resolve(null) },
  };
})();
