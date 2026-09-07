# Changelog

What each release changed, written for the people downloading it. The release
workflow reads the section matching the tag and publishes it verbatim as the
release body — so this file, not the commit log, is what users see. The
history is public too, but it is written for whoever maintains this; a
release note is written for whoever downloads it, and they are not the same
document.

One `## v<version>` heading per release, newest first.

## Unreleased

- **Schematic symbols, the library first.** The board's parts are on their
  way to being real schematic symbols with real pins, KiCad's way, and this
  release carries the library under them: KiCad's `.kicad_sym` format read
  and written, a built-in `Device` library (R, C, LED, SW_Push), a
  project's own `.rusty/symbols/`, and LCSC (嘉立创) parts imported by
  number from EasyEDA's component service. `rusty-cli symbol C2286` fetches
  a part, reads it into a symbol and keeps it in the data directory's
  `symbols/lcsc.kicad_sym`, a file KiCad can open too. The board editor
  does not draw them yet.
**Fixed — completion never offered anything that was not already imported.**
Typing `Out` in a file without `use esp_hal::gpio::Output` offered nothing,
and `Output::` nothing after it, because rust-analyzer only enables its
import-on-completion when the client can fetch the `use` line lazily, and
rusty never said it could. It does now: an item not yet in scope shows with
its path — `Output (use esp_hal::gpio::Output)` — and accepting it adds the
import at the top of the file, as VS Code does. A hover that shows
`{unknown}` for such a name is rust-analyzer's honest answer until the
import exists.

**Changed — the status bar says what rust-analyzer is doing.** A freshly
opened embedded project takes rust-analyzer a while to index, and every
completion in that time comes back empty; the status bar said "rust-analyzer"
in green the whole time. It now shows the server's own progress —
`rust-analyzer · Indexing 26% 12/45 (esp-hal)` — until the work is done, so
an empty popup reads as "not yet" rather than "never".

**Changed — the devkit is drawn as the board on the desk.** The module with
its antenna and shield can, printed with the chip and module names, the
USB-UART bridge, the EN/RST and BOOT buttons, the power LED, the RGB LED on
the devkits that carry one, and the connector — micro-USB on the classic
ESP32 devkit, USB-C elsewhere, two on the S3 and C6 — around the same pin
rows as before, for every Espressif part in the catalogue.

**Changed — rusty's QEMU ships in the installer.** The emulator with the
GPIO model is packaged with the app, so a fresh install simulates without
a download and a pin read back in the emulator is what the firmware set.
Espressif's build is fetched only where rusty has no build of its own
(Intel macOS today).

**Changed — a stock QEMU says what it cannot do.** Espressif's build of the
emulator has an empty GPIO write handler: a pin read back is always 0, so
`led.toggle()` followed by `led.is_set_high()` prints `false` for ever and
looks like a broken driver. The run now says so in the dock, in as many
words, and the Simulate panel flags such a copy beside Run and offers to
upgrade it to rusty's build, which models the pins — the same image alternates
`true`/`false` there. Real hardware was never affected.

**Fixed — a tool download that unpacked with the wrong tar.** With Git for
Windows ahead of System32 on PATH, `tar` is GNU tar, which reads a path
like `E:/…` as a remote host and fails with `Cannot connect to E:`. Every
archive rusty unpacks now goes through Windows' own bsdtar.

**Added — Initialize a repository.** A project that is not under git showed
"not inside a git repository" and nothing else; the Git panel now offers to
run `git init` there, in the dock like every other git command.

**Fixed — the pin map read no esp-hal 1.x project.** It looked for the
vendor's pin table in `esp-metadata`'s TOML, which esp-hal 1.0 replaced
with generated Rust in `esp-metadata-generated`; every current project got
"could not find esp-hal's description", and the pins the source named were
then painted red as "not on this part" — a claim nothing there could make.
The generated table is read now, and a project whose table genuinely
cannot be read lists its pins as unverified rather than as missing.

## v0.6.11

