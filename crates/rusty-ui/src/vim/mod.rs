//! Modal editing, as a state machine with no DOM in it.
//!
//! Everything here is a pure function of `(keys, text, cursor)`. The editor
//! reads [`Step`] and does the three things a browser makes it do — set
//! `value`, set the selection, call `preventDefault` — and nothing else. That
//! split is the whole reason this is testable: the board canvas got its
//! arithmetic wrong three times while none of it was reachable from a test,
//! and modal editing is far more arithmetic than the canvas ever was.
//!
//! **Indices are Unicode scalars, not UTF-16 code units.** The textarea counts
//! selections in UTF-16 and the conversion happens at the boundary, exactly as
//! the LSP client converts there — a `中` in a buffer must not shift every
//! motion after it.
//!
//! What is deliberately absent: windows (`Ctrl+W`), the Ex language beyond a
//! handful of commands, and macros. A sequence this does not know is *named*
//! in the status line rather than silently swallowed — half a Vim that eats
//! keys is worse than one that admits the gap.

use std::fmt::Write as _;

pub mod motion;
pub mod object;
#[cfg(test)]
mod tests;

use motion::{Motion, Span};
use object::Object;

/// Which keys mean what right now.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// Keys are commands. The cursor sits *on* a character.
    #[default]
    Normal,
    /// Keys are text. The editor behaves as it does with Vim off, so every
    /// shortcut anyone already learned keeps working.
    Insert,
    Visual,
    VisualLine,
}

impl Mode {
    /// What the status line shows. Vim's own words, because a Vim user reads
    /// them without thinking.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual => "VISUAL",
            Mode::VisualLine => "V-LINE",
        }
    }

    pub fn is_visual(self) -> bool {
        matches!(self, Mode::Visual | Mode::VisualLine)
    }
}

/// One key, as the browser reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    /// `KeyboardEvent.key`: a character like `a`, or a name like `Escape`.
    /// Shift is already baked in — the browser sends `A`, not shift+`a`.
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
}

impl Key {
    pub fn new(key: &str) -> Self {
        Self {
            key: key.to_string(),
            ctrl: false,
            alt: false,
        }
    }

    #[cfg(test)]
    pub fn ctrl(key: &str) -> Self {
        Self {
            key: key.to_string(),
            ctrl: true,
            alt: false,
        }
    }

    /// The single character this key types, if it types one.
    fn char(&self) -> Option<char> {
        let mut chars = self.key.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        }
    }
}

/// What the editor should do with the key.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Step {
    /// True when Vim took the key: the editor calls `preventDefault` *and*
    /// `stopPropagation`, so the global shortcuts never see it. False is the
    /// path that keeps Ctrl+S, the palette and the clipboard working.
    pub handled: bool,
    /// The new buffer, when this step changed it.
    pub text: Option<String>,
    /// Where the cursor belongs afterwards, in scalars.
    pub cursor: usize,
    /// What to select, for visual mode and for the block cursor. `None` means
    /// a plain caret.
    pub selection: Option<(usize, usize)>,
    /// Close the undo unit before applying. Vim's granularity is one command,
    /// not one burst of typing: `ciwfoo<Esc>` undoes in a single press.
    pub seal: bool,
    /// A request the editor answers because Vim cannot — saving, closing,
    /// searching. Keeps this module free of everything but text.
    pub ask: Option<Ask>,
}

/// The few things a command means outside the buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Save,
    Close,
    SaveAndClose,
    Undo,
    Redo,
    /// `/` and `?` — the editor's own find bar, rather than a second search
    /// implementation that would drift from it.
    Search { backwards: bool },
    SearchNext,
    SearchPrevious,
    /// Half a screen, which only the editor knows the height of.
    Scroll { down: bool },
    /// `Ctrl+O` / `Ctrl+I`, answered by the editor's navigation history.
    Jump { back: bool },
    /// `*` and `#`: search for the word under the cursor. The editor pulls
    /// the word out, because it is already the thing holding the caret.
    SearchWord { backwards: bool },
    /// `zz` `zt` `zb` — only the editor knows how tall the view is.
    Centre { at: View },
    /// `:s/…` — the replace bar that already exists, rather than a second
    /// substitution engine that would disagree with it about escaping.
    Replace,
    /// `gc` and `gcc`, over the lines a motion covered. Comment syntax is
    /// the document's language, which the editor knows and this does not.
    Comment { from: usize, to: usize },
}

