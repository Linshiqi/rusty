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

## The window, designed again

Not the WebView's shell ported: what the work is, and what the tools people
arrive from got right. The work is a loop — edit, build, run it (in the
simulator or on the board), watch what it does, go back to the code — with
the project's state (problems, size, toolchain, history) consulted along
the way. Three references: **Zed** for how little chrome an editor needs
and docks that hold panels on three sides; **Xcode** for the run loop as
one control — what to run on, the button, and an activity view saying what
is happening; **Wokwi** for the board beside the code.

```
┌ title bar ────────────────────────────────────────────────────────────────┐
│ rustfly ▾  ⎇ main ▾              [▶ │ ▣ ▾]                             ⌕ ≡  ─ □ ✕ │
├ left dock ┬ editor ──────────────────────────────────────┬ right dock ────┤
│ Project   │ main.rs   estimator.rs ×                     │ Board          │
│ Search    │                                              │ Assistant      │
│ Git       │                                              │                │
│           ├ bottom dock ─────────────────────────────────┤                │
│           │ Output  Problems  Terminal  Plot  Debug …    │                │
├ status bar┴──────────────────────────────────────────────┴────────────────┤
│ ◧ ⬓ ◨  ⓧ 2 ⚠ 3  ⚒ 0:12  ‹›                Ln 12, Col 4  UTF-8  LF  Rust  ▣  🔔 │
└───────────────────────────────────────────────────────────────────────────┘
```

- **The title bar is the run loop.** On the left, which project (its menu
  is recents, open, new, clone, the playgrounds) and which branch. In the
  middle, Xcode's shape: the run button, *where* it runs — the simulator, a
  serial port, a probe or a USB bootloader, one list — and stop; the button
  means simulate or flash by what is chosen, so there is one verb to learn,
  not six icons. Where it runs is an icon, named on hover. Build and debug
  are keys, menu rows and the palette, as in Zed. **What is running is not
  said here**: Xcode's activity view stood beside the button for one
  phase, and the user's verdict was that no IDE puts progress at the top
  centre — it is the status bar's, as in VS Code and JetBrains. On the right, the palette and the window's own
  controls. No menu bar row: the project menu and a `≡` app menu hold what
  menus held, and everything is in the palette.
- **Three docks, each a set of panels**, Zed's model. Left: the project's
  tree, search, Git. Bottom: output, problems, the terminal, and what a run
  brings — plot, waves, signals, flight, draw, debug, registers, calls —
  which arrive when there is something in them and go when hidden. Right:
  the board and the assistant, so the board stands beside the code while it
  runs, Wokwi's layout, for every project. Each dock opens and closes from
  the status bar's three glyphs and a key, keeps its size, and remembers
  which panel was in front.
- **Pages are documents.** What is read or edited at length rather than
  consulted beside code opens as a tab among the files: the welcome page,
  settings, the environment, the memory report, dependencies (crates,
  features, the build directory), the math toolbox, a new project, keyboard
  shortcuts, the board when it wants the whole width. One way to open, move,
  split and close anything big. There is no rail: a panel is reached by its
  dock's tab or its key, a page by the palette or a link.
- **The status bar says the state, and each item is a door**: the docks,
  problems by count, what is running — an icon for the kind of run in its
  state's colour, its clock while it runs, the whole verdict on hover — the
  language server, then the caret, the encoding, the line endings, the
  language and the chip. **Icons first, the words in the tooltip**, the
  user's rule for all chrome: text appears only where it carries a value —
  a count, a clock, a position — and a server's own words never reach the
  bar, where they once ran under the caret.
- **Messages are notifications**, a stack in the corner with their actions
  on them and a history behind a bell in the status bar — not a banner that
  covers the work.
- **The keyboard reaches everything**: the palette (Ctrl+Shift+P) lists
  every action with its key, the finder (Ctrl+P) every file, symbol and
  line; the keys are VS Code's wherever a VS Code key exists.

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

