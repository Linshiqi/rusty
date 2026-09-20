# Changelog

What each release changed, written for the people downloading it. The release
workflow reads the section matching the tag and publishes it verbatim as the
release body — so this file, not the commit log, is what users see. The
history is public too, but it is written for whoever maintains this; a
release note is written for whoever downloads it, and they are not the same
document.

One `## v<version>` heading per release, newest first.

## v0.6.47

**The screen on the board shows what your driver drew.** A display part that
names its controller — SSD1306 or SH1106, in the part's properties — now
draws the actual pixels the firmware sent it. Nothing is read back from one
of these panels, so every dot on the glass crossed the I2C bus, and rusty
reads that stream the way the panel does: the `ssd1306` crate, ESP-IDF's
driver and a hand-written init sequence all arrive as the picture they drew.
A panel the driver has not switched on is drawn faintly and says so.

**Addressable LEDs light up.** The emulator now models RMT, the peripheral a
WS2812 strip is driven by. Before, the codes went nowhere and the driver's
`wait()` never returned, which reads as the firmware hanging inside your own
`write`. There is a strip part on the sheet — as long as its value says —
lit by the bytes that actually reached the pin.

**A Wokwi diagram's LED strips come across too**, as strips of the length
the diagram gave them — a canvas of rows and columns, a ring or a single
pixel.

**A duty reaches its pin, at the frequency its timer sets.** The emulator
models LEDC, so a servo, a dimmed lamp or a motor driven the ordinary way
now moves in the simulator instead of standing still. A servo's horn follows
the width of the pulse rather than the bare fraction — 1.5 ms is the middle
of its travel whatever the period — and the two pulse widths its ends answer
to can be set on the part, since 500..2500 µs and 1000..2000 µs are both in
use and reading one as the other is forty degrees out at each end.

**An ESP32-C3 project now simulates six peripherals**: pins with their
interrupts, the ADC, I2C, SPI2, LEDC and RMT. Each of the last two used to
be a firmware that hung in a driver call with nothing on screen to say why.

Requires rusty's emulator `qemu-v5`, which the installer carries. An older
copy — one that models the pins and the buses but not these two — is
recognised as older and offered an upgrade from the Simulate panel, rather
than quietly leaving a servo still and a strip dark.

## v0.6.46

**Sensors that move.** A sensor module on the board sheet can now be an
MPU-6050, a BMP280 or a BME280: pick the model in the part's properties and
it answers on the I2C bus the way the real part does, so firmware using an
ordinary driver crate reads it. Sliders under the part set its readings —
acceleration and rotation, temperature, pressure, humidity — and move them
while the firmware runs. Between runs, a slider sets where the next run
starts, and it is saved with the board.

**Type into the simulated serial port.** While a simulation runs, the input
line at the foot of the Output panel sends what you type to the firmware's
serial port, like a serial monitor.

**Run the simulator from the command line.** `rusty-cli sim` builds the
project, boots it in the emulator and prints what the firmware says. Give it
text to expect or to fail on, a timeout, a file of steps — wait for a line,
press a button on the board, set a sensor reading, check a pin — and a file
to record every pin change into; its exit code says whether the run passed,
so it drops into CI. The same run is offered to other assistants over MCP as
the `simulate` tool (`rusty-cli mcp`), which they ask before using.

**Import a Wokwi diagram.** The board sheet's import button now also reads a
Wokwi `diagram.json`: LEDs, resistors, buttons, potentiometers, RGB LEDs,
seven-segment digits, buzzers, servos, an SSD1306 display and an MPU-6050
come across with their wiring. Anything rusty has no counterpart for is
listed in the Output panel rather than left out silently.

**The simulator says what it cannot do on your chip.** Projects for an
ESP32 or an ESP32-S3 now show, above the board, what the emulator does not
model there — on an ESP32, floating-point code stops the emulator, and the
ADC and the I2C and SPI buses are not modelled yet. The plan's other notes,
such as a board file drawn for another chip, are shown there too.

**Fixed:** a sensor that the firmware reads right after starting was not
yet on the bus when it looked, and neither was an analog value, because the
simulation started before rusty had connected to it — it now waits. And an
older copy of rusty's emulator in the data folder was used in preference to
the newer one that comes with the app, so reading an ADC or a bus waited for
ever; the newest copy is now used, and an older one is offered an upgrade.

## v0.6.45

**The Git history's lines curve like Fork's.** Where a line joins a commit on
another lane it turns into it with a rounded corner instead of cutting across
diagonally: a merge's branch leaves the merge with a curve, and a branch
bends back into the commit it grew from. The main line now stays straight
where a merged branch rejoins it, instead of jogging over into the branch's
lane.

**Branch labels are the colour of their line.** A branch's name in the
history is drawn in the same colour as the line it sits on; the checked-out
branch carries a tick, and a remote branch is a tint of the same colour.

## v0.6.44

**Inlay hints are drawn where they belong, as in VS Code.** A type now sits
right after the name it is about — `let total: f32 = both();` — and the rest
of the line moves over to make room, instead of the hint trailing at the end
of the line. Parameter names are back in front of the arguments they name
(`sample(sensor: &gyro)`). Hints stay attached to their name while you type,
and appear as soon as rust-analyzer has finished loading, where before they
could stay missing until the file was edited.

To make that possible the editor now draws its own caret and selection and
places every click itself: a click after a hint lands where you clicked, and
the input method's candidate window follows the caret. The selection is VS
Code's blue, the caret two pixels wide.

**New — Ctrl over a name makes it a link.** Holding Ctrl (⌘ on macOS) over
a name that has a definition underlines it and shows the hand, as VS Code
does; clicking goes there.

**Fixed — the built-in shell ("rusty bash") in the installed app.** The
terminal said "The shell exited with status 0." and showed nothing, every
time in an installed copy: on Windows the app cannot be the terminal's shell
itself, and now ships the shell as a program of its own.

