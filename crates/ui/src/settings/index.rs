//! The settings search: every setting row and page anchor as a fuzzy-matched entry.

use crate::fuzzy_list::{self, FuzzyRow};

use super::spec::SettingId;
use super::{Anchor, Section, Target};

/// Most results shown under the search box.
pub const MAX_RESULTS: usize = 8;

/// One search hit.
#[derive(Clone, Debug)]
pub struct Hit {
    /// Where it jumps.
    pub target: Target,
    /// What it is called.
    pub title: String,
    /// The page and group it is on.
    pub place: String,
}

/// Everything the search covers, in page order.
fn entries() -> Vec<(Target, FuzzyRow)> {
    let settings = SettingId::all().map(|id| {
        let spec = id.spec();
        let row = FuzzyRow {
            title: spec.label.to_owned(),
            subtitle: format!("{} · {}", spec.section.label(), spec.group),
            keywords: vec![spec.help.to_owned(), spec.keywords.to_owned()],
            ..FuzzyRow::default()
        };
        (Target::Setting(id), row)
    });
    let anchors = Anchor::ALL.iter().map(|&anchor| {
        let row = FuzzyRow {
            title: anchor.title().to_owned(),
            subtitle: anchor.section().label().to_owned(),
            keywords: vec![anchor.keywords().to_owned()],
            ..FuzzyRow::default()
        };
        (Target::Anchor(anchor), row)
    });
    let hotkeys = fogu_core::Hotkeys::default_keymap();
    let actions = hotkeys.bindings().keys().map(|&action| {
        let row = FuzzyRow {
            title: super::shortcuts::label(action),
            subtitle: "Shortcuts".to_owned(),
            keywords: vec![action.to_string(), "shortcut key binding".to_owned()],
            ..FuzzyRow::default()
        };
        (Target::Action(action), row)
    });
    let mut all: Vec<(Target, FuzzyRow)> = anchors.chain(settings).chain(actions).collect();
    all.sort_by_key(|(target, _)| Section::ALL.iter().position(|s| *s == target.section()));
    all
}

/// The best [`MAX_RESULTS`] matches of `query`.
pub fn search(query: &str) -> Vec<Hit> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let entries = entries();
    let rows: Vec<FuzzyRow> = entries.iter().map(|(_, row)| row.clone()).collect();
    fuzzy_list::rank(query, &rows)
        .into_iter()
        .take(MAX_RESULTS)
        .filter_map(|i| entries.get(i))
        .map(|(target, row)| Hit {
            target: *target,
            title: row.title.clone(),
            place: row.subtitle.clone(),
        })
        .collect()
}
