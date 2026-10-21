#![expect(clippy::unwrap_used, reason = "tests")]

use std::collections::HashSet;
use std::path::PathBuf;

use super::*;
use crate::scheme::ColorScheme;

struct Tmp(PathBuf);
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp() -> Tmp {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("fogu-style-{}-{n}", std::process::id()));
    std::fs::create_dir_all(dir.join(STYLES_DIR)).unwrap();
    Tmp(dir)
}

fn write(t: &Tmp, file: &str, text: &str) {
    std::fs::write(t.0.join(STYLES_DIR).join(file), text).unwrap();
}

#[test]
fn builtins_parse_and_are_distinct() {
    let catalog = StyleCatalog::builtins();
    assert_eq!(catalog.styles().len(), BUILTIN_SOURCES.len());
    let mut seen = HashSet::new();
    for style in catalog.styles() {
        let own: HashSet<String> = std::iter::once(&style.key)
            .chain(std::iter::once(&style.name))
            .chain(style.aliases.iter())
            .map(|name| name.to_ascii_lowercase())
            .collect();
        for name in own {
            assert!(seen.insert(name.clone()), "{name:?} is claimed twice");
        }
        let mut clean = style.clone();
        assert!(
            clean.sanitize().is_empty(),
            "{} has out-of-range values",
            style.key
        );
    }
    assert_eq!(catalog.default_style().key, "fogu");
}

#[test]
fn paired_schemes_are_builtin() {
    for style in StyleCatalog::builtins().styles() {
        if let Some(scheme) = &style.paired_scheme {
            assert!(
                ColorScheme::by_name(scheme).is_some(),
                "{} pairs with unknown scheme {scheme:?}",
                style.key
            );
        }
    }
}

#[test]
fn resolves_keys_names_and_aliases_in_any_case() {
    let catalog = StyleCatalog::builtins();
    for (name, key) in [
        ("fogu", "fogu"),
        ("EMBER", "fogu"),
        (" fluent ", "fluent"),
        ("windows", "fluent"),
        ("Mica", "fluent"),
        ("paper", "minimal"),
        ("ink", "minimal"),
        ("terminal", "retro"),
        ("Phosphor", "retro"),
    ] {
        assert_eq!(catalog.resolve(name).unwrap().key, key, "{name}");
    }
    assert!(catalog.resolve("nope").is_none());
}

#[test]
fn missing_directory_is_fine() {
    let dir = std::env::temp_dir().join(format!("fogu-style-none-{}", std::process::id()));
    let catalog = StyleCatalog::load(&dir);
    assert!(catalog.warnings().is_empty(), "{:?}", catalog.warnings());
    assert_eq!(catalog.styles().len(), BUILTIN_SOURCES.len());
}

#[test]
fn custom_file_extends_and_patches_nested_sections() {
    let t = tmp();
    write(
        &t,
        "mine.toml",
        r##"
name = "Mine"
extends = "mica"

[color]
accent = "#123456"

[shape]
card = 4

[shadow.floating]
blur = 30

[tabs]
style = "underline"
marker = "index"
"##,
    );
    let catalog = StyleCatalog::load(&t.0);
    assert!(catalog.warnings().is_empty(), "{:?}", catalog.warnings());
    let base = StyleCatalog::builtins().resolve("mica").unwrap().clone();
    let mine = catalog.resolve("mine").unwrap();
    assert_eq!(mine.key, "Mine");
    assert!(mine.aliases.is_empty(), "{:?}", mine.aliases);
    assert_eq!(mine.color.accent, RgbColor::from_hex(0x12_3456));
    assert!((mine.shape.card - 4.0).abs() < f32::EPSILON);
    assert!((mine.shadow.floating.blur - 30.0).abs() < f32::EPSILON);
    assert_eq!(mine.shadow.floating.alpha, base.shadow.floating.alpha);
    assert_eq!(mine.tabs.style, TabStyle::Underline);
    assert_eq!(mine.tabs.marker, TabMarker::Index);
    assert_eq!(mine.tabs.sizing, base.tabs.sizing);
    assert_eq!(mine.surface, base.surface);
    assert!(!StyleCatalog::is_builtin(mine));
}

