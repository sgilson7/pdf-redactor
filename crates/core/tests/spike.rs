use redactor_core::pdfwrite::{build, Page, TextSpan};

/// Proves the hand-rolled writer emits a structurally sound file. The output is
/// dropped in the target dir so it can be opened by hand after a run, and to
/// keep the test self-contained on CI.
#[test]
fn writes_a_pdf_from_a_jpeg() {
    let jpeg = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/page.jpg"))
        .expect("fixture page.jpg missing");

    // 1700x2200 px at 200 DPI == 612x792 pt == US Letter.
    let page = Page {
        jpeg,
        px_w: 1700,
        px_h: 2200,
        pt_w: 612.0,
        pt_h: 792.0,
        // Two surviving spans. The name and Unity ID are deliberately absent:
        // this is what "filtered before it is ever written" looks like.
        spans: vec![
            TextSpan { text: "CSC 116 - Lab 3 Part 1".into(), x: 54.0, y: 738.0, size: 12.0, width: 130.0 },
            TextSpan { text: "Name:".into(), x: 54.0, y: 713.0, size: 12.0, width: 32.0 },
            TextSpan { text: "Unity ID:".into(), x: 54.0, y: 688.0, size: 12.0, width: 48.0 },
            TextSpan { text: "Q1: A loop repeats until the condition is false.".into(), x: 54.0, y: 648.0, size: 12.0, width: 260.0 },
        ],
    };

    let pdf = build(&[page]);
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/spike.pdf");
    let _ = std::fs::write(&out, &pdf);

    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(pdf.ends_with(b"%%EOF\n"));

    let text = String::from_utf8_lossy(&pdf);
    // The whole point: none of these may appear anywhere in the output bytes.
    for forbidden in ["Jane", "Doe", "jdoe2", "/Info", "/Metadata", "/ID", "Producer"] {
        assert!(!text.contains(forbidden), "output leaked {:?}", forbidden);
    }
    // Exactly one revision - no incremental update history.
    assert_eq!(text.matches("%%EOF").count(), 1);
}
