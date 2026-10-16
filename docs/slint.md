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
│ ◉ rustfly ▾  ⎇ main ▾            [▶ │ ▣ ▾]                             ⌕ ≡  ─ □ ✕ │
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

- **The title bar is the run loop.** On the left, rusty's mark — the
  window's icon, which opens the window's menu as a Windows app's icon does
  — then which project (its menu
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
  line; the keys are VS Code's wherever a VS Code key exists, two-stroke
  chords included (Ctrl+K Ctrl+S), and Keyboard Shortcuts, a page, lists
  every action with its key and rebinds it — what shows a key shows the
  one in force.
- **A window is a project**, as in VS Code: New Window (Ctrl+Shift+N) is
  another rusty process on the welcome page, and a recent project opens
  here or, with Ctrl or its row's button, in a window of its own.

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
   implementations in the picker, Ctrl over a name drawn as a link where it
   has a definition, Alt+Left and Alt+Right (and the mouse's side buttons)
   through VS Code's navigation history — tabs picked, files opened, jumps
   and long moves, following the edits — not only the jumps,
   Ctrl+. for the server's fixes and the same fixes on the hover card under
   the problem, a click away; the bracket beside the caret boxed with its
   partner, and the name under it washed where else it occurs; F2 to
   rename across the project;
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
   *Done so far:* Run on the simulator target runs the simulator's plan —
   build, image, then rusty's QEMU or rusty's own emulator as the
   catalogue names — the boot step with its monitor and, when the emulator
   models the pins, its pin channel, the sheet declared down it before the
   firmware's first instruction and its circuit walked beside it. Every
   protocol line is read where every line arrives (`App::absorb`, behind
   `App::heard`): levels and duties — a pin in one or the other, the newer
   report deciding — the pin source, the display, tunables and declared
   sensors; what the instruments read is consumed and waits for them; only
   the firmware's own words reach the dock. Run while it runs restarts it.
   A tool the plan lacks is a notification with its Install. Proven by the
   probe on blink in rusty's QEMU v10: GPIO0 toggling from the emulator's
   registers, the display line, Stop.
   The board beside the code, in the right dock: the old window's drawing
   — `art`, `geometry`, `edit`, `layout`, `glow`, `readout`, with their
   seventy-two tests — moved into `rusty_embed::board` for both windows,
   and `board::picture` drawing each part alone in its own frame, an image
   the window places where the part is, so moving a part draws nothing
   again. What moves is the window's own over the pictures: each wire
   coloured by its net's level, each lamp's lens as bright as its current,
   an RGB lens's mixed colour, a digit's segments each lit for its share of
   the period, a strip's pixels as the bytes on its wire said, a screen as
   the firmware's SSD1306 or SH1106 driver drew it (or its `[rusty:disp]`
   line), a servo's horn where the pulse puts it, a motor's rotor turning
   with its duty and its bridge, a sounder's ring; each switch's cap and
   each keypad key pressed under the pointer; and every word — a lead's
   name, a reference, a value — as the window's text, never turned. The
   live layer's models are made once and changed row by row: a model
   replaced under a held cap took the cap's press area with it, and the
   release went nowhere. Under the board the bench: a knob, a source and
   each reading of a sensor as a slider at the window's own size whatever
   the zoom — hung under its part, a slider at a zoom that fits the board
   was off the pane — live only while a simulation runs, with what the
   converter last read in the source's tooltip. Proven on sense-board (the
   source slid to 1024, the firmware's own `knob` following), on the
   emulator probes for the display, the keypad (a key pressed, `down 1`
   printed), the strip (three pixels lit green, red and blue) and LEDC (a
   motor at 75% forward, a servo, a sounder).
   While nothing runs the sheet is a drawing (`app/sheet.rs`, over
   `board::edit` and `geometry`): a press on a part selects and drags it —
   bends belong to the sheet, so only the leg to the first bend stretches
   — a press on a pin pulls a wire that lands on a pin or a wire and
   otherwise begins one drawn click by click (Space turns the live leg,
   Backspace takes a click back, Escape drops it), a press on a wire
   selects it; R, M, Ctrl+D, Delete, Ctrl+Z and Ctrl+Y; an inspector for
   the reference, the value and every prop, a library to add from, and the
   file written four hundred milliseconds after the last change. Proven by
   the probe's edit run: a part dragged and turned, an LED added and wired
   to GPIO2, the wire undone and redone, the LED deleted, the file read
   back after each save.
   The run's instruments, each a bottom-dock panel that arrives with what
   it reads: Waves (a lane per pin on the trace's clock, zoomed and
   followed, exported as VCD), Draw (`rusty-draw`'s scenes in a space
   turned by dragging, the angles between arrows sharing a tail and the
   hand three make), Flight (the craft as it leans, a horizon, each motor's
   duty, and rusty's plant closed between the motors and the declared
   sensors at 50 Hz) and Signals (each source given a signal while the run
   goes on, the console's channels fed at the host's pace, and four
   instruments — time, spectrum, a response swept or read off a recording,
   a filter designed against the signal with its `no_std` code). Their
   arithmetic moved out of the old window into `rusty_embed::{waves,
   scene, lab}` with its tests, so both windows draw from one copy. Proven
   by probes: the keypad's sixteen lanes on the firmware's clock, the cross
   product example's three arrows and two right angles, rate-loop's loop
   closed through the plant, and filter-lab's played tone, converter and
   both channels, its spectrum, its design, and the recording measured at
   5 and 50 Hz.
   Debugging (`app/debug.rs`): F5 boots the firmware frozen in the
   simulator — built unoptimised, QEMU's gdbstub on the plan's port — and
   attaches the chip's gdb, or, with a probe chosen, builds, flashes and
   debugs on the board through probe-rs's debug adapter. Breakpoints are
   the editor margin's: a click beside a number sets one, they are placed
   when the session attaches and at once during one, and a dot moves where
   the debugger moved the breakpoint. A stop opens the file at the frame
   and marks the line; the Debug panel lists the stack and the frame's
   variables; the transport floats over the code (F5, F6, F10, F11,
   Shift+F11, Shift+F5). The Registers tab reads the chip's SVD and the
   picked peripheral's block at every stop. Proven on blink: a breakpoint on
   the line that sets the LED, the stop there with `on=true`, `led` and
   `peripherals` listed, and Stop ending both the session and the run. All of it read through `rusty_embed::period`, read again
   only when a level or a press changes and re-weighted when a duty does;
   at most a redraw a frame. Fitted whole; the wheel zooms about the
   pointer, a drag pans, a double-click fits again. A press joins two
   GPIOs or drives one, as the sheet says — proven on the C3 playground,
   GPIO4 low while its button was held and high after.
   Plot, in the bottom dock once a sample or a tunable arrives: the
   firmware's channels over the last six hundred samples, each against its
   own range with `rusty_embed::plot`'s floor (the store and `band` moved
   there, the old window calling them now), a chip per channel under the
   chart to show it alone; the tunables beside it, a slider only over a
   range the firmware gave and a typed number where it gave none, and what
   the firmware says it took shown. Proven on `examples/rate-loop`: ten
   channels, four tunables, `roll_p` asked for 1.2 and shown as the 0.6 the
   firmware clamped it to.
