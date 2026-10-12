//! A real driver's frame, decoded back into the picture it drew.
//!
//! The fixture is a capture, not a transcript somebody wrote: `ssd1306` and
//! `embedded-graphics` drew an eight-pixel square in each far corner and a
//! line across the middle, and these are the transactions rusty's QEMU
//! reported while they did it. What is proven here is the whole path —
//! the emulator's line format, its per-transaction reporting, and the
//! decoder — against code neither end of it was written beside.

use rusty_embed::screen::{Panel, Screen};

/// Feed a capture into a screen, exactly as the frontend does: a `w` starts
/// a message and a `w+` is more of the one before it.
fn play(capture: &str) -> Screen {
    let mut screen = Screen::of(Panel::Ssd1306);
    for line in capture.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let report = rusty_embed::parse_i2c_report(line)
            .unwrap_or_else(|| panic!("the capture holds a line no parser reads: {line}"));
        match report.verb.as_str() {
            "w" => screen.i2c(&report.bytes),
            "w+" => screen.i2c_more(&report.bytes),
            _ => {}
        }
    }
    screen
}

#[test]
fn a_frame_from_a_real_driver_decodes_into_what_it_drew() {
    let screen = play(include_str!("fixtures/display/ssd1306-frame.txt"));

    assert!(screen.is_on(), "the init sequence switched the panel on");
    assert!(screen.written(), "a framebuffer arrived");

    // The near corner, the far corner, and the line between them. The far
    // one is the last page of the last columns, so it is drawn only if the
    // addressing window walked the whole of the RAM — a decoder that lost
    // the page after the first wrap draws the first square and nothing
    // else.
    for y in 0..8 {
        assert!(screen.lit(0, y), "the near corner at row {y}");
        assert!(screen.lit(7, y), "the near corner's far column at row {y}");
        assert!(!screen.lit(8, y), "and nothing past it at row {y}");
    }
    for y in 56..64 {
        assert!(screen.lit(120, y), "the far corner at row {y}");
        assert!(
            screen.lit(127, y),
            "the far corner's last column at row {y}"
        );
        assert!(!screen.lit(119, y), "and nothing before it at row {y}");
    }
    for x in 0..128 {
        assert!(
            screen.lit(x, 32),
            "the line across the middle at column {x}"
        );
    }
    assert!(!screen.lit(64, 31), "and nothing on the row above it");
    assert!(!screen.lit(64, 33), "or below");

    // And exactly those three shapes: eight rows of the near square, eight
    // of the far one, and one run the width of the glass. A stray run
    // anywhere is a byte placed somewhere the driver did not put it, which
    // is the failure a handful of spot checks would pass through.
    let runs = screen.runs();
    let mut expected: Vec<(usize, usize, usize)> = (0..8).map(|y| (0, y, 8)).collect();
    expected.push((0, 32, 128));
    expected.extend((56..64).map(|y| (120, y, 8)));
    assert_eq!(runs, expected);
}

/// The same capture with every transaction cut in two and the second half
/// marked as a continuation — which is what the emulator reports when a
/// driver writes more than the FIFO holds. A decoder that read the first
/// byte of the second half as a control byte would draw nonsense, and this
/// is the only place that shows it.
#[test]
fn a_transaction_split_across_two_reports_draws_the_same_picture() {
    let whole = play(include_str!("fixtures/display/ssd1306-frame.txt"));

    let mut split = Screen::of(Panel::Ssd1306);
    for line in include_str!("fixtures/display/ssd1306-frame.txt").lines() {
        let Some(report) = rusty_embed::parse_i2c_report(line.trim()) else {
            continue;
        };
        let at = report.bytes.len() / 2;
        match report.verb.as_str() {
            "w" => split.i2c(&report.bytes[..at]),
            "w+" => split.i2c_more(&report.bytes[..at]),
            _ => continue,
        }
        split.i2c_more(&report.bytes[at..]);
    }

    assert_eq!(split.runs(), whole.runs());
}
