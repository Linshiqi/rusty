//! Where a part goes, where a wire runs, and where two wires meet.
//!
//! The sheet's three legibility problems, each of which makes a board that
//! is *correct* read as a mess, and each pure and tested here rather than
//! discovered by dragging things around:
//!
//! - **Parts landing on top of each other.** A click plants a part where
//!   the pointer is, and an import plants a dozen wherever the file said —
//!   which for a Wokwi diagram is its own canvas, not this one. Two bodies
//!   in the same place is not a drawing anybody can read, and neither is a
//!   body over somebody else's pin.
//! - **Wires through parts.** A wire with no bends steps out of each pin
//!   and meets itself with one elbow, which is right when nothing is in
//!   the way and draws a line straight through the display when something
//!   is.
//! - **Crossings that are not junctions.** Two wires that cross and two
//!   wires that join look identical without a dot, and a schematic has
//!   drawn that dot for a century.
//!
//! What is *not* here: nothing moves a part the author placed unless the
//! author asks. `arrange` is a command, not a rule that runs; a board
//! somebody laid out by hand is theirs.

use rusty_embed::{Instance, KIT_REFERENCE, Wire};

use super::geometry::{EditPart, ROW_PITCH, SNAP, part_box, pin_point, wire_ends};

/// A box on the sheet: `(x0, y0, x1, y1)`.
pub(super) type Rect = (f64, f64, f64, f64);

/// How far a part keeps from its neighbours, and a wire from a body.
///
/// Half a row pitch: close enough that a board stays compact, wide enough
/// that two parts read as two parts. A wire's clearance is its own, because
/// a line touching a body reads as a connection to it.
const PART_GAP: f64 = ROW_PITCH * 0.75;
const WIRE_CLEARANCE: f64 = ROW_PITCH * 0.5;

/// How far a part's *drawing* reaches past the box `part_box` answers with.
///
/// That box is the body and its leads; the reference sits a row above it and
/// the value a row below, and a pin's name beside its lead. Laid out on the
/// body alone, two parts a comfortable gap apart have their labels sitting
/// on each other — which is what the first arranged sheet looked like, and
/// why this margin is measured rather than guessed: a resistor's body is 60
/// by 14 and what it draws is 68 by 42, and a display's 82 by 48 against
/// 102 by 76.
const LABEL_MARGIN: f64 = ROW_PITCH;

/// A part's box as it is *drawn*, labels included — what the layout has to
/// keep apart, since what a reader sees on top of something else is the
/// drawing and not the body.
pub(super) fn drawn_box(part: &EditPart) -> Rect {
    grown(part_box(part), LABEL_MARGIN)
}

/// Whether two boxes share any area, once both have been given their gap.
pub(super) fn overlaps(a: Rect, b: Rect, gap: f64) -> bool {
    a.0 - gap < b.2 && b.0 - gap < a.2 && a.1 - gap < b.3 && b.1 - gap < a.3
}

/// A part's box, grown by `gap` on every side.
fn grown(r: Rect, gap: f64) -> Rect {
    (r.0 - gap, r.1 - gap, r.2 + gap, r.3 + gap)
}

/// Does this segment cross this box? Horizontal and vertical only, which
/// is every segment a schematic wire has.
fn segment_hits(a: (f64, f64), b: (f64, f64), r: Rect) -> bool {
    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
    x0 < r.2 && r.0 < x1 && y0 < r.3 && r.1 < y1
}

/// How many of these boxes a path passes through.
fn crossings(path: &[(f64, f64)], obstacles: &[Rect]) -> usize {
    path.windows(2)
        .map(|pair| {
            obstacles
                .iter()
                .filter(|r| segment_hits(pair[0], pair[1], **r))
                .count()
        })
        .sum()
}

/// The length of a path, which decides between two routes that are both
/// clear.
fn length(path: &[(f64, f64)]) -> f64 {
    path.windows(2)
        .map(|p| (p[1].0 - p[0].0).abs() + (p[1].1 - p[0].1).abs())
        .sum()
}

/// What a pixel of lane shared with another wire costs, and what one
/// crossing of another wire costs.
///
/// Sharing is the expensive one, and it is the one nobody thinks of: two
/// wires down the same lane are drawn as a single line, so a board with
/// four of them looks like a board with one and the reader cannot see
/// where any of them goes. A crossing is merely a crossing — every
/// schematic ever drawn has them — so it is worth a detour of about a
/// hundred pixels and no more.
const SHARED_LANE: f64 = 50.0;
const CROSSED_WIRE: f64 = 120.0;

