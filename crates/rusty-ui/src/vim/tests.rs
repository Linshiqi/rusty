//! What the keys mean, pinned.
//!
//! Written against real Vim's behaviour rather than against this
//! implementation: each test names the property a Vim user would notice
//! missing, because "it does something" is not the bar — the bar is that the
//! muscle memory of someone who has used Vim for a decade produces what they
//! expect on the first try.

use super::*;

/// Type a string of keys and report the buffer, the cursor and the mode.
///
/// Keys are one character each, which covers everything phase one parses;
/// `<Esc>` is spelled out because it is the one key with no character.
fn run_at(vim: &mut Vim, keys: &str, text: &str, cursor: usize) -> (String, usize) {
    let mut text = text.to_string();
    let mut at = cursor;
    let mut rest = keys;
    while !rest.is_empty() {
        let key = if let Some(tail) = rest.strip_prefix("<Esc>") {
            rest = tail;
            Key::new("Escape")
        } else if let Some(tail) = rest.strip_prefix("<C-r>") {
            rest = tail;
            Key::ctrl("r")
        } else {
            let c = rest.chars().next().expect("not empty");
            rest = &rest[c.len_utf8()..];
            Key::new(&c.to_string())
        };
        let step = vim.feed(&key, &text, at);
        if let Some(next) = step.text {
            text = next;
        }
        at = step.cursor;
    }
    (text, at)
}

/// The common shape: start in normal mode at the top of a buffer.
fn run(vim: &mut Vim, keys: &str, text: &str) -> (String, usize) {
    run_at(vim, keys, text, 0)
}

fn go(keys: &str, text: &str) -> (String, usize) {
    let mut vim = Vim::default();
    run(&mut vim, keys, text)
}

// ─────────────────────────────────────────────────────────────────────────────
// The thing that makes every other shortcut keep working
// ─────────────────────────────────────────────────────────────────────────────

/// The single most important property. Vim owns unmodified keys; every chord
/// except a short, explicit list belongs to the editor and the global
/// bindings — so Ctrl+S saves, Ctrl+K opens the palette, and Ctrl+A selects
/// all exactly as they did before Vim was switched on.
#[test]
fn chords_the_editor_owns_are_not_taken() {
    let mut vim = Vim::default();
    for key in ["s", "k", "a", "c", "v", "x", "z", "y", "f", "h", "p", "1"] {
        let step = vim.feed(&Key::ctrl(key), "hello", 0);
        assert!(
            !step.handled,
            "Ctrl+{key} was taken by Vim; it belongs to the editor",
        );
    }
}

/// The five it does take, and only in a command mode.
#[test]
fn the_claimed_chords_are_exactly_five() {
    let mut vim = Vim::default();
    let claimed = [
        ("r", Ask::Redo),
        ("o", Ask::Jump { back: true }),
        ("i", Ask::Jump { back: false }),
        ("d", Ask::Scroll { down: true }),
        ("u", Ask::Scroll { down: false }),
    ];
    for (key, expected) in claimed {
        let step = vim.feed(&Key::ctrl(key), "hello", 0);
        assert!(step.handled, "Ctrl+{key} should be Vim's");
        assert_eq!(
            step.ask,
            Some(expected),
            "Ctrl+{key} asked for the wrong thing"
        );
    }
}

/// Insert mode gives the keyboard back. Anyone who turned Vim on still has
/// completion, quick fixes and save where they were — the alternative is an
/// editor that feels broken the moment you start typing.
#[test]
fn insert_mode_claims_nothing_but_escape() {
    let mut vim = Vim {
        mode: Mode::Insert,
        ..Vim::default()
    };
    for key in [Key::ctrl("s"), Key::ctrl("r"), Key::new("a"), Key::new("d")] {
        let step = vim.feed(&key, "hello", 0);
        assert!(!step.handled, "insert mode took {key:?}");
    }
    let step = vim.feed(&Key::new("Escape"), "hello", 3);
    assert!(step.handled);
    assert_eq!(vim.mode, Mode::Normal);
    assert_eq!(step.cursor, 2, "Vim steps left when leaving insert mode");
}

