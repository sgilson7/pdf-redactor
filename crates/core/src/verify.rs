//! Proving the output is clean, by inspecting the output.
//!
//! Every other module asserts what it *intended* to do. This one re-reads the
//! finished bytes and checks what actually happened. A tool that claims a
//! redaction guarantee without re-examining its own output is asking to be
//! trusted rather than demonstrating it deserves to be.

use crate::matching::TextItem;
use crate::normalize::normalize_term;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum Finding {
    /// A structural key that should never appear in output we constructed.
    Structure(String),
    /// A term the user *approved* for redaction survived into the output. This
    /// means a redaction silently failed to take effect, so it blocks export.
    Leak { term: String, where_: &'static str },
    /// A term the user reviewed and chose *not* to redact is still present.
    ///
    /// This is not a defect. A student named Jane doing a worksheet that reads
    /// "Jane has 5 apples" should not have the problem statement destroyed, and
    /// a tool that forced that trade would push people into redacting nothing
    /// or redacting everything. It is still worth saying out loud at export
    /// time, so the choice stays deliberate rather than forgotten.
    Residual { term: String, count: usize },
    /// More than one revision, i.e. incremental-update history is present.
    MultipleRevisions(usize),
}

impl Finding {
    /// Blocking findings stop the download; advisory ones are reported.
    pub fn is_blocking(&self) -> bool {
        !matches!(self, Finding::Residual { .. })
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub pages: usize,
    pub redactions: usize,
    pub bytes: usize,
}

impl Report {
    /// True when nothing blocking was found. Advisory residuals do not fail.
    pub fn passed(&self) -> bool {
        !self.findings.iter().any(Finding::is_blocking)
    }

    pub fn blocking(&self) -> Vec<&Finding> {
        self.findings.iter().filter(|f| f.is_blocking()).collect()
    }

    pub fn advisories(&self) -> Vec<&Finding> {
        self.findings.iter().filter(|f| !f.is_blocking()).collect()
    }
}

/// Keys that carry identity or history. None of these are ever written by
/// `pdfwrite`, so finding one means something went badly wrong.
const FORBIDDEN: &[&str] = &[
    "/Info", "/Metadata", "/EmbeddedFile", "/JavaScript", "/JS", "/OpenAction",
    "/Annots", "/AcroForm", "/StructTreeRoot", "/OCProperties", "/Thumb",
    "/PieceInfo", "/Names", "/Producer", "/Creator", "/Author", "/CreationDate",
    "/ModDate", "/GoToR", "/Launch", "/URI",
];

/// Pull every literal string out of the file's content streams.
///
/// This works precisely because `pdfwrite` leaves content streams uncompressed.
/// If that ever changes, this check silently weakens - which is why the
/// round-trip test asserts a known span is found here.
pub fn extract_literals(pdf: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < pdf.len() {
        if pdf[i] == b'(' {
            let mut j = i + 1;
            let mut depth = 1;
            let mut buf = Vec::new();
            while j < pdf.len() && depth > 0 {
                match pdf[j] {
                    b'\\' if j + 1 < pdf.len() => {
                        buf.push(pdf[j + 1]);
                        j += 2;
                        continue;
                    }
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                buf.push(pdf[j]);
                j += 1;
            }
            out.push_str(&String::from_utf8_lossy(&buf));
            out.push(' ');
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Run every check against a finished document.
///
/// `approved` are the normalized terms the user actually redacted - finding one
/// of these is a defect. `declined` are terms that were offered and rejected;
/// finding one is expected and merely reported.
pub fn verify(
    pdf: &[u8],
    approved: &[String],
    declined: &[String],
    pages: usize,
    redactions: usize,
) -> Report {
    let mut findings = Vec::new();

    // 1. Structural. We know what we wrote; anything else is a defect.
    let hay = String::from_utf8_lossy(pdf);
    for key in FORBIDDEN {
        if hay.contains(key) {
            findings.push(Finding::Structure((*key).to_string()));
        }
    }

    // 2. Exactly one revision. Multiple %%EOF markers mean an earlier,
    //    unredacted version of a page is still recoverable from the file.
    let revs = hay.matches("%%EOF").count();
    if revs != 1 {
        findings.push(Finding::MultipleRevisions(revs));
    }

    // 3. Textual. Check the text layer the way a reader would, and the raw
    //    bytes the way an attacker would.
    let literals = normalize_term(&extract_literals(pdf));
    let raw = normalize_term(&hay);

    // Normalize the incoming terms too. Callers legitimately pass whatever the
    // user saw on screen ("Jane Doe"), while both haystacks are folded to
    // lowercase - so comparing them raw silently matches nothing and every
    // check passes. Folding here makes the function correct for any caller
    // rather than depending on one to pre-normalize.
    let fold = |terms: &[String]| -> Vec<(String, String)> {
        terms
            .iter()
            .map(|t| (t.clone(), normalize_term(t)))
            .filter(|(_, n)| n.chars().count() >= 3)
            .collect()
    };
    let approved_n = fold(approved);
    let declined_n = fold(declined);

    for (shown, term) in &approved_n {
        if literals.contains(term.as_str()) {
            findings.push(Finding::Leak { term: shown.clone(), where_: "text layer" });
        } else if raw.contains(term.as_str()) {
            findings.push(Finding::Leak { term: shown.clone(), where_: "raw bytes" });
        }
    }

    for (shown, term) in &declined_n {
        if approved_n.iter().any(|(_, a)| a == term) {
            continue;
        }
        let count = literals.matches(term.as_str()).count();
        if count > 0 {
            findings.push(Finding::Residual { term: shown.clone(), count });
        }
    }

    Report { findings, pages, redactions, bytes: pdf.len() }
}

/// Sanity check for the caller: were there pages with no text layer at all?
/// Those are the ones automatic detection could not examine, and the user needs
/// to be told which they are rather than left to assume they were covered.
pub fn pages_without_text(per_page: &[Vec<TextItem>]) -> Vec<usize> {
    per_page
        .iter()
        .enumerate()
        .filter(|(_, items)| items.iter().all(|i| i.text.trim().is_empty()))
        .map(|(i, _)| i + 1)
        .collect()
}