/// How much of this path runs *along* another wire rather than across it.
fn shared(path: &[(f64, f64)], others: &[Vec<(f64, f64)>]) -> f64 {
    let near = 1.0;
    let mut along = 0.0;
    for pair in path.windows(2) {
        let vertical = (pair[0].0 - pair[1].0).abs() < near;
        if !vertical && (pair[0].1 - pair[1].1).abs() >= near {
            continue;
        }
        // The segment as (lane, from..to) along its own axis.
        let lane = if vertical { pair[0].0 } else { pair[0].1 };
        let (from, to) = if vertical {
            (pair[0].1.min(pair[1].1), pair[0].1.max(pair[1].1))
        } else {
            (pair[0].0.min(pair[1].0), pair[0].0.max(pair[1].0))
        };
        for theirs in others {
            for other in theirs.windows(2) {
                let theirs_vertical = (other[0].0 - other[1].0).abs() < near;
                if theirs_vertical != vertical {
                    continue;
                }
                let their_lane = if vertical { other[0].0 } else { other[0].1 };
                if (their_lane - lane).abs() >= near {
                    continue;
                }
                let (a, b) = if vertical {
                    (other[0].1.min(other[1].1), other[0].1.max(other[1].1))
                } else {
                    (other[0].0.min(other[1].0), other[0].0.max(other[1].0))
                };
                along += (to.min(b) - from.max(a)).max(0.0);
            }
        }
    }
    along
}

/// How many other wires this path crosses — perpendicular segments that
/// meet away from either one's ends, which is the crossing a reader sees.
fn met(path: &[(f64, f64)], others: &[Vec<(f64, f64)>]) -> usize {
    let near = 1.0;
    let mut count = 0;
    for pair in path.windows(2) {
        for theirs in others {
            for other in theirs.windows(2) {
                let mine_vertical = (pair[0].0 - pair[1].0).abs() < near;
                let theirs_vertical = (other[0].0 - other[1].0).abs() < near;
                if mine_vertical == theirs_vertical {
                    continue;
                }
                let (v, h) = if mine_vertical {
                    (pair, other)
                } else {
                    (other, pair)
                };
                let x = v[0].0;
                let y = h[0].1;
                let within = |from: f64, to: f64, at: f64| {
                    at > from.min(to) + near && at < from.max(to) - near
                };
                if within(v[0].1, v[1].1, y) && within(h[0].0, h[1].0, x) {
                    count += 1;
                }
            }
        }
    }
    count
}

/// The free point nearest `wanted` where a part of this size fits.
///
/// Searched outward in rings on the grid rather than pushed in one
/// direction: a part dropped into a crowded corner should end up beside
/// the crowd, not hurled to the right of everything. The first ring that
/// has room wins, and the point nearest the pointer within it.
pub(super) fn free_spot(
    taken: &[Rect],
    size: (f64, f64),
    wanted: (f64, f64),
    grid: f64,
) -> (f64, f64) {
    let grid = if grid > 0.0 { grid } else { SNAP };
    let fits = |at: (f64, f64)| {
        let half = (size.0 / 2.0, size.1 / 2.0);
        let box_at = (at.0 - half.0, at.1 - half.1, at.0 + half.0, at.1 + half.1);
        !taken.iter().any(|r| overlaps(box_at, *r, PART_GAP))
    };
    if fits(wanted) {
        return wanted;
    }
    // Rings of whole grid steps. Sixty is six hundred pixels at the default
    // grid, which is wider than any sheet anybody is looking at; past that
    // the part goes where it was asked, because refusing to place it at all
    // would be worse than placing it on something.
    for ring in 1..60_i32 {
        let mut best: Option<(f64, (f64, f64))> = None;
        for dx in -ring..=ring {
            for dy in -ring..=ring {
                if dx.abs() != ring && dy.abs() != ring {
                    continue;
                }
                let at = (
                    wanted.0 + f64::from(dx) * grid,
                    wanted.1 + f64::from(dy) * grid,
                );
                if !fits(at) {
                    continue;
                }
                let away = (at.0 - wanted.0).hypot(at.1 - wanted.1);
                if best.is_none_or(|(d, _)| away < d) {
                    best = Some((away, at));
                }
            }
        }
        if let Some((_, at)) = best {
            return at;
        }
    }
    wanted
}

/// The boxes every part but `except` occupies.
pub(super) fn taken_boxes(parts: &[EditPart], except: Option<usize>) -> Vec<Rect> {
    parts
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != except)
        .map(|(_, part)| drawn_box(part))
        .collect()
}

/// What a route has to get past.
///
/// Three kinds of thing, because they cost different amounts to hit: a
/// body is a wall, a wire shared lane-for-lane is two lines drawn as one,
/// and a wire merely crossed is what every schematic has some of.
pub(super) struct Around<'a> {
    /// Every *other* part's drawn box.
    pub parts: &'a [Rect],
    /// The bodies of the wire's own two parts, without the label margin.
    ///
    /// A wire whose pin is on the far side of its own part has to go round
    /// it like anything else — a keypad wired to a header on its right
    /// through pins on its left drew four lines straight across its own
    /// keys otherwise. Excluded entirely, which is what this was, every
    /// such route reads as clear. The margin is left off because the stub
    /// already stands a whole row pitch beyond the body, and growing the
    /// box to meet it would make the first segment of every wire a
    /// crossing.
    pub own: &'a [Rect],
    /// The wires already laid down, as drawn.
    pub wires: &'a [Vec<(f64, f64)>],
}

/// One candidate route: what it costs, and how much of that cost is a
/// fault rather than a preference. A route with no faults ends the search.
struct Scored {
    cost: f64,
    faults: f64,
    path: Vec<(f64, f64)>,
}

