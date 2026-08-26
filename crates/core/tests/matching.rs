use redactor_core::matching::{find, TextItem};
use redactor_core::variants::{variants, Tier};

/// Build a run of fragments on one line, laid out flush unless a gap is asked
/// for. `(text, gap_before)` where gap is in points.
fn line(parts: &[(&str, f32)]) -> Vec<TextItem> {
    let mut x = 50.0f32;
    let mut out = Vec::new();
    for (t, gap) in parts {
        x += gap;
        let w = t.chars().count() as f32 * 6.0;
        out.push(TextItem { text: (*t).into(), x, y: 100.0, w, h: 12.0, eol: false, confidence: None });
        x += w;
    }
    if let Some(l) = out.last_mut() {
        l.eol = true;
    }
    out
}

fn hits(items: &[TextItem], name: &str) -> Vec<(String, Tier)> {
    let v = variants(name, &[]);
    find(items, &v).into_iter().map(|m| (m.matched, m.tier)).collect()
}

#[test]
fn finds_a_plain_name() {
    let items = line(&[("Name:", 0.0), ("Jane Doe", 6.0)]);
    let h = hits(&items, "Jane Doe");
    assert!(h.iter().any(|(m, t)| m == "Jane Doe" && *t == Tier::High), "{:?}", h);
}

/// The case that breaks naive per-fragment matching. Producers split runs at
/// kerning pairs, so the name arrives in pieces with no gaps between them.
#[test]
fn finds_a_name_split_across_fragments() {
    let items = line(&[("Name:", 0.0), ("Ja", 6.0), ("ne D", 0.0), ("oe", 0.0)]);
    let h = hits(&items, "Jane Doe");
    assert!(
        h.iter().any(|(_, t)| *t == Tier::High),
        "split name must still match at High: {:?}", h
    );
}

#[test]
fn split_match_produces_a_box_per_fragment() {
    let items = line(&[("Ja", 0.0), ("ne D", 0.0), ("oe", 0.0)]);
    let v = variants("Jane Doe", &[]);
    let m = find(&items, &v);
    let high: Vec<_> = m.iter().filter(|m| m.tier == Tier::High).collect();
    assert!(!high.is_empty());
    // Three fragments touched -> three boxes, together covering the whole name.
    assert_eq!(high[0].boxes.len(), 3, "{:?}", high[0].boxes);
}

#[test]
fn finds_unity_id_with_trailing_digits() {
    let items = line(&[("Unity ID:", 0.0), ("jdoe2", 6.0)]);
    let h = hits(&items, "Jane Doe");
    assert!(h.iter().any(|(m, _)| m == "jdoe2"), "trailing digit not claimed: {:?}", h);
}

#[test]
fn finds_email() {
    let items = line(&[("jdoe2@ncsu.edu", 0.0)]);
    let h = hits(&items, "Jane Doe");
    assert!(h.iter().any(|(m, _)| m.starts_with("jdoe2")), "{:?}", h);
}

#[test]
fn matches_across_a_line_break() {
    // "Jo-\nhnson" hyphenated by the wrap.
    let mut items = line(&[("Name: Jo-", 0.0)]);
    items[0].eol = true;
    items.push(TextItem { text: "hnson".into(), x: 50.0, y: 115.0, w: 30.0, h: 12.0, eol: true, confidence: None });
    let h = hits(&items, "Amy Johnson");
    assert!(h.iter().any(|(_, t)| *t <= Tier::Medium), "hyphenated wrap missed: {:?}", h);
}

// --- false positives: these must not be auto-applied ---

#[test]
fn does_not_fire_inside_a_longer_word() {
    let items = line(&[("The loop doesn't terminate", 0.0)]);
    let h = hits(&items, "Jane Doe");
    assert!(
        !h.iter().any(|(m, _)| m.to_lowercase() == "doe"),
        "'doe' fired inside 'doesn't': {:?}", h
    );
}

#[test]
fn bare_first_name_in_prose_is_not_high() {
    let items = line(&[("Jane has 5 apples and gives 2 away", 0.0)]);
    let h = hits(&items, "Jane Doe");
    assert!(
        !h.iter().any(|(m, t)| m == "Jane" && *t == Tier::High),
        "prose first-name must not be High: {:?}", h
    );
}

#[test]
fn common_word_name_is_not_high() {
    let items = line(&[("This will print the value", 0.0)]);
    let h = hits(&items, "Will Smith");
    assert!(
        !h.iter().any(|(m, t)| m.to_lowercase() == "will" && *t <= Tier::Medium),
        "'will' must stay Low: {:?}", h
    );
}

