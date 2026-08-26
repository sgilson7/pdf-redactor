//! A record of what was redacted, that is not itself a record of who.
//!
//! Doing the redaction is half the obligation; being able to show months later
//! that you did it, to someone who was not there, is the other half. That
//! evidence currently lives in a modal for a few seconds and is then gone.
//!
//! The difficulty is that the obvious audit log - "redacted 'Jane Doe' 9 times
//! in jane_doe_lab3.pdf" - is itself a FERPA record, and a worse one than the
//! document, because it concentrates identities into a single file that reads
//! like paperwork and gets emailed around.
//!
//! So the safety property here is structural, exactly as it is in `pdfwrite`:
//! not "we strip the sensitive fields" but "there is no parameter through which
//! they could arrive". Everything below is a count, a page number, a setting,
//! or a hash - and the hashes are validated to be hashes, so a caller cannot
//! smuggle a name through a field typed as one.

use crate::verify::{Finding, Report};
use std::collections::BTreeMap;

/// Where a redaction came from, for the record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Matched against the document's own text layer.
    Text,
    /// Matched against text recovered by OCR.
    Ocr,
    /// Drawn by hand.
    Manual,
}

/// What kind of thing a term was, without saying what it said.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TermKind {
    /// A name variant.
    Name,
    /// An identifier found by shape - email, ORCID, phone.
    Identifier,
}

/// One searched term, identified by position rather than content.
///
/// Deliberately not a salted hash of the name. A class roster is a population
/// of a few hundred, so any hash of a name drawn from it can be brute-forced
/// in microseconds - which would be pseudonymous while looking anonymous, the
/// worst of both. An ordinal means nothing outside its own entry.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TermStat {
    pub ordinal: usize,
    pub kind: TermKind,
    pub applied: usize,
    pub declined: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub dpi: u32,
    pub jpeg_quality: f32,
    pub text_layer: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrStats {
    pub engine: String,
    pub pages_scanned: Vec<usize>,
    pub mean_confidence: f32,
    pub words_below_threshold: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checks {
    pub structure: &'static str,
    pub text_layer: &'static str,
    pub revisions: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Advisories {
    /// Terms offered and declined that still appear. Counts only.
    pub residual_terms: usize,
    /// Terms that occur only inside longer words.
    pub partial_word_terms: usize,
    /// The audit-critical field: where the guarantee was weakest.
    pub pages_without_text_layer: Vec<usize>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Redactions {
    pub total: usize,
    /// Page number -> count. BTreeMap so the JSON is byte-stable and two
    /// manifests of the same corpus diff cleanly.
    pub by_page: BTreeMap<usize, usize>,
    pub by_source: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub input: String,
    pub output: String,
    pub processed_at: String,
    pub pages: usize,
    pub settings: Settings,
    pub redactions: Redactions,
    pub terms: Vec<TermStat>,
    pub checks: Checks,
    pub advisories: Advisories,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr: Option<OcrStats>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: &'static str,
    pub build: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema: &'static str,
    /// Stated in the file so a reader can see the intent, not infer it.
    pub contains_personal_data: bool,
    pub note: &'static str,
    pub how_to_identify: &'static str,
    pub tool: Tool,
    pub documents: Vec<Entry>,
}

const SCHEMA: &str = "pdf-redactor-manifest/1";
const NOTE: &str = "Documents are identified by SHA-256 of their bytes. No names, \
matched text, filenames, or document content appear in this file by design.";
const HOW: &str = "shasum -a 256 <your-originals>/*.pdf   # compare against \"input\"";

#[derive(Debug, PartialEq)]
pub enum BuildError {
    /// A field typed as a hash did not contain one.
    NotAHash(&'static str),
    /// A timestamp that is not a plain ISO-8601 instant.
    NotATimestamp,
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// An ISO-8601 instant and nothing else: digits, `-`, `:`, `T`, `.`, `Z`.
fn is_timestamp(s: &str) -> bool {
    (20..=30).contains(&s.len())
        && s.ends_with('Z')
        && s.chars().all(|c| c.is_ascii_digit() || matches!(c, '-' | ':' | 'T' | '.' | 'Z'))
}

/// Assemble one document's entry.
///
/// The hash and timestamp arguments are validated rather than trusted: they are
/// the only free-form strings in the whole structure, so they are the only way
/// document text could get in. Rejecting anything that is not shaped like a
/// hash closes that door.
#[allow(clippy::too_many_arguments)]
pub fn build_entry(
    input_sha256: &str,
    output_sha256: &str,
    processed_at: &str,
    pages: usize,
    settings: Settings,
    by_page: BTreeMap<usize, usize>,
    by_source: BTreeMap<Source, usize>,
    terms: Vec<TermStat>,
    pages_without_text: Vec<usize>,
    ocr: Option<OcrStats>,
    report: &Report,
) -> Result<Entry, BuildError> {
    if !is_sha256_hex(input_sha256) {
        return Err(BuildError::NotAHash("input"));
    }
    if !is_sha256_hex(output_sha256) {
        return Err(BuildError::NotAHash("output"));
    }
    if !is_timestamp(processed_at) {
        return Err(BuildError::NotATimestamp);
    }

    // Translate findings into counts, discarding the terms they name.
    let mut residual = 0;
    let mut partial = 0;
    let mut structure_ok = true;
    let mut text_ok = true;
    let mut revisions = 1;
    for f in &report.findings {
        match f {
            Finding::Residual { .. } => residual += 1,
            Finding::PartialWord { .. } => partial += 1,
            Finding::Structure(_) => structure_ok = false,
            Finding::Leak { .. } => text_ok = false,
            Finding::MultipleRevisions(n) => revisions = *n,
        }
    }

    let source_names = by_source
        .into_iter()
        .map(|(k, v)| {
            let name = match k {
                Source::Text => "text",
                Source::Ocr => "ocr",
                Source::Manual => "manual",
            };
            (name.to_string(), v)
        })
        .collect();

    Ok(Entry {
        input: format!("sha256:{}", input_sha256),
        output: format!("sha256:{}", output_sha256),
        processed_at: processed_at.to_string(),
        pages,
        settings,
        redactions: Redactions {
            total: report.redactions,
            by_page,
            by_source: source_names,
        },
        terms,
        checks: Checks {
            structure: if structure_ok { "pass" } else { "fail" },
            text_layer: if text_ok { "pass" } else { "fail" },
            revisions,
        },
        advisories: Advisories {
            residual_terms: residual,
            partial_word_terms: partial,
            pages_without_text_layer: pages_without_text,
        },
        ocr,
    })
}

/// Wrap entries into the downloadable document.
pub fn manifest(build: &str, documents: Vec<Entry>) -> Manifest {
    Manifest {
        schema: SCHEMA,
        contains_personal_data: false,
        note: NOTE,
        how_to_identify: HOW,
        tool: Tool { name: "pdf-redactor", build: build.to_string() },
        documents,
    }
}