7. **The rest of the workbench**: Git, project search, crates and features,
   disk and memory, the terminal, the assistant and its tools, Markdown
   pages.
   Search, in the left dock: ripgrep's engine as it is typed, case, word
   and regex switches, include and exclude globs, every hit under its file
   with the match picked out. The Terminal, in the bottom dock: the shell
   `rusty_term` starts in a pty, drawn as its styled runs on a grid of the
   editor's font — each run at its column, a wide character in its own two
   cells, so a line of Chinese does not push the cursor off its place —
   the sixteen colours the theme's, the cube and greys xterm's. Keys go as
   a terminal sends them (Ctrl+letter the control code, so Ctrl+C
   interrupts), Ctrl+Shift+V pastes, the wheel scrolls back, and a shell
   that exits says so with a button for another. Proven with PowerShell:
   `echo rusty-term-$((6*7))` answered `rusty-term-42`, the pty took the
   panel's ten rows, and `exit` left the notice.
   Git, in two places, as VS Code and Fork split it: the left dock's panel
   is what is checked out with fetch, pull and push (their counts in the
   tooltips), the commit box with amend, the staged and unstaged lists with
   their verbs on hover, and the stashes; the History page in the editor
   area is every ref down the left, the graph (`rusty_git::log`'s lines,
   each row drawing the half of the lines that reach it from above and the
   half that leave it, clipped, so a filled row covers nothing), a search
   that dims and steps, and a commit opened below with its files and their
   patch, one column or two. A file's diff is a page of its own. Reads run
   on a thread; writes are dock commands; the stamp is asked every few
   seconds while either shows. One right-click menu serves the window
   (`app/menu.rs`), and a name git would refuse is refused as it is typed.
   The pure halves moved into `rusty-git` (`log`, `plan`) for both windows.
   Proven on a scratch repository with a merge and a tag: a file staged,
   committed with a two-line message, the graph read, the merge opened,
   `bad name` refused for its space, `probe/branch` made from a commit and
   checked out, and a search stepping to its first match.
   Memory, a page: every build the project has, the one measured marked and
   one built for another triple said so; the flash image and the static RAM
   against the part's capacity, coloured as they near it; the unattributed
   bytes; every crate's share with a bar against the largest; the sections.
   A build re-measures a page that has been looked at. Proven on
   `examples/blink-rust`: 15.6 KB of flash, `esp_hal` the largest crate.
   Crates, a page: the workspace's figures, its direct dependencies against
   crates.io (asked on the button, one request per crate) with Upgrade as
   a `cargo add` in the dock, the duplicate versions with who pulled each
   in, a switch per feature with what flipping it alone changes, and the
   build directory — its trees, what in them is stale and why, a sweep, the
   incremental caches dropped, a tree or a cache removed after asking, the
   idle threshold, the auto-sweep switch (now run after a build that
   succeeds), and the shared `target-dir` snippet to copy. The analysis is
   loaded the first time the page shows. Proven on `examples/blink-rust`
   (139 dependencies, 13 duplicated, 1.9 GB of build trees) and on a crate
   with features, one flipped.
   The assistant, in the right dock: a question runs `rusty_ai`'s agent
   loop on a thread with a runtime of its own, the workbench's read-only
   tools and the open file's lines (a chip takes it off), every event drawn
   as it streams — reasoning folded, each tool call with its verdict, the
   answer as Markdown — and the loop's history is the transcript after.
   Stop keeps what streamed, marked; the meter sums every round, the cached
   share beside it; a cap a provider names is kept for the session. The
   model is Settings' (a preset, the endpoint and dialect, the model or the
   endpoint's list of them, the output budget, the key into the system's
   credential store and never back, a check that says what one request
   established). Markdown is read once (`src/markdown.rs`): the blocks
   `StyledText` cannot take — headings, code, quotes, tables, rules — are
   read there and only each block's inline text is handed to it, escaped;
   code blocks take the editor's own colours. Proven against a loopback
   model in the OpenAI dialect: `list_files` called, the answer's heading,
   nested list, Rust block and table drawn, 4.2k in with 1.7k cached.
   A Markdown file opens as its page — the toggle over it shows the source
   — with the pictures in the project drawn and one elsewhere named. Its
   formulas are RaTeX's (*Formulas*, under Decided): a paragraph holding
   one is broken into lines by rusty (`markdown/flow.rs`), measured by the
   window's own `StyledText` and set as a non-wrapping text per run and a
   picture per formula on one baseline, for the width the paragraph reports
   back; a formula set apart is centred; one RaTeX refuses is its source
   with the reason on the pointer. The book's raw HTML is read — a
   `<figure>`'s picture centred with its caption under it, a `<span>` its
   text, a `<script>` named and not run. An image file opened from the tree
   is a picture page (`Tab::Picture`), fitted, zoomed by the wheel, on a
   checkerboard; an SVG is its source with its picture over it, drawn from
   the draft. Proven with `RUSTY_PROBE_BOOK` on a copy of the book's
   chapter 4 and its symbol table, in both themes.
   The math toolbox, a page: the rows typed and worked out as they are
   typed (Enter makes the next, Backspace in an empty one takes it away), a
   slider for a row that is one number, the selected row's working with
   its turns played on the aircraft, the value in full beside an attitude
   indicator, and the view of space or of the plane drawn by the Draw tab's
   pieces (`SpacePieces`). What composes the view moved into
   `rusty_embed::scene::math` for both windows. Formula lines take a face
   with the operators (`symbols-font`): `⊗` was a box in the monospace
   face and the fallback alike. The assistant now gets the live values a
   run says, as the old window sent them.
