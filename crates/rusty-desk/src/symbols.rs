//! The finder's symbol modes, VS Code's: `@` lists the file in front's
//! symbols — its outline, as the server reads it — and `#` the project's,
//! asked of the server as the name is typed. This is the pure half: what a
//! query asks for, the rows a list of symbols makes, and which answer to a
//! workspace search may be shown (`app/symbols.rs` asks and shows).
//!
//! **An answer is kept with its ask.** The workspace is asked again after a
//! pause in the typing, and the server answers when it answers: a slow
//! reply to `#gp` landing after `#gpio` was asked is the answer to a
//! question nobody is asking any more, and is dropped. Only the newest ask's
//! answer is ever kept ([`Search::answered`]), and while it is on its way
//! the last answer kept is shown only as far as the name now typed still
//! matches it ([`Search::view`]) — never a row the query on screen does not
//! match.

use rusty_i18n::t;
use rusty_lsp::Symbol;

use crate::picker::Pick;

/// What a finder query starting `@` or `#` asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// `@`: the file in front's symbols, narrowed by `filter`; `@:` groups
    /// them by kind, as VS Code's does.
    Outline { filter: String, grouped: bool },
    /// `#`: the project's symbols whose names match `query`.
    Workspace { query: String },
}

/// The symbol mode a finder query is in, or `None` for one that is not.
pub fn target(query: &str) -> Option<Target> {
    let query = query.trim_start();
    if let Some(rest) = query.strip_prefix('@') {
        let (grouped, filter) = match rest.strip_prefix(':') {
            Some(filter) => (true, filter),
            None => (false, rest),
        };
        return Some(Target::Outline {
            filter: filter.trim().to_string(),
            grouped,
        });
    }
    query.strip_prefix('#').map(|rest| Target::Workspace {
        query: rest.trim().to_string(),
    })
}

/// One row of the finder in a symbol mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub title: String,
    /// What it sits in, dim: `impl Vector`, `mod tests`.
    pub detail: String,
    /// Its line in the file, or the file and line in the project.
    pub right: String,
    /// The kind's icon (`Icons.symbol`) and the token colour it is drawn
    /// in (`Theme.token`) — 0 for a row that only says something.
    pub glyph: i32,
    pub tint: i32,
    /// `@:`'s heading for the kind the rows from here are, with how many.
    pub group: String,
    pub pick: Pick,
}

impl Row {
    /// A row that only says something: picking it closes the finder.
    pub fn said(text: String) -> Row {
        Row {
            title: text,
            detail: String::new(),
            right: String::new(),
            glyph: 0,
            tint: 0,
            group: String::new(),
            pick: Pick::Nothing,
        }
    }
}

/// Every kind the client names a symbol by (`rusty_lsp`'s words for the
/// protocol's kinds), in the order `@:` lists their groups: what holds code
/// first, then the code, then what the code holds.
pub const KINDS: [&str; 27] = [
    "module",
    "namespace",
    "package",
    "file",
    "struct",
    "class",
    "enum",
    "variant",
    "trait",
    "impl",
    "type",
    "function",
    "method",
    "constructor",
    "operator",
    "field",
    "property",
    "key",
    "constant",
    "variable",
    "string",
    "number",
    "boolean",
    "array",
    "null",
    "event",
    "symbol",
];

/// A kind's icon and the colour it is drawn in: a function as functions are
/// coloured in the code, a type as types are. The icons are `Icons.symbol`'s
/// numbers; the colours `Theme.token`'s kinds (`editor/rows.rs`).
pub fn glyph(kind: &str) -> (i32, i32) {
    match kind {
        "function" | "method" | "constructor" | "operator" => (1, 6),
        "field" | "property" | "key" => (2, 9),
        "variable" => (3, 9),
        "struct" | "class" => (4, 5),
        "enum" => (5, 5),
        "variant" => (6, 9),
        "trait" => (7, 5),
        "module" | "namespace" | "package" | "file" => (8, 10),
        "constant" | "string" | "number" | "boolean" | "array" | "null" => (9, 3),
        "impl" => (10, 1),
        "type" => (11, 5),
        _ => (12, 0),
    }
}