**Added — the Disk section of the Crates panel.** Where this project's
builds went: the build directory's size by profile and target, how much of
it is stale — artifacts of dependency versions the lockfile no longer
resolves, of packages no longer in the graph, incremental caches idle past a
threshold you choose or beyond a crate's newest four variants — and the
volume's free space. One button sweeps the stale part (the current build is
never touched; a version the lockfile moves back to is simply rebuilt); each
tree, its incremental caches, `cargo doc`'s output, simulator images and
cargo's own caches can be removed on their own, after asking with the size.
Nothing in a tree a build is holding the lock on is removed. An opt-in
sweeps after every successful cargo command. The section also shows the
`~/.cargo/config.toml` snippet for one shared build directory across
projects — each dependency compiled once — and, where debug symbols
dominate, the profile setting that shrinks them; rusty does not write
either file for you. `rusty-cli disk` and `rusty-cli sweep [--apply]` do the
same headless, and the assistant has `disk_report`.

**Changed.** A cargo command is refused before it starts when the volume it
would write to has under 2 GB free, with the number — a build that dies
half-way reports `IO failure on output stream`, which reads as a broken
compiler.

## v0.6.10

**Fixed — a stash of an untracked file opened as "no files changed".** The
files a stash saves with `--include-untracked` live in a third parent commit
that the stash's own diff never reaches; the Stashes view now lists them,
as added, with their patches. And the history no longer shows a stash's
internal commits — `index on main`, `untracked files on main` — as rows and
lanes of their own: stashes are read in the Stashes view, the history is
for the branches.

## v0.6.9

**Fixed — completion that offered nothing.** rust-analyzer sends completion
items in the order it found them and puts the ranking in a separate field;
rusty kept the first hundred unsorted, so `v.` showed a hundred arbitrary
methods and typing `le` narrowed them to nothing — and a popup narrowed to
nothing still swallowed Enter and Tab. Items are ranked first now, more of
them are kept, and a popup with nothing to show gets out of the way.

**Added — bracket pairs.** Typing `{`, `(`, `[` or `"` brings the closer and
puts the caret between; typing the closer against one steps over it; Enter
between `{}` gives the three-line block with the caret indented; a `}` on a
blank line lines up with its `{`; Backspace inside an empty pair removes
both; an opener typed over a selection wraps it. In Vim's insert mode too.

## v0.6.8

**Fixed — two Git menu items that did nothing.** "Discard changes…" (and
"Delete file…" for an untracked one) never asked and never ran: the
confirmation went through `window.confirm`, which inside the app is a shim
the dialog plugin installs — one that answers with a promise, read as "no",
and calls a command the plugin no longer has. Closing a tab with unsaved
changes was silently refused for the same reason. Both now ask through a
native dialog. "Open in editor" did open the file, but behind the Git
panel; it switches to the editor now, as does double-clicking a file in the
Changes list or in a commit.

The board editor, gone over for how it feels and what it claims.

**Fixed — polarity.** Every lamp and button assumed active-high wiring: lit
when the pin was high, pressed drove the pin high. Most devkits' onboard
LEDs light when the pin is *low*, and most buttons are to ground with a
pull-up, so the board could show the opposite of the desk — and a pull-up
button *released* in the emulator when you pressed it. Each LED, RGB LED,
seven-segment and button now has a polarity in its properties (and an
`active_low = true` key in `.rusty/sim.toml`); a new button starts as a
pull-up button; the emulator's pin is driven to the level the wiring means.

**Added — on the sheet.**

- A moving part is lifted (shadow, grabbing cursor) and leaves a dashed
  footprint where it came from until you drop it; the devkit does the same.
- Multi-pin parts name their stubs — R G B, a…g, SDA SCL, PWM IN1 IN2 — so
  you know which dot you are wiring before the wire lands. The properties
  panel uses the same names.
- A wire brightens under the pointer.
- Parts can be renamed in the properties panel. A name you type survives
  rewiring; the editor's own labels (`GPIO26`) keep following the pin.
- Ctrl+D duplicates the selected part.
- LEDs are drawn as LEDs — dome, flange, legs — and glow when lit; the RGB
  LED is the same dome in its mixed colour; the seven-segment digit sits in
  a dark bezel. A pressed button sinks, and the potentiometer shows a knob
  that turns with its slider.
- Wires can be pulled from the chip's side too: drag a devkit pin onto a
  part's gold stub. The stub that will take it lights up under the pointer.
