//! Turning approved redaction boxes into a safe invisible text layer.
//!
//! The searchable layer is only trustworthy if the redacted text is never
//! written in the first place. Covering it, deleting it afterwards, or trusting
//! a post-hoc scrub all leave room for a mistake to survive. So filtering
//! happens here, before `pdfwrite` ever sees a span.

use crate::matching::{Rect, TextItem};
use crate::pdfwrite::TextSpan;

/// Does the glyph cell `[x0,x1)` of `item` touch any redaction box?
fn covered(x0: f32, x1: f32, item: &TextItem, boxes: &[Rect]) -> bool {
    boxes.iter().any(|b| {
        x0 < b.x + b.w && b.x < x1 && item.y < b.y + b.h && b.y < item.y + item.h
    })
}

/// Build the output text layer: every fragment, minus every character whose
/// cell intersects a redaction box.
///
/// Filtering is per-character rather than per-fragment so that redacting a name
/// inside a paragraph does not cost the whole paragraph its searchability. The
/// geometry is the same proportional interpolation used to place the boxes, and
/// the boxes were already padded outward, so this errs toward dropping too much.
///
/// `page_h` is the page height in points. Input coordinates are viewport space
/// (origin top-left, y down); output is PDF user space (origin bottom-left,
/// y up), which is the one conversion the writer relies on being correct.
pub fn filter_spans(items: &[TextItem], boxes: &[Rect], page_h: f32) -> Vec<TextSpan> {
    let mut out = Vec::new();

    for it in items {
        let n = it.text.chars().count();
        if n == 0 || it.w <= 0.0 {
            continue;
        }
        let cw = it.w / n as f32;
        let chars: Vec<char> = it.text.chars().collect();

        // Walk the fragment, accumulating runs of surviving characters.
        let mut run: Option<(usize, usize)> = None;
        let flush = |run: &mut Option<(usize, usize)>, out: &mut Vec<TextSpan>| {
            if let Some((s, e)) = run.take() {
                let text: String = chars[s..e].iter().collect();
                if text.trim().is_empty() {
                    return;
                }
                let x = it.x + cw * s as f32;
                let w = cw * (e - s) as f32;
                // `y` is the top of the line box and `h` the font height, so the
                // baseline sits exactly at `y + h` in viewport space (the JS
                // side derives both from the same pdf.js transform). Flip that
                // into PDF user space, where y grows upward from the bottom.
                let baseline_from_top = it.y + it.h;
                out.push(TextSpan {
                    text,
                    x,
                    y: page_h - baseline_from_top,
                    size: it.h,
                    width: w,
                });
            }
        };

        for i in 0..n {
            let x0 = it.x + cw * i as f32;
            if covered(x0, x0 + cw, it, boxes) {
                flush(&mut run, &mut out);
            } else {
                run = Some(match run {
                    Some((s, _)) => (s, i + 1),
                    None => (i, i + 1),
                });
            }
        }
        flush(&mut run, &mut out);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str, x: f32) -> TextItem {
        TextItem {
            text: text.into(),
            x,
            y: 100.0,
            w: text.chars().count() as f32 * 6.0,
            h: 12.0,
            eol: true,
        }
    }

    #[test]
    fn keeps_everything_when_nothing_is_redacted() {
        let items = vec![item("Name: Jane Doe", 50.0)];
        let s = filter_spans(&items, &[], 792.0);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].text, "Name: Jane Doe");
    }

    #[test]
    fn drops_only_the_covered_characters() {
        let items = vec![item("Name: Jane Doe", 50.0)];
        // "Jane Doe" starts at char 6 -> x = 50 + 36 = 86, runs to x = 134.
        let boxes = vec![Rect { x: 86.0, y: 98.0, w: 48.0, h: 16.0 }];
        let s = filter_spans(&items, &boxes, 792.0);
        let joined: String = s.iter().map(|x| x.text.clone()).collect::<Vec<_>>().join("|");
        assert!(joined.contains("Name:"), "surrounding text lost: {:?}", joined);
        assert!(!joined.to_lowercase().contains("jane"), "leaked: {:?}", joined);
        assert!(!joined.to_lowercase().contains("doe"), "leaked: {:?}", joined);
    }

    #[test]
    fn a_box_covering_everything_leaves_nothing() {
        let items = vec![item("Jane Doe", 50.0)];
        let boxes = vec![Rect { x: 0.0, y: 0.0, w: 600.0, h: 700.0 }];
        assert!(filter_spans(&items, &boxes, 792.0).is_empty());
    }

    #[test]
    fn converts_to_pdf_user_space() {
        let items = vec![item("Hi", 50.0)];
        let s = filter_spans(&items, &[], 792.0);
        // Viewport y=100 (from top) must become a user-space y near the top of
        // an 792pt page, not near the bottom.
        assert!(s[0].y > 600.0, "y not flipped: {}", s[0].y);
    }

    #[test]
    fn a_box_on_another_line_does_not_affect_this_one() {
        let items = vec![item("Jane Doe", 50.0)];
        let boxes = vec![Rect { x: 50.0, y: 300.0, w: 100.0, h: 16.0 }];
        assert_eq!(filter_spans(&items, &boxes, 792.0).len(), 1);
    }
}