/// An orthogonal route between two pins that goes **round** what is in the
/// way, as the interior bends a wire carries.
///
/// The shape is a schematic's: out of each pin along the pin's own
/// direction, then at most two turns. Every candidate is scored the same
/// way — a body crossed costs far more than a wire shared, a wire shared
/// more than a wire crossed, a crossing more than a corner, and a corner a
/// little more than length — so a clear L beats a clear Z, a Z that misses
/// the display beats an L that goes through it, and four wires leaving one
/// part for four pins in a row fan out into four lanes instead of stacking
/// into one line.
///
/// The ends themselves are not returned: a wire's ends are its pins, and
/// the caller already has them.
pub(super) fn route(
    a: (f64, f64),
    out_a: (f64, f64),
    b: (f64, f64),
    out_b: (f64, f64),
    around: &Around,
) -> Vec<(f64, f64)> {
    let step = ROW_PITCH;
    let clear: Vec<Rect> = around
        .parts
        .iter()
        .map(|r| grown(*r, WIRE_CLEARANCE))
        .chain(around.own.iter().copied())
        .collect();

    // The shapes between two stubs: the two Ls, and the Zs on lanes
    // between and a little beyond the two ends — a channel the parts leave
    // free is usually one of these.
    let shapes = |p1: (f64, f64), p2: (f64, f64)| {
        let mut candidates: Vec<Vec<(f64, f64)>> = Vec::new();
        candidates.push(vec![p1, (p2.0, p1.1), p2]);
        candidates.push(vec![p1, (p1.0, p2.1), p2]);
        let lanes = |from: f64, to: f64| {
            let mut out = vec![(from + to) / 2.0];
            for k in 1..=6 {
                let reach = ROW_PITCH * f64::from(k);
                out.push(from.min(to) - reach);
                out.push(from.max(to) + reach);
                out.push((from + to) / 2.0 + reach);
                out.push((from + to) / 2.0 - reach);
            }
            out
        };
        for x in lanes(p1.0, p2.0) {
            candidates.push(vec![p1, (x, p1.1), (x, p2.1), p2]);
        }
        for y in lanes(p1.1, p2.1) {
            candidates.push(vec![p1, (p1.0, y), (p2.0, y), p2]);
        }
        candidates
    };

    // What is wrong with a route, and what merely costs: a body crossed, a
    // lane shared and a wire crossed are faults, and a route with none of
    // them is finished being searched for.
    let faults = |path: &Vec<(f64, f64)>| {
        crossings(path, &clear) as f64 * 10_000.0
            + shared(path, around.wires) * SHARED_LANE
            + met(path, around.wires) as f64 * CROSSED_WIRE
    };
    let score = |path: &Vec<(f64, f64)>| {
        let corners = path.len().saturating_sub(2) as f64;
        faults(path) + length(path) + corners * ROW_PITCH * 0.5
    };

    // How far the wire runs straight out of each pin before it turns.
    //
    // One row pitch is the schematic default, and it is what every wire
    // used to get. The longer ones are the lanes a *fan* needs: four wires
    // leaving one edge for four pins in a row turn at the same x if they
    // all turn after one pitch, and four lines down one lane are drawn as
    // one. They are searched only when the short stub leaves a fault,
    // because the search is the square of this list and most wires are the
    // only wire in their corner.
    const REACHES: [f64; 6] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let mut best: Option<Scored> = None;
    for widen in [false, true] {
        let reaches: &[f64] = if widen { &REACHES } else { &REACHES[..1] };
        for out in reaches {
            for back in reaches {
                let p1 = (a.0 + out_a.0 * step * out, a.1 + out_a.1 * step * out);
                let p2 = (b.0 + out_b.0 * step * back, b.1 + out_b.1 * step * back);
                for path in shapes(p1, p2) {
                    let found = Scored {
                        cost: score(&path),
                        faults: faults(&path),
                        path,
                    };
                    if best.as_ref().is_none_or(|had| found.cost < had.cost) {
                        best = Some(found);
                    }
                }
            }
        }
        if best.as_ref().is_some_and(|found| found.faults == 0.0) {
            break;
        }
    }
    let best = best.map_or_else(
        || {
            vec![
                (a.0 + out_a.0 * step, a.1 + out_a.1 * step),
                (b.0 + out_b.0 * step, b.1 + out_b.1 * step),
            ]
        },
        |found| found.path,
    );
    // Duplicate points are what an L with a shared coordinate produces, and
    // a bend on top of another is a grab handle nobody can pick apart.
    let mut bends: Vec<(f64, f64)> = Vec::new();
    for point in best {
        if bends
            .last()
            .is_none_or(|last| (last.0 - point.0).abs() > 0.01 || (last.1 - point.1).abs() > 0.01)
        {
            bends.push(point);
        }
    }
    bends
}