8. **The editor's depth**, now that everything it is for exists: folding,
   sticky scroll, the minimap, indent guides, test lenses, inlay hints,
   semantic colours, Vim, two groups.

   Indent guides and test lenses first, and the pure halves moved into
   `rusty_edit` (`guides`, `lens`, `testrun`) for both windows. A guide is
   a hairline mark at each level's column, the levels counted once per
   version of the text, since a blank line takes its neighbours'. A lens
   sits after everything on the line VS Code would give it — ▶ Run Test |
   Debug over a test or a module of them, ▶ Run over an example's `main`
   where `cargo run` would not flash a board — and carries the last run's
   tick or cross. Run Test is `cargo test <filter> -- --show-output` at the
   project root in an empty dock, read as it goes; Debug builds, asks the
   binaries which holds the test, and runs that one under gdb or LLDB's
   adapter. Proven by the probe on `examples/draw-vectors`: two lenses
   found, one of them run, and both carry the run's tick.

   Folding, sticky scroll and the minimap next. The folds live in the
   document, carried across every edit (`document::carry`: one below an
   edit moves, one whose hidden lines it reached opens, one being typed on
   stays), and the rows are the screen's: `row_of` and `line_at_row` are
   the one crossing, for a click, the caret, Up and Down, the hover card,
   the popups and the margin. A cursor put on a hidden line opens what
   hides it. Sticky lines and the minimap are `rusty_edit::sticky` and
   `rusty_edit::minimap`, moved out of the old window; the minimap is an
   image painted in Rust for the rows it shows, again only when they or
   their painting change. Each is a switch in Settings ▸ Editor, kept in
   `workbench.toml` with the indent guides'. The probe folds all of
   `draw-vectors` (64 rows to 18), steps Down over a block (9 to 13),
   jumps into one (it alone opens), and scrolls into `impl Vector` to see
   its line stick.

   Semantic colours and inlay hints, drawn in the line. A hint takes room,
   so from a line's first hint on every x and every column goes through
   one walk of its characters and hints (`editor/inlays.rs`): the pieces,
   the caret, a selection, a squiggle, a click, the popups. The caret at a
   hint's column stands on the side of the code it is not about — before
   `: f32`, after `sensor: ` — and `rusty_lsp::follow::hints`, moved out of
   the old window with the semantic overlay (`rusty_edit::semantic`), moves
   hints and colours through every edit by the same rule. Both are asked
   for together a moment after typing stops and when the server says it
   settled, around the view for a long file, and an answer about another
   text is asked for again rather than drawn. Inlay hints are a switch in
   Settings ▸ Editor. Found on the way: a relative project root on the
   command line gave the server `file://examples/…`, refused as no file —
   the root is made absolute now. The probe waits for rust-analyzer's
   answer on `draw-vectors` (261 colours, 23 hints), clicks just past a
   hint and lands on its column, and types there to see it carried along.

   Vim's keys: the machine moved to `rusty_edit::vim` as it was, and the
   window feeds it (`editor/modal.rs`) after the completion popup and
   before the editor's own keys, so insert mode leaves every shortcut
   where it was. Normal and visual mode draw a block over the character
   the cursor is on, which in visual mode is not the selection's head, so
   the cursor the machine set is remembered while the selection is the one
   it set. Undo is Vim's: the history holds one step from a command into
   insert mode until Escape. Found on the way, in the shared machine: `cw`
   on a one-letter word ran on to the next word's end, as `ce` does from
   there — `let a =` lost its `=`. The probe switches Vim on in
   `draw-vectors`, changes `a` to `first` with `cw` and takes it back with
   one `u`, then deletes two lines with `Vjd` and puts them back.

   Two groups, side by side and no more. `tabs` and `active` are always
   the group with the keys and the other group waits beside them, so every
   path that opens, picks or closes a tab works on one group without
   knowing there are two, and giving the keys to the other group swaps the
   two. The code view is one component, moved to the side that has the
   keys; the other side draws its file from a second rows model over its
   own document (`editor/shade.rs`, the `Shade` global), and a press there
   swaps the groups and is made again in the code view. One file open on
   both sides is one document in two views: an edit, a painting or the
   server's answer draws both, and closing one view of a file with changes
   asks nothing while the other holds it. A group left with no tabs ends
   the split. Ctrl+\ splits, and split moves the keys; each document keeps
   where its view was scrolled, so a tab comes back where it was left. The
   probe splits `lib.rs`, opens `Cargo.toml` on the right and types there,
   presses in the left view to move the keys, and types there.

   Then VS Code's strip and divider, after v0.7.0-rc.1 drew the split
   button over the last tab and left the divider fixed at half: each strip's
   tabs sit in a lane ending where its buttons begin, scrolled sideways by
   the wheel, with the tab in front brought into view as it changes; the
   divider drags, keeps each group 220 px wide, goes back to half on a
   double-click and is kept in `workbench.toml` with the docks' sizes; and a
   tab's right-click menu has VS Code's rows — the closes, the paths, Reveal
   and Split right (`app/tab_menu.rs`), each an action the palette also
   offers for the tab in front. `RUSTY_PROBE_TABS` opens a book's chapters
   until the strip overflows, wheels it, drags and double-clicks the
   divider, and uses the menu on both strips.

   A tab in a window of its own, VS Code's Move into New Window: from the
   palette, from a tab's menu, or by letting a tab go beyond the main
   window's edge. A second Slint window (`TornWindow`) with the same code
   view, and a second surface driving it (`Surface::beside`) over the same documents — Slint
   gives each window its own `Code`, so the editor's Rust half only had to
   stop assuming which window it draws into (`editor/host.rs`), and an
   answer from another thread names the surface that asked rather than
   reaching for "the editor". One file open in both windows is one document
   under the split's rule; what is about the file happens in its window,
   what is about the project in the main window, which comes forward. Its
   keys are the main window's keymap — rebound keys, two-stroke chords, the
   history's Alt+Left — Ctrl over a name draws the link there, and its text
   is the editor's one size, which Ctrl and the wheel there set too. The
   probe's `RUSTY_PROBE_TORN` moves the file into a window of its own,
   types there, opens it in the main window too and types there, undoes
   from the other window, saves it on a rebound key and closes it, then
   drags `Cargo.toml` out and moves it back; a file moved out from its
   tab's menu is then renamed in the tree, which the window follows, and
   deleted there, which closes it; and last, in a window of its own again,
   a link is drawn and followed, Ctrl+K Ctrl+S opens Keyboard Shortcuts in
   the main window, Alt+Left walks back, and Ctrl and the wheel there size
   the text in both windows.

   Then tabs that drag, reported for v0.7.1 as "a tab cannot be dragged
   out, or dragged in". Before, a tab let go of beyond the window's edge
   was all a drag did: along its strip and onto the other group it went
   nowhere, and a window of its own's tab did not drag at all. Now a tab
   carried more than four pixels moves along its strip to the gap a
   marker shows, onto the other group's strip or code, onto the right half
   of the only group's code to open the split with it there under a
   translucent overlay, or out into a window of its own — and from there
   back onto the main window's strips or code, followed through the
   screen, since Slint carries no drag between windows. Where it lands is
   `crate::tab_drag`, pure and tested; the strips are measured by Rust
   (`DocTab.width`), since nothing outside a `for` can read what it laid
   out, and the panes and lanes are read off the window at every move
   (`EditorArea.tab-geometry`). Each group is clipped to its pane now too:
   with the divider dragged tight, a page wider than its pane was printed
   under the other group's code. `RUSTY_PROBE_TABDRAG` (`probe_tabs.rs`)
   drives every one of those by the pointer, Escape and the strip's
   scrolling under a held tab included, and drags the divider to both ends
   with the History page in one group and code in the other.