- Rubber-band selection: drag on the empty sheet to select several parts,
  Shift+click to add or remove one, Ctrl+A for all. A selection moves
  together and Delete removes it together; the properties panel says how
  many are selected. Panning moved to the middle button or Ctrl/Alt+drag.
- The sheet redraws only the part that changed, so dragging on a crowded
  board no longer stutters.

## v0.6.7

**Fixed.** The editor split follows one rule: "beside" is the right group,
from either side. Open to the side and the split button move a file *into*
the right group and never out of it, so the right group no longer vanishes
when one of its files is sent "to the side"; it closes only when its last
tab is closed. The split button and Open to the side appear only in the left
group's strip, since there is nothing further right of the right group.

**Added.** The Changes view's right-click menu on a file has Fork's items:
Stage or Unstage, Discard changes… (it asks first, and says whether the file
goes back to what is staged or to the last commit; an untracked file is
deleted), Stage all, Stash this file…, and Copy full path.

## v0.6.6

**Changed.** The title bar's centre holds a search icon and the project's
verbs, nothing else. The search box that carried the project's name went: it
read as a second search field in front of the finder's own. The icon,
Ctrl+P and View ▸ Go to file… open the finder.

**Added.** The commit box asks who you are when git does not know. With no
`user.name` or `user.email` configured, `git commit` refused with "Author
identity unknown" in the dock; the Changes view now shows a name and email
form in its place, saves them with `git config --global` (or for this
repository only), and enables Commit once git confirms them.

## v0.6.5

**Added.**

- Two editors side by side. Right-click a file in the tree or a tab and
  choose "Open to the side", or press the split button at the end of the tab
  strip (Ctrl+\) to move the current file into a second group. Each group
  has its own tabs and find bar; a file lives in one group at a time, and
  the second group closes when its last tab does. The layout, both strips
  included, comes back when the project is reopened.
- A file finder in the title bar. The project's name is now a search box:
  click it or press Ctrl+P, type part of a file name, Enter opens it in the
  group you are in and Ctrl+Enter opens it beside.
- The file list folds away: click the Files switcher again, press Ctrl+B, or
  use View ▸ Show or hide the file list. Remembered across sessions.

## v0.6.4

**Fixed.**

- On a fresh machine, the first build of an ESP32 (Xtensa) project after the
  setup sheet had installed espup died with `linker xtensa-esp32-elf-gcc not
  found`. espup makes its linker reachable by writing the user's environment
  for *new* processes, and rusty was already running. rusty now reads the
  environment espup exports and hands it to every build it starts, so the
  first build works without restarting rusty or the shell.
- A short side-by-side diff was drawn double-spaced: the grid that keeps the
  centre line running the full height of the pane also stretched its rows to
  fill it. Rows keep their own height now.
- The Git panel's branch picker shows the checked-out branch itself, marked,
  instead of "All branches" with the branch repeated beside it. A branch the
  history is filtered to shows in its place, and the menu highlights the
  choice in force.

## v0.6.3

**Changed.** The left rail only switches panels now. It had grown into one
column of sixteen icons — panel switchers, the project's verbs, the
debugger's transport and each panel's own actions, all at the same weight,
with Run in a different place on every panel. Each kind has its own home:

- Build, Run, Debug and Flash sit in the title bar beside the project's name,
  in the same place on every panel; Run turns into Stop while something
  runs. The simulation plan is asked for when a project opens, so they know
  whether the machine can simulate before the Simulate panel has been
  visited, and say what is missing if it cannot.
- The debugger's continue, step and stop float over the working area while a
  session is live — the same reach from the editor and from the board — and
  go away with it.
- Each panel's actions moved into the row that names the panel: the Files
  header (New file and New folder beside Refresh), the Git branch row
  (refresh, fetch, pull, push, new branch), the Crates and Toolchain
  headings, and the board sheet's corner (save, undo, redo, zoom, fit, grid).
- Save sits at the right of the file's header, beside its unsaved dot.

## v0.6.2

**Fixed.**

- Opening a Markdown file with an HTML block in it — a `<figure>` around an
  image, as a book chapter has — froze the window. The page view's reader
  never consumed the block's closing event and spun on it for ever, on the
  thread everything else runs on. Block-level HTML is shown as the markup it
  is, like inline HTML already was, and the reader now always moves past an
  event it does not know. A whole real book is under a timed test.
