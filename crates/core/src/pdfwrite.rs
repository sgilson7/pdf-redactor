//! Minimal PDF writer.
//!
//! This module exists instead of a PDF library on purpose. The security claim
//! this tool makes is "the output file contains nothing but the bytes we chose
//! to write" — no `/Info`, no XMP, no `/ID`, no timestamps, no producer string,
//! no prior revisions. A general-purpose library gives you no way to *prove*
//! that, because any version bump can start emitting a new key. Here the entire
//! output is visible in one screenful of `write_*` calls.
//!
//! The subset emitted is deliberately tiny:
//!   catalog -> page tree -> N pages
//!   each page: one DCTDecode (JPEG) image XObject drawn full-bleed,
//!              plus an optional invisible-text content stream for search.

/// A page of the output document.
pub struct Page {
    /// JPEG bytes, embedded verbatim as a DCTDecode stream. Already has the
    /// redaction boxes burned into its pixels by the caller.
    pub jpeg: Vec<u8>,
    /// Pixel dimensions of `jpeg`.
    pub px_w: u32,
    pub px_h: u32,
    /// Page size in PDF points. The image is scaled to exactly fill this.
    pub pt_w: f32,
    pub pt_h: f32,
    /// Invisible text spans, already filtered so that nothing overlapping a
    /// redaction survives. Empty when the caller opted out of a text layer.
    pub spans: Vec<TextSpan>,
}

/// One run of invisible (render mode 3) text, positioned in PDF user space.
pub struct TextSpan {
    pub text: String,
    /// Baseline origin, PDF user space: origin bottom-left, y up, points.
    pub x: f32,
    pub y: f32,
    /// Font size in points.
    pub size: f32,
    /// Target advance width in points. Used to set horizontal scaling so the
    /// invisible glyphs track the visible ones and selection rectangles land
    /// roughly where the reader expects.
    pub width: f32,
}

/// Objects are written sequentially; `offsets[i]` is the byte offset of object
/// number `i + 1`, which is what the xref table needs.
struct Writer {
    buf: Vec<u8>,
    offsets: Vec<usize>,
}

impl Writer {
    fn new() -> Self {
        // PDF 1.7. The binary comment line on the second row tells transfer
        // agents this is not a text file and stops them mangling line endings.
        let mut buf = Vec::new();
        buf.extend_from_slice(b"%PDF-1.7\n");
        buf.extend_from_slice(&[b'%', 0xE2, 0xE3, 0xCF, 0xD3, b'\n']);
        Writer { buf, offsets: Vec::new() }
    }

    /// Reserve the next object number without writing it yet, so objects can
    /// reference each other before they exist.
    fn reserve(&mut self) -> u32 {
        self.offsets.push(0);
        self.offsets.len() as u32
    }

    fn begin(&mut self, id: u32) {
        self.offsets[(id - 1) as usize] = self.buf.len();
        self.buf.extend_from_slice(format!("{} 0 obj\n", id).as_bytes());
    }

    fn end(&mut self) {
        self.buf.extend_from_slice(b"\nendobj\n");
    }

    fn put(&mut self, s: &str) {
        self.buf.extend_from_slice(s.as_bytes());
    }

    /// A stream object. `dict_body` is the dictionary contents *without* the
    /// enclosing `<<`/`>>` and without `/Length`, which is supplied here.
    fn stream(&mut self, id: u32, dict_body: &str, data: &[u8]) {
        self.begin(id);
        self.put(&format!("<<{}/Length {}>>\nstream\n", dict_body, data.len()));
        self.buf.extend_from_slice(data);
        self.buf.extend_from_slice(b"\nendstream");
        self.end();
    }
}

/// Encode a string as a PDF literal for a WinAnsiEncoding font.
///
/// A PDF string literal is a *byte* string, so UTF-8 cannot be dropped in
/// verbatim - it would decode as mojibake. Since the invisible layer exists
/// only to make the output searchable, characters outside the encodable range
/// are folded rather than embedded: NFKD strips the accent from "é" leaving a
/// searchable "e", and anything still unrepresentable becomes a space. This is
/// why no font is embedded, which in turn is why no font program from the
/// source document can ride along into the output.
fn escape(s: &str) -> Vec<u8> {
    use unicode_normalization::UnicodeNormalization;
    let mut out = Vec::with_capacity(s.len() + 8);
    for c in s.nfkd() {
        // Drop combining marks left over from decomposition.
        if matches!(c, '\u{0300}'..='\u{036F}') {
            continue;
        }
        match c {
            '(' => out.extend_from_slice(b"\\("),
            ')' => out.extend_from_slice(b"\\)"),
            '\\' => out.extend_from_slice(b"\\\\"),
            // Printable ASCII is identical in WinAnsi.
            c if (' '..='~').contains(&c) => out.push(c as u8),
            _ => out.push(b' '),
        }
    }
    out
}

