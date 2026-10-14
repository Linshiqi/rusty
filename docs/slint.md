# rusty in Slint

The window is moving off Tauri and the Leptos/WebView frontend onto
[Slint](https://slint.dev), on the `slint` branch. This is the design and
the order of the work.

## Why

- **One process, no wire.** The frontend was wasm in a WebView, talking to
  the backend through a command layer: `rusty-ipc`'s names, `rusty-app`'s
  commands, serde on both sides, the `backend` feature split so model types
  compile to wasm, and `mock.js` to drive the page without the app. In Slint
  the window and the work are one Rust program: a panel calls `rusty_git`
  the way `rusty-cli` does. Every one of those layers goes.
- **The editor's floor was the textarea.** Typing measured 30–45 ms a key at
  388 KB and the editor had to lay a transparent `<textarea>` over a `<pre>`
  and keep the two glyph for glyph (*Large files*, *Code folding* and
  *What the editor draws beside the text* in CLAUDE.md are mostly about
  that). Here the editor places every character itself and Slint draws.
- **Startup, memory and the platform.** No WebView2 runtime, no browser
  process, no 300 ms of wasm instantiation.

## What was measured first

`crates/rusty-desk` started as a spike of the riskiest part, the code
surface, on a 24,706-line, 1 MB Rust file with Chinese comments and tabs,
driven by its own probe (`RUSTY_PROBE=<dir> rusty <root> <file>`: it types,
scrolls, photographs itself through `Window::take_snapshot` and reads each
frame's time off the renderer's notifier):

| | median | p90 | max |
|---|---|---|---|
| a key to the frame that shows it | 14.9 ms | 15.3 ms | 15.9 ms |
| a scroll step to its frame | 16.0 ms | 16.8 ms | 19.3 ms |

Both are the next frame at 60 Hz. The WebView editor took 30–45 ms a key at
a third of that size. Opening is not yet good: the whole file is painted
before the window shows (2.5 s here); painting moves to a thread, the lines
on screen first.

How it is built, because the rest of the editor follows from it:

- **Rust measures and places; Slint draws.** A row is handed to the view as
  pieces — a run of one token's colour and the x it starts at — so the
  view does no text layout, and the caret, a click and every overlay use
  the arithmetic that placed the text (`editor/metrics.rs`). ASCII is one
  width, measured once over a run of 64; anything else is measured the
  first time it is seen, over a run of 16 — measured once, a `中`'s
  rounding was eight pixels by the end of a line. A tab goes to the next
  stop of four.
- **The rows are a `Model` over the document**, so a ListView asks for the
  forty rows on screen and an edit names the rows that changed. Opening a
  big file builds forty rows, not twenty-four thousand.
- **All text arrives through one `TextInput`**, sitting at the caret: a
  key that is not text goes to the editor's handler first and never
  reaches it; a typed character and an input method's committed text
  arrive alike as an edit of the input, which the editor takes and
  clears; while an IME composes, the composition shows there, in place,
  and nothing is intercepted. **Not yet tried with a real Chinese IME** —
  that is the first thing a person at the keyboard has to check.

## The shape of it

```
crates/
  rusty-core, rusty-embed, rusty-lsp, rusty-dbg, rusty-git, rusty-term,
  rusty-edit, rusty-ai, rusty-mcu, rusty-draw, rusty-i18n, rusty-cli
                     — unchanged: the libraries the window and the CLI share
  rusty-desk         — the window
    ui/              — .slint: the window, a theme global, widgets, one
                       file per panel, the editor surface
    src/
      main.rs        — start, services, the event loop
      editor/        — the document, metrics, rows, keys, folds, history…
      <panel>/       — one module per panel, each owning its state and
                       binding its callbacks
      work.rs        — running blocking work off the UI thread and putting
                       the answer back on it
```

The rules:

1. **State lives in Rust; the `.slint` files draw it.** A panel's module
   owns its state, pushes what changed into properties and models, and
   handles the callbacks. No business logic in `.slint`, no `RefCell` held
   across a callback.
2. **The UI thread never blocks.** Anything that touches the disk, a
   process, the network or a language server runs on a worker and comes
   back through `slint::Weak::upgrade_in_event_loop` (`work.rs`). That is
   the whole of what the Tauri command layer did.
3. **Long-running sessions own a slot**, as they did in `AppState`: a new
   terminal, debug session or simulation replaces the old one's slot, and
   the old one's loop ends because its slot is gone.
4. **What a model can compute is a pure function with tests** — the Vim
   machine, the signal lab, the scene, folding, completion ranking, the git
   graph's lanes, the activity view. Those modules of `rusty-ui` move over
   almost as they are; only views are rewritten.
5. **One door per concern**, as before: one function reads the protocol
   lines (`absorb`), one draws a row of text (the editor and the terminal
   share it), one measures text.

## The order

Each phase ends with the window usable for what it has, the probe and the
tests green, and CLAUDE.md saying how it works.

0. **The editor spike** — done: rendering, typing, scrolling, CJK, tabs,
   measured above.
1. **The shell**: the window and its title bar, the rail, panels, the dock,
   the status bar, the theme (dark and light, the tokens of
   `style/input.css`), the interface languages, settings and
   `workbench.toml`, the command palette and key bindings, menus, dialogs
   (`rfd`), the clipboard (`arboard`).
2. **The editor**, to and past what the WebView one did: tabs and two
   groups, painting off the thread, selection, undo, find and replace,
   folding, multiple cursors, Vim, bracket pairs, rust-analyzer and clangd
   (diagnostics, completion, hover, signature, actions, rename,
   navigation, inlay hints, semantic colours), the minimap, sticky scroll,
   indent guides, test lenses. And the gaps the comparison with VS Code
   found: closing asks about unsaved files and drafts survive a restart,
   line endings and encodings are kept, the line commands, F12/F8, the
   selection reaching code actions, completion's documentation.
3. **The project panels**: files, search, Git, crates, features, disk,
   memory, the environment, the wizard, the first-run setup, the
   playground.
4. **Running things**: the dock's output, problems and terminal, build,
   flash, monitor, the device picker, the simulator and its sheet, debug
   and registers, plot, waves, signals, flight, draw, the math toolbox.
5. **The assistant and Markdown pages.**
6. **Shipping**: installers and the updater without Tauri, the bundled
   tools, the release workflow.

Then `rusty-ui`, `rusty-app` and `rusty-ipc` are deleted, and with them the
`backend` feature split, which existed for the wasm frontend.

## Decided

- **The licence is GPLv3**, for rusty and for Slint, which it uses under
  its GPLv3 option — rusty was PolyForm Noncommercial, which that option
  cannot be combined with. Every dependency was checked against it (all
  permissive or GPL-compatible; the one unusual licence, `terminfo`'s
  WTFPL, is compatible). The "Made with Slint" badge is on the README and
  on every release page, though GPLv3 does not ask for it.

## Decisions still open

- **HTML.** What rusty renders as HTML today is Markdown: book chapters,
  the assistant's answers, hover documentation, release notes. The plan is
  a Markdown renderer of its own in Slint — headings, paragraphs as
  `StyledText` (emphasis, code, links, colour), code blocks through the
  editor's own painting, lists, tables, images — which is fast and themed
  like everything else. Blitz (HTML and CSS, painted on the CPU into an
  image) is the fallback if raw HTML in documents turns out to matter more
  than that. Neither draws MathML; formulas show as their source until a
  math renderer is chosen, as a formula the parser refuses does today.
- **Shipping without Tauri**: `cargo-packager` builds the same installers
  (NSIS, DMG, AppImage, deb) and its updater checks signed releases; whether
  the existing updater key carries over is to be checked before the first
  release from this branch.