#[test]
fn nested_matches_are_suppressed() {
    // "Jane Doe" should yield one High hit, not also bare "jane" and "doe".
    let items = line(&[("Jane Doe", 0.0)]);
    let v = variants("Jane Doe", &[]);
    let m = find(&items, &v);
    assert_eq!(m.len(), 1, "expected one merged hit, got {:?}",
        m.iter().map(|x| (&x.matched, x.tier)).collect::<Vec<_>>());
}

#[test]
fn diacritics_and_case_still_match() {
    let items = line(&[("JOSÉ GARCÍA", 0.0)]);
    let h = hits(&items, "Jose Garcia");
    assert!(h.iter().any(|(_, t)| *t == Tier::High), "{:?}", h);
}

#[test]
fn zero_width_characters_do_not_hide_a_name() {
    let items = line(&[("Ja\u{200B}ne Doe", 0.0)]);
    let h = hits(&items, "Jane Doe");
    assert!(h.iter().any(|(_, t)| *t == Tier::High), "zero-width defeated match: {:?}", h);
}

#[test]
fn typo_is_found_at_low_tier_only() {
    let items = line(&[("Submitted by Johnsen", 0.0)]);
    let h = hits(&items, "Amy Johnson");
    let t = h.iter().find(|(m, _)| m.to_lowercase() == "johnsen");
    assert!(t.is_some(), "typo not found: {:?}", h);
    assert_eq!(t.unwrap().1, Tier::Low, "typo must be Low");
}

#[test]
fn empty_page_yields_nothing() {
    assert!(find(&[], &variants("Jane Doe", &[])).is_empty());
}

/// A wrapped line whose fragments carry no EOL flag must not glue the last word
/// of one line to the first of the next. Real documents do this constantly, and
/// the glued word then fails the word-boundary test so the match is lost.
#[test]
fn a_wrapped_line_does_not_glue_words_together() {
    let items = vec![
        TextItem { text: "a directory containing provision.sh".into(),
                   x: 54.0, y: 100.0, w: 190.0, h: 12.0, eol: false, confidence: None },
        // Next line: back at the left margin, lower down, and NOT flagged eol.
        TextItem { text: "docker build -t csc584-env .".into(),
                   x: 54.0, y: 116.0, w: 150.0, h: 12.0, eol: false, confidence: None },
    ];
    let h = hits(&items, "Docker");
    assert!(
        h.iter().any(|(m, _)| m.eq_ignore_ascii_case("docker")),
        "word after a wrap was missed: {:?}", h
    );
}

/// The same, for a person's name split by the wrap.
#[test]
fn a_name_at_the_start_of_a_wrapped_line_is_found() {
    let items = vec![
        TextItem { text: "This assignment was submitted by".into(),
                   x: 54.0, y: 100.0, w: 170.0, h: 12.0, eol: false, confidence: None },
        TextItem { text: "Jane Doe on March 3.".into(),
                   x: 54.0, y: 116.0, w: 110.0, h: 12.0, eol: false, confidence: None },
    ];
    let h = hits(&items, "Jane Doe");
    assert!(h.iter().any(|(_, t)| *t == Tier::High),
        "name after a wrap was missed entirely: {:?}", h);
}

/// The fix must not break mid-word fragmentation on a single line.
#[test]
fn same_line_fragments_still_join_without_a_break() {
    let items = line(&[("Name:", 0.0), ("Ja", 6.0), ("ne D", 0.0), ("oe", 0.0)]);
    let h = hits(&items, "Jane Doe");
    assert!(h.iter().any(|(_, t)| *t == Tier::High), "{:?}", h);
}

/// An affiliation marker set flush against a name must not fuse into it.
/// "Benyamin Tabarsi" followed by a superscript "1" became "benyamin tabarsi1",
/// and the trailing digit failed the word-boundary test, so a paper's own
/// author line went unredacted.
#[test]
fn a_superscript_marker_does_not_swallow_the_name() {
    let items = vec![
        TextItem { text: "Benyamin Tabarsi".into(), x: 46.8, y: 367.4, w: 72.1, h: 9.5, eol: false, confidence: None },
        // Superscript: smaller font, slightly raised, flush against the name.
        TextItem { text: "1".into(), x: 118.9, y: 364.4, w: 3.5, h: 6.6, eol: false, confidence: None },
        TextItem { text: " · Heidi Reichert".into(), x: 133.2, y: 367.4, w: 62.1, h: 9.5, eol: true, confidence: None },
    ];
    let h = hits(&items, "Benyamin Tabarsi");
    assert!(
        h.iter().any(|(_, t)| *t == Tier::High),
        "name fused with its affiliation marker: {:?}", h
    );
}