9. **Shipping**: installers and the updater without Tauri, the bundled
   tools, the release workflow.

   *Done:* `rusty_embed::update` is the feed (`rusty-update.json` on the
   newest release), the version comparison and the signature check: an
   ordinary minisign `.minisig`, published beside each download and carried
   in the feed, whose trusted comment must name the version the feed
   announces. **Nothing from before Slint updates onto it** — the feed has
   a name the Tauri updater never asked for, so an old copy finds nothing
   and is installed over by hand. The window checks quietly a moment after it opens and says
   only a newer release, in a note; Check for updates opens Settings ▸
   Updates and says whatever it found; the download runs in the
   background, its progress in the status bar, and the restart that
   installs it is asked for. On Windows the installer is our own
   (`crates/rusty-setup`): the app and its tools in one tar, compressed
   whole with xz and appended to it, installed per user in
   `%LOCALAPPDATA%\Programs\rusty` with an entry of its own in Settings ▸
   Apps (`dev.rusty.workbench`), run by the updater
   as `--quiet --run --wait <pid>`, and, cut down to its own part, the
   uninstaller.
   Staged beside the install and moved into place, with a list of what it
   put there, so an update removes what it no longer carries and the
   uninstaller removes exactly that. macOS gets an app bundle, ad-hoc
   signed, in a disk image and as `.app.tar.gz`; Linux an AppImage and a
   `.deb`. `crates/rusty-release` packs the installer and signs and checks
   every updater artifact in the workflow. The tools are found beside the
   executable (`bundled_dir`), wherever each package puts them. Proven
   here: a silent install and an uninstall back to nothing, in a sandbox;
   a debug build pointed at a local feed found a newer version, downloaded
   it, verified it and offered the restart; and the repository's key signs
   what the key in the code verifies. The macOS and Linux packaging is the
   release workflow's, and is proven by its first run.

   Then `rusty-ui`, `rusty-app` and `rusty-ipc` were deleted, in v0.7.0,
   and the `backend` feature split with them: it existed so the wasm
   frontend could take each crate's `model` alone. The tab strip came back with them: what each project had open, in
   which group, is `workbench.toml`'s `open_tabs` as the Tauri window wrote
   it, put back when the project opens — and the window, started with no
   project, opens the last one (`strip.rs`). What the Tauri window had and
   this one did not, as v0.7.0's notes say: the call hierarchy, macro
   expansion, the finder's `@` and `#`, New window, a file or a commit in a
   window of its own, and a choice of the terminal's shell — a file in a
   window of its own has since come back (item 8, above).

   Since then, navigation by what the code means: Go to type definition
   beside references and implementations in the picker; Expand macro
   recursively, rust-analyzer's expansion as a read-only document with no
   file, beside the code (`app/expansion.rs`); the call hierarchy as a
   Calls panel in the bottom dock, asked a level at a time and turned in or
   out by a pair of icons (`crate::calls`, the old window's tree and its
   tests, and `app/calls.rs`); and the editor's right-click menu that
   offers them. The probe's `RUSTY_PROBE_NAV` drives all three on
   `draw-vectors`' example.

   Then the tree's own verbs, reported from the release candidate as "the
   tree cannot make, copy, paste or rename anything": VS Code's explorer
   (`app/tree.rs`, the arithmetic in `crate::tree`). A right-click on a
   folder, a file or the space below the rows opens its own menu; a new
   entry is named in a box among its future siblings and a rename in its
   own row, the name refused under the box as it is typed — committed text
   only — by `rusty_edit::name_problem`, the rule the library's rename
   keeps, and by the tree's entries for a name that is taken. Cut, copy and
   paste are the menu's and Ctrl+X, C and V while the tree has the keys,
   which a click on a row gives it (the file opens; a double-click or Enter
   gives the keys to the file), with the arrows, Enter, Space, F2 and
   Delete. Delete goes to the recycle bin after asking; a drag moves, the
   folder it would land in lit by `rusty_edit::drop_target`. A rename, a
   drop or a cut pasted carries every tab, document, draft, fold, open
   folder and breakpoint with the path, and the servers hear the old name
   closed and the new one opened. Reveal is `crate::reveal`, for the tabs'
   menu as well. The rows are one model changed in place, so the box being
   typed in survives a re-read of the tree. `RUSTY_PROBE_TREE` works all of
   it by pointer events at the rows and keys, on a copy of `draw-vectors`.

   Then two the release candidate was missed for, in the user's words:
   no New Window, two projects never open at once, and no way to see the
   keys. New Window starts this executable again with `--new-window`
   (`src/instances.rs`, the Tauri window's design carried over: the
   AppImage started rather than its mount, the child reaped, a process
   group of its own, no console), and the instances register by a held
   lock file each, now with the project each has open beside it — so a
   project open in one window is refused in another rather than given two
   language servers, and the update's restart says how many other windows
   stay on the old version (the Slint installer renames aside what they
   hold; it closes nothing). A recent project opens here, or in a new
   window by Ctrl+Enter, Ctrl+click or its row's button; the Open dialog
   has a new-window row too. Keyboard Shortcuts is a page: every action
   and its key, searched by name or by pressing the key, a key recorded by
   clicking it — two strokes at most, Enter to keep — refused when it
   types or is Alt with a character, and what it collides with said on
   its row. `actions::Keymap` is the table with the rebound keys over it,
   kept in `workbench.toml`'s `keybinds`, which the old window wrote and
   nothing had read since; the window's key handler, the palette, the
   editor's, the tabs' and the tree's menus, the debugger's tooltips and
   the welcome page all read it. Two-stroke chords are new: a stroke that
   begins one is held, said in a note, and while it is held the code asks
   the window about its next key before taking it for its own — ahead of
   the path by which the code hands every key it does not use to the
   bindings (`Shell::invoke_key`), which is how any other key reaches the
   keymap from the code. Proven by
   `RUSTY_PROBE_KEYS` (Ctrl+K Ctrl+S from the code, Save moved to Ctrl+F7
   and used, Ctrl+S then saving nothing) and `RUSTY_PROBE_WINDOWS` (a
   second rusty on the welcome page, another project opened there, the
   first's refused, both recents kept).

   Then what v0.7.0-rc.1's user found missing beside VS Code. Ctrl and the
   wheel size the text under the pointer, Ctrl+= stays the window's: the
   editor's text (`Surface::set_scale` — the font, the one row height and
   the margin set together from `metrics::text_size`, every character
   measured again, the top line kept), a Markdown page's own scale and the
   terminal's font, each a tenth a notch and kept in `[layout]`, each put
   back from the palette. And the surfaces that answered a right-click with
   nothing have VS Code's menus (`app/context.rs`): Output, Problems (a
   problem copied as VS Code's JSON of it), the terminal (a selection made
   by dragging over its cells, Copy, Paste, Select all, Clear, Kill,
   Restart), the search results, a Markdown page and a dock's tabs, whose
   *Hide* takes a panel off its strip until it is shown again. A page's
   text takes every press of every button, so the page has a touch area
   over it that hears the right button and the wheel, and makes a left
   click again underneath for the links. The probe's `RUSTY_PROBE_ZOOM`
   and `RUSTY_PROBE_MENUS` (`probe_surfaces.rs`) drive both with the events
   a pointer makes. Found on the way, in Ctrl+='s own zoom: the display's
   factor was learned before the window was on the display, where it is
   1.0, so a 125% display was put back to 100% — a window of 1600×1025
   pixels at a scale of 1, laid out at 1280×820 and drawn into that corner.
   The display's factor is now whatever the window has that rusty did not
   ask for, worked out again whenever the window takes a new size — which
   is how the factor arrives, the window being made a moment into the
   event loop — and a new factor is followed by the logical size it makes
   (`zoom::apply`).