## v0.6.43

**New — more than one cursor.** Alt+Click adds a cursor, Ctrl+Alt+Up and
Ctrl+Alt+Down add one on the line above or below, Ctrl+D selects the word under
the cursor and then its next occurrence, and Ctrl+Shift+L every occurrence at
once — VS Code's keys. Typing, Backspace and Delete, Enter, Tab, the arrow
keys, Home and End (with Shift to select) and cut and copy act at every cursor;
pasting as many lines as there are cursors puts one line at each; one Ctrl+Z
undoes the whole edit; Escape goes back to one cursor. Not in Vim mode, where
the keys are Vim's.

**New — inlay hints.** The types rust-analyzer infers now appear in grey at the
end of the line — `let total = 0.0;` is followed by `total: f32` — along with
the type at each step of a long method chain and the name of the block a
closing brace ends.
They appear as soon as rust-analyzer has finished indexing, without an edit to
wake them.

**New — sticky scroll.** While you scroll through a long function, its first
line — and the `impl` or `mod` it sits in — stays pinned at the top of the
editor. Click one to jump to it.

**New — a minimap.** A miniature of the whole file down the editor's right
edge, in the file's own colours, with the part on screen shaded. Click or drag
it to scroll.

**New — indent guides.** A faint vertical line at each indentation level, and
a brighter one for the block the cursor is in.

Each of the four can be switched off in Settings ▸ Editor.

**New — rusty's analyses for other assistants.** `rusty-cli mcp` serves the
built-in assistant's tools over the Model Context Protocol, so Claude Code,
Cursor or any other MCP client can ask what your project targets and what is
wrong with its toolchain, where the firmware's bytes went, what a Cargo
feature costs, and which chips and boards rusty knows — computed, not guessed
from your files. For Claude Code: `claude mcp add rusty -- rusty-cli mcp
<project folder>`. Every tool only reads.

**Fixed — the assistant's Cargo tools said to open a project** when one was
open, unless the Crates panel had been visited first. They now load the
project's dependency graph the first time a question needs it.

## v0.6.42

**The same file on both sides of the split.** *Open to the side*, the split
button and Ctrl+\ used to move a file to the right-hand side; now they open it
there as well, as VS Code does. Both sides show one document: what you type on
one side appears on the other straight away, undo on either side undoes the
last change made on either, saving from either saves both, and each side keeps
its own cursor, scroll position and folds. The split button no longer needs a
second file open.

**New — call hierarchy.** *Show call hierarchy*, from the editor's right-click
menu, the View menu or the command palette, lists who calls the function under
the cursor in a new Calls tab of the bottom panel — or, switched to *Outgoing
calls*, what it calls. Each caller opens to show its own callers, a click goes
to the call and a double-click to the function, and a function called more
than once from one place shows how many times.

**New — expand macro.** *Expand macro recursively* opens what the macro call
under the cursor expands to, all the way down, beside your code as a read-only
Rust file.

**New — a tab shows where its file is.** Hovering over a tab shows the file's
full path, and a tab's right-click menu can reveal the file in Explorer (Finder
on macOS) and copy its full or relative path.

## v0.6.41

**Large files open, edit and scroll.** Files over 5,000 lines used to open
read-only and stop being coloured part of the way down. Now anything up to the
2 MB limit is editable and coloured throughout, and typing stays quick however
long the file is: after a pause, only the lines you changed are re-coloured
(a line in a 24,000-line file takes under a millisecond, where the whole file
took a second), and the editor draws the lines on screen rather than all of
them, so scrolling to the middle of such a file draws in about 20 ms.

**New — Go to references, implementations and type definition.** Shift+F12
lists every use of the symbol under the cursor, Ctrl+F12 its implementations,
and *Go to type definition* where its type is defined — from the View menu, the
command palette or the editor's right-click menu. Each list shows the line of
code beside every file and line number; a single implementation or type
definition opens straight away.

**New — Go to symbol and Go to line.** In the file finder (Ctrl+P), type `@` to
list the symbols of the file in front, `#` to search symbols across the
project, or `:` and a line number to jump to that line. Ctrl+Shift+O, Ctrl+T
and Ctrl+G open the finder with each already typed.

**New — matching brackets and occurrences are marked.** With the cursor beside
a bracket, it and its partner are outlined; resting the cursor on a name
highlights the other places that name is used in the file.

**Fixed — Ctrl+/ comments lines again.** It toggled a line's comment and then
toggled it straight back, so on a single line it did nothing, and over a
selection it left the first line out.

**Fixed — memory with long files.** Undo history of a very long file is capped
by size as well as by steps, so a few long files open for a long time no
longer hold hundreds of megabytes.

## v0.6.40

**Fixed — Windows: folders can be renamed, moved and deleted while
rust-analyzer is running.** rust-analyzer kept the project's folders locked,
so renaming a folder such as `src` from the file tree failed with "access
denied". rusty now watches the files and tells rust-analyzer what changed, so
nothing is locked. *Include in this repository…* no longer restarts
rust-analyzer either.

**Fixed — closing a file hands it back to the disk.** A file closed in the
editor stayed, as far as rust-analyzer knew, the way it was when it was open,
even after a `git checkout` or another editor changed it.

**Fixed — Vim mode and folded code.** With a region folded, Vim's keys acted a
line or more away from the cursor anywhere below the fold. Positions now map
across folds, and moving the cursor into a folded region opens it.

**Fixed — text objects in Vim's visual mode.** `viw`, `vi(`, `va"` and the rest
select the word, the inside of the brackets or the quoted string, as in Vim.
Before, `i` switched to insert mode and the next key was typed into the file.

**Fixed — `j` and `k` keep their column across short lines**, as in Vim: going
down over a short or empty line, the cursor comes back to the column it started
in on the next line that is long enough.