/// The rows for the file's outline: in file order while nothing is typed,
/// then only those whose names match, best first ([`rank`]). Grouped, each
/// kind's rows together in [`KINDS`] order, the first of each carrying the
/// kind's heading.
pub fn outline_rows(symbols: &[Symbol], filter: &str, grouped: bool) -> Vec<Row> {
    let names: Vec<String> = symbols.iter().map(|s| s.name.clone()).collect();
    let mut order = rank(filter, &names);
    if grouped {
        // Stable, so file order — or the ranking — holds within a kind.
        order.sort_by_key(|&at| kind_rank(&symbols[at].kind));
    }
    let mut rows: Vec<Row> = order
        .iter()
        .map(|&at| {
            let symbol = &symbols[at];
            let (glyph, tint) = glyph(&symbol.kind);
            Row {
                title: symbol.name.clone(),
                detail: symbol.container.clone().unwrap_or_default(),
                right: t!(
                    "symbols.line",
                    line = (symbol.location.line + 1).to_string()
                ),
                glyph,
                tint,
                group: String::new(),
                pick: Pick::Outline(symbol.location.clone()),
            }
        })
        .collect();
    if grouped {
        let mut start = 0;
        while start < order.len() {
            let kind = &symbols[order[start]].kind;
            let count = order[start..]
                .iter()
                .take_while(|&&at| symbols[at].kind == *kind)
                .count();
            rows[start].group = group_label(kind, count);
            start += count;
        }
    }
    rows
}

/// Which names a typed symbol filter keeps, best first: VS Code's rule for
/// its symbol pickers, the completion popup's (`rusty_lsp::complete`). A
/// name that starts with what was typed comes first, the same case ahead;
/// then a name it only matches fuzzily, its first character where a word
/// starts — `cr` finds `cross` and `the_cross_product`, and not `Vector`,
/// which the finder's file matching (`picker::rank`) would keep for its
/// `c` and its `r`. Ties keep file order; nothing typed keeps everything.
pub fn rank(filter: &str, names: &[String]) -> Vec<usize> {
    let typed = filter.trim();
    if typed.is_empty() {
        return (0..names.len()).collect();
    }
    let lower = typed.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    let mut kept: Vec<((u8, u8, u32), usize)> = names
        .iter()
        .enumerate()
        .filter_map(|(at, name)| {
            if name.to_lowercase().starts_with(&lower) {
                Some(((0, u8::from(!name.starts_with(typed)), 0), at))
            } else {
                rusty_lsp::complete::fuzzy_score(&chars, name)
                    .map(|score| ((1, 0, u32::MAX - score), at))
            }
        })
        .collect();
    kept.sort_unstable();
    kept.into_iter().map(|(_, at)| at).collect()
}

/// The rows for the project's symbols, in the order the server ranked them:
/// each with what it sits in and where it is, the file as the project names
/// it.
pub fn workspace_rows(symbols: &[Symbol]) -> Vec<Row> {
    symbols
        .iter()
        .map(|symbol| {
            let (glyph, tint) = glyph(&symbol.kind);
            let location = &symbol.location;
            Row {
                title: symbol.name.clone(),
                detail: symbol.container.clone().unwrap_or_default(),
                right: format!("{}:{}", location.path, location.line + 1),
                glyph,
                tint,
                group: String::new(),
                pick: Pick::Place(location.clone()),
            }
        })
        .collect()
}

fn kind_rank(kind: &str) -> usize {
    KINDS.iter().position(|k| *k == kind).unwrap_or(KINDS.len())
}

/// `@:`'s heading for `count` symbols of `kind`: `functions (5)`.
fn group_label(kind: &str, count: usize) -> String {
    let count = count.to_string();
    let known = if KINDS.contains(&kind) {
        kind
    } else {
        "symbol"
    };
    // A computed key, so the catalogue's scan cannot see it: the test below
    // holds every kind to an entry.
    rusty_i18n::translate(&format!("symbols.group.{known}"))
        .map(|label| rusty_i18n::fill(&label, &[("count", count.clone())]))
        .unwrap_or_else(|| format!("{known} ({count})"))
}