/// A sequence with no meaning is reported, not eaten. A key that vanishes
/// teaches people the editor is broken; a named refusal teaches them the
/// command set.
#[test]
fn an_unknown_sequence_says_so() {
    let mut vim = Vim::default();
    let step = vim.feed(&Key::new("q"), "hello", 0);
    assert!(step.handled, "normal mode still owns the key");
    assert_eq!(vim.rejected.as_deref(), Some("q"));
    assert!(vim.hint().contains("no command"));

    // And it clears on the next key that parses.
    vim.feed(&Key::new("l"), "hello", 0);
    assert_eq!(vim.rejected, None);
}

/// A half-typed command is shown, the way Vim shows `2d` in the corner.
#[test]
fn a_pending_command_is_visible() {
    let mut vim = Vim::default();
    vim.feed(&Key::new("2"), "hello", 0);
    vim.feed(&Key::new("d"), "hello", 0);
    assert_eq!(vim.hint(), "2d");
}

// ─────────────────────────────────────────────────────────────────────────────
// Motions
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn hjkl_moves_and_stops_at_the_edges() {
    let text = "abc\ndef";
    assert_eq!(go("ll", text).1, 2);
    // `l` does not run off the end of the line: normal mode's cursor is on a
    // character, and there is none after `c`.
    assert_eq!(go("lllll", text).1, 2);
    assert_eq!(go("h", text).1, 0, "`h` stops at the line start");
    assert_eq!(go("j", text).1, 4, "down keeps the column");
    assert_eq!(go("llj", text).1, 6);
    assert_eq!(go("jk", text).1, 0);
}

#[test]
fn a_shorter_line_below_clamps_the_column_without_losing_it() {
    // Vim remembers the column across a short line. This does not yet, and
    // the test says which half is guaranteed: the clamp.
    let (_, at) = go("lllj", "abcdef\nxy");
    assert_eq!(at, 9, "clamped to the end of the short line");
}

#[test]
fn word_motions_treat_punctuation_as_its_own_word() {
    let text = "foo.bar baz";
    assert_eq!(go("w", text).1, 3, "`w` stops at the dot");
    assert_eq!(go("ww", text).1, 4);
    assert_eq!(go("W", text).1, 8, "`W` only stops at blanks");
    assert_eq!(go("$b", text).1, 8);
    assert_eq!(go("e", text).1, 2, "`e` lands on the word's last character");
}

#[test]
fn line_motions() {
    let text = "  indented\nsecond";
    assert_eq!(go("$", text).1, 9);
    assert_eq!(go("$0", text).1, 0);
    assert_eq!(go("$^", text).1, 2, "`^` is the first non-blank");
    assert_eq!(go("G", text).1, 11, "`G` is the last line's first word");
    assert_eq!(go("Ggg", text).1, 2, "`gg` is the first line's");
}

#[test]
fn find_within_the_line_only() {
    let text = "a-b-c\nd-e";
    assert_eq!(go("f-", text).1, 1);
    assert_eq!(go("2f-", text).1, 3);
    assert_eq!(go("t-", text).1, 0, "`t` stops before");
    assert_eq!(go("$F-", text).1, 3);
    // No `-` after the cursor on this line, and find never crosses one.
    assert_eq!(go("$f-", text).1, 4, "a find that fails moves nothing");
}

#[test]
fn semicolon_repeats_a_find_and_comma_reverses_it() {
    let text = "a-b-c-d";
    assert_eq!(go("f-;", text).1, 3);
    assert_eq!(go("f-;;", text).1, 5);
    assert_eq!(go("f-;,", text).1, 1, "`,` goes back the other way");
    // The one that catches an off-by-one: repeating `t` has to step off the
    // character it stopped in front of, or `;` never moves again.
    assert_eq!(go("t-;", text).1, 2);
}