- The line between old and new in a side-by-side diff runs the full height
  of the pane, not just as far as the last row.

## v0.6.1

**Fixed.** An essay-length commit message painted over the files and the
patch below it. v0.6.0 moved the pane's buttons into a row beside the message
and, in doing so, capped the row instead of the message: a flex child takes
its content's height, not the clamped container's, so the text ran past the
cap. The message block scrolls itself again, with the buttons on its first
row.

## v0.6.0

More of the repository, and two things a fresh machine tripped on.

**Clone from a URL.** File ▸ Clone repository… (also offered by the Git
panel when no project is open): paste the address GitHub shows, choose the
folder it lands in, and the dialog says which directory it will create before
anything runs. The clone streams into the dock like every other command, and
the new checkout opens as the project when git finishes — remote already
attached, since that is what a clone is.

**Images compared as pictures.** A PNG, SVG, JPEG, GIF, WebP, BMP or icon in
a diff — a commit's file, a working-tree change, a stash's — shows before and
after side by side on a checkerboard, with sizes, instead of git's "binary
files differ". A side the file does not have (the old of an added picture)
says so.

**The commit pane hides and tears off.** Two buttons by the message, as Fork
has them: one folds the opened commit down to a strip so the graph can have
the panel, the other opens the commit in a window of its own. The message,
the files and the patch now sit on three different grounds, so the eye finds
the boundaries without reading for them.

**Fixed.**

- The simulator on a machine that had only ever installed rusty's own QEMU
  stopped at "-bios argument not set, and ROM code binary not found": that
  build does not find its `share/qemu` beside itself on Windows. rusty names
  the directory on the command line, which both builds accept.
- The environment check could declare a machine ready while the status bar
  said rust-analyzer was missing. The Toolchain panel took the file rustup
  puts on every PATH — its proxy — for the component itself; it now asks the
  binary, as the editor always did, and a proxy with nothing behind it
  counts as not installed. The sheet then offers the one-click install.

## v0.5.3

**Fixed.** The dock hides behind an X, as VS Code's panel does, instead of a
downward chevron that read as "expand" on a dock already open.

## v0.5.2

Three things a first run on a fresh Windows machine, behind a busy proxy,
turned up — and one more from a day of reading diffs side by side.

**Downloads resume.** A tool archive that stopped part-way — the RISC-V C
toolchain is 420 MB, and a slow link through a proxy gets a fixed time per
attempt — used to start over from zero on the next route, and the next, and
never finish. What has arrived now stays on disk, the next attempt asks the
server for the rest, and a route that was delivering is asked again before
the ladder moves on.

**`espup install` names its version.** espup's own "latest" lookup asks
GitHub's API, which refuses unauthenticated calls once the shared address
behind a proxy has used its hourly quota — a 403 that read as espup being
broken. The setup and the Toolchain panel now install a pinned Xtensa Rust
release, fetched from GitHub's release downloads, which have no such quota;
`espup update` moves a machine forward later.