The order is the product's, not an editor's: rusty is a workbench for
embedded work in **Rust, C and C++, on many parts, extensible as data, and
simulated** — and a phase is worth doing first when it moves one of those.
So the editor stops at what writing code needs, and the loop that makes
this an embedded workbench — the part, the build, the board, the simulator
— comes before anything that only makes the editor more like VS Code.

Each phase ends with the window usable for what it has, the probe and the
tests green, and CLAUDE.md saying how it works.

0. **The editor spike** — done: rendering, typing, scrolling, CJK, tabs,
   measured above.
1. **The shell** — done, as designed above: the frameless window and its
   title bar, the three docks and the editor area with its tabs, the status
   bar, notifications, the theme, the interface languages, `workbench.toml`,
   the actions and their keys, the palette, the welcome page, settings,
   dialogs, the clipboard.
2. **Writing code** — done: selection, every caret and selection key,
   multiple cursors, undo in VS Code's units, the clipboard with whole
   lines, bracket pairs, indentation, the line commands, painting off the
   thread; drafts that survive closing and a restart, encodings and line
   endings kept; find and replace (case, whole word, regular expressions,
   in selection) and go to line; rustfmt on save.
3. **The three languages, alike.** rust-analyzer for Rust, clangd for C and
   C++, and both in one Cargo project whose build script compiles C —
   `rusty-lsp` already routes by `ServerKind` and runs the companion; the
   window gets diagnostics, completion, hover, signature, definition and
   references, code actions and rename through one door for all three. What a language is — its extensions, its
   server, its comment marker, its formatter — is one table, not a `match`
   per feature: rustfmt for Rust, clang-format for C and C++ only where the
   project carries a `.clang-format` (a style imposed on vendor code is a
   diff nobody asked for).
   *Done so far:* the language table; the servers started with the
   project, told every open, edit, save and close in the order they
   happened; what they found squiggled, counted in the status bar, listed
   in Problems and stepped through with F8; the hover card with the
   problems under the pointer and the server's signature and docs;
   completion as you type (the rules moved to `rusty_lsp::complete`, one
   copy for both windows), its imports brought along, and the signature of
   the call being typed; F12 and Ctrl+click to a definition — a
   dependency's or a toolchain header's opened read-only — references and
   implementations in the picker, Alt+Left and Alt+Right through the jumps,
   Ctrl+. for the server's fixes, F2 to rename across the project;
   clang-format on save — proven on a Cargo project with rust-analyzer and
   a CMake one with clangd. Inlay hints and semantic colours moved to the
   editor's depth (8): the painter already colours all three languages, and
   a hint inside a line moves every caret and click after it — the hardest
   geometry an editor has, for polish rather than for writing code.
4. **The project is a part.** Opening a folder runs detection —
   Cargo, PlatformIO or CMake; the chip, from wherever that build system
   says it; what the machine lacks to build it — and the window says it:
   the chip in the status bar, the run target in the title bar chosen from
   the catalogue (the simulator where the part has an `emulation`, the
   board where it has a flasher, neither invented), the Environment page,
   the first-run setup, the wizard and the playground. **The window names
   no part**: every difference between two chips is a key of the
   catalogue, and a test holds `rusty-desk`'s source to that.
   *Done:* detection as the old window ran it (the firmware crate
   found below a workspace, the playground marked); the chip in the status
   bar, its vendor, build system and where it was read in the tooltip; the
   run target in the title bar; the project's problems first in Problems,
   their fix a button on the note that names them; the Output dock, where
   whatever the window runs is shown line by line with how it ended, one
   run at a time, stoppable; the Environment page — the verdict and the
   one button that installs what is missing, then each tool in the group
   of work it serves — reading `rusty_embed::setup`'s answer, which the
   old window's page now reads too (its rules moved there, tests and
   all), and installing one tool at a time through `rusty_embed::install`,
   the first failure ending the queue; and the first run, a note when the
   machine cannot build that opens the page. The wizard: the parts a
   project can be started for, what the selected one commits the project
   to beside it, then the runtime (C where the part has a template), the
   layout and the generator's options, then a name refused while it is
   typed, the consequences and the command; the generator checked before
   the button is offered, run in Output or rusty's template written, the
   workspace wrapped round it, and the project opened on its entry point —
   proven by the probe writing a template part's project and opening it.
   The playgrounds on the welcome page, each opened on its entry point and
   kept out of the recents, the part's name in the title bar and a bar
   across the top to restore the example — its drafts dropped with it, or
   they would come back over it — or keep it as a project, what is on
   screen written first. Code beside the board waits for the simulator (6). `the_window_names_no_part`
   scans every source and `.slint` file for a catalogue id, a family's
   name or a vendor's — proven by planting one.