**Fixed — the split editor.** The right side remembers its own cursor when you
switch between its files (it was taking the left side's), the Edit menu's
Undo, Cut, Copy, Paste and Rename act on the side you are working in, and
moving a file to the side no longer leaves the other side blank.

**Fixed — lines indented with tabs.** Find highlights, the completion popup,
the Vim cursor and hover now line up with the text on such lines; they sat up
to three columns to the left.

**Fixed — double-clicking a word selects just the word**, not the space after
it, so copying it no longer brings a space along.

## v0.6.39

**Fixed — a long rust-analyzer message no longer breaks the status bar.** While
rust-analyzer starts up, the status bar shows what it is doing in its own
words, and some of those are very long — scanning the standard library
includes the whole folder path. The message now stops at a fixed width with
an ellipsis, and hovering over it shows all of it; the items beside it stay
where they are. When the window is narrow, this message is what gets shorter,
and no item in the status bar wraps onto a second line any more.

## v0.6.38

**Added — Ctrl+Tab switches between open files.** As in VS Code: a quick
Ctrl+Tab goes back to the file you were in before, and another takes you
back again, so two files can be flipped between. Hold Ctrl and keep pressing
Tab to see the open files, most recently used first, and move down the list
(Shift+Tab moves up); let go of Ctrl to open the highlighted one, or press Esc
to stay where you are. It works wherever the focus is, the terminal included,
and with the editor split it switches within the side you are working in. Both
shortcuts can be changed in Settings ▸ Keyboard, and "Switch to recent file"
is in the command palette and the View menu. Ctrl+Tab in the editor used to
insert four spaces; it no longer does.

## v0.6.37

**Fixed — Vim mode has a cursor again.** In normal and visual mode the cursor
had disappeared entirely, so there was no telling where the next key would
act. It is back as a block: on an empty line and at the end of a line too,
and in visual mode on the end of the selection that moves. It blinks while
the editor has focus and hides when it does not, as a caret does.

**Changed — copy and cut act on the selection when there is one, in Vim mode
too.** With a word selected by double-clicking or dragging, Ctrl+C copied the
whole line instead of the word whenever Vim keys were on. Now, as in VS Code,
Ctrl+C and Ctrl+X take the selection when something is selected and the whole
line only when nothing is, and Ctrl+V over a selection replaces it.

## v0.6.36

**Fixed — "Stage all" no longer stages nothing because of one file, and a
failure says what actually went wrong.** In a project where one path could
not be staged, nothing was staged and the error shown was a harmless line
about line endings (`LF will be replaced by CRLF`) — git prints that warning
first and the real reason further down, which rusty dropped. Everything that
can be staged now is, and the message is git's actual error.

**Added — an empty repository inside a project is recognised and can be
folded in.** A workspace generated with an older rusty keeps the generator's
own empty `.git` inside `firmware/`, which git refuses to stage. Its row in
Changes now says *empty repository*, "Stage all" leaves it out, and
right-clicking it offers *Include in this repository…*: its `.git` goes to the
recycle bin (only ever one with no commits, checked again first) and its files
become ordinary files of the project. rust-analyzer restarts briefly while it
happens.

**Fixed — moving something to the recycle bin could fail at random.** Deleting
from the file tree sometimes failed with an internal error, depending on which
background thread happened to run it. It works every time now.

## v0.6.35

**Fixed — Vim's visual mode can select to the right.** Pressing `v` and then
`l` repeatedly stopped growing after two characters, and `V` with `j` stopped
after two lines: the editor worked out where the cursor was from the start of
the selection, which in visual mode is where the selection began, not where
the cursor is. Selections now grow and shrink one step per key in every
direction, as they do in Vim; clicking elsewhere still moves the cursor to
where you clicked.

## v0.6.34

**Fixed — compile errors show where the code is, not only in Output after a
build.** An error like `cannot find type Vector3d in this scope`, an unused
import or a borrow error had no squiggle in the editor, nothing in Problems and
no mark in the file tree: rust-analyzer's own analysis does not report those,
only the compiler does, and rusty was not asking the compiler. It asks now —
once the project has loaded and after every save — and those errors appear
within a few seconds, in the editor, in Problems, and on the file even when it
is not open. They stay put, too: they used to flash up for a moment and vanish.

**Added — the file tree shows what is wrong.** A file with errors is drawn in
red with how many there are, one with only warnings in amber, and each folder
above it takes the colour of the worst thing inside, as VS Code's explorer
does. Fix the error and save, and the colour goes.

**Added — remotes in the Git panel.** A repository started with `git init`
could not be connected to GitHub from rusty at all. The sidebar now has a
Remotes section with **+** to add one by name and URL, and a right-click on a
remote to fetch it, change its URL, rename it, copy its URL or remove it.
Pressing Push when there is no remote yet asks for one, then pushes once it is
added — the first push of a new repository is one step. A remote shows up the
moment it is added, even before anything has been fetched from it, and one
added in a terminal appears on its own.

**Added — copy, cut and paste the whole line.** With nothing selected, Ctrl+C
copies the line the caret is on, Ctrl+X cuts it, and Ctrl+V puts a line copied
that way back as a line above the caret's line — on an empty line, it lands
right there — as in VS Code. It works in Vim mode too: in normal mode the keys
act on the cursor's line, a cut in visual mode cuts the selection, and a paste
in normal mode goes in at the cursor.

**Fixed — Vim's normal mode could still be typed into by an input method.**
The guard meant to stop it was never actually switched on. Chinese input and
other text that arrives without a key press now leave normal mode's buffer
alone, as they were always supposed to.

## v0.6.33

**Fixed — an Xtensa project whose dependencies never load now says why, and
the way out is an upgrade.** On an ESP32, S2 or S3 project, completion, hover
and go-to-definition worked in your own code while `esp_hal::` and every
other crate answered nothing, with *dependencies unresolved* in the status
bar. The last two releases blamed rust-analyzer and offered nothing to do.
The actual cause is narrower: rust-analyzer decides how to talk to cargo from
the toolchain's version number, and Xtensa Rust 1.95.0.0 reports the one
version it gets wrong. The cargo itself is fine — asked the other way, it
reads the whole dependency graph.

So the fix is to move forward, not back. When rusty finds that toolchain, the
Problems tab says so and carries the command:

```
espup install --toolchain-version 1.97.0.0
```

rust-analyzer stays whatever is current. Nothing is pinned or downgraded, and
your project builds exactly as before either way — only the editor's view of
your dependencies was affected.

**Added — Settings ▸ Editor ▸ rust-analyzer binary.** For pointing rusty at a
copy it would not find by itself, such as the one an editor bundles. Leave it
empty and rusty finds its own, as before. It takes effect the next time a
project opens.

## v0.6.32

**Fixed — two tabs are never the same word.** A workspace has three
`Cargo.toml`s, two `lib.rs`es and two `main.rs`es, and the strip showed each
of them as its bare file name — eleven tabs with six labels between them, and
no way to tell which one you were looking at. A tab now carries as much of the
path above it as it takes to tell it from the others, and no more:
`Cargo.toml core`, `Cargo.toml firmware`, `lib.rs core/src`, `main.rs bin`.
A name nobody else is using is still written alone.

**Fixed — a remembered tab whose file is gone is dropped, not kept.** v0.6.30
made the restored *active* file fail quietly; the rest stayed on the strip as
names, so clicking one raised *could not read build.rs* about a file from a
layout that no longer exists. The strip is remembered per project directory,
and a directory can hold a different project than it did last week — which is
exactly what generating over a path you have used before produces. Every tab
is now checked when the strip is read.

**Changed — the status bar says what is actually wrong.** An Xtensa project
whose toolchain is older than the rust-analyzer analysing it sat on
*rust-analyzer: partly loaded* for the whole session, which is true and tells
you nothing you can act on. Where rusty recognises that failure it now says
*rust-analyzer: dependencies unresolved* — which is what it costs — with the
full explanation still in the tooltip.

## v0.6.31

**Changed — a file outside the module tree now reads as inert.** Its name was
dimmed in v0.6.30; the code itself was not, so an open file still looked like
code being analysed when nothing in it is. The whole buffer is drawn drained
now. The squiggle dims with it and stays plainly a squiggle; the hover card
over it does not dim, so the diagnostic and whatever rust-analyzer can do
about it are read at full strength.

This goes deliberately past VS Code, and the protocol is the reason. Measured
off the wire, rust-analyzer's `unlinked-file` arrives as a *hint* spanning the
first **two characters** of the file, with no `Unnecessary` tag — so there is
nothing for VS Code's own dimming to act on, and it dims nothing. rusty does
not need the diagnostic: it reads the `mod` declarations itself, which is also
why it can dim the file in the tree before anybody opens it.

## v0.6.30

**Fixed — a new file or folder is named where it will be.** The box appeared
above the whole tree with the target folder's path beside it, which is a form,
not a file being made. It is now a row inside the folder, indented with its
future siblings, exactly as VS Code does it; starting one in a collapsed
folder opens that folder first.

**Fixed — typing a project name with an input method no longer raises
errors.** A Chinese IME shows its own pinyin segmentation in the field while
composing, so typing `flyegg` passed through `f'l` and `f'l'y` — and every one
of those was sent off to be checked as a crate name and came back as a red
error about a name nobody had typed. Half-composed text is no longer taken as
a name. And a name cargo really would refuse now says so *under the field*,
naming the character that is wrong, with Create disabled until it is fixed —
instead of a banner, or a failure after you have already chosen a folder.

**Changed — a file outside the module tree is dimmed, and says nothing else.**
It used to be a full-width notice with a sentence of explanation and a button,
which is a paragraph where a shade of grey is the message. The file's name is
now simply dimmed — in the tree, in its tab, and in the header — with the
reason on hover, as VS Code dims a file the project does not build. The fix is
where every other fix is: hover the underlined code and rust-analyzer offers
`Insert mod …;`, `Insert pub mod …;` or `Insert pub(crate) mod …;`.

**Fixed — the dim clears the moment the file is declared.** Adding the `mod`
line left the file dimmed anyway: rust-analyzer's own verdict stays until it
re-analyses that file, and it was being allowed to outvote a reading of the
declarations taken a second ago. And "nothing is dimmed" and "this reading
cannot tell" are now different answers, where both used to look like a clean
bill of health.

**Changed — what a toolchain too old for rust-analyzer actually costs.** The
note added in v0.6.29 said the workspace did not load "so completion, hover
and navigation answer nothing there". Measured, that is wrong in the direction
that matters: rust-analyzer retries without the dependency graph and carries
on, so your own code still gets completion, hover and go-to-definition — what
is lost is `esp_hal::` and every other crate. The note says that now, because
a tool that overstates its own breakage is one you stop believing.

## v0.6.29

**Added — a file no `mod` declares is dimmed in the tree, and says so.** A
Rust file that nothing in the project declares with `mod` or `pub mod` is
not part of any crate, so rust-analyzer offers nothing in it at all: no
completion, no hover, no go-to-definition — for ever, while the red
squiggles keep arriving. Nothing on screen used to say that. Such files are
now drawn dimmed, as VS Code dims a file outside the project, with the
reason on hover. Opening one shows a notice above the editor with a button
that writes the missing `mod` line into the parent module for you.

**Added — quick fixes are one click from the pointer.** Hovering anything
with an error or a warning under it now offers what rust-analyzer can do
about it, as buttons on the tooltip you are already reading. `impl
core::ops::Mul for Quaternion {}` offers *Implement missing members*, which
fills in the associated type and the method; an unresolved name offers its
import. They were reachable only by clicking into the line and pressing
Ctrl+. before.

**Added — auto save.** Off by default, in Settings ▸ Editor: the file is
written a second after you stop typing, the way VS Code's *After delay*
works. Ctrl+S is unchanged and still formats with rustfmt first; auto save
never reformats under your fingers.

**Fixed — switching projects is faster.** Opening a project waited for the
full Cargo dependency analysis and for the previous project's rust-analyzer
to shut down before the window could draw anything of the new one — so the
workbench sat on the old project for the best part of a second, and longer
for a project generated a moment ago, whose dependencies cargo has never
resolved. Neither is needed to show the new project: the analysis is read
when a panel that uses it asks, and the old language server is buried in the
background.

**Fixed — a new project from the wizard can be committed.** The workspace
layout left the generator's own `git init` inside `firmware/`, so the
project's first `git add` stopped with `'firmware/' does not have a commit
checked out` and the Changes list showed `firmware/` as one entry instead
of the files in it. That repository is now removed — only ever one with no
commits in it.

**Fixed — a project whose remembered tab is gone opens quietly.** The open
tabs are remembered per directory, and a directory can hold a different
project than it did last week — creating a project where one used to be
greeted it with a red *could not read …* about a file nobody had asked for.
The tab now simply drops off the strip.

**Changed — a toolchain too old for rust-analyzer is named.** rust-analyzer
passes `--lockfile-path` to any cargo that calls itself nightly, and
Espressif's Xtensa fork does while being built from a snapshot that predates
the flag — so `cargo metadata` fails, the workspace never loads, and Output
fills with pages of cargo usage text that read as rusty being broken. That
failure now carries a sentence saying what it is and that `espup update`
fixes it, above the server's own words.

## v0.6.28

**Fixed — when rust-analyzer cannot load your project, rusty now says so.**
A language server that fails to load a workspace — a manifest it cannot
read, a member directory that is not there, a toolchain it cannot find —
carries on parsing files, so errors still appear in the panel while
completion, hover and go-to-definition quietly answer nothing at all, for
as long as the window stays open. That looked exactly like a working
editor. The status bar now turns red with *rust-analyzer: workspace not
loaded*, its tooltip carries the server's own reason, and that reason is
written once into Output.

## v0.6.27

**Fixed — code completion is reliable.** The list opens on the first letter
of a word, as VS Code's does, and keeps asking rust-analyzer as the word
grows, so it no longer stays frozen at what the first two letters found, or
stays away for a whole word because rust-analyzer was busy for a moment. A
list that arrives after you have moved on — pressed Enter, moved the caret,
typed past the word — is dropped, instead of opening where the caret went
and taking your next Enter.

**Changed — completion matches the way VS Code does.** Matching is fuzzy:
`itr` finds `iter` and `hm` finds `HashMap`, while names that start with
what you typed come first, in rust-analyzer's order. Accepting a function
or a macro adds its parentheses with the caret between them and shows the
signature, and postfix templates such as `.if` and `.match` are offered.
Each row shows the item's type or signature, Up and Down wrap round, the
selection goes back to the best match as you type, and Escape closes the
list before it leaves Vim's insert mode.

**Fixed — the import added by accepting a completion could come from
another item** when the list had been asked for again in between. It is
now fetched for the list the item was picked from.

## v0.6.26

**Changed — the Git panel is fast.** A save anywhere in the project used to
re-read the whole repository — nine `git` runs — and redraw every row of
the history. Now a save re-reads the working tree alone, and everything
else is decided by a fingerprint of the repository's own files, taken
without running `git` at all. The history draws the rows on screen instead
of all thousand; clicking a commit runs one `git` instead of four, keeps
the commit you were reading on screen until the next one arrives, and
shows a commit opened before at once. A commit, checkout or fetch made in a
terminal appears within a few seconds without pressing refresh. A very long
diff draws its first 1,500 lines and offers the rest.

**Added — branches, remotes and tags down the side, as Fork has them.**
Each local branch shows how far it is ahead of and behind its upstream, and
says so when the upstream has been deleted. A click goes to the commit, a
double-click checks the branch out — a remote branch as a local branch
tracking it — and the funnel shows that branch's history alone. Right-click
a branch to merge it into the current one, rebase onto it, rename, push or
delete it (on the remote too); a tag to check it out, push or delete it; a
commit to put a branch or a tag on it. The row above shows what is checked
out and how it stands against its upstream, and Pull and Push show how many
commits they would move.

**Added — search the history.** Type part of a hash, a subject, an author
or a branch name: the commits that match stay bright, Enter and Shift+Enter
step through them, and the box says which match you are on. The arrow keys,
Page Up and Down, Home and End walk the log.

**Added — a merge, rebase, cherry-pick or revert that stops on a conflict
says so** at the top of the panel, with how many files still conflict and
Continue and Abort. Names for a new branch, a rename or a new tag are
checked as you type, with git's reason, instead of failing afterwards.

**Fixed — a local branch with a slash in its name** (`feature/x`) was drawn
in the history as a remote branch.

## v0.6.25

**Added — the file tree moves things.** Drag a file or a folder onto a
folder to move it there, onto the empty space below the tree to move it to
the project root; the folder that will receive it is highlighted while you
drag, and a folder never drops into itself. Open tabs follow the file. A
move onto a name that already exists is refused rather than replacing
anything.

**Added — the tree's right-click menu has what VS Code's has.** Cut, Copy
and Paste (a copy pasted beside its original becomes `name copy`), Rename
in place, Delete to the Recycle Bin or Trash after a confirmation, Copy
path and Copy relative path, and Reveal in File Explorer or Finder. A
folder's menu starts with New file and New folder, a file's with Open and
Open to the side, as in VS Code.

**Added — the wizard can create a workspace.** Choose *Workspace* under
Options and the new project is two crates: `core`, whose logic touches no
hardware and whose tests run on this machine with `cargo test` at the
root, and `firmware`, the chip's binary, excluded from the workspace so
those tests never try to build it for the host — the layout rusty already
builds, flashes and simulates from the firmware's own directory.

## v0.6.24

**Added — Ctrl+wheel resizes a Markdown page.** Hold Ctrl and scroll over
a page to make its text larger or smaller, the way the editor's text
already resizes. The page has its own size, separate from the editor's,
and remembers it; figures, formulas and code blocks grow with the text and
the column re-wraps. Settings ▸ Editor has a *Page text size* stepper
beside the editor's.

## v0.6.23

**Added — updates install from inside the app.** Shortly after launch,
rusty asks the release feed whether a newer version exists. When one does,
a sheet shows what changed — the same notes you are reading now — with
*Download and install*, *Later* and *Skip this version*. The download runs
in the background while you keep working, its signature is checked against
the key built into the app before anything is kept, and the update installs
when you choose *Restart now*. Help ▸ *Check for updates…* asks at any time
and says what it found either way, and Settings ▸ Updates shows the same.

**Fixed — `rusty-cli --version` reported 0.6.12 on every release since.**
The release stamped its version into the app alone; the CLI carries it too
now.

## v0.6.22

**Added — code blocks in a Markdown page are highlighted.** A fenced block
whose fence names a language — `rust`, `toml`, `bash`, `c`, `python` and
the rest of the grammars the editor knows — is drawn in the same colours
the editor uses for that language, in the page and in the assistant's
answers. A block with no language, or one rusty has no grammar for, stays
as written.

**Fixed — formulas showed `&nbsp;` where a space belonged.** A formula
spaced with `\ `, `~` or `\nobreakspace` rendered the six characters
`&nbsp;` between its terms, so `(a,\ b)` read as `(a,&nbsp;b)`. It is a
space now.

## v0.6.21

**Added — the assistant drawer resizes.** Drag its left edge to make it
wider or narrower, between 300 and 900 pixels; it was a fixed 400. The
width is remembered, and View ▸ Reset layout puts it back with the other
dividers.

## v0.6.20

**Changed — the output limit starts at 200,000 tokens.** A model that
reasons spends part of its budget thinking before the answer begins, and
the old default of 4,096 was gone before the first word. Profiles that
still carried 4,096 — the old default was the only value there had ever
been — read as the new one; a limit you set yourself is kept. A provider
whose model allows less refuses the request and names its cap; rusty asks
again at that cap and remembers it for the session, so the one default
serves every provider without a second refusal. Settings ▸ Assistant now has
a Max output field, and the note under a cut-off answer shows the count the
provider actually stopped at.

**Fixed — a switch's knob sat outside its track when on.** The toggles in
Settings (Vim keys, and the others) drew the knob past the right edge of the
track in the on position. It sits inside now.

## v0.6.19

**Fixed — a reasoning model's answer never appeared.** Models that stream
their reasoning — DeepSeek's thinking modes, Anthropic's extended thinking,
and the local servers that copy the field — could spend the whole output
budget thinking, and the drawer showed your question with nothing under it
while the meter read 4096 tokens out. The reasoning now shows, folded under
"Reasoning" and open while it streams, and the answer follows it. It is kept
for you to read and is never sent back to the model.

**Added — the drawer says when an answer was cut off.** When the model hits
the output limit, a note under the answer says so, with the number, and
opens the settings where the limit lives. A model that reasons spends part
of that limit thinking, so the note matters most there — raise the limit
for such a model.

## v0.6.18

**Fixed — switching tabs lost your place.** Every document opened at the
previous document's scroll position: the working area kept its offset across
the switch, so a chapter you had read half of came back at the wrong place,
and a freshly opened file did not start at the top. Each tab now remembers
where you left it — the scroll position and, in the source view, the caret —
and comes back exactly there; a new file opens at the top. A jump into a
file (a search hit, a problem, go-to-definition, Back) still lands on its
target.

**Changed — the chip's pins live in the status bar.** The pin map no longer
floats over the editor's corner. The right end of the status bar reads
`ESP32 pins` and opens the map upwards on click; clicking a pin jumps to
where it is named and closes the map, and a click anywhere else closes it.
The dependency and board counts that sat there before are gone. The map is
reachable from every panel, not only Files, and it starts closed rather than
remembering that it was open.

## v0.6.17

**Added — the assistant can read the project.** Ask about a chapter, a
source file or a configuration file and the model reads it before answering:
three new tools list the project's files, search their text the way the
Search panel does, and read a file with line numbers. They see the project
exactly as the Files panel does — inside the project only, `.gitignore`
honoured, build output and dot directories left out — and they never write.
A long file or a long list of results is cut at a limit and says so, so the
model cannot mistake the first half of a file for the whole of it. Before
this, the assistant could name your chip and could not open your README.

**Added — the file you are looking at goes with your question.** The
assistant drawer shows the open file as a chip above the input, the way VS
Code sends the active editor; its × leaves it out of that one question, and
the button that takes its place puts it back. Unsaved edits travel too,
since the question is usually about them. The chip stays on the message in
the conversation, so you can see later which file a question was about.

**Changed — Settings, rebuilt.** Every page is a title over grouped rows,
in the shape of macOS System Settings: a label on the left, its control on
the right, a switch for a yes or no, a segmented control for a handful of
choices, and at most one line of explanation under a group. The paragraph
under every field and the summary under every sidebar entry are gone.

**Changed — the assistant drawer starts quiet.** An empty conversation is
one line and the input. The paragraph about the tools, the suggested
questions and the row of tool names are gone; the tools are listed under
Settings ▸ Assistant.

**Fixed — a saved API key was reported as not saved.** The check ran
alongside the save and sometimes finished first, so the badge said "not
saved" while every request used the key perfectly well. It now says
"saved" the moment the save completes.

## v0.6.16

**Added — a book's chapter reads as a book's chapter.** The Markdown page
now draws formulas, figures and the raw HTML a book carries. `$…$` and
`$$…$$` render as real mathematics — including `aligned` systems, sub- and
superscripts on one base, and the rest of what a physics chapter writes — and
a formula rusty cannot render is shown as its source with the reason in the
tooltip rather than dropped. A `<figure>` with an `<img>` and a caption is a
figure; `<kbd>`, `<sub>`, `<sup>`, `<details>` and the other tags a document
uses render as what they are; anything that would run or embed something is
named and left alone. Pictures the page refers to are read from the project,
relative to the page — `figures/fig-01.svg` beside its chapter shows up as the
drawing. A picture on another machine still does not load, because loading
it would tell that machine you opened the file; its alt text says so.

**Added — an image file opens as a picture.** Click an SVG or a PNG in the
tree and you see the image, where before an SVG opened as its markup and a
PNG as a notice that it was not text. An SVG keeps the source one click away,
with the same toggle a Markdown page has, and the picture follows your edits
as you make them.

**Added — the panel below shows only the tabs that have something to say.**
Problems, Output and Terminal are always there; Waves, Plot, Debug,
Registers, Flight and Devices appear when something puts them there — a run
that starts printing telemetry brings Plot, a sensor declaration brings
Flight, a debug session brings Debug and Registers — and go when you close
them with the × on the tab. The View menu still lists all nine.

**Added — Test, in the title bar between Build and Run.** One click runs
`cargo test` at the project root, where the testable crates of the standard
layout live, with the output in the panel below. Where the opened directory
is the firmware crate itself, Test refuses and says why — the tests would be
built for the chip, which has no test harness — in the panel and in a
banner, rather than by going grey.

**Fixed — a tab reopened from the last session closed itself on the first
click.** rusty restores your open tabs at startup and reads only the active
file; clicking any of the others closed it instead of opening it. They open.

## v0.6.15

**Added — the sheet shows its voltages.** The last release solved the
circuit for the firmware and said so plainly: the panel had not caught up.
It has now.

Click a wire and the probe says what its net is *at* — `1.65 V` beside the
high/low it already told you, which on a divider is the same net saying two
useful things. Select a part and the panel shows what is across it, what is
going through it and what it is dissipating: `1.35 V  4.08 mA  5.49 mW`,
under the value that decides all three, so changing 330 to 1k and watching
the current move is one glance. It follows a running firmware, because the
levels it solves from are the ones the emulator is reporting.

Readings are in the units a meter shows — `4.08 mA`, not `4.0799e-3 A`.

**And where there is no number, the reason is in its place** rather than a
blank. A lamp with no forward voltage says so beside that lamp; a rail
called `VCC` says it names a net without saying what it is at; a part left
with one end loose is named by that end, not by an internal node number.
Each names the property that would answer it.

**Fixed — a current's sign.** The solver's own note about which way it
measures said the opposite of what it does, which is how a reading gets
built backwards. Nothing shipped was wrong; the trap is.

## v0.6.14

**Added — a KiCad schematic opens here, and goes back without the trip
costing anything.** Two buttons in the board sheet's corner import a
`.kicad_sch` and export one. What comes in keeps its parts, its values and
its wiring; what goes out is *your original file with only what you changed
rewritten* — every hierarchical sheet, bus, text box, footprint field and
uuid comes back byte for byte, because the file is patched rather than
regenerated. Move one resistor on a two-hundred-part board and one symbol is
rewritten; every wire still holds its own bytes. A schematic rusty has never
seen before is written whole instead, as a starting point to lay out.

An imported board can also be *run*. rusty drives pins through the devkit,
and a schematic drawn elsewhere has a microcontroller of its own — so its
GPIO-named pins are joined to the devkit's rows and the board simulates. If
two parts both look like the microcontroller, neither is joined and both are
named: that is a question with no right answer.

**Added — what the firmware's converter reads comes from the circuit you
drew.** rusty solves the sheet now, and walks it forward in step with the
firmware running in the emulator. Put a resistor and a capacitor between two
pins, drive one of them, and `adc.read_oneshot()` on the other *climbs
through that RC* — where before it read whatever the sheet had declared, and
a pin driven high would have arrived instantly or not at all.

The solver is written for this rather than borrowed, so every answer is
checked against arithmetic you can do by hand: a divider's ratio, the
current through a lamp's series resistor, the time constant of an RC.
Capacitors charge and inductors decay. What the *panel* shows has not caught
up yet — there are no voltages drawn on the sheet in this release — but what
the firmware sees has.

**Where the sheet does not say enough, it says so and names the property.**
A resistor whose value is a colour, a rail called `VCC` — which names a net
without saying what it is at — a lamp with no forward voltage, a capacitor
whose value is a part number. Each is refused with the property that would
answer it rather than filled in with a guess, because a plausible wrong
number is worse here than no number.

**Added — a potentiometer reaches the converter**, where both its ends sit
on rails and there is nothing left to assume, so a knob on the sheet becomes
real ADC counts through an ordinary `adc.read_oneshot()`.

**Added — three things the board editor was missing.** A wire dropped on
another wire branches, so a third connection to a net is one gesture instead
of hunting for the pin. The devkit turns and mirrors like any other part.
And a resistor's value is read, which is what makes the numbers above
possible.

**Added — the sheet checks more.** A pin with no wire on it, on a part whose
other pins are wired, is now a finding — and a pin you meant to leave
unconnected can be marked so, which is what keeps that finding worth
reading. Two pins that both drive, wired together, is another.

**Fixed — a voltage divider is no longer reported as a short.** Two rails
joined *through a resistor* are the commonest analog circuit there is; only
a connection with nothing in it is a short. Every divider drawn on a sheet
had been flagged.

## v0.6.13

**Added — the installer carries the tools, so a fresh install can already
simulate, debug and flash.** rusty's QEMU, both Espressif debuggers, espflash
and the LLDB adapter ship inside the installer instead of being four things
to find and download first. Nothing is duplicated: a copy you installed
yourself is still the one used — the bundled tool is a floor under a machine
that has nothing, not a preference. The exception is the emulator, where
rusty's build is used even if another is on your PATH, because a stock QEMU
has none of the pin, converter or bus models and the board view would quietly
stop meaning anything.

Rust itself is not in the installer and is not meant to be: which toolchain a
project needs is decided by its own `rust-toolchain.toml`, rustup is the only
thing that installs one correctly, and a frozen copy would be out of date
within weeks. The first-run screen still offers to set that up.

**Added — the simulator reads as well as writes: an ADC, an I2C bus and an
SPI wire.** Firmware in the simulator can now call `adc.read_oneshot()`,
`i2c.write_read()` and `spi.transfer()` — the ordinary drivers, written as
they would be for the part on your desk — and get the board the sheet
describes. Before, none of those three returned a wrong answer: they never
returned at all, because nothing was modelled at those registers and the
driver waited inside your own `read` call. Drag an analog source and the
number the firmware samples moves; give a part an I2C address and the
registers behind it, and its driver reads them; put a display on a chip
select and the bytes it is sent are shown. `examples/sense-board` is the
worked end of all three.

**Added — a part joins a bus by saying so, and being wired to one.** An
`addr` property puts a part on I2C and `regs` is what it answers; `cs` and
`miso` do the same for SPI. Its kind does not decide — a sensor, a display
and a breakout imported from LCSC all reach a bus the same way. A part with
an address whose SDA and SCL reach no GPIO is named rather than quietly
honoured: the emulator would answer it and the board on your desk would not.

**Fixed — a bus scan finds only what is there.** An I2C address nobody
declared does not acknowledge, so firmware probing for an optional device
finds it exactly when the sheet says it is fitted.

**Fixed — a package with several units is several parts.** A quad op-amp
imported from KiCad came out as one symbol with every unit's pins on top of
each other. It is now one symbol per unit, `LM324_A` and `LM324_B`, each
placed and wired on its own.

**Added — a sensor on the board, and a probe on every wire.** A sensor
module can be placed and wired like anything else; its value names the
channel the firmware declared with `[rusty:sensor]`, and the sliders under
it feed that channel. Selecting a wire now says what net it is on — high,
low, or floating with nothing driving it — and lists every pin joined to
it.

**Added — a pin edge interrupts the firmware in the simulator.** rusty's
emulator keeps the GPIO interrupt registers now and raises the line the
interrupt matrix carries, so firmware that asks to be woken by a button —
how nearly every real button is read — runs its handler instead of waiting
for ever. Polling worked before; interrupts did not, and nothing said so.

**Added — rails, net labels, a buzzer and a servo.** A ground or supply
symbol is that rail wherever it is drawn, so a lamp's cathode no longer
needs a wire across the whole sheet; two labels carrying the same name are
one net, which is what a schematic uses instead of a long wire. A buzzer
sounds by the same rule a lamp lights by — and, unlike a lamp, is not asked
for a series resistor. A servo's horn follows the duty on its signal pin.

**Added — `rusty-cli`'s board probe.** `cargo run -p rusty-embed --example
board_probe -- <project>` boots the project in the emulator with the pin
channel attached, replays the sheet's rules over the pins the emulator
actually reports, and says what each part did: a lamp that lit and went out
is a lamp the firmware is driving. Then it presses every button on the
sheet and requires the pin it reaches to move. It exits non-zero when a lamp
wired to a GPIO never lights or a button moves nothing, so a board can be
kept working by a machine rather than by somebody looking at it.

**Changed — the sheet's parts look like the parts.** A 5 mm LED with its
flat and its long anode leg, a resistor wearing the colour bands of its own
value, a capacitor, a tactile switch whose cap sinks when it is pressed, a
screen on a carrier board that shows what the firmware prints on the screen
itself, a motor in its can — beside a devkit that is already drawn as the
board on the desk. Wires attach to the end of a leg, where they do on the
bench. A part imported from LCSC that says what it is — `LED`, `R`, `SW` —
is drawn as that part; anything else is drawn as a package with its pins
down both sides and its name on the body. Placing a part now shows the part
under the cursor rather than an empty rectangle.

## v0.6.12

**Added — the board is a schematic.** Parts on the Simulate sheet are KiCad
symbols with real pins — `Device:R`, `Device:C`, `Device:LED`,
`Device:SW_Push`, and the simulator's own pot, analog source, display,
RGB lens, digit and motor — and wires join pin to pin: a part's pin to
the devkit's header, or part to part. What lights, conducts and drives
what is read off the wires: GPIO → R → LED → GND lights the LED, the
wrong way round stays dark, a capacitor passes nothing, a button to
ground drives its GPIO low while pressed, and a lamp with no series
resistor is pointed out in words. A part LCSC (嘉立创) sells is imported
by its number from the library panel (or `rusty-cli symbol C2286`), read
from EasyEDA's own drawing into the same symbol format and kept in the
data directory's `symbols/lcsc.kicad_sym`, which KiCad can open; a
project's own `.kicad_sym` files under `.rusty/symbols/` join the
library too. `.rusty/sim.toml` is now `[[part]]` and `[[wire]]` (version
2); a first-format file is read as the circuit it claimed and rewritten
the first time you save, and `.rusty/parts/*.toml` lamps are retired in
favour of symbols.

**Fixed — the bundled QEMU would not boot under `cargo tauri dev`.** The
emulator that ships with the app was started with its ROM directory spelled
as a Windows verbatim path (`\\?\E:\…`), which QEMU cannot join a file
name to, so every run ended in `ROM code binary not found` with the ROM
sitting exactly where the bundle had put it. The directory is spelled
plainly now.

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