/// Where `zz`, `zt` and `zb` put the cursor's line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Middle,
    Top,
    Bottom,
}

/// A yanked or deleted span, and whether it was whole lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Register {
    text: String,
    linewise: bool,
}

/// The editor's modal state, one per open document.
#[derive(Clone, Debug, Default)]
pub struct Vim {
    pub mode: Mode,
    /// Keys typed toward a command that is not finished. Shown in the status
    /// line, the way Vim shows a half-typed `2d`.
    pub pending: String,
    /// The last sequence this does not implement, for the status line to
    /// name. Cleared by the next key that parses.
    pub rejected: Option<String>,
    /// Where visual mode started, in scalars.
    anchor: usize,
    register: Register,
    /// The keys of the last buffer-changing command, replayed by `.`.
    last_change: Option<String>,
    /// True while `.` is replaying, so the replay does not record itself.
    replaying: bool,
    /// The last `f`/`t` search, replayed by `;` and `,`.
    last_find: Option<(char, char)>,
}

impl Vim {
    /// Feed one key. The only entry point.
    ///
    /// Returns [`Step::handled`] false for anything Vim does not want, which
    /// is how Ctrl+S, the command palette and the clipboard keep working
    /// while Vim is on.
    pub fn feed(&mut self, key: &Key, text: &str, cursor: usize) -> Step {
        let cursor = cursor.min(text.chars().count());

        // Insert mode claims almost nothing on purpose. Anyone who turned
        // Vim on still has completion, quick fixes, save and the palette
        // exactly where they were.
        if self.mode == Mode::Insert {
            return self.insert_key(key, text, cursor);
        }

        if let Some(step) = self.control_key(key, text, cursor) {
            return step;
        }
        // Any other chord belongs to the editor and to the globals: Ctrl+S
        // saves, Ctrl+K opens the palette, Ctrl+A selects all. Vim's own
        // versions of those are unmodified keys, which cost nothing to reach.
        if key.ctrl || key.alt {
            return self.pass(cursor);
        }

        match key.key.as_str() {
            "Escape" => {
                self.pending.clear();
                self.rejected = None;
                let cursor = if self.mode.is_visual() {
                    self.mode = Mode::Normal;
                    cursor
                } else {
                    cursor
                };
                return self.consumed(text, cursor);
            }
            // Motions arrive as arrows too — a Vim user who reaches for one
            // should not fall out of the model.
            "ArrowLeft" => return self.run_motion(Motion::Left, 1, text, cursor),
            "ArrowRight" => return self.run_motion(Motion::Right, 1, text, cursor),
            "ArrowUp" => return self.run_motion(Motion::Up, 1, text, cursor),
            "ArrowDown" => return self.run_motion(Motion::Down, 1, text, cursor),
            "Enter" => {
                // A colon line is finished by Enter, not by its own length —
                // `:w` and `:wq` are prefixes of each other.
                if let Some(ex) = self.pending.strip_prefix(':') {
                    let ask = match ex {
                        _ if ex.starts_with("s/") || ex.starts_with("%s/") => Some(Ask::Replace),
                        "w" => Some(Ask::Save),
                        "q" | "q!" => Some(Ask::Close),
                        "wq" | "x" => Some(Ask::SaveAndClose),
                        _ => None,
                    };
                    let unknown = ex.to_string();
                    self.pending.clear();
                    let mut step = self.consumed(text, cursor);
                    match ask {
                        Some(ask) => step.ask = Some(ask),
                        None => self.rejected = Some(format!(":{unknown}")),
                    }
                    return step;
                }
                return self.run_motion(Motion::Down, 1, text, cursor);
            }
            "Backspace" => return self.run_motion(Motion::Left, 1, text, cursor),
            _ => {}
        }

        let Some(character) = key.char() else {
            // A named key with no meaning here — F-keys, Home, Tab. Left to
            // the editor rather than eaten.
            return self.pass(cursor);
        };

        // In visual mode an operator acts on the selection at once. Routing
        // it through the operator grammar would leave `vlld` waiting for a
        // motion that is never coming, and the key would look ignored.
        if self.mode.is_visual()
            && self.pending.is_empty()
            && let Some(command) = visual_command(character)
        {
            return self.run(command, text, cursor);
        }

        self.pending.push(character);
        if self.pending.starts_with(':') {
            // Collecting an Ex line: no key means anything until Enter.
            self.rejected = None;
            return self.consumed(text, cursor);
        }
        let pending = self.pending.clone();
        match parse(&pending) {
            Parsed::Incomplete => {
                self.rejected = None;
                self.consumed(text, cursor)
            }
            Parsed::Unknown => {
                // Named, not swallowed. A key that vanishes teaches people the
                // editor is broken; a key that says "no such command" teaches
                // them the command.
                self.rejected = Some(pending);
                self.pending.clear();
                self.consumed(text, cursor)
            }
            Parsed::Command(command) => {
                self.pending.clear();
                self.rejected = None;
                let changes = command.changes_buffer();
                let step = self.run(command, text, cursor);
                if changes && !self.replaying {
                    self.last_change = Some(pending);
                }
                step
            }
        }
    }