**The Windows linker is checked first.** An `-msvc` Rust links through
Visual Studio's `link.exe`, and without the C++ build tools every `cargo
install` compiled for a minute and died with "linker `link.exe` not found".
The Toolchain panel and the environment check now look for it the way rustc
does, and a machine without it is told that one thing — with the link —
before anything else is offered.

**Fixed.** The hunk header of a side-by-side diff is drawn once on each side,
as Fork draws it, instead of once across both — the single header crossed
the line between old and new.

## v0.5.1

The Git panel, rounded off from a day of using it beside Fork.

**Diffs side by side.** Every diff the panel shows — a commit's file, a
working-tree change, what a stash holds — can be read as two columns, old on
the left and new on the right, or as one column in git's own order; both
carry line numbers, and the choice is remembered. A file with nothing to lay
out (a binary) shows what git said instead of an empty pane.

**The boundaries move.** The log against the opened commit, the message
against its files, the files against the patch, the two columns of the
Changes view, and the line between old and new in a side-by-side diff are
draggable dividers, remembered like the sidebar's and the dock's, and reset
with them by View ▸ Reset layout.

**Right-click.** A commit offers copy hash, a new branch from here, a
detached checkout, cherry-pick and revert; a file offers open in editor and
copy path. Double-clicking a file opens it. Every write runs as the same
visible `git` command a button would.

**Branches are a picker.** One button names the branch the history is
filtered to and opens a menu of them all — local, then remote, the
checked-out one marked — in place of a row of chips that a repository with
thirty branches turned into a paragraph.

**And the rest.** Amend, as a checkbox by the commit button — turning it on
fills the box with the last commit's whole message, so a reworded summary
cannot silently lose the paragraphs under it. A stash opens below its list
when clicked, files and patches like a commit. When the log is cut off at
its newest four hundred commits, a link asks for older ones. The graph's
lines no longer look cut at the row under the pointer, and the commit box
lost the sentence beside its button that said what the disabled button
already said.

## v0.5.0

The repository joins the workbench: a Git panel with Fork as the reference,
so the history, the working tree and the branches are read and moved without
leaving for a second window.

**Three views behind one strip of branches.**

*History* is the log as a graph — lanes for branches, merges bending back in,
branch and tag labels on the commits that carry them — beside the commit
list. Open a commit and its message, the files it touched and each file's
patch are below. The strip filters the log to one branch; the history follows
the disk, so a commit made in a terminal appears without a click.

*Changes* is the working tree in two lists: what the next commit would carry
and what it would not, each file's diff on the right, and a click moves a
file between the two (or all of them at once). A commit box below takes a
message of any length — Ctrl+Enter commits.

*Stashes* puts the working tree aside, untracked files included, with a note;
each stash can be applied, popped or dropped.

Branches: check one out, create one from the branch selected in the strip (or
from HEAD), delete one the safe way — `git branch -d` refuses unmerged work,
and that refusal is the right answer. The rail carries fetch, pull and push;
a push with no upstream yet sets one on `origin`.

Every write is a visible `git` command in the dock, so the exact line and
everything git says back are readable — a checkout refused on a dirty tree, a
push rejected — except staging, which is instant and reversible and would
bury the commands that matter. It is the user's own `git` doing the work:
their config, their credentials, their hooks. A project that is not a
repository says so in the panel rather than raising an error.

## v0.4.0

Tests get an entry point where you look for one, and a debugger behind it —
including on Windows, which until now could not debug host code at all.

**Run and debug a test from beside it.** A `▶ Run Test | Debug` lens sits by
every `#[test]` and every module holding one, on the attribute line above it,
in place of the small arrow that used to hide at the left edge of the margin.
Run is what the arrow did: `cargo test <name> -- --nocapture`, in the dock.
Debug is new. It builds the test binaries, asks each one which of them holds
the test you clicked, and runs that one under a debugger with your breakpoints
already placed — stepping, the call stack and locals in the Debug panel, the
same as a firmware session. What the test prints goes to the Output tab.

**Debugging on Windows.** rusty drove gdb and nothing else, and gdb reads
DWARF. Rust's default Windows target emits a PDB, so Debug could only have
set breakpoints that never hit and shown addresses where your source should
be. It now speaks the Debug Adapter Protocol as well, which is how LLDB is
driven, and chooses the debugger from what the target actually produces.

The adapter is one click in the Toolchain panel, the way QEMU and the esp
debuggers already were: rusty fetches CodeLLDB into its own tools directory
and runs it from there. It carries its own LLDB, so there is nothing else to
install and no editor to have. An `lldb-dap` already on your PATH is used if
it answers. Where a platform publishes no build, the panel links the release
page rather than offering a button that could only fail.

**Fixed.**

- The editor could take the whole window with it. Moving the pointer over
  code arms a short timer for the hover card, and closing the file or
  switching project inside that moment left the timer reading state that had
  gone — which ends the interface, not just the hover. The window kept its
  last frame and answered nothing: no error, and even the close button dead.
- Run Test ran in the firmware crate on the standard embedded layout, where
  a bare-metal target has no test harness, so it failed with "can't find
  crate for `test`" against tests that were fine. Host commands run at the
  project you opened, which is where the testable crates are.

