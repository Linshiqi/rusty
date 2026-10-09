# Project templates

Every directory here is a proven project rusty writes out itself: as a
playground (File ▸ Playground), and — for a chip whose catalogue entry says
`generator = { template = "<name>" }` — as the project the wizard makes.

A template is a directory and its `template.toml`:

```toml
chip = "ch32x035f8u6"   # the part its files name; absent for one that is no chip's
playground = 4          # its place in the playground list; absent: not offered as one

[[file]]
path = "Cargo.toml"     # where it goes in the project
from = "Cargo.toml.in"  # relative to this directory
```

`build.rs` reads every manifest and compiles the files in, so adding a
template is adding a directory: no code names it. Every source carries an
`.in`, so no tool walking the repository takes a template for a project of
its own.

What the wizard does to a template's files when it writes one for a part:
the package is renamed in `Cargo.toml` and `Cargo.lock` (the lock's package
is called `playground`), the template's chip id becomes the chosen part's in
`Cargo.toml` and `.rusty/sim.toml`, and the board's ground wires move to the
chosen package's first GND row. Everything else is written as it is.