    /// Insert mode: only the keys that leave it, or that Vim adds.
    fn insert_key(&mut self, key: &Key, text: &str, cursor: usize) -> Step {
        if key.key == "Escape" && !key.ctrl && !key.alt {
            self.mode = Mode::Normal;
            // Vim steps left on the way out, so the cursor lands on the last
            // character typed rather than past it.
            let cursor = motion::left(text, cursor, 1);
            let mut step = self.consumed(text, cursor);
            step.seal = true;
            return step;
        }
        self.pass(cursor)
    }

    /// The short, explicit list of chords Vim takes.
    ///
    /// Everything absent from it stays with the editor and the global
    /// bindings — which is why Ctrl+A still selects all and Ctrl+C still
    /// copies. Vim's own increment and its "leave insert mode" are the two
    /// this deliberately gives up; `Esc` and `ggVG` cost nothing.
    fn control_key(&mut self, key: &Key, text: &str, cursor: usize) -> Option<Step> {
        if !key.ctrl || key.alt {
            return None;
        }
        let ask = match key.key.to_ascii_lowercase().as_str() {
            "r" => Ask::Redo,
            "o" => Ask::Jump { back: true },
            "i" => Ask::Jump { back: false },
            "d" => Ask::Scroll { down: true },
            "u" => Ask::Scroll { down: false },
            _ => return None,
        };
        let mut step = self.consumed(text, cursor);
        step.ask = Some(ask);
        Some(step)
    }

    fn run(&mut self, command: Command, text: &str, cursor: usize) -> Step {
        match command {
            Command::Move(motion, count) => self.run_motion(motion, count, text, cursor),
            Command::Operate { op, target, count } => self.operate(op, target, count, text, cursor),
            Command::Simple(simple, count) => self.simple(simple, count, text, cursor),
        }
    }

    fn run_motion(&mut self, motion: Motion, count: usize, text: &str, cursor: usize) -> Step {
        let Some(span) = motion::apply(motion, text, cursor, count, &self.last_find) else {
            return self.consumed(text, cursor);
        };
        if let Motion::Find { key, target } = motion {
            self.last_find = Some((key, target));
        }
        // `span.cursor`, never `span.end`: for an inclusive motion like `e`
        // the operator range runs one past the character the cursor lands on,
        // and using the range here puts the cursor off the end of every word.
        self.consumed(text, span.cursor)
    }