Debug still refuses rather than pretending: when nothing on the machine can
read the target's debug information it says which adapter to install, and a
test name that matches in two binaries is refused instead of one of them
being run silently.

## v0.3.1

A review release: a hundred findings from a top-to-bottom read of the
code, each fixed with a test that would have caught it. Nothing new to
learn; a good deal that now does what it already claimed to.

**Fixed, and you would have hit these.**

- Project search for anything with an angle bracket — `Vec<u8>`, `->`,
  `=>` — found nothing, silently, because the literal escaper turned the
  brackets into word-boundary assertions. Replace used a different escaper
  and so could rewrite matches the panel never listed.
- Typing a Chinese (or any multi-byte) character while a region was folded
  crashed the window: the fold arithmetic split a character in half.
- With Vim keys on, an input method could type into normal mode. The
  read-only guard that prevents it had been left on a menu item instead of
  the editor.
- The Registers tab, stopped at a breakpoint with a peripheral selected,
  asked gdb for the same block over and over — each answer re-triggered the
  read.
- Settings ▸ Assistant ▸ "Test connectivity" said "Reachable" without ever
  contacting the endpoint: every failure was swallowed into an empty model
  list, and Anthropic was never asked at all. It now reports what it did
  and did not check.
- Switching projects leaked the rust-analyzer session's pull thread — and
  every open document's text with it — and a file watcher per switch. Both
  now stop with the project.
- The Toolchain panel probed `rustup` from rusty's own directory, so a
  project pinned to a different toolchain was told its target was missing
  and offered a fix that installed into the wrong toolchain.
- A project with no board file was drawn with the classic ESP32 header
  whatever its chip, so a C3 sheet offered pins the part does not have.
- Switching a project's chip refused a `.cargo/config.toml` with Windows
  line endings, or a `channel="esp"` written without spaces, as "changed
  since this was planned".
- `workbench.toml` writes shared one temporary file with no lock; two
  windows saving at once could corrupt it, after which the recent-projects
  list came back empty.
- Help ▸ Report a problem on Windows handed the URL to `cmd /C start`
  unquoted; a `&` in it ran a second command.
- The shortcut overrides and the interface scale loaded only when a project
  came back through the recents list — not through the picker, a reload or
  a detached window.

**In the window.** The error banner is an overlay and no longer shoves the
workspace down forty pixels on arrival, and it stays until dismissed rather
than vanishing when some unrelated background call succeeds. The View menu
and the command palette list all nine dock tabs, not five. The terminal no
longer grabs the keyboard every time the dock is resized. Some sixty pieces
of English that had escaped translation — palette headings, the waves
header, the flight blockers, the memory table — are in the catalogue, and a
test now reads the source for prose that bypasses it. Every language has
translations for the tools the first-run check installs.

**On the machine.** `cargo test` no longer requires an installed esp gdb,
rustfmt or an OS keychain to pass, so the public CI is green again. The
frontend served by `trunk serve` alone is interactive again. Assistant
requests go through the configured proxy, time out, and can be cancelled.

## v0.3.0

A flight controller can now be developed at a desk, and a fresh install
tells you what it needs before you find out the hard way.

**A first-run check.** A machine with no Rust, no target or no espflash used
to produce a workbench that could do nothing and said so only if you found
the right panel. rusty now checks on launch, lists what is missing, says
which command installs each thing and *where it lands* — cargo's bin, rustup's
home, or rusty's own data directory — and installs them in the order that
works, stopping at the first failure rather than reporting a ready machine
that is not. Help ▸ "Check my environment…" runs it on purpose.

**The loop closes.** The simulator now models a rigid body between the motors
and the gyro, so a rate loop can be watched *settling* rather than only
answering. Firmware declares the sensors it wants fed (`[rusty:sensor]
gyro=3 rad/s -35..35`), rusty feeds them (`Igyro=…`, a whole sample per
line so a fused attitude never reads a torn one), and the Flight tab draws
the aircraft where it is actually pointing — the one-second test for a
reversed axis. The board protocol carries numbers in both directions now:
`[rusty:pwm]` for how hard a pin is driven, `[rusty:sensor]` and `A34=` for
what goes in. `examples/rate-loop` is the worked end; `flight_probe` proves
it headless and requires a bad tune to look bad. It is a model and says so:
no aerodynamics past damping, and nothing about your aircraft but the sign
of each axis, the motor order and whether the loop is stable in shape.