/// Re-route every wire that has no bends of its own, round the parts.
///
/// Only the untouched ones: a wire somebody has bent is a wire somebody
/// has an opinion about, and rerouting it would throw that away. `arrange`
/// is the command that clears them all first.
pub(super) fn reroute(parts: &[EditPart], wires: &mut [Wire], only_empty: bool) {
    let boxes = box_index(parts);
    // What is already on the sheet, so the next wire can keep off it.
    // Grown one wire at a time in the order they are stored, which is why
    // the answer is the same every time this runs.
    let mut drawn: Vec<Vec<(f64, f64)>> = Vec::new();
    for wire in wires.iter_mut() {
        let Some(ends) = wire_ends(parts, wire) else {
            continue;
        };
        if only_empty && !wire.bends.is_empty() {
            drawn.push(super::geometry::wire_path(&ends, &wire.bends));
            continue;
        }
        let (obstacles, own) = obstacles_for(&boxes, wire);
        let [(a, out_a), (b, out_b)] = ends;
        wire.bends = route(
            a,
            out_a,
            b,
            out_b,
            &Around {
                parts: &obstacles,
                own: &own,
                wires: &drawn,
            },
        );
        drawn.push(super::geometry::wire_path(&ends, &wire.bends));
    }
}

/// Route one wire clear of the parts *and* of every wire already on the
/// sheet — a wire the author has just drawn, and the ghost that shows them
/// what they are about to get.
///
/// `reroute` over a one-wire slice was what this was, and it had no way to
/// see the rest of the board: a wire drawn by hand could land exactly on
/// top of one already there, which is two wires drawn as one line.
pub(super) fn route_beside(parts: &[EditPart], wires: &[Wire], wire: &mut Wire) {
    let Some(ends) = wire_ends(parts, wire) else {
        return;
    };
    let drawn: Vec<Vec<(f64, f64)>> = wires
        .iter()
        .filter_map(|other| {
            let theirs = wire_ends(parts, other)?;
            Some(super::geometry::wire_path(&theirs, &other.bends))
        })
        .collect();
    let (obstacles, own) = obstacles_for(&box_index(parts), wire);
    let [(a, out_a), (b, out_b)] = ends;
    wire.bends = route(
        a,
        out_a,
        b,
        out_b,
        &Around {
            parts: &obstacles,
            own: &own,
            wires: &drawn,
        },
    );
}

/// Every part's two boxes: what it draws, and the body a wire of its own
/// has to get round.
fn box_index(parts: &[EditPart]) -> Vec<(String, Rect, Rect)> {
    parts
        .iter()
        .map(|p| (p.inst.reference.clone(), drawn_box(p), part_box(p)))
        .collect()
}

/// This wire's walls: everybody else's drawing, and its own two bodies.
fn obstacles_for(boxes: &[(String, Rect, Rect)], wire: &Wire) -> (Vec<Rect>, Vec<Rect>) {
    let mine = |reference: &str| reference == wire.from.part || reference == wire.to.part;
    let others = boxes
        .iter()
        .filter(|(reference, _, _)| !mine(reference))
        .map(|(_, drawn, _)| *drawn)
        .collect();
    let own = boxes
        .iter()
        .filter(|(reference, _, _)| mine(reference))
        .map(|(_, _, body)| *body)
        .collect();
    (others, own)
}

/// Re-route the wires a move has *broken* — the ones whose path now runs
/// through a part — and leave every other one exactly as it was.
///
/// The bends a wire carries are the author's to keep: KiCad's rule, and the
/// one this editor already follows when a part is dragged and its leg
/// stretches. But a part dropped on top of a wire makes that wire wrong,
/// and leaving a line through a body because it was once right is the mess
/// this whole module is about. So the test is the drawing, not who drew it:
/// a route that crosses nothing is never touched.
pub(super) fn reroute_broken(parts: &[EditPart], wires: &mut [Wire]) -> usize {
    let boxes = box_index(parts);
    let mut fixed = 0;
    for wire in wires.iter_mut() {
        let Some(ends) = wire_ends(parts, wire) else {
            continue;
        };
        let (obstacles, own) = obstacles_for(&boxes, wire);
        let walls: Vec<Rect> = obstacles.iter().chain(own.iter()).copied().collect();
        let drawn = super::geometry::wire_path(&ends, &wire.bends);
        if crossings(&drawn, &walls) == 0 {
            continue;
        }
        let [(a, out_a), (b, out_b)] = ends;
        let around = route(
            a,
            out_a,
            b,
            out_b,
            &Around {
                parts: &obstacles,
                own: &own,
                wires: &[],
            },
        );
        let mut path = vec![a];
        path.extend(around.iter().copied());
        path.push(b);
        // Only if the new one is actually better: a wire between two parts
        // with something unavoidably in the way keeps the author's own
        // route rather than being shuffled into another bad one.
        if crossings(&path, &walls) < crossings(&drawn, &walls) {
            wire.bends = around;
            fixed += 1;
        }
    }
    fixed
}

/// Where a part should sit, and which side of the devkit it belongs to.
struct Placement {
    index: usize,
    /// The y of the devkit pins it reaches, which is what decides its order
    /// down the column: a part wired to GPIO2 sits where GPIO2 is.
    anchor: f64,
    /// Left of the devkit, or right of it.
    left: bool,
    size: (f64, f64),
}

