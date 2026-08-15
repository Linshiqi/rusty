# Contributing

**Please open an issue before writing any code. Unsolicited pull requests
are closed unread.**

That is unfriendly-sounding and it is meant kindly, so here is the reasoning.

This is a one-person project maintained alongside other work. A patch costs
its author an afternoon and costs the maintainer a review, a decision about
whether it fits, and then years of keeping it working as `esp-hal`, Tauri and
rust-analyzer all move underneath it. The second and third costs are the
large ones, and they land on somebody who did not choose the feature.

A generated patch shifts that balance further: producing a plausible diff is
now nearly free, while judging whether it is *right* costs exactly what it
always did. A queue of those is not help — it is a denial of service with
good intentions.

## What is genuinely useful

**Bug reports.** Far more valuable than patches, and there is no queue for
them. The best ones say what you expected, what happened, your chip and
board, and the Output panel's text if a command failed.

**"This was confusing."** A workbench exists to remove confusion, so that is
a defect like any other. Say where you got stuck; you do not need a fix.

**Hardware reports.** This is verified against an ESP32-C3 on a desk. If you
have a part that is not that — an S3, a C6, an STM32 — telling us what it
detected and what it should have detected is worth more than any amount of
code. Half the real bugs in this project were found by flashing to a board
and watching, and one board is one board.

**A chip or a board definition.** Those are TOML in three layers, no code
required. See `docs/extensibility.md`.

## If you do want to write code

1. Open an issue describing the problem — the *problem*, not your solution.
2. Wait for a reply. If it fits, we will say so and agree on the shape.
3. Then write it.

Step 2 is the whole point. It costs you nothing and saves you a wasted
afternoon when the answer is "this belongs in a different layer" or "this
was tried and here is why it did not work".

A patch that arrives with no issue behind it gets closed with a link to this
file. Nothing personal — it is the only way one person can keep the door
open at all.

## What a change has to carry

If we do agree on one:

- **`cargo test --workspace` and `cargo clippy --workspace --all-targets`
  green**, plus the wasm check in `CLAUDE.md`. All three run in CI.
- **A test that names the property**, not the number. `assert_eq!(crates, 8)`
  breaks when an upstream crate splits; a test called
  `disabling_defaults_removes_serde` does not.
- **The `backend` feature split respected.** Anything that spawns a process
  or reads a file stays out of the model layers, which compile to wasm.
- **Refuse rather than guess.** When a tool cannot answer, it says what is
  missing in terms the caller can act on. A plausible wrong answer costs
  somebody an hour.
- **A commit message that explains *why*.** The history here is written to be
  read; see any of it for the register.

`CLAUDE.md` is the design document and the list of traps this project has
already fallen into. Read it before changing anything structural — several
of those entries cost a day each to learn.

## Licence

The source is published under [PolyForm Noncommercial 1.0.0](LICENSE.md).
Read it, run it, change it, share your changes — but not commercially. If you
want a commercial arrangement, open an issue.

By contributing you agree your contribution is licensed on the same terms.