5. **Build, flash, watch.** The build system's own plan run in the dock,
   its errors in Problems and on the lines they are about, the size of the
   image against the part's memory; flashing through the part's flasher to
   the device chosen once in the title bar; the serial monitor through the
   one `absorb`, so a board's telemetry and parameters reach Plot as a
   simulation's do.
   *Done so far:* Build (Ctrl+Shift+B) writes what is on screen, then runs
   `rusty_embed::buildsys`'s plan a step at a time in the firmware's
   directory, refusing a cargo write on a nearly full volume
   (`rusty_core::disk::too_full_for`, the old window's guard moved where
   both can call it); the status bar's activity — `rusty_embed::activity`,
   moved out of the old window with its tests — runs a clock and then holds
   the verdict, the image weighed against the part's memory in it; the
   compiler's errors (`rusty_embed::buildlog`, rustc's two-line form and
   GCC's one) are listed in Problems under *The last build*, and every
   line of output that names a place is a link there. Every line any run
   prints reaches the window through one door (`App::heard`), and runs go
   through `flash::launch`, so a UF2 copy and a DFU image run as any
   command does. The rules for which board a verb uses when none is chosen
   (`rusty_embed::only_candidate`, `serial_capable`, `usable_boot`) moved
   into the model. The target list is one list from the title bar's
   target icon — the simulator where the part has one, the serial ports,
   the probes and the bootloaders the part can be written through — kept
   once chosen. Run (Ctrl+F5) runs on it: the simulator (phase 6), or
   Flash. Flash (Ctrl+U) builds, plans against the image that build made
   (`flash::plan`, PlatformIO's own upload for a PlatformIO project), asks
   before a write the plan warns about, and stays attached as a monitor;
   Monitor (Ctrl+Shift+M) attaches without writing, over rusty's own serial
   link for a part espflash cannot talk to, typed into from a line under
   the dock. With nothing chosen the one board plugged in is the answer;
   otherwise the list opens with the verb waiting on it, and offers no
   simulator to a board verb. A monitor holding the port is stopped first,
   and the flash starts from its end. Proven as far as this desk allows —
   the list, the ask, the choice — and **not against a board**: the
   machine it was written on had four COM ports, none a board, and no
   probe. Telemetry reaching Plot waits for Plot (6); every line already
   passes the one door it will be read at.
6. **The simulator.** The sheet beside the code, the run in rusty's QEMU or
   rusty's own emulator as the catalogue says, the pin channel, the parts
   and their behaviours, Plot, Waves, Signals, Flight and Draw, debugging
   in the simulator and on the board.
7. **The rest of the workbench**: Git, project search, crates and features,
   disk and memory, the terminal, the assistant and its tools, Markdown
   pages.
8. **The editor's depth**, now that everything it is for exists: folding,
   sticky scroll, the minimap, indent guides, test lenses, inlay hints,
   semantic colours, Vim, two groups.
9. **Shipping**: installers and the updater without Tauri, the bundled
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