/// Lay the whole sheet out again: parts in two columns beside the devkit in
/// the order of the pins they reach, and every wire re-routed.
///
/// **The order is the whole of it.** A part wired to GPIO2 placed beside
/// GPIO2 needs a wire that does not cross anybody else's, and a sheet whose
/// parts are in header order has almost no crossings left to avoid. Sorting
/// by the pin is what turns a tangle into a drawing; the routing that
/// follows only tidies what is left.
///
/// The devkit does not move — it is the thing everything else is placed
/// against, and moving it would drag the whole board across the canvas.
pub(super) fn arrange(parts: &mut [EditPart], wires: &mut [Wire]) {
    let Some(kit) = parts.iter().position(|p| p.is_kit()) else {
        return;
    };
    let kit_box = drawn_box(&parts[kit]);
    let kit_middle = (kit_box.0 + kit_box.2) / 2.0;

    // Where each part's devkit pins are, which is what it is placed against.
    let mut anchors: Vec<Option<(f64, bool)>> = vec![None; parts.len()];
    for (index, part) in parts.iter().enumerate() {
        if index == kit {
            continue;
        }
        let mut ys: Vec<f64> = Vec::new();
        let mut lefts: Vec<bool> = Vec::new();
        for wire in wires.iter() {
            let (mine, theirs) = if wire.from.part == part.inst.reference {
                (&wire.from, &wire.to)
            } else if wire.to.part == part.inst.reference {
                (&wire.to, &wire.from)
            } else {
                continue;
            };
            let _ = mine;
            if theirs.part != KIT_REFERENCE {
                continue;
            }
            if let Some(pin) = parts[kit].pin(&theirs.pin) {
                let at = pin_point(&parts[kit], pin);
                ys.push(at.1);
                lefts.push(at.0 < kit_middle);
            }
        }
        if ys.is_empty() {
            continue;
        }
        let y = ys.iter().sum::<f64>() / ys.len() as f64;
        let left = lefts.iter().filter(|l| **l).count() * 2 >= lefts.len();
        anchors[index] = Some((y, left));
    }

    // A part wired only to other parts follows whichever of them has an
    // anchor — a resistor between a pin and a lamp belongs beside them
    // both, and a rail belongs beside whatever it feeds.
    for _ in 0..3 {
        for index in 0..parts.len() {
            if index == kit || anchors[index].is_some() {
                continue;
            }
            let reference = parts[index].inst.reference.clone();
            let mut found: Option<(f64, bool)> = None;
            for wire in wires.iter() {
                let other = if wire.from.part == reference {
                    &wire.to.part
                } else if wire.to.part == reference {
                    &wire.from.part
                } else {
                    continue;
                };
                if let Some(at) = parts
                    .iter()
                    .position(|p| p.inst.reference == *other)
                    .and_then(|i| anchors.get(i).copied().flatten())
                {
                    found = Some(at);
                    break;
                }
            }
            anchors[index] = found;
        }
    }

    let mut wanted: Vec<Placement> = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        if index == kit {
            continue;
        }
        let (x0, y0, x1, y1) = drawn_box(part);
        let (anchor, left) =
            anchors[index].unwrap_or((kit_box.1 + (index as f64) * ROW_PITCH, true));
        wanted.push(Placement {
            index,
            anchor,
            left,
            size: (x1 - x0, y1 - y0),
        });
    }
    wanted.sort_by(|a, b| a.anchor.total_cmp(&b.anchor).then(a.index.cmp(&b.index)));

    // Two columns, each stacked from the top of the devkit downwards. A
    // column that runs out of the sheet's patience starts another further
    // out, so a board with thirty parts is wide rather than endless.
    const COLUMN_HEIGHT: f64 = ROW_PITCH * 46.0;
    let mut columns: [Vec<(f64, f64)>; 2] = [Vec::new(), Vec::new()];
    for (side, column_of) in columns.iter_mut().enumerate() {
        let left = side == 0;
        let mut y = kit_box.1;
        let mut column = 0usize;
        for place in wanted.iter().filter(|p| p.left == left) {
            if y > kit_box.1 + COLUMN_HEIGHT && column < 3 {
                column += 1;
                y = kit_box.1;
            }
            let reach =
                ROW_PITCH * 6.0 + place.size.0 / 2.0 + f64::from(column as u16) * ROW_PITCH * 14.0;
            let x = if left {
                kit_box.0 - reach
            } else {
                kit_box.2 + reach
            };
            column_of.push((x, y + place.size.1 / 2.0));
            y += place.size.1 + PART_GAP * 2.0;
        }
    }

    let mut next = [0usize, 0usize];
    for place in &wanted {
        let side = usize::from(!place.left);
        let Some(at) = columns[side].get(next[side]).copied() else {
            continue;
        };
        next[side] += 1;
        let part: &mut Instance = &mut parts[place.index].inst;
        part.x = snap(at.0);
        part.y = snap(at.1);
    }

    for wire in wires.iter_mut() {
        wire.bends.clear();
    }
    reroute(parts, wires, false);
}

fn snap(value: f64) -> f64 {
    (value / SNAP).round() * SNAP
}