/// Reference-list order, as every bibliography writes it.
#[test]
fn last_name_then_initial_is_offered() {
    let items = line(&[("Tabarsi B, Yasir T, Reichert H (2025) Herald", 0.0)]);
    let h = hits(&items, "Benyamin Tabarsi");
    assert!(
        h.iter().any(|(m, _)| m.to_lowercase().starts_with("tabarsi b")),
        "citation form not offered: {:?}", h
    );
}

// --- OCR-derived matches ------------------------------------------------
//
// OCR word boxes arrive as ordinary TextItems carrying a confidence. Nothing
// in the matcher is OCR-aware beyond the demotion rule: the point is that a
// scanned page and a text page take the same path.

/// Words as an OCR engine returns them: one item per word, no EOL flags,
/// positioned by geometry alone.
fn ocr_words(words: &[(&str, f32, f32, f32)], conf: f32) -> Vec<TextItem> {
    words.iter().map(|(t, x, y, w)| TextItem {
        text: (*t).into(), x: *x, y: *y, w: *w, h: 10.0, eol: false,
        confidence: Some(conf),
    }).collect()
}

fn line_of_words(conf: f32) -> Vec<TextItem> {
    // "Name: Jane Doe" as four separate OCR word boxes with real gaps.
    ocr_words(&[("Name:", 50.0, 100.0, 24.0), ("Jane", 78.0, 100.0, 20.0),
                ("Doe", 102.0, 100.0, 16.0)], conf)
}

#[test]
fn a_name_split_across_ocr_word_boxes_is_found() {
    let h = hits(&line_of_words(0.95), "Jane Doe");
    assert!(h.iter().any(|(m, _)| m == "Jane Doe"),
        "OCR words did not join into a name: {:?}", h);
}

#[test]
fn a_confident_ocr_name_stays_high() {
    let m = find(&line_of_words(0.95), &variants("Jane Doe", &[]));
    let hit = m.iter().find(|m| m.matched == "Jane Doe").expect("no match");
    assert_eq!(hit.tier, Tier::High, "confident OCR should still be pre-applied");
    assert_eq!(hit.source, redactor_core::matching::Source::Ocr);
    assert!((hit.confidence.unwrap() - 0.95).abs() < 1e-6);
}

#[test]
fn a_middling_ocr_name_is_demoted_to_medium() {
    let m = find(&line_of_words(0.70), &variants("Jane Doe", &[]));
    let hit = m.iter().find(|m| m.matched == "Jane Doe").expect("no match");
    assert_eq!(hit.tier, Tier::Medium, "shaky OCR must not be pre-applied");
}

#[test]
fn a_poor_ocr_name_is_demoted_to_low() {
    let m = find(&line_of_words(0.40), &variants("Jane Doe", &[]));
    let hit = m.iter().find(|m| m.matched == "Jane Doe").expect("no match");
    assert_eq!(hit.tier, Tier::Low);
}

/// Confidence is the minimum, not the mean: one badly read word makes the
/// whole match doubtful, and averaging would hide it behind its neighbours.
#[test]
fn confidence_is_the_weakest_word_not_the_average() {
    let mut items = line_of_words(0.99);
    items[2].confidence = Some(0.50);         // "Doe" read poorly
    let m = find(&items, &variants("Jane Doe", &[]));
    let hit = m.iter().find(|m| m.matched == "Jane Doe").expect("no match");
    assert_eq!(hit.tier, Tier::Low, "a weak word must drag the match down");
}

/// Text-layer matches are unaffected by any of the above.
#[test]
fn text_layer_matches_report_no_confidence() {
    let m = find(&line(&[("Name: Jane Doe", 0.0)]), &variants("Jane Doe", &[]));
    let hit = &m[0];
    assert_eq!(hit.source, redactor_core::matching::Source::Text);
    assert!(hit.confidence.is_none());
    assert_eq!(hit.tier, Tier::High);
}

/// Identifier scanning works on OCR items too, which is what makes an email
/// inside a screenshot findable at all.
#[test]
fn an_email_in_an_image_is_found_by_shape() {
    let items = ocr_words(&[("Contact:", 50.0, 100.0, 34.0),
                            ("btaghiz@ncsu.edu", 88.0, 100.0, 70.0)], 0.93);
    let found = redactor_core::identifiers::find_identifiers(&items);
    assert_eq!(found.len(), 1, "{:?}", found.iter().map(|c| &c.text).collect::<Vec<_>>());
    assert_eq!(found[0].text, "btaghiz@ncsu.edu");
}