    fn operate(
        &mut self,
        op: Op,
        target: Target,
        count: usize,
        text: &str,
        cursor: usize,
    ) -> Step {
        // `cw` is `ce`, which is Vim's one deliberate inconsistency and one
        // every user relies on: changing a word must not eat the space after
        // it, or every rename needs a space typed back.
        let target = match (op, target) {
            (Op::Change, Target::Motion(Motion::WordForward))
                if !on_blank(text, cursor) =>
            {
                Target::Motion(Motion::WordEnd)
            }
            (Op::Change, Target::Motion(Motion::BigWordForward))
                if !on_blank(text, cursor) =>
            {
                Target::Motion(Motion::WordEnd)
            }
            (_, target) => target,
        };

        let range = match target {
            Target::Motion(motion) => {
                let Some(span) = motion::apply(motion, text, cursor, count, &self.last_find) else {
                    return self.consumed(text, cursor);
                };
                if let Motion::Find { key, target } = motion {
                    self.last_find = Some((key, target));
                }
                span
            }
            Target::Line => motion::whole_lines(text, cursor, count),
            Target::Object(object) => {
                let Some(span) = object::apply(object, text, cursor) else {
                    return self.consumed(text, cursor);
                };
                span
            }
            Target::Selection => {
                let (from, to) = self.visual_range(text, cursor);
                Span {
                    start: from,
                    end: to,
                    cursor: from,
                    linewise: self.mode == Mode::VisualLine,
                }
            }
        };
        let (start, end) = (range.start.min(range.end), range.start.max(range.end));
        let taken: String = text.chars().skip(start).take(end - start).collect();
        self.register = Register {
            text: taken,
            linewise: range.linewise,
        };

        if self.mode.is_visual() {
            self.mode = Mode::Normal;
        }

        // Indent and comment act on whole lines however they were reached:
        // `>j` shifts both lines, not the characters between the two carets.
        if matches!(op, Op::Indent | Op::Outdent | Op::Comment) {
            let first = motion::line_start(text, start);
            let last = motion::line_end(text, end.max(start).saturating_sub(
                usize::from(range.linewise && end > start),
            ));
            return match op {
                Op::Comment => {
                    let mut step = self.consumed(text, start);
                    step.ask = Some(Ask::Comment {
                        from: first,
                        to: last,
                    });
                    step.seal = true;
                    step
                }
                _ => {
                    let out = shift(text, first, last, op == Op::Indent);
                    let at = motion::line_first_word(&out, first.min(out.chars().count()));
                    let mut step = self.consumed(&out, at);
                    step.text = Some(out);
                    step.seal = true;
                    step
                }
            };
        }

        match op {
            Op::Yank => {
                // Yank leaves the buffer alone and parks the cursor at the
                // start of what it took, as Vim does.
                self.consumed(text, start)
            }
            // Handled above; the compiler needs the arm.
            Op::Indent | Op::Outdent | Op::Comment => self.consumed(text, start),
            Op::Delete | Op::Change => {
                let mut out: String = text.chars().take(start).collect();
                out.extend(text.chars().skip(end));
                if op == Op::Change {
                    self.mode = Mode::Insert;
                }
                let mut step = self.consumed(&out, start);
                step.text = Some(out);
                step.seal = true;
                step
            }
        }
    }

    fn simple(&mut self, simple: Simple, count: usize, text: &str, cursor: usize) -> Step {
        match simple {
            Simple::Insert => self.enter_insert(text, cursor),
            Simple::InsertLineStart => {
                let at = motion::line_first_word(text, cursor);
                self.enter_insert(text, at)
            }
            Simple::Append => {
                let at = motion::right_for_append(text, cursor);
                self.enter_insert(text, at)
            }
            Simple::AppendLineEnd => {
                let at = motion::line_end(text, cursor);
                self.enter_insert(text, at)
            }
            Simple::OpenBelow => {
                let at = motion::line_end(text, cursor);
                let mut out: String = text.chars().take(at).collect();
                out.push('\n');
                out.extend(text.chars().skip(at));
                let mut step = self.enter_insert(&out, at + 1);
                step.text = Some(out);
                step.seal = true;
                step
            }
            Simple::OpenAbove => {
                let at = motion::line_start(text, cursor);
                let mut out: String = text.chars().take(at).collect();
                out.push('\n');
                out.extend(text.chars().skip(at));
                let mut step = self.enter_insert(&out, at);
                step.text = Some(out);
                step.seal = true;
                step
            }
            Simple::DeleteChar => {
                let end = (cursor + count).min(motion::line_end(text, cursor));
                if end <= cursor {
                    return self.consumed(text, cursor);
                }
                self.register = Register {
                    text: text.chars().skip(cursor).take(end - cursor).collect(),
                    linewise: false,
                };
                let mut out: String = text.chars().take(cursor).collect();
                out.extend(text.chars().skip(end));
                let at = cursor.min(motion::last_column(&out, cursor));
                let mut step = self.consumed(&out, at);
                step.text = Some(out);
                step.seal = true;
                step
            }
            Simple::Paste { after } => self.paste(after, count, text, cursor),
            Simple::Visual => {
                self.mode = if self.mode == Mode::Visual {
                    Mode::Normal
                } else {
                    self.anchor = cursor;
                    Mode::Visual
                };
                self.consumed(text, cursor)
            }
            Simple::VisualLine => {
                self.mode = if self.mode == Mode::VisualLine {
                    Mode::Normal
                } else {
                    self.anchor = cursor;
                    Mode::VisualLine
                };
                self.consumed(text, cursor)
            }
            Simple::Replace(with) => {
                let end = motion::line_end(text, cursor);
                if cursor >= end {
                    return self.consumed(text, cursor);
                }
                let mut out: String = text.chars().take(cursor).collect();
                out.push(with);
                out.extend(text.chars().skip(cursor + 1));
                let mut step = self.consumed(&out, cursor);
                step.text = Some(out);
                step.seal = true;
                step
            }
            Simple::JoinLines => {
                let Some(out) = motion::join(text, cursor, count.max(2)) else {
                    return self.consumed(text, cursor);
                };
                let at = motion::line_end(text, cursor).min(out.chars().count());
                let mut step = self.consumed(&out, at);
                step.text = Some(out);
                step.seal = true;
                step
            }
            Simple::Ask(ask) => {
                let mut step = self.consumed(text, cursor);
                step.ask = Some(ask);
                step
            }
            Simple::Repeat => {
                let Some(keys) = self.last_change.clone() else {
                    return self.consumed(text, cursor);
                };
                self.replaying = true;
                let mut text = text.to_string();
                let mut at = cursor;
                let mut last = self.consumed(&text, at);
                for character in keys.chars() {
                    let step = self.feed(&Key::new(&character.to_string()), &text, at);
                    if let Some(next) = step.text.clone() {
                        text = next;
                    }
                    at = step.cursor;
                    last = step;
                }
                self.replaying = false;
                // A repeat that ended in insert mode has to come back out —
                // `.` replays the change, not the typing that followed it.
                if self.mode == Mode::Insert {
                    self.mode = Mode::Normal;
                }
                last.text = Some(text);
                last.cursor = at;
                last.seal = true;
                last
            }
        }
    }

