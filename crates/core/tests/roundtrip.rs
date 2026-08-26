//! The test that matters: a full document through the real pipeline, then
//! verified the way an adversary would check it.

use redactor_core::matching::{find, merge_boxes, TextItem};
use redactor_core::pdfwrite::{build, Page};
use redactor_core::redact::filter_spans;
use redactor_core::variants::{variants, Tier};
use redactor_core::verify::{extract_literals, verify, Finding};

/// Terms that were applied, and terms that were offered but declined.
fn split_terms<'a>(
    matches: &'a [redactor_core::matching::Match],
    vars: &'a [redactor_core::variants::Variant],
) -> (Vec<String>, Vec<String>) {
    let approved: Vec<String> = vars
        .iter()
        .filter(|v| v.tier == Tier::High)
        .map(|v| v.term.clone())
        .collect();
    let declined: Vec<String> = vars
        .iter()
        .filter(|v| v.tier != Tier::High)
        .map(|v| v.term.clone())
        .collect();
    let _ = matches;
    (approved, declined)
}

const PAGE_W: f32 = 612.0;
const PAGE_H: f32 = 792.0;

/// A page resembling a real submission: header, name, unity id, and body text.
fn submission() -> Vec<TextItem> {
    let rows: &[(&str, f32)] = &[
        ("CSC 116 - Lab 3 Part 1", 60.0),
        ("Name: Jane Doe", 85.0),
        ("Unity ID: jdoe2  Email: jdoe2@ncsu.edu", 110.0),
        ("Q1: A loop repeats until the condition is false.", 150.0),
        ("Q2: Jane has 5 apples; the array will hold them.", 175.0),
    ];
    rows.iter()
        .map(|(t, y)| TextItem {
            text: (*t).into(),
            x: 54.0,
            y: *y,
            w: t.chars().count() as f32 * 5.5,
            h: 12.0,
            eol: true,
        })
        .collect()
}

fn jpeg() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/page.jpg")).unwrap()
}

#[test]
fn full_pipeline_produces_a_verifiably_clean_pdf() {
    let items = submission();
    let vars = variants("Jane Doe", &["jdoe2@ncsu.edu".into()]);

    // Only High-confidence hits are applied without review, exactly as the UI
    // will pre-check them.
    let matches = find(&items, &vars);
    let approved: Vec<_> = matches.iter().filter(|m| m.tier == Tier::High).collect();
    assert!(!approved.is_empty(), "nothing auto-detected");

    let boxes = merge_boxes(approved.iter().flat_map(|m| m.boxes.clone()).collect());
    let spans = filter_spans(&items, &boxes, PAGE_H);

    let pdf = build(&[Page {
        jpeg: jpeg(),
        px_w: 1700,
        px_h: 2200,
        pt_w: PAGE_W,
        pt_h: PAGE_H,
        spans,
    }]);

    let (approved, declined) = split_terms(&matches, &vars);
    let report = verify(&pdf, &approved, &declined, 1, boxes.len());
    assert!(
        report.passed(),
        "verification failed: {:?}", report.blocking()
    );

    // "Jane" survives in "Q2: Jane has 5 apples" because the bare first name is
    // Medium and was never approved. That is reported, not treated as failure -
    // redacting it would have destroyed the problem statement.
    assert!(
        report.advisories().iter().any(|f| matches!(f, Finding::Residual { term, .. } if term == "jane")),
        "expected an advisory about the declined first name: {:?}", report.findings
    );

    // The surviving text layer must still be useful, or the feature is pointless.
    let text = extract_literals(&pdf);
    assert!(text.contains("CSC 116"), "body text lost: {:?}", text);
    assert!(text.contains("loop repeats"), "body text lost: {:?}", text);
}

#[test]
fn verification_catches_a_leak_that_slipped_through() {
    // Same document, but with no redactions applied at all. Verification must
    // refuse it - this is the negative control proving the check has teeth.
    let items = submission();
    let vars = variants("Jane Doe", &[]);
    let spans = filter_spans(&items, &[], PAGE_H);

    let pdf = build(&[Page {
        jpeg: jpeg(), px_w: 1700, px_h: 2200, pt_w: PAGE_W, pt_h: PAGE_H, spans,
    }]);

    let approved: Vec<String> = vars.iter().map(|v| v.term.clone()).collect();
    let report = verify(&pdf, &approved, &[], 1, 0);
    assert!(!report.passed(), "verifier missed an obvious leak");
    assert!(
        report.findings.iter().any(|f| matches!(f, Finding::Leak { .. })),
        "expected a Leak finding, got {:?}", report.findings
    );
}