/// Build the complete output PDF.
///
/// The trailer carries `/Size` and `/Root` and nothing else. There is
/// deliberately no `/Info` and no `/ID`: an `/ID` is derived from file content
/// and timestamps in most producers, and is a fingerprinting vector we have no
/// use for.
pub fn build(pages: &[Page]) -> Vec<u8> {
    let mut w = Writer::new();

    let catalog_id = w.reserve();
    let pages_id = w.reserve();
    // One shared Helvetica. Base-14, so nothing is embedded and no font program
    // from the source document can ride along into the output.
    let font_id = w.reserve();

    let mut page_ids = Vec::with_capacity(pages.len());
    for _ in pages {
        page_ids.push((w.reserve(), w.reserve(), w.reserve())); // page, image, content
    }

    w.begin(catalog_id);
    w.put(&format!("<</Type/Catalog/Pages {} 0 R>>", pages_id));
    w.end();

    w.begin(pages_id);
    let kids: Vec<String> = page_ids.iter().map(|(p, _, _)| format!("{} 0 R", p)).collect();
    w.put(&format!(
        "<</Type/Pages/Count {}/Kids[{}]>>",
        pages.len(),
        kids.join(" ")
    ));
    w.end();

    w.begin(font_id);
    w.put("<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>");
    w.end();

    for (page, &(pid, img_id, content_id)) in pages.iter().zip(page_ids.iter()) {
        // Page object. No /Annots, no /Rotate (rotation is baked into the
        // raster), no /Thumb, no /PieceInfo, no /Group.
        w.begin(pid);
        w.put(&format!(
            "<</Type/Page/Parent {} 0 R/MediaBox[0 0 {:.2} {:.2}]\
             /Resources<</XObject<</Im0 {} 0 R>>/Font<</F1 {} 0 R>>>>\
             /Contents {} 0 R>>",
            pages_id, page.pt_w, page.pt_h, img_id, font_id, content_id
        ));
        w.end();

        w.stream(
            img_id,
            &format!(
                "/Type/XObject/Subtype/Image/Width {}/Height {}\
                 /ColorSpace/DeviceRGB/BitsPerComponent 8/Filter/DCTDecode",
                page.px_w, page.px_h
            ),
            &page.jpeg,
        );

        // Content stream: draw the image full-bleed, then lay invisible text.
        //
        // Deliberately left uncompressed. It costs a few KB against a JPEG
        // measured in hundreds, and it buys something worth more: the output is
        // auditable with `strings out.pdf | grep -i <name>`. A reviewer should
        // not have to take the tool's word for it.
        let mut c: Vec<u8> = Vec::new();
        c.extend_from_slice(
            format!("q\n{:.2} 0 0 {:.2} 0 0 cm\n/Im0 Do\nQ\n", page.pt_w, page.pt_h).as_bytes(),
        );
        if !page.spans.is_empty() {
            // 3 Tr is the invisible render mode: the glyphs contribute to text
            // extraction and selection but paint nothing.
            c.extend_from_slice(b"BT\n3 Tr\n");
            for s in &page.spans {
                let natural = helvetica_width(&s.text) * s.size;
                let scale = if natural > 0.0 {
                    (s.width / natural * 100.0).clamp(1.0, 1000.0)
                } else {
                    100.0
                };
                c.extend_from_slice(
                    format!(
                        "/F1 {:.2} Tf\n{:.2} Tz\n1 0 0 1 {:.2} {:.2} Tm\n(",
                        s.size, scale, s.x, s.y
                    )
                    .as_bytes(),
                );
                c.extend_from_slice(&escape(&s.text));
                c.extend_from_slice(b") Tj\n");
            }
            c.extend_from_slice(b"ET\n");
        }
        w.stream(content_id, "", &c);
    }

    // Cross-reference table.
    let xref_pos = w.buf.len();
    let count = w.offsets.len() + 1;
    w.put(&format!("xref\n0 {}\n", count));
    w.put("0000000000 65535 f \n");
    for i in 0..w.offsets.len() {
        let off = w.offsets[i];
        w.put(&format!("{:010} 00000 n \n", off));
    }
    w.put(&format!(
        "trailer\n<</Size {}/Root {} 0 R>>\nstartxref\n{}\n%%EOF\n",
        count, catalog_id, xref_pos
    ));

    w.buf
}

/// Helvetica advance widths, in units of 1/1000 em, for the WinAnsi range we
/// can actually encode. Used only to scale invisible text so selection
/// rectangles roughly match the visible glyphs underneath.
fn helvetica_width(s: &str) -> f32 {
    const W: [u16; 95] = [
        278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556,
        556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722,
        722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722,
        667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556,
        556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500,
        500, 334, 260, 334, 584,
    ];
    s.chars()
        .map(|c| {
            let i = c as usize;
            if (32..127).contains(&i) { W[i - 32] as f32 / 1000.0 } else { 0.556 }
        })
        .sum()
}