#[test]
fn percent_matches_the_bracket_and_finds_one_on_the_line() {
    assert_eq!(
        go("%", "if (a) {}").1,
        5,
        "from before the pair, the next one"
    );
    assert_eq!(go("f)%", "if (a) {}").1, 3, "and back again");
    assert_eq!(go("%", "fn(nested(x))").1, 12, "depth is counted");
}

// ─────────────────────────────────────────────────────────────────────────────
// Operators
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn dw_is_exclusive_and_de_is_inclusive() {
    // The distinction Vim's own documentation spends a page on. Getting it
    // wrong leaves a stray character behind on every `de`.
    assert_eq!(go("dw", "foo bar").0, "bar");
    assert_eq!(go("de", "foo bar").0, " bar");
}

#[test]
fn dd_takes_the_whole_line_including_its_newline() {
    assert_eq!(go("dd", "one\ntwo\nthree").0, "two\nthree");
    assert_eq!(go("2dd", "one\ntwo\nthree").0, "three");
    // The last line has no newline after it, and deleting it must not leave
    // an empty one.
    assert_eq!(go("Gdd", "one\ntwo").0, "one\n");
}

#[test]
fn counts_multiply_between_the_operator_and_the_motion() {
    // `2d3w` is six words, which is the rule nobody remembers and everybody
    // relies on.
    assert_eq!(go("2d3w", "a b c d e f g").0, "g");
}

#[test]
fn change_leaves_you_in_insert_mode() {
    let mut vim = Vim::default();
    let (text, at) = run_at(&mut vim, "cw", "foo bar", 0);
    assert_eq!(text, " bar");
    assert_eq!(at, 0);
    assert_eq!(vim.mode, Mode::Insert, "`c` ends in insert mode");
}

#[test]
fn d_and_c_to_the_line_end_have_their_shorthands() {
    assert_eq!(go("llD", "foo bar").0, "fo");
    let mut vim = Vim::default();
    assert_eq!(run(&mut vim, "llC", "foo bar").0, "fo");
    assert_eq!(vim.mode, Mode::Insert);
}

#[test]
fn x_deletes_forward_and_stops_at_the_line_end() {
    assert_eq!(go("x", "abc").0, "bc");
    assert_eq!(go("3x", "abcdef").0, "def");
    // `x` at the end of a line must not swallow the newline.
    assert_eq!(go("$5x", "ab\ncd").0, "a\ncd");
}

// ─────────────────────────────────────────────────────────────────────────────
// Yank and put
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn yank_then_put_is_characterwise() {
    let mut vim = Vim::default();
    let (text, at) = run(&mut vim, "yw$p", "ab cd");
    assert_eq!(text, "ab cdab ");
    // On the last character of what was put, which is index 7 of the eight
    // this buffer now has — Vim leaves you where the paste ended.
    assert_eq!(at, 7);
}

#[test]
fn a_linewise_yank_puts_on_its_own_line() {
    // The property that makes `yyp` duplicate a line rather than smear it
    // into the middle of the next one.
    assert_eq!(go("yyp", "one\ntwo").0, "one\none\ntwo");
    assert_eq!(go("yyP", "one\ntwo").0, "one\none\ntwo");
    assert_eq!(go("ddp", "one\ntwo").0, "two\none\n");
}

#[test]
fn yank_parks_the_cursor_at_the_start_of_what_it_took() {
    assert_eq!(go("$yb", "foo bar").1, 4);
}

// ─────────────────────────────────────────────────────────────────────────────
// Text objects — the reason `ciw` is in phase one
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn ciw_changes_the_word_under_the_cursor_from_anywhere_in_it() {
    for at in 4..7 {
        let mut vim = Vim::default();
        let (text, _) = run_at(&mut vim, "ciw", "foo bar baz", at);
        assert_eq!(text, "foo  baz", "from column {at}");
        assert_eq!(vim.mode, Mode::Insert);
    }
}