## Decided

- **Shipping is our own, not a packager's.** `cargo-packager` was the
  plan. An installer of our own is one small crate that does exactly what
  rusty needs — per user, a list of what it installed, an update that
  removes what it no longer carries — and the three others (a bundle, an
  AppImage, a `.deb`) are a directory layout each, which a workflow step
  writes.
- **No upgrade from the Tauri window.** Copies of rusty before 0.7.0 are
  not offered 0.7.0 and are not taken over by its installer: they are
  uninstalled by hand and the new one installed beside where they were.
  Settings, drafts and projects are rusty's own files and carry over.
- **The licence is GPLv3**, for rusty and for Slint, which it uses under
  its GPLv3 option — rusty was PolyForm Noncommercial, which that option
  cannot be combined with. Every dependency was checked against it (all
  permissive or GPL-compatible; the one unusual licence, `terminfo`'s
  WTFPL, is compatible). The "Made with Slint" badge is on the README and
  on every release page, though GPLv3 does not ask for it.

- **Formulas are RaTeX's**, measured, not assumed. The old window drew
  MathML from `pulldown-latex` and Slint has no MathML, so four engines
  were run over every formula of the book the page is used for
  (`cf-drone-rs/book/src`, 4,283 inline and 221 display, collected with
  the page's own pulldown-cmark options), each in a binary of its own
  beside a baseline that does no rendering. Correctness is counted two
  ways: whether each formula renders at all, and whether the characters it
  draws — letters, digits, operators, CJK, delimiters left out because
  each engine builds a stretched one from different pieces — are the ones
  MathJax draws for the same source; the disagreements were then looked
  at, as were 26 hard formulas side by side.

  | Engine | Renders | Wrong or refused, looked at | Per formula | Binary | Clean release build | Licence |
  |---|---|---|---|---|---|---|
  | RaTeX 0.1.14 (KaTeX's layout in Rust) | 4,504 | none: 19 character disagreements, all glyph choice (`⊥` for `⟂`, a macron for a combining bar, `\vec`'s arrow drawn as a path) | 2.7 ms | 5.1 MB | 2 min 24 s | MIT, KaTeX fonts OFL |
  | latex-rust 2.1.1 | 4,413 | 90 refused (CJK in `\text`), `\operatorname{Re}` drawn as `ℜ`, text-style fractions at full size, `array` rows misplaced | 1.3 ms | 2.2 MB | 33 s | MIT/Apache, STIX OFL |
  | typst 0.15.1 + mitex 0.2.4 | 4,504 compile | `\\[2pt]` printed as `[2pt]` in `cases`; ink cut at the line box unless padded; mitex's own package files to vendor | 3.5 ms | 46.0 MB | 12 min 29 s | Apache-2.0, fonts OFL |
  | MathJax 3 on boa (mathjax-svg-rs 0.4) | 4,494 | 10 refused (`\ell` and friends need a font loaded later), 32 `\boldsymbol` drawn as red source | 72 ms, 1.7 s to start | 13.2 MB | 7 min 55 s | MIT, MathJax Apache-2.0 |

  The times are for all 4,504 on one thread, the builds were measured on a
  machine busy with other builds, and the binaries carry `serde_json` (a
  binary that renders nothing is 0.15 MB). Added to rusty itself the
  release `rusty.exe` went from 62.46 MB to 67.01 MB (+4.5 MB) for all of
  it — RaTeX and KaTeX's fonts, GIF, WebP and ICO decoding for the picture
  page, and the code. What decided it: the only engine with nothing wrong on
  the book, a ninth of typst's size, and quick enough to draw a chapter of
  three hundred formulas in a tenth of a second on eight cores. ReX was
  not measured: it is not on crates.io (`rex` there is another project),
  and four that are were measured instead.

## Decisions still open

- **HTML.** What rusty renders as HTML is Markdown: book chapters, the
  assistant's answers, hover documentation, release notes. The page is a
  renderer of its own in Slint — headings, paragraphs as `StyledText`,
  code through the editor's painting, lists, tables, pictures, formulas,
  and the raw HTML books use (figures, spans, links) read by a small tag
  reader. Blitz (HTML and CSS, painted on the CPU into an image) remains
  the fallback if raw HTML in documents turns out to matter more than that.
  The assistant's answers and the release notes still show a formula as
  its source: only the file page lays paragraphs out with formulas in them.