    fn paste(&mut self, after: bool, count: usize, text: &str, cursor: usize) -> Step {
        if self.register.text.is_empty() {
            return self.consumed(text, cursor);
        }
        let payload = self.register.text.repeat(count);
        // A linewise put lands on its own line. Pasting below the *last*
        // line, which has no newline after it, has to open one first — or
        // `ddp` at the end of a buffer smears the line onto the one above.
        let mut opener = String::new();
        let (at, cursor_after) = if self.register.linewise {
            let at = if after {
                let end = motion::line_end(text, cursor);
                if end >= text.chars().count() {
                    opener.push('\n');
                    end
                } else {
                    end + 1
                }
            } else {
                motion::line_start(text, cursor)
            };
            (at, at + opener.chars().count())
        } else if after {
            let at = motion::right_for_append(text, cursor);
            (at, at + payload.chars().count() - 1)
        } else {
            (cursor, cursor + payload.chars().count() - 1)
        };
        let mut out: String = text.chars().take(at).collect();
        out.push_str(&opener);
        out.push_str(&payload);
        // A linewise register pasted at the end of a buffer with no trailing
        // newline needs one, or the paste lands on the last line instead of
        // below it.
        if self.register.linewise && !payload.ends_with('\n') {
            out.push('\n');
        }
        out.extend(text.chars().skip(at));
        let mut step = self.consumed(&out, cursor_after.min(out.chars().count()));
        step.text = Some(out);
        step.seal = true;
        step
    }

    fn enter_insert(&mut self, text: &str, cursor: usize) -> Step {
        self.mode = Mode::Insert;
        let mut step = self.consumed(text, cursor);
        step.seal = true;
        step
    }

    /// Visual mode's span, ordered, with the anchor's character included —
    /// Vim selects *through* the character under the cursor.
    fn visual_range(&self, text: &str, cursor: usize) -> (usize, usize) {
        let (from, to) = if self.anchor <= cursor {
            (self.anchor, cursor)
        } else {
            (cursor, self.anchor)
        };
        if self.mode == Mode::VisualLine {
            let start = motion::line_start(text, from);
            let end = (motion::line_end(text, to) + 1).min(text.chars().count());
            (start, end)
        } else {
            (from, (to + 1).min(text.chars().count()))
        }
    }

    fn consumed(&self, text: &str, cursor: usize) -> Step {
        Step {
            handled: true,
            text: None,
            cursor,
            selection: self.selection(text, cursor),
            seal: false,
            ask: None,
        }
    }

    fn pass(&self, cursor: usize) -> Step {
        Step {
            handled: false,
            cursor,
            ..Step::default()
        }
    }

    /// What the editor should select: the visual range, or the single
    /// character the block cursor sits on.
    fn selection(&self, text: &str, cursor: usize) -> Option<(usize, usize)> {
        match self.mode {
            Mode::Insert => None,
            Mode::Visual | Mode::VisualLine => Some(self.visual_range(text, cursor)),
            // Normal mode has no selection: the block is the *caret*, drawn
            // by `caret-shape: block`. Selecting the character instead was
            // the first design, and it could not put a cursor on an empty
            // line — there is nothing there to select — so a blank line
            // showed no cursor at all.
            Mode::Normal => None,
        }
    }