#[test]
fn daw_takes_the_space_with_it() {
    // The difference that keeps a sentence spaced correctly.
    assert_eq!(go("wdaw", "foo bar baz").0, "foo baz");
    assert_eq!(go("wdiw", "foo bar baz").0, "foo  baz");
    // At the end of a line there is no trailing space, so it takes the
    // leading one instead.
    assert_eq!(go("$daw", "foo bar").0, "foo");
}

#[test]
fn quoted_objects_find_the_string_the_cursor_is_in() {
    let text = r#"let s = "hello";"#;
    assert_eq!(go(r#"ci""#, text).0, r#"let s = "";"#);
    assert_eq!(go(r#"da""#, text).0, "let s = ;");
    // From inside the string, not just from before it.
    let mut vim = Vim::default();
    assert_eq!(run_at(&mut vim, r#"ci""#, text, 11).0, r#"let s = "";"#);
}

#[test]
fn bracket_objects_take_the_innermost_pair() {
    let text = "fn(a, g(b), c)";
    let mut vim = Vim::default();
    assert_eq!(run_at(&mut vim, "di(", text, 8).0, "fn(a, g(), c)");
    let mut vim = Vim::default();
    assert_eq!(run_at(&mut vim, "da(", text, 8).0, "fn(a, g, c)");
    // From outside every pair, Vim's `i(` has nothing to enclose and does
    // nothing at all. Deleting the outer pair here would be this
    // implementation inventing a rule.
    assert_eq!(go("di(", text).0, text);
    // From inside the outer pair but outside the inner one, the outer.
    let mut vim = Vim::default();
    assert_eq!(run_at(&mut vim, "di(", text, 3).0, "fn()");
}

#[test]
fn an_empty_pair_has_an_inside_that_deletes_nothing() {
    // `di(` on `()` must not eat a bracket — an empty span is the right
    // answer, and an off-by-one here corrupts the source silently.
    assert_eq!(go("di(", "fn()").0, "fn()");
}

// ─────────────────────────────────────────────────────────────────────────────
// Insert entry points
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn the_insert_entries_land_where_vim_puts_them() {
    let text = "  foo";
    let mut vim = Vim::default();
    assert_eq!(run(&mut vim, "$i", text).1, 4);
    let mut vim = Vim::default();
    assert_eq!(run(&mut vim, "$a", text).1, 5, "`a` is one past");
    let mut vim = Vim::default();
    assert_eq!(run(&mut vim, "A", text).1, 5, "`A` is the line end");
    let mut vim = Vim::default();
    assert_eq!(run(&mut vim, "$I", text).1, 2, "`I` is the first non-blank");
}

#[test]
fn o_and_shift_o_open_a_line_and_leave_the_cursor_on_it() {
    let mut vim = Vim::default();
    let (text, at) = run(&mut vim, "o", "one\ntwo");
    assert_eq!(text, "one\n\ntwo");
    assert_eq!(at, 4);
    assert_eq!(vim.mode, Mode::Insert);

    let mut vim = Vim::default();
    let (text, at) = run(&mut vim, "jO", "one\ntwo");
    assert_eq!(text, "one\n\ntwo");
    assert_eq!(at, 4);
}

// ─────────────────────────────────────────────────────────────────────────────
// Visual mode
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn visual_selects_through_the_character_under_the_cursor() {
    let mut vim = Vim::default();
    let step = vim.feed(&Key::new("v"), "hello", 1);
    assert_eq!(vim.mode, Mode::Visual);
    assert_eq!(step.selection, Some((1, 2)), "one character, not zero");

    let step = vim.feed(&Key::new("l"), "hello", 1);
    assert_eq!(step.selection, Some((1, 3)));
}

#[test]
fn an_operator_in_visual_mode_acts_on_the_selection() {
    let mut vim = Vim::default();
    let (text, _) = run(&mut vim, "vlld", "hello");
    assert_eq!(text, "lo");
    assert_eq!(vim.mode, Mode::Normal, "and drops back to normal");
}

#[test]
fn visual_line_takes_whole_lines() {
    let mut vim = Vim::default();
    let (text, _) = run(&mut vim, "Vd", "one\ntwo\nthree");
    assert_eq!(text, "two\nthree");
    let mut vim = Vim::default();
    let (text, _) = run(&mut vim, "Vjd", "one\ntwo\nthree");
    assert_eq!(text, "three");
}

#[test]
fn escape_leaves_visual_mode_without_touching_the_buffer() {
    let mut vim = Vim::default();
    let (text, _) = run(&mut vim, "vll<Esc>", "hello");
    assert_eq!(text, "hello");
    assert_eq!(vim.mode, Mode::Normal);
}

// ─────────────────────────────────────────────────────────────────────────────
// The dot
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn dot_repeats_the_last_change_but_not_a_motion() {
    assert_eq!(go("dw.", "a b c d").0, "c d");
    // A motion is not a change, so `.` still repeats the delete — and it
    // repeats it *where the cursor now is*, which is what makes `.` worth
    // having. Two `l`s later that is the third word.
    assert_eq!(go("dwll.", "a b c d").0, "b d");
}

#[test]
fn dot_does_not_repeat_a_yank() {
    // Yanking changes nothing, so `.` must reach past it to the last real
    // change — repeating a yank would silently clobber the register.
    let (text, _) = go("dwyw.", "a b c d");
    assert_eq!(text, "c d");
}

// ─────────────────────────────────────────────────────────────────────────────
// The things the editor answers
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn the_keys_vim_cannot_answer_are_asked_for_rather_than_reimplemented() {
    let mut vim = Vim::default();
    // `u` is the editor's own history, not a second undo stack that would
    // disagree with Ctrl+Z.
    assert_eq!(vim.feed(&Key::new("u"), "x", 0).ask, Some(Ask::Undo));
    assert_eq!(vim.feed(&Key::ctrl("r"), "x", 0).ask, Some(Ask::Redo));
    // `/` opens the find bar that already exists rather than a second search.
    assert_eq!(
        vim.feed(&Key::new("/"), "x", 0).ask,
        Some(Ask::Search { backwards: false })
    );
    assert_eq!(vim.feed(&Key::new("n"), "x", 0).ask, Some(Ask::SearchNext));
}

#[test]
fn zz_saves_and_closes() {
    let mut vim = Vim::default();
    assert!(
        vim.feed(&Key::new("Z"), "x", 0).ask.is_none(),
        "still pending"
    );
    assert_eq!(
        vim.feed(&Key::new("Z"), "x", 0).ask,
        Some(Ask::SaveAndClose)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Undo granularity, and the boundary the editor has to respect
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn a_command_that_changes_the_buffer_seals_the_undo_unit() {
    // Vim undoes a whole command at once. The editor coalesces by time, so
    // without this flag `ciwfoo<Esc>` would undo one keystroke at a time.
    let mut vim = Vim::default();
    let step = vim.feed(&Key::new("x"), "abc", 0);
    assert!(step.seal, "a change seals");

    let mut vim = Vim::default();
    let step = vim.feed(&Key::new("l"), "abc", 0);
    assert!(!step.seal, "a motion does not");
}

// ─────────────────────────────────────────────────────────────────────────────
// Text that is not ASCII
// ─────────────────────────────────────────────────────────────────────────────

/// Indices are Unicode scalars, and a CJK comment must not shift every motion
/// after it. The same trap the LSP client has a 中文 comment in its tests for.
#[test]
fn motions_count_scalars_not_bytes() {
    let text = "中文 comment";
    assert_eq!(go("l", text).1, 1, "one character, not three bytes");
    assert_eq!(go("w", text).1, 3);
    assert_eq!(go("x", text).0, "文 comment");
    // Ten scalars, so the last one is index 9 — and `$` sits *on* it.
    assert_eq!(go("$", text).1, 9);
}

#[test]
fn a_word_object_around_wide_characters_takes_the_word() {
    let mut vim = Vim::default();
    let (text, _) = run_at(&mut vim, "diw", "let 名前 = 1;", 4);
    assert_eq!(text, "let  = 1;");
}

// ─────────────────────────────────────────────────────────────────────────────
// Counts
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn zero_is_a_motion_and_not_the_start_of_a_count() {
    // `d0` deletes to the line start. Reading the `0` as a count would make
    // it `d` with no motion, which waits for ever.
    let mut vim = Vim::default();
    let (text, _) = run_at(&mut vim, "d0", "foo bar", 4);
    assert_eq!(text, "bar");
    // But a zero inside a count is a digit.
    assert_eq!(go("10l", "0123456789abc").1, 10);
}

#[test]
fn join_puts_one_space_where_the_newline_was() {
    assert_eq!(go("J", "one\n   two").0, "one two");
    assert_eq!(go("3J", "a\nb\nc").0, "a b c");
}

#[test]
fn r_replaces_one_character_and_stays_in_normal_mode() {
    let mut vim = Vim::default();
    let (text, at) = run(&mut vim, "rx", "abc");
    assert_eq!(text, "xbc");
    assert_eq!(at, 0);
    assert_eq!(vim.mode, Mode::Normal);
}

#[test]
fn a_colon_line_is_finished_by_enter_not_by_its_length() {
    // `:w` is a prefix of `:wq`, so nothing may fire until Enter — a colon
    // line that acted on the first matching prefix would save and close when
    // asked only to save.
    let mut vim = Vim::default();
    assert!(vim.feed(&Key::new(":"), "x", 0).ask.is_none());
    assert!(
        vim.feed(&Key::new("w"), "x", 0).ask.is_none(),
        ":w has not run yet"
    );
    assert!(vim.feed(&Key::new("q"), "x", 0).ask.is_none());
    assert_eq!(
        vim.feed(&Key::new("Enter"), "x", 0).ask,
        Some(Ask::SaveAndClose),
    );

    let mut vim = Vim::default();
    for key in [":", "w"] {
        vim.feed(&Key::new(key), "x", 0);
    }
    assert_eq!(vim.feed(&Key::new("Enter"), "x", 0).ask, Some(Ask::Save));
}

#[test]
fn an_ex_command_that_does_not_exist_is_named() {
    let mut vim = Vim::default();
    for key in [":", "z", "z"] {
        vim.feed(&Key::new(key), "x", 0);
    }
    let step = vim.feed(&Key::new("Enter"), "x", 0);
    assert_eq!(step.ask, None);
    assert_eq!(vim.rejected.as_deref(), Some(":zz"));
}

#[test]
fn a_colon_line_does_not_run_normal_mode_commands_while_it_is_open() {
    // The trap: `:` then `d` must be text for the line, not a delete waiting
    // for a motion.
    let mut vim = Vim::default();
    let (text, _) = run(&mut vim, ":dd", "one\ntwo");
    assert_eq!(text, "one\ntwo", "nothing was deleted");
}

// ─────────────────────────────────────────────────────────────────────────────
// Indent — the phase-one omission, and the most-pressed key of the lot
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn shift_right_and_left_move_one_level() {
    assert_eq!(go(">>", "fn main() {}").0, "    fn main() {}");
    assert_eq!(go("<<", "    fn main() {}").0, "fn main() {}");
    // Outdenting a line with no indent left is a no-op, not a panic and not
    // a line that loses its first four characters.
    assert_eq!(go("<<", "fn main() {}").0, "fn main() {}");
}

#[test]
fn an_indent_with_a_motion_takes_whole_lines() {
    // `>j` shifts both lines, not the characters between the two carets —
    // an indent that respected a characterwise range would insert spaces
    // into the middle of the second line.
    let text = "one\ntwo\nthree";
    assert_eq!(go(">j", text).0, "    one\n    two\nthree");
    assert_eq!(go("3>>", text).0, "    one\n    two\n    three");
}

#[test]
fn indenting_leaves_blank_lines_alone() {
    // Otherwise every commented-out block leaves a file full of trailing
    // whitespace, which the next rustfmt or reviewer has to clean up.
    assert_eq!(go(">j", "one\n\nthree").0, "    one\n\nthree");
}

#[test]
fn indent_in_visual_mode_uses_the_selection() {
    let mut vim = Vim::default();
    let (text, _) = run(&mut vim, "Vj>", "one\ntwo\nthree");
    assert_eq!(text, "    one\n    two\nthree");
    assert_eq!(vim.mode, Mode::Normal);
}

#[test]
fn indent_puts_the_cursor_on_the_first_word() {
    // Vim's rule, and the one that makes `>>` then `i` land where you mean.
    assert_eq!(go(">>", "fn main() {}").1, 4);
}

// ─────────────────────────────────────────────────────────────────────────────
// The rest of the daily set
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn star_and_hash_ask_for_the_word_under_the_cursor() {
    let mut vim = Vim::default();
    assert_eq!(
        vim.feed(&Key::new("*"), "let radio = 1;", 4).ask,
        Some(Ask::SearchWord { backwards: false }),
    );
    assert_eq!(
        vim.feed(&Key::new("#"), "let radio = 1;", 4).ask,
        Some(Ask::SearchWord { backwards: true }),
    );
}

#[test]
fn z_commands_ask_where_to_put_the_view() {
    let mut vim = Vim::default();
    assert!(
        vim.feed(&Key::new("z"), "x", 0).ask.is_none(),
        "still pending"
    );
    assert_eq!(
        vim.feed(&Key::new("z"), "x", 0).ask,
        Some(Ask::Centre { at: View::Middle }),
    );
    for (key, at) in [("t", View::Top), ("b", View::Bottom)] {
        let mut vim = Vim::default();
        vim.feed(&Key::new("z"), "x", 0);
        assert_eq!(
            vim.feed(&Key::new(key), "x", 0).ask,
            Some(Ask::Centre { at })
        );
    }
}

#[test]
fn a_substitute_line_opens_the_replace_bar() {
    // Not parsed here: this editor's replace is literal and Vim's is a regex
    // dialect. Quietly treating `\(` as one or the other would substitute
    // something nobody asked for.
    let mut vim = Vim::default();
    for key in [":", "%", "s", "/", "a", "/", "b", "/", "g"] {
        vim.feed(&Key::new(key), "x", 0);
    }
    assert_eq!(vim.feed(&Key::new("Enter"), "x", 0).ask, Some(Ask::Replace));
}

#[test]
fn gc_asks_the_editor_to_comment_the_lines_a_motion_covered() {
    // The range is handed over, not the syntax: `//` versus `#` is the
    // document's language, which this module deliberately cannot see.
    let mut vim = Vim::default();
    let step = vim.feed(&Key::new("c"), "one\ntwo", 0);
    assert!(step.ask.is_none(), "`c` alone is still an operator");

    let mut vim = Vim::default();
    for key in ["g", "c", "c"] {
        let step = vim.feed(&Key::new(key), "one\ntwo", 0);
        if let Some(Ask::Comment { from, to }) = step.ask {
            assert_eq!((from, to), (0, 3), "the first line only");
            return;
        }
    }
    panic!("gcc never asked to comment");
}

#[test]
fn gc_with_a_motion_spans_the_lines_it_crossed() {
    let mut vim = Vim::default();
    let mut last = None;
    for key in ["g", "c", "j"] {
        last = vim.feed(&Key::new(key), "one\ntwo\nthree", 0).ask;
    }
    assert_eq!(last, Some(Ask::Comment { from: 0, to: 7 }));
}
