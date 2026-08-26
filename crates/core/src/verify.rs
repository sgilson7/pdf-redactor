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
    /// The term occurs only *inside* longer words, e.g. "Docker" within
    /// "Dockerfile". Matching deliberately requires word boundaries, so these
    /// were never redacted. Reported rather than ignored: whether a substring
    /// hit matters is a judgement only the reader can make.
    PartialWord { term: String, count: usize },
    /// More than one revision, i.e. incremental-update history is present.
    MultipleRevisions(usize),
}

impl Finding {
    /// Blocking findings stop the download; advisory ones are reported.
    pub fn is_blocking(&self) -> bool {
        !matches!(self, Finding::Residual { .. } | Finding::PartialWord { .. })
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

/// Count occurrences of `needle` in `hay` that sit on word boundaries, plus
/// those that occur only inside a longer word.
///
/// Verification has to apply exactly the rule the matcher applied. The matcher
/// requires boundaries, so "Docker" never redacts the "Docker" inside
/// "Dockerfile" - and a verifier using plain substring containment then reports
/// a leak in a document that was redacted correctly, blocking a download it
/// should have allowed.
fn count_occurrences(hay: &[char], needle: &[char]) -> (usize, usize) {
    let (mut whole, mut partial) = (0, 0);
    if needle.is_empty() || needle.len() > hay.len() {
        return (0, 0);
    }
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if hay[i..i + needle.len()] == *needle {
            let end = i + needle.len();
            let before = i == 0 || !hay[i - 1].is_alphanumeric();
            let after = end >= hay.len() || !hay[end].is_alphanumeric();
            if before && after {
                whole += 1;
            } else {
                partial += 1;
            }
            i = end;
            continue;
        }
        i += 1;
    }
    (whole, partial)
}

/// Find `needle` in `hay`.
fn find_at(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    find_at(hay, needle, 0).is_some()
}

/// The file split into the parts worth inspecting.
pub struct Views {
    /// Every byte outside a stream payload: the object dictionaries, the xref
    /// table, the trailer. Structural checks belong here and nowhere else.
    pub dicts: Vec<u8>,
    /// The payloads of non-image streams, i.e. our content streams.
    pub content: Vec<u8>,
}

/// Split a document into dictionaries and content, discarding image payloads.
///
/// Scanning the whole file instead is both wrong and ruinous. A JPEG is
/// effectively random bytes, so a short token like `/JS` turns up in it by
/// chance roughly every 16 MB - reporting a leak in a file that has none. And
/// normalizing tens of megabytes of image data costs gigabytes of allocation,
/// which on a large document simply aborts the module. Image payloads cannot
/// contain recoverable text anyway: that is the point of rasterizing.
pub fn views(pdf: &[u8]) -> Views {
    let mut dicts = Vec::with_capacity(pdf.len() / 16);
    let mut content = Vec::new();
    let mut i = 0;
    let mut copied = 0;

    while let Some(kw) = find_at(pdf, b"stream", i) {
        // "endstream" contains "stream", so a naive scan re-enters on a
        // stream's own terminator and swallows the rest of the file - trailer
        // and all.
        if kw >= 3 && &pdf[kw - 3..kw] == b"end" {
            i = kw + b"stream".len();
            continue;
        }

        // Decide from the dictionary immediately before the keyword.
        let back = kw.saturating_sub(400);
        let dict = &pdf[back..kw];
        let is_image = contains(dict, b"/DCTDecode");

        // Payload starts after the keyword and its end-of-line marker.
        let mut start = kw + b"stream".len();
        if pdf.get(start) == Some(&b'\r') {
            start += 1;
        }
        if pdf.get(start) == Some(&b'\n') {
            start += 1;
        }

        // Prefer the declared /Length; fall back to searching for the keyword.
        let end = declared_length(dict)
            .filter(|n| start + n <= pdf.len())
            .map(|n| start + n)
            .or_else(|| find_at(pdf, b"endstream", start))
            .unwrap_or(pdf.len());

        dicts.extend_from_slice(&pdf[copied..start]);
        if !is_image {
            content.extend_from_slice(&pdf[start..end]);
        }
        copied = end;
        i = end.max(kw + 1);
    }
    dicts.extend_from_slice(&pdf[copied..]);

    Views { dicts, content }
}

/// Parse `/Length N` out of a stream dictionary.
fn declared_length(dict: &[u8]) -> Option<usize> {
    let at = find_at(dict, b"/Length", 0)?;
    let mut j = at + b"/Length".len();
    while dict.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
        j += 1;
    }
    let s = j;
    while dict.get(j).is_some_and(|c| c.is_ascii_digit()) {
        j += 1;
    }
    if j == s {
        return None;
    }
    std::str::from_utf8(&dict[s..j]).ok()?.parse().ok()
}

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
    let v = views(pdf);

    // 1. Structural. We know what we wrote; anything else is a defect.
    //    Checked against dictionaries only - see `views`.
    for key in FORBIDDEN {
        if contains(&v.dicts, key.as_bytes()) {
            findings.push(Finding::Structure((*key).to_string()));
        }
    }

    // 2. Exactly one revision. Multiple %%EOF markers mean an earlier,
    //    unredacted version of a page is still recoverable from the file.
    let mut revs = 0;
    let mut at = 0;
    while let Some(p) = find_at(&v.dicts, b"%%EOF", at) {
        revs += 1;
        at = p + 5;
    }
    if revs != 1 {
        findings.push(Finding::MultipleRevisions(revs));
    }

    // 3. Textual. Check the text layer the way a reader would, and the
    //    dictionaries the way an attacker would.
    let literals = normalize_term(&extract_literals(&v.content));
    let raw = normalize_term(&String::from_utf8_lossy(&v.dicts));

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

    // Compare over chars once rather than per term.
    let lit: Vec<char> = literals.chars().collect();
    let raw_c: Vec<char> = raw.chars().collect();

    let mut partials: Vec<(String, usize)> = Vec::new();
    for (shown, term) in &approved_n {
        let n: Vec<char> = term.chars().collect();
        let (whole_lit, part_lit) = count_occurrences(&lit, &n);
        if whole_lit > 0 {
            findings.push(Finding::Leak { term: shown.clone(), where_: "text layer" });
        } else if count_occurrences(&raw_c, &n).0 > 0 {
            findings.push(Finding::Leak { term: shown.clone(), where_: "raw bytes" });
        }
        if part_lit > 0 {
            partials.push((shown.clone(), part_lit));
        }
    }

    for (shown, term) in &declined_n {
        if approved_n.iter().any(|(_, a)| a == term) {
            continue;
        }
        let n: Vec<char> = term.chars().collect();
        let (whole, _) = count_occurrences(&lit, &n);
        if whole > 0 {
            findings.push(Finding::Residual { term: shown.clone(), count: whole });
        }
    }

    // Collapse per-variant partials: several approved forms of one name
    // ("Docker", "docker") otherwise each report the same longer words.
    partials.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    partials.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
    for (term, count) in partials {
        findings.push(Finding::PartialWord { term, count });
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