#[test]
fn extends_defaults_to_the_default_style() {
    let t = tmp();
    write(&t, "plain.toml", "name = \"Plain\"\n");
    let catalog = StyleCatalog::load(&t.0);
    let plain = catalog.resolve("Plain").unwrap();
    assert_eq!(plain.surface, catalog.default_style().surface);
}

#[test]
fn bad_files_are_skipped_with_warnings() {
    let t = tmp();
    write(&t, "a-bad.toml", "not valid toml [[[");
    write(&t, "b-reserved.toml", "name = \"Windows\"\n");
    write(
        &t,
        "c-unknown-base.toml",
        "name = \"Orphan\"\nextends = \"nope\"\n",
    );
    write(
        &t,
        "d-unknown-key.toml",
        "name = \"Typo\"\n[tabs]\nstyel = \"pill\"\n",
    );
    write(&t, "e-empty.toml", "name = \"  \"\n");
    std::fs::write(
        t.0.join(STYLES_DIR).join("f-huge.toml"),
        "#".repeat(300 * 1024),
    )
    .unwrap();
    let catalog = StyleCatalog::load(&t.0);
    assert_eq!(catalog.styles().len(), BUILTIN_SOURCES.len());
    let messages: Vec<&str> = catalog
        .warnings()
        .iter()
        .map(|w| w.message.as_str())
        .collect();
    assert_eq!(messages.len(), 6, "{messages:?}");
    assert!(
        messages.iter().any(|m| m.contains("reserved")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("unknown built-in")),
        "{messages:?}"
    );
    assert!(messages.iter().any(|m| m.contains("styel")), "{messages:?}");
    assert!(
        messages.iter().any(|m| m.contains("byte limit")),
        "{messages:?}"
    );
}

#[test]
fn first_custom_name_wins() {
    let t = tmp();
    write(&t, "a.toml", "name = \"Same\"\n[shape]\ncard = 1\n");
    write(&t, "b.toml", "name = \"same\"\n[shape]\ncard = 2\n");
    let catalog = StyleCatalog::load(&t.0);
    assert_eq!(catalog.styles().len(), BUILTIN_SOURCES.len() + 1);
    assert_eq!(catalog.warnings().len(), 1);
    assert!((catalog.resolve("Same").unwrap().shape.card - 1.0).abs() < f32::EPSILON);
}

#[test]
fn out_of_range_values_are_clamped_with_a_warning() {
    let t = tmp();
    write(
        &t,
        "wild.toml",
        "name = \"Wild\"\n[bar]\nheight = 200\n[tabs]\nmin_width = 300\nmax_width = 100\n",
    );
    let catalog = StyleCatalog::load(&t.0);
    let wild = catalog.resolve("Wild").unwrap();
    assert!((wild.bar.height - 64.0).abs() < f32::EPSILON);
    assert!(wild.tabs.max_width >= wild.tabs.min_width);
    assert_eq!(catalog.warnings().len(), 2, "{:?}", catalog.warnings());
}

#[test]
fn check_and_save_write_a_new_file_and_refuse_clashes() {
    let t = tmp();
    let catalog = StyleCatalog::load(&t.0);
    let text = "name = \"Code Made\"\nextends = \"synthwave\"\n[tabs]\nmarker = \"index\"\n";
    let (key, path) = catalog.save(&t.0, text, None).unwrap();
    assert_eq!(key, "Code Made");
    assert_eq!(path, t.0.join(STYLES_DIR).join("code-made.toml"));
    let catalog = StyleCatalog::load(&t.0);
    assert_eq!(catalog.source("Code Made"), Some(path.as_path()));
    // Saving it again as new clashes with the loaded style; editing its own file does not.
    assert!(
        catalog
            .check(text, None)
            .unwrap_err()
            .contains("already named")
    );
    let edited = text.replace("index", "none");
    catalog.save(&t.0, &edited, Some(&path)).unwrap();
    let catalog = StyleCatalog::load(&t.0);
    assert_eq!(
        catalog.resolve("code made").unwrap().tabs.marker,
        TabMarker::None
    );
    assert!(
        catalog
            .check("name = \"Ember\"\n", None)
            .unwrap_err()
            .contains("reserved")
    );
}