/// Where a dot belongs: a point two or more wire ends share, or where one
/// wire's end sits on another's line.
///
/// Two wires crossing and two wires joining are the same picture without
/// it, and which one it is decides what the firmware reads. Two ends at a
/// pin is a dot because the pin is a conductor too — KiCad's rule, three
/// things meeting — and a wire ending on another's line is the T a branch
/// makes. Computed from the drawn paths rather than the net model, because
/// what a reader needs marked is what is drawn.
pub(super) fn junctions(parts: &[EditPart], wires: &[Wire]) -> Vec<(f64, f64)> {
    let paths: Vec<Vec<(f64, f64)>> = wires
        .iter()
        .filter_map(|wire| {
            let ends = wire_ends(parts, wire)?;
            Some(super::geometry::wire_path(&ends, &wire.bends))
        })
        .collect();
    let same = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 0.6 && (a.1 - b.1).abs() < 0.6;

    let mut dots: Vec<(f64, f64)> = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        for end in [path.first(), path.last()].into_iter().flatten() {
            let mut ends_here = 1;
            let mut passes = false;
            for (other, theirs) in paths.iter().enumerate() {
                if other == index {
                    continue;
                }
                for one in [theirs.first(), theirs.last()].into_iter().flatten() {
                    if same(*one, *end) {
                        ends_here += 1;
                    }
                }
                passes = passes
                    || theirs.windows(2).any(|pair| {
                        on_segment(*end, pair[0], pair[1])
                            && !same(pair[0], *end)
                            && !same(pair[1], *end)
                    });
            }
            if (ends_here >= 2 || passes) && !dots.iter().any(|d| same(*d, *end)) {
                dots.push(*end);
            }
        }
    }
    dots
}