    /// The status line's right-hand side: the half-typed command, or the one
    /// this does not know.
    pub fn hint(&self) -> String {
        let mut out = String::new();
        if let Some(rejected) = &self.rejected {
            let _ = write!(out, "no command `{rejected}`");
        } else {
            out.push_str(&self.pending);
        }
        out
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Parsing
// ─────────────────────────────────────────────────────────────────────────────

/// One level of indent. Four spaces, because that is what rustfmt emits and
/// what every file this editor opens is already using; reading it from the
/// buffer would guess wrong on the first blank file.
const SHIFT: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
    /// `gc` — handed to the editor, which knows the language.
    Comment,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Motion(Motion),
    /// `dd`, `cc`, `yy` — the whole line, count of them.
    Line,
    Object(Object),
    /// An operator pressed in visual mode acts on what is selected.
    Selection,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Simple {
    Insert,
    InsertLineStart,
    Append,
    AppendLineEnd,
    OpenBelow,
    OpenAbove,
    DeleteChar,
    Paste { after: bool },
    Visual,
    VisualLine,
    Replace(char),
    JoinLines,
    Repeat,
    Ask(Ask),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Command {
    Move(Motion, usize),
    Operate {
        op: Op,
        target: Target,
        count: usize,
    },
    Simple(Simple, usize),
}

impl Command {
    fn changes_buffer(&self) -> bool {
        match self {
            Command::Move(..) => false,
            Command::Operate { op, .. } => *op != Op::Yank,
            Command::Simple(simple, _) => !matches!(
                simple,
                Simple::Visual | Simple::VisualLine | Simple::Ask(_) | Simple::Repeat
            ),
        }
    }
}

enum Parsed {
    /// A prefix of something real — keep waiting.
    Incomplete,
    /// Nothing starts like this.
    Unknown,
    Command(Command),
}

/// Parse the keys typed so far.
///
/// Deliberately re-parses the whole pending string on every key rather than
/// stepping a machine: the grammar is small, the strings are three characters
/// long, and a pure `&str -> Parsed` is a function a test can enumerate.
fn parse(pending: &str) -> Parsed {
    let (count, rest) = take_count(pending);
    if rest.is_empty() {
        return Parsed::Incomplete;
    }
    let count = count.unwrap_or(1);
    let mut chars = rest.chars();
    let first = chars.next().expect("rest is not empty");
    let tail: String = chars.collect();

    // Operators, which take a second half.
    if let Some(op) = operator(first) {
        return parse_operator_target(op, first, count, &tail);
    }

    if !tail.is_empty() {
        // Two-key commands that are not operators.
        return match (first, tail.as_str()) {
            ('g', "g") => Parsed::Command(Command::Move(Motion::FirstLine, count)),
            ('g', "c") => Parsed::Incomplete,
            ('g', "cc") => Parsed::Command(Command::Operate {
                op: Op::Comment,
                target: Target::Line,
                count,
            }),
            ('g', rest) if rest.starts_with('c') => {
                parse_operator_target(Op::Comment, 'c', count, &rest[1..])
            }
            ('g', _) => Parsed::Unknown,
            ('z', "z") => Parsed::Command(Command::Simple(
                Simple::Ask(Ask::Centre { at: View::Middle }),
                1,
            )),
            ('z', "t") => Parsed::Command(Command::Simple(
                Simple::Ask(Ask::Centre { at: View::Top }),
                1,
            )),
            ('z', "b") => Parsed::Command(Command::Simple(
                Simple::Ask(Ask::Centre { at: View::Bottom }),
                1,
            )),
            ('z', _) => Parsed::Unknown,
            ('Z', "Z") => Parsed::Command(Command::Simple(Simple::Ask(Ask::SaveAndClose), 1)),
            ('Z', _) => Parsed::Unknown,
            ('f' | 'F' | 't' | 'T', target) => {
                let Some(target) = target.chars().next() else {
                    return Parsed::Incomplete;
                };
                Parsed::Command(Command::Move(Motion::Find { key: first, target }, count))
            }
            ('r', with) => {
                let Some(with) = with.chars().next() else {
                    return Parsed::Incomplete;
                };
                Parsed::Command(Command::Simple(Simple::Replace(with), count))
            }
            _ => Parsed::Unknown,
        };
    }

    match first {
        // Waiting for a second key.
        'g' | 'Z' | 'z' | 'f' | 'F' | 't' | 'T' | 'r' => Parsed::Incomplete,

        // The word under the cursor, and where the view sits.
        '*' => Parsed::Command(Command::Simple(
            Simple::Ask(Ask::SearchWord { backwards: false }),
            1,
        )),
        '#' => Parsed::Command(Command::Simple(
            Simple::Ask(Ask::SearchWord { backwards: true }),
            1,
        )),

        'h' => Parsed::Command(Command::Move(Motion::Left, count)),
        'l' | ' ' => Parsed::Command(Command::Move(Motion::Right, count)),
        'j' => Parsed::Command(Command::Move(Motion::Down, count)),
        'k' => Parsed::Command(Command::Move(Motion::Up, count)),
        '0' => Parsed::Command(Command::Move(Motion::LineStart, 1)),
        '^' => Parsed::Command(Command::Move(Motion::FirstWord, 1)),
        '$' => Parsed::Command(Command::Move(Motion::LineEnd, count)),
        'w' => Parsed::Command(Command::Move(Motion::WordForward, count)),
        'W' => Parsed::Command(Command::Move(Motion::BigWordForward, count)),
        'b' => Parsed::Command(Command::Move(Motion::WordBack, count)),
        'B' => Parsed::Command(Command::Move(Motion::BigWordBack, count)),
        'e' => Parsed::Command(Command::Move(Motion::WordEnd, count)),
        'G' => Parsed::Command(Command::Move(Motion::LastLine, count)),
        '{' => Parsed::Command(Command::Move(Motion::ParagraphBack, count)),
        '}' => Parsed::Command(Command::Move(Motion::ParagraphForward, count)),
        '%' => Parsed::Command(Command::Move(Motion::MatchPair, 1)),
        ';' => Parsed::Command(Command::Move(Motion::RepeatFind { reverse: false }, count)),
        ',' => Parsed::Command(Command::Move(Motion::RepeatFind { reverse: true }, count)),

        'i' => Parsed::Command(Command::Simple(Simple::Insert, count)),
        'I' => Parsed::Command(Command::Simple(Simple::InsertLineStart, count)),
        'a' => Parsed::Command(Command::Simple(Simple::Append, count)),
        'A' => Parsed::Command(Command::Simple(Simple::AppendLineEnd, count)),
        'o' => Parsed::Command(Command::Simple(Simple::OpenBelow, count)),
        'O' => Parsed::Command(Command::Simple(Simple::OpenAbove, count)),
        'x' => Parsed::Command(Command::Simple(Simple::DeleteChar, count)),
        'p' => Parsed::Command(Command::Simple(Simple::Paste { after: true }, count)),
        'P' => Parsed::Command(Command::Simple(Simple::Paste { after: false }, count)),
        'v' => Parsed::Command(Command::Simple(Simple::Visual, 1)),
        'V' => Parsed::Command(Command::Simple(Simple::VisualLine, 1)),
        'J' => Parsed::Command(Command::Simple(Simple::JoinLines, count)),
        '.' => Parsed::Command(Command::Simple(Simple::Repeat, count)),
        'u' => Parsed::Command(Command::Simple(Simple::Ask(Ask::Undo), count)),
        '/' => Parsed::Command(Command::Simple(
            Simple::Ask(Ask::Search { backwards: false }),
            1,
        )),
        '?' => Parsed::Command(Command::Simple(
            Simple::Ask(Ask::Search { backwards: true }),
            1,
        )),
        'n' => Parsed::Command(Command::Simple(Simple::Ask(Ask::SearchNext), count)),
        'N' => Parsed::Command(Command::Simple(Simple::Ask(Ask::SearchPrevious), count)),

        // `D`, `C`, `S` are Vim's own shorthands, and people type them
        // constantly — spelling them out here is cheaper than explaining
        // their absence.
        'D' => Parsed::Command(Command::Operate {
            op: Op::Delete,
            target: Target::Motion(Motion::LineEnd),
            count,
        }),
        'C' => Parsed::Command(Command::Operate {
            op: Op::Change,
            target: Target::Motion(Motion::LineEnd),
            count,
        }),
        'Y' => Parsed::Command(Command::Operate {
            op: Op::Yank,
            target: Target::Line,
            count,
        }),
        'S' => Parsed::Command(Command::Operate {
            op: Op::Change,
            target: Target::Line,
            count,
        }),
        's' => Parsed::Command(Command::Operate {
            op: Op::Change,
            target: Target::Motion(Motion::Right),
            count,
        }),

        _ => Parsed::Unknown,
    }
}

/// True when the cursor is on whitespace — the one thing `cw` asks before
/// deciding whether it is really `ce`.
/// Add or remove one level of indent on every line the range touches.
///
/// Blank lines are left alone when indenting — Vim does, and a file full of
/// trailing whitespace is what happens when they are not.
fn shift(text: &str, from: usize, to: usize, deeper: bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut at = 0usize;
    let mut line_start = 0usize;

    while at <= chars.len() {
        let ends = at == chars.len() || chars[at] == '\n';
        if ends {
            let line: String = chars[line_start..at].iter().collect();
            let touched = line_start <= to && at >= from;
            if touched && !(deeper && line.trim().is_empty()) {
                if deeper {
                    out.push_str(&" ".repeat(SHIFT));
                    out.push_str(&line);
                } else {
                    let drop = line
                        .chars()
                        .take(SHIFT)
                        .take_while(|c| *c == ' ')
                        .count();
                    out.push_str(&line[drop..]);
                }
            } else {
                out.push_str(&line);
            }
            if at < chars.len() {
                out.push('\n');
            }
            line_start = at + 1;
        }
        at += 1;
    }
    out
}

fn on_blank(text: &str, cursor: usize) -> bool {
    text.chars().nth(cursor).is_none_or(char::is_whitespace)
}

fn operator(key: char) -> Option<Op> {
    match key {
        'd' => Some(Op::Delete),
        'c' => Some(Op::Change),
        'y' => Some(Op::Yank),
        '>' => Some(Op::Indent),
        '<' => Some(Op::Outdent),
        _ => None,
    }
}

/// The half of an operator command after the operator: `w`, `d` (the line),
/// `iw`, `2j`, and so on.
fn parse_operator_target(op: Op, key: char, count: usize, tail: &str) -> Parsed {
    if tail.is_empty() {
        return Parsed::Incomplete;
    }
    let (inner, rest) = take_count(tail);
    if rest.is_empty() {
        return Parsed::Incomplete;
    }
    let count = count * inner.unwrap_or(1);
    let mut chars = rest.chars();
    let first = chars.next().expect("rest is not empty");
    let tail: String = chars.collect();

    // `dd` / `cc` / `yy`: the operator doubled means whole lines.
    if first == key && tail.is_empty() {
        return Parsed::Command(Command::Operate {
            op,
            target: Target::Line,
            count,
        });
    }

    if matches!(first, 'i' | 'a') {
        let Some(kind) = tail.chars().next() else {
            return Parsed::Incomplete;
        };
        return match object::of(first == 'i', kind) {
            Some(object) => Parsed::Command(Command::Operate {
                op,
                target: Target::Object(object),
                count,
            }),
            None => Parsed::Unknown,
        };
    }

    // Anything else has to be a motion, and it is the same table as a bare
    // motion — `dw` and `w` must never disagree about what a word is.
    let sub = format!("{first}{tail}");
    match parse(&sub) {
        Parsed::Command(Command::Move(motion, inner)) => Parsed::Command(Command::Operate {
            op,
            target: Target::Motion(motion),
            count: count * inner,
        }),
        Parsed::Incomplete => Parsed::Incomplete,
        _ => Parsed::Unknown,
    }
}

/// Split a leading count off. `0` is a motion, not a count, so a count never
/// starts with it — `d0` deletes to the line start.
fn take_count(keys: &str) -> (Option<usize>, &str) {
    let digits: String = keys
        .chars()
        .enumerate()
        .take_while(|(index, c)| c.is_ascii_digit() && !(*index == 0 && *c == '0'))
        .map(|(_, c)| c)
        .collect();
    if digits.is_empty() {
        return (None, keys);
    }
    (digits.parse().ok(), &keys[digits.len()..])
}

/// Vim's operators applied to a visual selection: `d`, `c`, `y`, `x`, `s`.
///
/// Separate from [`parse`] because in visual mode the target is already
/// chosen, and routing it through the operator grammar would wait for a
/// motion that is never coming.
fn visual_command(key: char) -> Option<Command> {
    let op = match key {
        'd' | 'x' => Op::Delete,
        'c' | 's' => Op::Change,
        'y' => Op::Yank,
        '>' => Op::Indent,
        '<' => Op::Outdent,
        _ => return None,
    };
    Some(Command::Operate {
        op,
        target: Target::Selection,
        count: 1,
    })
}
