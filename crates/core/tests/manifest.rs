//! The manifest exists to be shared. These tests are what make that safe.

use redactor_core::manifest::*;
use redactor_core::matching::{find, merge_boxes, TextItem};
use redactor_core::pdfwrite::{build, Page};
use redactor_core::redact::filter_spans;
use redactor_core::variants::{variants, Tier};
use redactor_core::verify::verify;
use std::collections::BTreeMap;

const HASH_A: &str = "9f2a1c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f809";
const HASH_B: &str = "1b7c4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90a1b";
const WHEN: &str = "2026-08-26T20:11:03Z";

fn settings() -> Settings {
    Settings { dpi: 200, jpeg_quality: 0.9, text_layer: true }
}

/// A document dense with exactly the things that must never escape.
fn loaded_page() -> Vec<TextItem> {
    [
        "CSC 116 - Lab 3 Part 1",
        "Name: Jane Doe",
        "Unity ID: jdoe2  Email: jdoe2@ncsu.edu",
        "Benyamin Tabarsi, btaghiz@ncsu.edu",
        "Q2: Jane has 5 apples.",
    ]
    .iter()
    .enumerate()
    .map(|(i, t)| TextItem {
        text: (*t).into(),
        x: 54.0,
        y: 60.0 + i as f32 * 20.0,
        w: t.chars().count() as f32 * 5.5,
        h: 12.0,
        eol: true,
        confidence: None,
    })
    .collect()
}

fn jpeg() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/page.jpg")).unwrap()
}

/// Run the real pipeline, then build a manifest from its real report.
fn realistic_manifest() -> Manifest {
    let items = loaded_page();
    let vars = variants("Jane Doe", &["jdoe2@ncsu.edu".into()]);
    let hits = find(&items, &vars);
    let boxes = merge_boxes(
        hits.iter().filter(|m| m.tier == Tier::High).flat_map(|m| m.boxes.clone()).collect(),
    );
    let spans = filter_spans(&items, &boxes, 792.0);
    let pdf = build(&[Page {
        jpeg: jpeg(), px_w: 1700, px_h: 2200, pt_w: 612.0, pt_h: 792.0, spans,
    }]);

    let approved: Vec<String> =
        hits.iter().filter(|m| m.tier == Tier::High).map(|m| m.matched.clone()).collect();
    let declined: Vec<String> =
        hits.iter().filter(|m| m.tier != Tier::High).map(|m| m.matched.clone()).collect();
    let report = verify(&pdf, &approved, &declined, 1, boxes.len());

    let mut by_page = BTreeMap::new();
    by_page.insert(1usize, boxes.len());
    let mut by_source = BTreeMap::new();
    by_source.insert(Source::Text, boxes.len());

    let entry = build_entry(
        HASH_A, HASH_B, WHEN, 1, settings(), by_page, by_source,
        vec![
            TermStat { ordinal: 1, kind: TermKind::Name, applied: 4, declined: 2 },
            TermStat { ordinal: 2, kind: TermKind::Identifier, applied: 1, declined: 0 },
        ],
        vec![3, 7],
        Some(OcrStats {
            engine: "tesseract-wasm 0.11.0".into(),
            pages_scanned: vec![3, 7],
            mean_confidence: 0.93,
            words_below_threshold: 4,
        }),
        &report,
    )
    .expect("entry should build");

    manifest("4751b505", vec![entry])
}

/// The central assertion. Everything else in this file supports it.
#[test]
fn a_manifest_cannot_contain_document_text() {
    let json = serde_json::to_string_pretty(&realistic_manifest()).unwrap();
    for forbidden in [
        "Jane", "jane", "Doe", "doe", "jdoe2", "ncsu", "Benyamin", "Tabarsi",
        "btaghiz", "lab3", "Lab 3", "CSC", "apples", "Unity",
    ] {
        assert!(!json.contains(forbidden), "manifest leaked {:?}\n{}", forbidden, json);
    }
}

#[test]
fn a_manifest_says_what_it_is() {
    let json = serde_json::to_string(&realistic_manifest()).unwrap();
    assert!(json.contains("\"containsPersonalData\":false"));
    assert!(json.contains("pdf-redactor-manifest/1"));
    // It carries its own instructions for mapping entries back to files.
    assert!(json.contains("shasum -a 256"));
}

/// The hash fields are the only free-form strings in the structure, so they are
/// the only route by which text could get in. They are validated, not trusted.
#[test]
fn a_name_cannot_be_smuggled_through_a_hash_field() {
    let mut by_page = BTreeMap::new();
    by_page.insert(1usize, 1usize);
    let report = verify(&[], &[], &[], 1, 1);

    let attempt = |a: &str, b: &str, when: &str| {
        build_entry(a, b, when, 1, settings(), by_page.clone(), BTreeMap::new(),
                    vec![], vec![], None, &report)
    };

    let err = |r: Result<Entry, BuildError>| r.err().expect("should have been refused");

    assert_eq!(err(attempt("Jane Doe", HASH_B, WHEN)), BuildError::NotAHash("input"));
    assert_eq!(err(attempt(HASH_A, "jane_doe_lab3.pdf", WHEN)), BuildError::NotAHash("output"));
    assert_eq!(err(attempt(HASH_A, HASH_B, "Jane Doe")), BuildError::NotATimestamp);
    // Right length, wrong alphabet - still refused.
    assert_eq!(err(attempt(&"z".repeat(64), HASH_B, WHEN)), BuildError::NotAHash("input"));
    assert!(attempt(HASH_A, HASH_B, WHEN).is_ok());
}

/// Manifests are compared across tool versions to catch silent regressions, so
/// the same inputs must always produce the same bytes.
#[test]
fn serialisation_is_stable_for_diffing() {
    let a = serde_json::to_string_pretty(&realistic_manifest()).unwrap();
    let b = serde_json::to_string_pretty(&realistic_manifest()).unwrap();
    assert_eq!(a, b);
    // Page keys sorted numerically, not by insertion.
    let mut by_page = BTreeMap::new();
    for p in [9usize, 2, 40, 1] {
        by_page.insert(p, 1usize);
    }
    let report = verify(&[], &[], &[], 1, 4);
    let e = build_entry(HASH_A, HASH_B, WHEN, 40, settings(), by_page, BTreeMap::new(),
                        vec![], vec![], None, &report).unwrap();
    let j = serde_json::to_string(&e.redactions.by_page).unwrap();
    assert_eq!(j, r#"{"1":1,"2":1,"9":1,"40":1}"#);
}

/// A failed verification must be recorded as failed, not quietly recorded as fine.
#[test]
fn a_failed_check_is_recorded_honestly() {
    let items = loaded_page();
    let spans = filter_spans(&items, &[], 792.0);   // nothing redacted
    let pdf = build(&[Page {
        jpeg: jpeg(), px_w: 1700, px_h: 2200, pt_w: 612.0, pt_h: 792.0, spans,
    }]);
    let report = verify(&pdf, &["Jane Doe".to_string()], &[], 1, 0);
    assert!(!report.passed());

    let e = build_entry(HASH_A, HASH_B, WHEN, 1, settings(), BTreeMap::new(),
                        BTreeMap::new(), vec![], vec![], None, &report).unwrap();
    assert_eq!(e.checks.text_layer, "fail");
    // ...and still without naming what leaked.
    let json = serde_json::to_string(&e).unwrap();
    assert!(!json.contains("Jane"), "{}", json);
}