**Pins the emulator actually drives.** rusty ships its own build of
Espressif's QEMU with a real GPIO model — the stock one has none, so a LED
lit because the firmware *said* it set a pin, never because the pin went
high. With rusty's build a LED lights from the register, a button is read
through `Input::is_high()`, and the board's caption says which of the two
emulators is running rather than promising one over the other. Built for
Windows, macOS and Linux, downloaded on demand, falling back to Espressif's
with everything working exactly as before.

**In the editor.** Code folding. A file watcher that follows the disk — a
`git checkout` in a terminal updates the tree and reloads unedited files;
an edited one is marked, never replaced. Run arrows beside `#[test]`s. The
panel's actions moved from a toolbar row into the left rail, which gives
every panel forty pixels back.

**On the board.** A motor part: a toy car's drive or a fan, wired for PWM
speed and optionally direction. Parts can be mirrored as well as rotated,
so a seven-segment display faces the chip without its wires crossing.

**Fixed.** Mirroring a part survived only until the project was reopened;
the memory panel's numbers shifted sideways; the chip of a workspace whose
firmware crate is `exclude`d was findable and now is found — the build
follows the chip while the tree follows the user.

## v0.2.1

The macOS and Linux installers, which v0.2.0 shipped without. `bundle.targets`
named `nsis` — a format only Windows can build — so the other two platforms
compiled the app, packaged nothing, and the run went green anyway. The build
now names what each platform should produce, and **fails** when a platform
produces no installer at all.

Everything below is in this release too.

## v0.2.0

The first public build, and the first with the source published. An embedded
Rust workbench for ESP32: it knows which chip a project is for, what fits in
flash, what is on the serial port, and it runs firmware without hardware.

**New since the last build**

- **Vim keys**, off by default, switched on in Settings > Editor. Modes,
  motions, operators, text objects, visual mode and `.`, with the mode in the
  status bar. Ctrl+S, Ctrl+A, Ctrl+C and the rest are untouched, and insert
  mode behaves exactly as it does with this off.
- **A way back.** Jumping to a definition had none: Back and Forward now walk
  the positions the caret has visited, from the View menu, `Alt+←/→`, or
  Vim's `Ctrl+O`/`Ctrl+I`.
- **Live telemetry and tuning.** The Plot panel draws named channels from a
  running board and changes gains **without a reflash**, over the serial line
  the firmware is already printing to.
- **Completion while you type a name**, not only after a dot.
- **Hovering a squiggle** answers what is wrong with it, not what type it is.

**Editing** — files, syntax highlighting, and rust-analyzer behind it for
completion, diagnostics, hover, go-to-definition, signature help and quick
fixes. Multi-tab, project-wide search on ripgrep's engine, format on save.

**The device** — flash and monitor over espflash or a probe, with defmt
decoded. Serial ports are named by the board they look like rather than by
their COM number. A memory report attributes flash and RAM per crate from the
ELF, and refuses to guess a capacity for a chip it does not know.

**Without hardware** — build, image and boot the same binary espflash would
burn, in Espressif's QEMU. A board view lights LEDs, digits and displays from
what the firmware prints; buttons and potentiometers travel back the same way.
Waveforms are captured with the firmware's own timestamps and export as VCD.

**Debugging** — breakpoints in the gutter, stepping, the call stack and
variables, and the chip's registers read live from the vendor's own SVD.

**Tuning** — a Plot panel draws named telemetry channels from a running board
and changes gains **without a reflash**, over the serial line the firmware is
already printing to. `examples/pid-tune` is a whole tuning loop in 200 lines.

**Meeting C** — vendor SDKs are C, so the workbench detects `cc`, `bindgen`,
`esp-idf-sys` and C sources, scaffolds both FFI directions, and refuses before
writing over anything.

Known limits, stated rather than discovered: the board view shows what the
firmware *says* it set, because QEMU's peripheral models expose no GPIO
readback. In-app updating checks and links; it does not install. STM32 is
detected but not yet served the way ESP32 is.