#[test]
fn output_carries_no_metadata_and_one_revision() {
    let pdf = build(&[Page {
        jpeg: jpeg(), px_w: 1700, px_h: 2200, pt_w: PAGE_W, pt_h: PAGE_H, spans: vec![],
    }]);
    let report = verify(&pdf, &[], &[], 1, 0);
    assert!(report.passed(), "{:?}", report.findings);
    assert_eq!(String::from_utf8_lossy(&pdf).matches("%%EOF").count(), 1);
}

#[test]
fn multi_page_documents_build() {
    let mk = || Page {
        jpeg: jpeg(), px_w: 1700, px_h: 2200, pt_w: PAGE_W, pt_h: PAGE_H,
        spans: filter_spans(&submission(), &[], PAGE_H),
    };
    let pdf = build(&[mk(), mk(), mk()]);
    assert!(String::from_utf8_lossy(&pdf).contains("/Count 3"));
    assert_eq!(String::from_utf8_lossy(&pdf).matches("%%EOF").count(), 1);
}

/// Regression: the UI passes terms exactly as the user saw them on screen,
/// which are mixed-case, while the document text is folded to lowercase before
/// comparison. Comparing the two raw silently matched nothing, so every leak
/// check passed. Verification must fold its own inputs.
#[test]
fn verification_is_case_insensitive_about_its_inputs() {
    let items = submission();
    let spans = filter_spans(&items, &[], PAGE_H);
    let pdf = build(&[Page {
        jpeg: jpeg(), px_w: 1700, px_h: 2200, pt_w: PAGE_W, pt_h: PAGE_H, spans,
    }]);

    // Raw display casing, as app.js sends it - not pre-normalized.
    let report = verify(&pdf, &["Jane Doe".to_string()], &[], 1, 0);
    assert!(
        !report.passed(),
        "a mixed-case approved term must still be detected as a leak"
    );

    // And the advisory path folds too.
    let report = verify(&pdf, &[], &["Jane".to_string()], 1, 0);
    assert!(
        report.advisories().iter().any(|f| matches!(f, Finding::Residual { .. })),
        "mixed-case declined term produced no advisory: {:?}", report.findings
    );
}

/// Regression: image payloads must be excluded from every check.
///
/// A JPEG is effectively random bytes, so short structural tokens turn up in
/// one by chance - a 125-page document produced a bogus "/JS" finding and
/// blocked its own download. Normalizing that much binary also allocates
/// gigabytes and aborts the module outright.
#[test]
fn image_payloads_are_never_scanned() {
    // A payload that would trip every check if it were inspected.
    let mut fake = b"\xFF\xD8\xFF\xE0 /JS /Info /Annots Jane Doe jdoe2 %%EOF ".to_vec();
    fake.extend_from_slice(&vec![0x5Au8; 4096]);

    // Redact properly, so anything the verifier finds came from the image.
    let items = submission();
    let vars = variants("Jane Doe", &[]);
    let boxes = merge_boxes(
        find(&items, &vars)
            .iter()
            .filter(|m| m.tier == Tier::High)
            .flat_map(|m| m.boxes.clone())
            .collect(),
    );
    let pdf = build(&[Page {
        jpeg: fake,
        px_w: 10, px_h: 10, pt_w: PAGE_W, pt_h: PAGE_H,
        spans: filter_spans(&items, &boxes, PAGE_H),
    }]);

    let report = verify(&pdf, &["Jane Doe".to_string()], &[], 1, 0);
    assert!(
        report.passed(),
        "image bytes leaked into the checks: {:?}", report.blocking()
    );

    // And the split still finds the real content: the text layer is intact.
    let text = extract_literals(&redactor_core::verify::views(&pdf).content);
    assert!(text.contains("CSC 116"), "content stream lost: {:?}", text);
}

/// Typographic punctuation must survive into the text layer as its ASCII
/// equivalent. Dropping a curly apostrophe to a space turns "didn't" into
/// "didn t", which breaks ordinary word search across an anonymised corpus.
#[test]
fn smart_punctuation_becomes_searchable_ascii() {
    let spans = vec![redactor_core::pdfwrite::TextSpan {
        text: "they didn\u{2019}t \u{201C}quote\u{201D} \u{2014} yes\u{2026}".into(),
        x: 50.0, y: 700.0, size: 12.0, width: 200.0,
    }];
    let pdf = build(&[Page {
        jpeg: jpeg(), px_w: 1700, px_h: 2200, pt_w: PAGE_W, pt_h: PAGE_H, spans,
    }]);
    let text = extract_literals(&redactor_core::verify::views(&pdf).content);
    assert!(text.contains("didn't"), "apostrophe lost: {:?}", text);
    assert!(text.contains("\"quote\""), "quotes lost: {:?}", text);
    assert!(text.contains("- yes..."), "dash/ellipsis lost: {:?}", text);
}
