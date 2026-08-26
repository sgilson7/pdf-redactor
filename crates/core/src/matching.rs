//! Finding name variants in a page's extracted text and turning hits into boxes.
//!
//! The central difficulty is that PDF text arrives in arbitrary fragments. A
//! line reading "Name: Jane Doe" can come back as ["Name:", "Ja", "ne D", "oe"]
//! because the producer split runs at kerning pairs. Matching per-fragment
//! therefore finds nothing. Everything here operates on one joined, normalized
//! string per page, with index maps that lead back to the individual fragments
//! so boxes land on the right glyphs.

use crate::normalize::normalize;
use crate::variants::{Kind, Tier, Variant};

/// One text fragment as reported by the PDF text layer.
///
/// Coordinates are pdf.js viewport-at-scale-1 space: origin top-left, y down,
/// units of PDF points. Storing boxes in this one space keeps them independent
/// of preview zoom and export DPI, and it already accounts for page rotation.
///
/// `y` is the top of the line box and `h` the font height, so the text baseline
/// is at `y + h`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct TextItem {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// The producer marked a line break after this fragment.
    pub eol: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Match {
    pub tier: Tier,
    pub label: &'static str,
    /// The text as it appears in the document, for the review list.
    pub matched: String,
    /// Surrounding text, so the user can tell a name from a coincidence.
    pub context: String,
    /// One box per fragment the match spans. A match crossing a line break
    /// legitimately produces two boxes.
    pub boxes: Vec<Rect>,
    /// Half-open char range in the joined source text. Used to suppress
    /// lower-tier matches nested inside higher-tier ones.
    pub start: usize,
    pub end: usize,
}

/// Anti-aliasing bleeds past the reported glyph box. Under-padding leaves a
/// legible sliver of the name at the edge of the black rectangle, which is a
/// silent failure - it looks redacted.
const PAD: f32 = 2.0;

/// Join fragments into one string, deciding separators from geometry.
///
/// Returns the joined text plus, for each char, which fragment produced it and
/// the char offset within that fragment.
fn join(items: &[TextItem]) -> (String, Vec<(usize, usize)>) {
    let mut text = String::new();
    let mut map: Vec<(usize, usize)> = Vec::new();
    let mut prev: Option<&TextItem> = None;

    for (i, it) in items.iter().enumerate() {
        if let Some(p) = prev {
            if p.eol {
                text.push('\n');
                map.push((i, 0));
            } else {
                // A fragment split mid-word ("Ja" + "ne") sits flush against the
                // previous one; a real word gap leaves horizontal space. Using
                // the gap rather than always inserting a separator is what lets
                // "Jane Doe" survive being cut into four pieces.
                let gap = it.x - (p.x + p.w);
                if gap > p.h * 0.2 {
                    text.push(' ');
                    map.push((i, 0));
                }
            }
        }
        for (ci, ch) in it.text.chars().enumerate() {
            text.push(ch);
            map.push((i, ci));
        }
        prev = Some(it);
    }
    (text, map)
}

/// Levenshtein distance, abandoned once it provably exceeds `cap`.
fn edit_distance(a: &[char], b: &[char], cap: usize) -> usize {
    if a.len().abs_diff(b.len()) > cap {
        return cap + 1;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            row_min = row_min.min(cur[j]);
        }
        if row_min > cap {
            return cap + 1;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// Find every variant occurrence on one page.
pub fn find(items: &[TextItem], vars: &[Variant]) -> Vec<Match> {
    let (joined, item_map) = join(items);
    let norm = normalize(&joined);
    let nchars: Vec<char> = norm.text.chars().collect();
    let jchars: Vec<char> = joined.chars().collect();

    let mut raw: Vec<Match> = Vec::new();

    for v in vars {
        let term: Vec<char> = v.term.chars().collect();
        if term.is_empty() || term.len() > nchars.len() {
            continue;
        }

        // Exact scan over the normalized text.
        let mut i = 0;
        while i + term.len() <= nchars.len() {
            if nchars[i..i + term.len()] == term[..] {
                let mut end = i + term.len();
                // A Unity ID variant "jdoe" should also claim the "2" in "jdoe2".
                if v.kind == Kind::Id {
                    while end < nchars.len() && nchars[end].is_ascii_digit() {
                        end += 1;
                    }
                }
                let before_ok = i == 0 || !is_word_char(nchars[i - 1]);
                let after_ok = end >= nchars.len() || !is_word_char(nchars[end]);
                if before_ok && after_ok {
                    if let Some(m) = build(&norm.map, &item_map, items, &jchars, i, end, v, v.tier) {
                        raw.push(m);
                    }
                }
                i = end.max(i + 1);
            } else {
                i += 1;
            }
        }

        // Typo tolerance, single-token name variants only. The gate is the
        // variant's *shape*, not its tier - "johnson" is a Medium variant, but
        // a near-miss like "Johnsen" is speculative regardless of what the
        // exact form would have scored, so the hit is always emitted at Low.
        if v.kind == Kind::Name && !v.term.contains(' ') && term.len() >= 5 {
            let cap = if term.len() >= 8 { 2 } else { 1 };
            for (s, e) in tokens(&nchars) {
                if e - s == term.len() && nchars[s..e] == term[..] {
                    continue; // exact, already recorded
                }
                if edit_distance(&nchars[s..e], &term, cap) <= cap {
                    if let Some(m) = build(&norm.map, &item_map, items, &jchars, s, e, v, Tier::Low) {
                        raw.push(m);
                    }
                }
            }
        }
    }

    suppress_nested(raw)
}

/// Word-ish spans of the normalized text, for token-level fuzzy matching.
fn tokens(chars: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, &c) in chars.iter().enumerate() {
        match (is_word_char(c), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, chars.len()));
    }
    out
}