/// Whether a point lies on a horizontal or vertical segment.
fn on_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> bool {
    let near = 0.6;
    if (a.0 - b.0).abs() < near {
        return (p.0 - a.0).abs() < near
            && p.1 >= a.1.min(b.1) - near
            && p.1 <= a.1.max(b.1) + near;
    }
    if (a.1 - b.1).abs() < near {
        return (p.1 - a.1).abs() < near
            && p.0 >= a.0.min(b.0) - near
            && p.0 <= a.0.max(b.0) + near;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_embed::PinRef;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        (x, y, x + w, y + h)
    }

    /// An empty sheet with these bodies on it: the first wire's view.
    fn clear_sheet(parts: &[Rect]) -> Around<'_> {
        Around {
            parts,
            own: &[],
            wires: &[],
        }
    }

    /// A part dropped where another one is goes *beside* it, on the grid,
    /// and as near as it can be — not into the distance and not on top.
    #[test]
    fn a_part_lands_beside_what_is_already_there() {
        let taken = vec![rect(0.0, 0.0, 40.0, 40.0)];
        let spot = free_spot(&taken, (40.0, 40.0), (20.0, 20.0), SNAP);
        assert!(
            !overlaps(
                (spot.0 - 20.0, spot.1 - 20.0, spot.0 + 20.0, spot.1 + 20.0),
                taken[0],
                PART_GAP
            ),
            "{spot:?} still sits on the part that was there"
        );
        assert!(
            (spot.0 - 20.0).hypot(spot.1 - 20.0) < 120.0,
            "{spot:?} is further away than it needed to go"
        );
        let steps = ((spot.0 - 20.0) / SNAP, (spot.1 - 20.0) / SNAP);
        assert!(
            steps.0.fract().abs() < 1e-9 && steps.1.fract().abs() < 1e-9,
            "{spot:?} is not a whole number of grid steps from the pointer"
        );

        // Empty sheet: exactly where it was asked for.
        assert_eq!(
            free_spot(&[], (40.0, 40.0), (17.0, 33.0), SNAP),
            (17.0, 33.0)
        );
    }

    /// A wire between two pins with a body between them goes round it. The
    /// straight elbow is what it would draw otherwise, and that line
    /// crosses the body.
    #[test]
    fn a_wire_goes_round_what_is_in_the_way() {
        let a = (0.0, 0.0);
        let b = (200.0, 0.0);
        let wall = rect(80.0, -60.0, 40.0, 120.0);
        let straight = route(a, (1.0, 0.0), b, (-1.0, 0.0), &clear_sheet(&[]));
        let mut path = vec![a];
        path.extend(straight.iter().copied());
        path.push(b);
        assert_eq!(crossings(&path, &[wall]), 1, "the clear route is straight");

        let around = route(a, (1.0, 0.0), b, (-1.0, 0.0), &clear_sheet(&[wall]));
        let mut path = vec![a];
        path.extend(around.iter().copied());
        path.push(b);
        assert_eq!(
            crossings(&path, &[grown(wall, WIRE_CLEARANCE)]),
            0,
            "{around:?} still cuts through the part"
        );
        // And every segment is horizontal or vertical: a schematic has no
        // diagonals, and a router that produced one would be drawing
        // something no other editor draws.
        for pair in path.windows(2) {
            assert!(
                (pair[0].0 - pair[1].0).abs() < 0.01 || (pair[0].1 - pair[1].1).abs() < 0.01,
                "{pair:?} is a diagonal"
            );
        }
    }

    /// A pin on the far side of its own part is wired *round* the part.
    ///
    /// The cheapest elbow goes straight back across the body it just left,
    /// which on a keypad is four lines drawn over its own keys. Its own
    /// two parts used to be left out of the obstacles altogether, so every
    /// such route scored as clear.
    #[test]
    fn a_wire_goes_round_its_own_part_as_well() {
        let body = rect(-60.0, -40.0, 120.0, 80.0);
        // The pin is on the body's left edge and the target is far to the
        // right, so the L with no corners runs the length of the body.
        let a = (-60.0, 0.0);
        let b = (400.0, 0.0);
        let path_of = |bends: Vec<(f64, f64)>| {
            let mut path = vec![a];
            path.extend(bends);
            path.push(b);
            path
        };
        let through = path_of(route(a, (-1.0, 0.0), b, (-1.0, 0.0), &clear_sheet(&[])));
        assert!(
            crossings(&through, &[body]) > 0,
            "the fixture is wrong: {through:?} misses the body it should cut through"
        );

        let around = path_of(route(
            a,
            (-1.0, 0.0),
            b,
            (-1.0, 0.0),
            &Around {
                parts: &[],
                own: &[body],
                wires: &[],
            },
        ));
        assert_eq!(
            crossings(&around, &[body]),
            0,
            "{around:?} still runs across its own part"
        );
    }

    /// Four wires from one part to four pins in a row take four lanes.
    ///
    /// Down one lane they are drawn as a single line, and a reader cannot
    /// see where any of the four goes — which is worse than the crossing
    /// that avoiding it sometimes costs, and is the thing that makes a
    /// correct board look like a mess.
    #[test]
    fn wires_leaving_one_part_fan_out_instead_of_stacking() {
        let parts = vec![dip("KP1", 0.0, 0.0), dip("J1", 420.0, 0.0)];
        // The drawing decides which side a pin is on, so the fixture asks
        // it rather than assuming a numbering.
        let left_pins = |part: &EditPart| {
            let middle = {
                let b = part_box(part);
                (b.0 + b.2) / 2.0
            };
            let mut names: Vec<(String, f64)> = part
                .symbol
                .as_ref()
                .unwrap()
                .pins
                .iter()
                .filter_map(|pin| {
                    let at = pin_point(part, pin);
                    (at.0 < middle).then(|| (pin.number.clone(), at.1))
                })
                .collect();
            names.sort_by(|a, b| a.1.total_cmp(&b.1));
            names
        };
        let (from, to) = (left_pins(&parts[0]), left_pins(&parts[1]));
        assert_eq!(from.len(), 4, "the fixture should draw four pins a side");
        let mut wires: Vec<Wire> = from
            .iter()
            .zip(to.iter())
            .map(|((one, _), (two, _))| wire(&format!("KP1.{one}"), &format!("J1.{two}")))
            .collect();
        reroute(&parts, &mut wires, false);

        let paths: Vec<Vec<(f64, f64)>> = wires
            .iter()
            .map(|w| {
                let ends = wire_ends(&parts, w).unwrap();
                super::super::geometry::wire_path(&ends, &w.bends)
            })
            .collect();
        for (index, one) in paths.iter().enumerate() {
            for (other, two) in paths.iter().enumerate() {
                if index >= other {
                    continue;
                }
                let along = shared(one, std::slice::from_ref(two));
                assert!(
                    along < 1.0,
                    "wires {index} and {other} run {along} pixels down the same lane\
                     \n  {one:?}\n  {two:?}"
                );
            }
            assert_eq!(
                crossings(one, &[part_box(&parts[0]), part_box(&parts[1])]),
                0,
                "wire {index} runs through a body: {one:?}"
            );
        }
    }

    /// Three wires meeting at a pin get a dot; two crossing wires do not.
    /// Those are the same picture otherwise, and they mean opposite things.
    #[test]
    fn a_dot_marks_a_join_and_never_a_crossing() {
        // Nothing collinear: a wire that ended *on* another wire's line
        // would be a T, and rightly get its own dot.
        let parts = vec![
            fake("U1", 0.0, 0.0),
            fake("R1", 140.0, -90.0),
            fake("R2", 140.0, 90.0),
            fake("R3", 260.0, 200.0),
        ];
        // Three wires at U1's pin 1.
        let wires = vec![
            wire("U1.1", "R1.1"),
            wire("U1.1", "R2.1"),
            wire("U1.1", "R3.1"),
        ];
        let dots = junctions(&parts, &wires);
        assert_eq!(dots.len(), 1, "{dots:?}");

        // Two wires that merely cross: no dot anywhere.
        let wires = vec![wire("R1.1", "R2.1"), wire("U1.1", "R3.1")];
        assert!(junctions(&parts, &wires).is_empty());

        // And two wires meeting at one pin is a dot: the pin is a conductor
        // too, so three things meet there.
        let wires = vec![wire("U1.1", "R1.1"), wire("U1.1", "R2.1")];
        assert_eq!(junctions(&parts, &wires).len(), 1);
    }

    /// Arranging puts the parts in the order of the pins they reach, which
    /// is what leaves nothing to cross — and it never leaves two parts on
    /// top of each other.
    #[test]
    fn arranging_orders_parts_by_the_pin_they_reach_and_never_stacks_them() {
        let mut parts = vec![
            kit(),
            fake("R1", 0.0, 0.0),
            fake("R2", 0.0, 0.0),
            fake("R3", 0.0, 0.0),
        ];
        // R1 to the *lower* of the two pins on one side and R2 to the
        // upper: they must come out in the pins' order, not the order they
        // were placed in. R3 hangs off the other side, and must land there.
        let mut wires = vec![
            wire("R1.1", "U1.P2"),
            wire("R2.1", "U1.P1"),
            wire("R3.1", "U1.P3"),
        ];
        let kit_pin = |name: &str| {
            let kit = parts.iter().find(|p| p.is_kit()).unwrap();
            pin_point(kit, kit.pin(name).unwrap())
        };
        let (p1, p2, p3) = (kit_pin("P1"), kit_pin("P2"), kit_pin("P3"));
        assert!(p1.1 < p2.1, "the fixture's P1 is the upper pin");
        arrange(&mut parts, &mut wires);
        let part = |reference: &str| {
            parts
                .iter()
                .find(|p| p.inst.reference == reference)
                .unwrap()
        };
        let y = |reference: &str| part(reference).inst.y;
        assert!(y("R2") < y("R1"), "in the order of the pins they reach");
        // And on the side its pin is: the drawing decides which side a pin
        // is on, so the layout follows the drawing rather than the symbol.
        let kit_middle = {
            let b = part_box(part("U1"));
            (b.0 + b.2) / 2.0
        };
        assert_eq!(
            part("R3").inst.x > kit_middle,
            p3.0 > kit_middle,
            "R3 belongs on the side its pin is"
        );
        for (i, one) in parts.iter().enumerate() {
            for (j, two) in parts.iter().enumerate() {
                if i >= j || one.is_kit() || two.is_kit() {
                    continue;
                }
                assert!(
                    !overlaps(drawn_box(one), drawn_box(two), 0.0),
                    "{} and {} are on top of each other",
                    one.inst.reference,
                    two.inst.reference
                );
            }
        }
    }

    // ---- fixtures -------------------------------------------------------

    fn symbol(reference: &str, pins: &[(&str, f64, f64)]) -> rusty_embed::Symbol {
        rusty_embed::Symbol {
            library: "test".into(),
            name: "part".into(),
            reference: reference.into(),
            value: String::new(),
            description: None,
            pins: pins
                .iter()
                .map(|(number, x, y)| rusty_embed::Pin {
                    number: (*number).into(),
                    name: (*number).into(),
                    kind: rusty_embed::PinKind::Passive,
                    at: (*x, *y),
                    length: 2.54,
                    angle: 0,
                    hidden: false,
                })
                .collect(),
            graphics: Vec::new(),
        }
    }

    fn instance(reference: &str, x: f64, y: f64) -> Instance {
        Instance {
            reference: reference.into(),
            symbol: "test:part".into(),
            value: String::new(),
            x,
            y,
            rot: 0,
            mirror: false,
            props: Default::default(),
        }
    }

    fn fake(reference: &str, x: f64, y: f64) -> EditPart {
        EditPart {
            inst: instance(reference, x, y),
            symbol: Some(symbol("R", &[("1", -3.81, 0.0), ("2", 3.81, 0.0)])),
        }
    }

    /// Eight pins, which a part nobody has heard of is drawn with four down
    /// each side — a keypad's shape, and the one that puts a pin on the far
    /// side of its own body from where its wire has to go.
    fn dip(reference: &str, x: f64, y: f64) -> EditPart {
        EditPart {
            inst: instance(reference, x, y),
            symbol: Some(symbol(
                "U",
                &[
                    ("1", -10.0, 7.62),
                    ("2", -10.0, 2.54),
                    ("3", -10.0, -2.54),
                    ("4", -10.0, -7.62),
                    ("5", 10.0, -7.62),
                    ("6", 10.0, -2.54),
                    ("7", 10.0, 2.54),
                    ("8", 10.0, 7.62),
                ],
            )),
        }
    }

    /// A stand-in devkit: three pins down one side, far enough apart that
    /// their order is unmistakable.
    fn kit() -> EditPart {
        EditPart {
            inst: instance(KIT_REFERENCE, 400.0, 0.0),
            // Four, so the drawing puts two down each side — which is the
            // rule the layout follows, since the drawing is what decides
            // where a pin is.
            symbol: Some(symbol(
                "U",
                &[
                    ("P1", -10.0, 10.0),
                    ("P2", -10.0, 0.0),
                    ("P3", 10.0, 10.0),
                    ("P4", 10.0, 0.0),
                ],
            )),
        }
    }

    fn wire(from: &str, to: &str) -> Wire {
        Wire {
            from: PinRef::parse(from).unwrap(),
            to: PinRef::parse(to).unwrap(),
            bends: Vec::new(),
        }
    }
}
