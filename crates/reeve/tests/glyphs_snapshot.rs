/// The web's copy of the glyph set, generated from `reeve::glyphs`, the one definition. A change here is a
/// deliberate change of the set: run with UPDATE_SNAPSHOTS=1 to rewrite `web/src/lib/glyphs.json`.
#[test]
fn web_glyphs_match_the_cli_table() {
    let mut set = serde_json::Map::new();
    for l in reeve::glyphs::Level::ALL {
        set.insert(
            l.name().into(),
            serde_json::json!({"glyph": l.glyph(), "ascii": l.ascii()}),
        );
    }
    let doc = serde_json::to_string_pretty(&set).unwrap() + "\n";
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/src/lib/glyphs.json");
    if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
        std::fs::write(&path, &doc).unwrap();
        return;
    }
    let stored = std::fs::read_to_string(&path)
        .expect("glyphs.json exists; run with UPDATE_SNAPSHOTS=1 once");
    assert_eq!(
        stored, doc,
        "the glyph set changed; if intended, rerun with UPDATE_SNAPSHOTS=1"
    );
}
