//! Character advance widths, used to locate glyphs inside a text fragment.
//!
//! PDF text extraction reports one advance width for a whole run, not per
//! character, so finding where "Doe" starts inside a fragment means estimating.
//! Dividing the run width evenly is the obvious approach and it is wrong: real
//! text fonts are proportional, so a prefix full of narrow characters
//! ("Unity ID: jdoe2  Email: ") gets over-measured and the redaction box lands
//! to the right of the text it is supposed to cover — leaving the first
//! character legible, which looks redacted but is not.
//!
//! Using Helvetica's ratios instead is still an estimate when the document uses
//! a different face, but the *relative* proportions of characters are similar
//! across text fonts, which removes almost all of the drift.

/// Helvetica advance widths for printable ASCII, in units of 1/1000 em.
const W: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722,
    722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722,
    667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556,
    556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500,
    500, 334, 260, 334, 584,
];

/// Relative advance of one character, in ems.
pub fn advance(c: char) -> f32 {
    let i = c as usize;
    if (32..127).contains(&i) {
        W[i - 32] as f32 / 1000.0
    } else {
        // CJK and other wide scripts are far closer to one em than to Helvetica's
        // average Latin advance.
        if i > 0x2E80 { 1.0 } else { 0.556 }
    }
}

/// Total relative advance of a string.
pub fn width(s: &str) -> f32 {
    s.chars().map(advance).sum()
}

/// Where character `idx` begins within `chars`, as a fraction of the whole
/// string's advance. Returns 0.0..=1.0.
pub fn fraction_at(chars: &[char], idx: usize) -> f32 {
    let total: f32 = chars.iter().copied().map(advance).sum();
    if total <= 0.0 {
        return 0.0;
    }
    let upto: f32 = chars[..idx.min(chars.len())].iter().copied().map(advance).sum();
    upto / total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_characters_measure_narrower() {
        assert!(advance('i') < advance('m'));
        assert!(advance(' ') < advance('W'));
    }

    #[test]
    fn a_narrow_prefix_is_not_over_measured() {
        // The exact case that left "Email: j" visible: a prefix loaded with
        // spaces and thin glyphs. Even division would put the boundary at
        // 26/40 = 0.65; the true position is meaningfully left of that.
        let s: Vec<char> = "Unity ID: jdoe2    Email: jdoe2@ncsu.edu".chars().collect();
        let f = fraction_at(&s, 26);
        let uniform = 26.0 / 40.0;
        assert!(f < uniform - 0.02, "expected {} well below {}", f, uniform);
    }

    #[test]
    fn boundaries_are_sane() {
        let s: Vec<char> = "hello".chars().collect();
        assert_eq!(fraction_at(&s, 0), 0.0);
        assert!((fraction_at(&s, 5) - 1.0).abs() < 1e-6);
    }
}
