# Changelog

What each release changed, written for the people downloading it. The release
workflow reads the section matching the tag and publishes it verbatim as the
release body — so this file, not the commit log, is what users see. The
history is public too, but it is written for whoever maintains this; a
release note is written for whoever downloads it, and they are not the same
document.

One `## v<version>` heading per release, newest first.

## Unreleased

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