/// The project's symbols as asked of the server while a name is typed.
#[derive(Debug, Default)]
pub struct Search {
    next: u64,
    /// The newest ask: its number and the name it asked about.
    asked: Option<(u64, String)>,
    /// The newest answer kept: the name it answers, and the symbols — or
    /// the server's refusal, in its words.
    answer: Option<(String, Result<Vec<Symbol>, String>)>,
    /// Answers that came after a newer ask, and were dropped.
    dropped: u32,
    /// The name the typing's pause is running for, to be asked when it ends.
    pausing: Option<String>,
}

/// What a workspace search can show for the name typed now.
#[derive(Debug, PartialEq, Eq)]
pub enum View<'a> {
    /// The answer to exactly this name.
    Answer(&'a [Symbol]),
    /// The server could not answer this name, in its words.
    Failed(&'a str),
    /// The answer to an earlier name, narrowed to the symbols this name
    /// still matches — shown while this name's own answer is on its way.
    Narrowed(Vec<&'a Symbol>),
    /// Nothing to show until this name's answer comes.
    Waiting,
}

impl Search {
    /// A new ask for `query`: its number, behind which every earlier ask's
    /// answer now falls.
    pub fn ask(&mut self, query: &str) -> u64 {
        self.next += 1;
        self.asked = Some((self.next, query.to_string()));
        self.pausing = None;
        self.next
    }

    /// Whether `query` has been asked — its answer is here or on its way.
    pub fn has_asked(&self, query: &str) -> bool {
        self.asked.as_ref().is_some_and(|(_, asked)| asked == query)
    }

    /// Whether a pause is to start, after which `query` is asked: not when
    /// it has been asked, and not when the pause already runs for it. The
    /// rows are made again whenever the server settles, and a server that
    /// is loading says so often — a pause started again each time never
    /// ran out, and nothing was asked.
    pub fn wants(&mut self, query: &str) -> bool {
        if self.has_asked(query) || self.pausing.as_deref() == Some(query) {
            return false;
        }
        self.pausing = Some(query.to_string());
        true
    }

    /// The pause was stopped before it ran out.
    pub fn unpause(&mut self) {
        self.pausing = None;
    }

    /// The answer to ask `asked`: kept, and true, only while that ask is
    /// still the newest.
    pub fn answered(&mut self, asked: u64, answer: Result<Vec<Symbol>, String>) -> bool {
        match &self.asked {
            Some((newest, query)) if *newest == asked => {
                self.answer = Some((query.clone(), answer));
                true
            }
            _ => {
                self.dropped += 1;
                false
            }
        }
    }

    /// Nothing is waiting any more: every answer still on its way is
    /// dropped, and what was kept is forgotten — the finder closed, or a
    /// new one opened.
    pub fn forget(&mut self) {
        self.next += 1;
        self.asked = None;
        self.answer = None;
        self.pausing = None;
    }

    /// The answer kept is no answer: an empty one, or a refusal, from a
    /// server that may since have finished loading — to be asked again.
    /// Whether there was one.
    pub fn forget_empty(&mut self) -> bool {
        let empty = self
            .answer
            .as_ref()
            .is_some_and(|(_, answer)| answer.as_ref().is_ok_and(Vec::is_empty) || answer.is_err());
        if empty {
            self.answer = None;
            self.asked = None;
        }
        empty
    }

    /// The name asked last.
    pub fn asked_for(&self) -> Option<&str> {
        self.asked.as_ref().map(|(_, query)| query.as_str())
    }

    /// The name the answer kept answers.
    pub fn shown_for(&self) -> Option<&str> {
        self.answer.as_ref().map(|(query, _)| query.as_str())
    }

    /// How many answers came too late to be shown.
    pub fn dropped(&self) -> u32 {
        self.dropped
    }

    /// What to show while `query` is typed.
    pub fn view(&self, query: &str) -> View<'_> {
        match &self.answer {
            Some((answered, Ok(symbols))) if answered == query => View::Answer(symbols),
            Some((answered, Err(reason))) if answered == query => View::Failed(reason),
            Some((_, Ok(symbols))) => {
                let names: Vec<String> = symbols.iter().map(|s| s.name.clone()).collect();
                let still: Vec<&Symbol> = rank(query, &names)
                    .into_iter()
                    .map(|at| &symbols[at])
                    .collect();
                if still.is_empty() {
                    View::Waiting
                } else {
                    View::Narrowed(still)
                }
            }
            _ => View::Waiting,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_lsp::Location;

    fn symbol(name: &str, kind: &str, container: Option<&str>, line: u32) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: kind.to_string(),
            container: container.map(str::to_string),
            depth: u32::from(container.is_some()),
            location: Location {
                path: "src/lib.rs".to_string(),
                line,
                col: 4,
                external: false,
            },
        }
    }

    /// `draw-vectors`' `src/lib.rs`, as rust-analyzer outlines it.
    fn outline() -> Vec<Symbol> {
        vec![
            symbol("Vector", "struct", None, 8),
            symbol("x", "field", Some("Vector"), 9),
            symbol("impl Vector", "impl", None, 14),
            symbol("new", "function", Some("impl Vector"), 15),
            symbol("dot", "method", Some("impl Vector"), 19),
            symbol("cross", "method", Some("impl Vector"), 24),
            symbol("tests", "module", None, 43),
            symbol(
                "the_cross_product_is_square_to_both",
                "function",
                Some("tests"),
                50,
            ),
        ]
    }

    fn titles(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.title.as_str()).collect()
    }

    #[test]
    fn a_query_says_which_symbols_it_asks_for() {
        assert_eq!(
            target("@"),
            Some(Target::Outline {
                filter: String::new(),
                grouped: false
            })
        );
        assert_eq!(
            target("@ cr "),
            Some(Target::Outline {
                filter: "cr".into(),
                grouped: false
            })
        );
        assert_eq!(
            target("@:cr"),
            Some(Target::Outline {
                filter: "cr".into(),
                grouped: true
            })
        );
        assert_eq!(
            target("#gpio"),
            Some(Target::Workspace {
                query: "gpio".into()
            })
        );
        assert_eq!(
            target("#"),
            Some(Target::Workspace {
                query: String::new()
            })
        );
        assert_eq!(target("main.rs"), None);
        assert_eq!(target(":42"), None);
    }

    /// In file order while nothing is typed; then only what matches, a name
    /// that starts with the query first, each with its container and line.
    #[test]
    fn an_outline_is_in_file_order_until_something_is_typed() {
        let symbols = outline();
        let all = outline_rows(&symbols, "", false);
        assert_eq!(
            titles(&all),
            [
                "Vector",
                "x",
                "impl Vector",
                "new",
                "dot",
                "cross",
                "tests",
                "the_cross_product_is_square_to_both"
            ]
        );
        assert_eq!(all[5].detail, "impl Vector");
        assert_eq!(all[5].glyph, glyph("method").0);
        assert_eq!(all[5].pick, Pick::Outline(symbols[5].location.clone()));
        let typed = outline_rows(&symbols, "cro", false);
        assert_eq!(
            titles(&typed),
            ["cross", "the_cross_product_is_square_to_both"]
        );
        assert!(outline_rows(&symbols, "zzz", false).is_empty());
    }

    /// VS Code's rule, not the finder's: a typed name keeps what starts
    /// with it and what it matches from a word's start, and nothing that
    /// only holds its letters somewhere — `cr` is not `Vector`.
    #[test]
    fn a_symbol_filter_matches_from_a_word_start() {
        let titles_for = |filter: &str| {
            outline_rows(&outline(), filter, false)
                .into_iter()
                .map(|row| row.title)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            titles_for("cr"),
            ["cross", "the_cross_product_is_square_to_both"]
        );
        assert_eq!(titles_for("vec"), ["Vector", "impl Vector"]);
        assert_eq!(titles_for("V"), ["Vector", "impl Vector"]);
        assert!(titles_for("ector").is_empty());
    }

    /// `@:` keeps a kind's rows together, in file order within it, and
    /// heads each kind's first row with how many there are.
    #[test]
    fn grouped_the_kinds_are_together_and_counted() {
        let rows = outline_rows(&outline(), "", true);
        let shown: Vec<(&str, bool)> = rows
            .iter()
            .map(|r| (r.title.as_str(), !r.group.is_empty()))
            .collect();
        assert_eq!(
            shown,
            [
                ("tests", true),
                ("Vector", true),
                ("impl Vector", true),
                ("new", true),
                ("the_cross_product_is_square_to_both", false),
                ("dot", true),
                ("cross", false),
                ("x", true),
            ]
        );
        assert!(rows[3].group.contains('2'), "{}", rows[3].group);
    }

    /// The workspace's rows say where each symbol is, as the project names
    /// the file, and go there as a place.
    #[test]
    fn a_workspace_row_names_its_file_and_line() {
        let symbols = vec![symbol("cross", "method", Some("Vector"), 24)];
        let rows = workspace_rows(&symbols);
        assert_eq!(rows[0].right, "src/lib.rs:25");
        assert_eq!(rows[0].detail, "Vector");
        assert_eq!(rows[0].pick, Pick::Place(symbols[0].location.clone()));
    }

    /// A slow reply to `#gp`, landing after `#gpio` was asked, is never
    /// shown: not when it lands, and not after `gpio`'s own.
    #[test]
    fn a_slow_reply_to_an_earlier_name_is_never_shown() {
        let mut search = Search::default();
        let gp = search.ask("gp");
        let gpio = search.ask("gpio");
        let theirs = vec![symbol("gpu_reset", "function", None, 3)];
        assert!(!search.answered(gp, Ok(theirs)), "overtaken by gpio's ask");
        assert_eq!(search.view("gpio"), View::Waiting);
        let ours = vec![symbol("Gpio", "struct", None, 1)];
        assert!(search.answered(gpio, Ok(ours.clone())));
        assert_eq!(search.view("gpio"), View::Answer(&ours));
        assert_eq!(search.shown_for(), Some("gpio"));
        assert_eq!(search.dropped(), 1);
    }

    /// While a new name's answer is on its way, the last answer is shown
    /// only as far as the new name matches it.
    #[test]
    fn an_earlier_answer_is_narrowed_to_the_name_typed_now() {
        let mut search = Search::default();
        let cro = search.ask("cro");
        let found = vec![
            symbol("cross", "method", None, 24),
            symbol("crop", "function", None, 30),
        ];
        assert!(search.answered(cro, Ok(found.clone())));
        search.ask("cros");
        assert_eq!(search.view("cros"), View::Narrowed(vec![&found[0]]));
        assert_eq!(search.view("zz"), View::Waiting);
        search.forget();
        assert_eq!(search.view("cro"), View::Waiting, "closed: nothing kept");
        assert!(!search.answered(cro, Ok(found)), "nor anything late");
    }

    /// The rows made again while a name waits for its pause do not start
    /// the pause again: only a new name does, and an asked one never.
    #[test]
    fn a_pause_runs_out_however_often_the_rows_are_made() {
        let mut search = Search::default();
        assert!(search.wants("cro"), "typed: the pause starts");
        assert!(!search.wants("cro"), "the rows again: it keeps running");
        assert!(search.wants("cros"), "typed on: it starts again");
        search.ask("cros");
        assert!(!search.wants("cros"), "asked: nothing to wait for");
        assert!(search.wants("cross"));
        search.unpause();
        assert!(search.wants("cross"), "stopped: started again");
        search.forget();
        assert!(search.wants("cross"));
    }

    #[test]
    fn a_refusal_is_said_and_an_empty_answer_asked_again() {
        let mut search = Search::default();
        let asked = search.ask("gp");
        assert!(search.answered(asked, Err("content modified".into())));
        assert_eq!(search.view("gp"), View::Failed("content modified"));
        assert!(search.forget_empty());
        assert!(!search.has_asked("gp"), "asked again");
        let asked = search.ask("gp");
        assert!(search.answered(asked, Ok(Vec::new())));
        assert!(search.forget_empty());
        assert!(!search.has_asked("gp"));
        // An answer with something in it is kept, and says so.
        let asked = search.ask("gp");
        let found = vec![symbol("Gpio", "struct", None, 1)];
        assert!(search.answered(asked, Ok(found)));
        assert!(!search.forget_empty());
        assert_eq!(search.shown_for(), Some("gp"));
    }

    /// Every kind has an icon and a heading in every language's catalogue.
    #[test]
    fn every_kind_has_a_heading() {
        for kind in KINDS {
            let key = format!("symbols.group.{kind}");
            let label = rusty_i18n::translate(&key);
            assert!(label.is_some(), "{key} is not in the catalogue");
            assert!(label.unwrap().contains("{count}"), "{key} has no count");
        }
        assert_eq!(glyph("no such kind"), (12, 0));
    }
}
