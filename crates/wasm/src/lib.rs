//! Thin bridge between the browser and `redactor-core`.
//!
//! Deliberately contains no logic worth testing: everything that decides what
//! gets redacted lives in `core`, where it runs under `cargo test` without a
//! browser. This file only moves bytes and JSON across the boundary.

use redactor_core::identifiers::find_identifiers;
use redactor_core::matching::{find, merge_boxes, Rect, TextItem};
use redactor_core::pdfwrite::{build, Page};
use redactor_core::redact::filter_spans;
use redactor_core::variants::{variants, Tier};
use redactor_core::verify::verify;
use wasm_bindgen::prelude::*;

/// Panics inside wasm otherwise surface as an opaque "unreachable executed".
#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|info| {
        web_sys_log(&format!("redactor panic: {}", info));
    }));
}

#[wasm_bindgen(inline_js = "export function web_sys_log(s) { console.error(s); }")]
extern "C" {
    fn web_sys_log(s: &str);
}

#[derive(serde::Serialize)]
struct VariantOut {
    term: String,
    label: String,
    tier: Tier,
}

/// The variant set for a name, so the UI can show what will be searched for.
#[wasm_bindgen]
pub fn list_variants(name: &str, extras_json: &str) -> Result<String, JsValue> {
    let extras: Vec<String> = serde_json::from_str(extras_json).unwrap_or_default();
    let out: Vec<VariantOut> = variants(name, &extras)
        .into_iter()
        .map(|v| VariantOut { term: v.term, label: v.label.to_string(), tier: v.tier })
        .collect();
    serde_json::to_string(&out).map_err(err)
}

/// Find every variant occurrence on one page's text items.
#[wasm_bindgen]
pub fn find_matches(items_json: &str, name: &str, extras_json: &str) -> Result<String, JsValue> {
    let items: Vec<TextItem> = serde_json::from_str(items_json).map_err(err)?;
    let extras: Vec<String> = serde_json::from_str(extras_json).unwrap_or_default();
    let vars = variants(name, &extras);
    serde_json::to_string(&find(&items, &vars)).map_err(err)
}

#[derive(serde::Serialize)]
struct IdOut {
    kind: String,
    text: String,
    boxes: Vec<Rect>,
}

/// Find identifiers by shape rather than by name: emails, ORCID iDs, phone
/// numbers. A username is only derivable from a name when the two happen to be
/// related, which is often not the case.
#[wasm_bindgen(js_name = findIdentifiers)]
pub fn find_identifiers_js(items_json: &str) -> Result<String, JsValue> {
    let items: Vec<TextItem> = serde_json::from_str(items_json).map_err(err)?;
    let out: Vec<IdOut> = find_identifiers(&items)
        .into_iter()
        .map(|c| IdOut { kind: c.kind.label().to_string(), text: c.text, boxes: c.boxes })
        .collect();
    serde_json::to_string(&out).map_err(err)
}

/// Accumulates pages so the caller can stream one at a time and keep peak
/// memory flat regardless of document length.
#[wasm_bindgen]
pub struct Builder {
    pages: Vec<Page>,
    redactions: usize,
    // Kept separately because `finish` drains `pages`.
    count: usize,
}

#[wasm_bindgen]
impl Builder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Builder {
        Builder { pages: Vec::new(), redactions: 0, count: 0 }
    }

    /// Add one finished page.
    ///
    /// `jpeg` already has the redaction boxes burned into its pixels - that
    /// happens on the canvas in JS, before encoding, which is what makes the
    /// covered content unrecoverable rather than merely hidden. The boxes are
    /// passed again here only so the text layer can be filtered to match.
    #[wasm_bindgen(js_name = addPage)]
    pub fn add_page(
        &mut self,
        jpeg: &[u8],
        px_w: u32,
        px_h: u32,
        pt_w: f32,
        pt_h: f32,
        items_json: &str,
        boxes_json: &str,
        want_text_layer: bool,
    ) -> Result<(), JsValue> {
        let boxes: Vec<Rect> = serde_json::from_str(boxes_json).map_err(err)?;
        let spans = if want_text_layer {
            let items: Vec<TextItem> = serde_json::from_str(items_json).map_err(err)?;
            filter_spans(&items, &boxes, pt_h)
        } else {
            Vec::new()
        };
        self.redactions += boxes.len();
        self.count += 1;
        self.pages.push(Page { jpeg: jpeg.to_vec(), px_w, px_h, pt_w, pt_h, spans });
        Ok(())
    }

    /// Emit the finished document.
    ///
    /// Takes the pages rather than borrowing them, so the accumulated JPEGs are
    /// released as soon as the output buffer exists. On a 125-page document
    /// that is the difference between holding one copy and two.
    #[wasm_bindgen(js_name = finish)]
    pub fn finish(&mut self) -> Vec<u8> {
        let pages = std::mem::take(&mut self.pages);
        let out = build(&pages);
        drop(pages);
        out
    }

    #[wasm_bindgen(getter, js_name = pageCount)]
    pub fn page_count(&self) -> usize {
        self.count
    }

    #[wasm_bindgen(getter, js_name = redactionCount)]
    pub fn redaction_count(&self) -> usize {
        self.redactions
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

/// Re-read a finished document and report what is actually in it.
#[wasm_bindgen(js_name = verifyOutput)]
pub fn verify_output(
    pdf: &[u8],
    approved_json: &str,
    declined_json: &str,
    pages: usize,
    redactions: usize,
) -> Result<String, JsValue> {
    let approved: Vec<String> = serde_json::from_str(approved_json).unwrap_or_default();
    let declined: Vec<String> = serde_json::from_str(declined_json).unwrap_or_default();
    let report = verify(pdf, &approved, &declined, pages, redactions);
    serde_json::to_string(&report).map_err(err)
}

/// Collapse overlapping rectangles before they are painted.
#[wasm_bindgen(js_name = mergeBoxes)]
pub fn merge_boxes_js(boxes_json: &str) -> Result<String, JsValue> {
    let boxes: Vec<Rect> = serde_json::from_str(boxes_json).map_err(err)?;
    serde_json::to_string(&merge_boxes(boxes)).map_err(err)
}

fn err<E: std::fmt::Display>(e: E) -> JsValue {
    JsValue::from_str(&e.to_string())
}