/// Turn a normalized-space char range into a Match with boxes in page space.
fn build(
    nmap: &[usize],
    item_map: &[(usize, usize)],
    items: &[TextItem],
    jchars: &[char],
    ns: usize,
    ne: usize,
    v: &Variant,
    tier: Tier,
) -> Option<Match> {
    if ns >= nmap.len() || ne == 0 {
        return None;
    }
    // Normalized indices -> joined-string indices.
    let js = nmap[ns];
    let je = if ne <= nmap.len() - 1 { nmap[ne - 1] + 1 } else { jchars.len() };
    if js >= je || je > jchars.len() {
        return None;
    }

    // Group the covered joined-chars by which fragment they came from.
    let mut spans: Vec<(usize, usize, usize)> = Vec::new(); // (item, first_ci, last_ci)
    for j in js..je {
        if j >= item_map.len() {
            break;
        }
        let (item, ci) = item_map[j];
        match spans.last_mut() {
            Some(last) if last.0 == item => last.2 = ci,
            _ => spans.push((item, ci, ci)),
        }
    }

    let mut boxes = Vec::new();
    for (idx, first, last) in spans {
        let it = &items[idx];
        let n = it.text.chars().count();
        if n == 0 || it.w <= 0.0 {
            continue;
        }
        // Interpolate across the fragment's advance width. Proportional fonts
        // make this approximate, which is what PAD absorbs.
        let cw = it.w / n as f32;
        let x0 = it.x + cw * first as f32;
        let x1 = it.x + cw * (last + 1) as f32;
        boxes.push(Rect {
            x: x0 - PAD,
            y: it.y - PAD,
            w: (x1 - x0) + PAD * 2.0,
            h: it.h + PAD * 2.0,
        });
    }
    if boxes.is_empty() {
        return None;
    }

    let matched: String = jchars[js..je].iter().collect();
    let cs = js.saturating_sub(30);
    let ce = (je + 30).min(jchars.len());
    let context: String = jchars[cs..ce].iter().collect::<String>().replace('\n', " ");

    Some(Match {
        tier,
        label: v.label,
        matched,
        context: context.trim().to_string(),
        boxes,
        start: js,
        end: je,
    })
}

/// Drop matches fully contained in a stronger match.
///
/// "Jane Doe" firing at High makes the "jane" and "doe" Medium hits over the
/// same text pure noise in the review list.
fn suppress_nested(mut v: Vec<Match>) -> Vec<Match> {
    v.sort_by(|a, b| {
        a.tier
            .cmp(&b.tier)
            .then((b.end - b.start).cmp(&(a.end - a.start)))
            .then(a.start.cmp(&b.start))
    });
    let mut kept: Vec<Match> = Vec::new();
    for m in v {
        let nested = kept
            .iter()
            .any(|k| k.start <= m.start && m.end <= k.end && k.tier <= m.tier);
        if !nested {
            kept.push(m);
        }
    }
    kept.sort_by(|a, b| a.start.cmp(&b.start).then(a.tier.cmp(&b.tier)));
    kept
}

/// Merge overlapping rectangles so the export paints fewer, cleaner boxes.
pub fn merge_boxes(mut rects: Vec<Rect>) -> Vec<Rect> {
    let mut merged = true;
    while merged {
        merged = false;
        'outer: for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                if rects[i].overlaps(&rects[j]) {
                    let a = rects[i];
                    let b = rects[j];
                    let x = a.x.min(b.x);
                    let y = a.y.min(b.y);
                    let r = (a.x + a.w).max(b.x + b.w);
                    let bo = (a.y + a.h).max(b.y + b.h);
                    rects[i] = Rect { x, y, w: r - x, h: bo - y };
                    rects.remove(j);
                    merged = true;
                    break 'outer;
                }
            }
        }
    }
    rects
}
