//! Finding identifiers that a name search cannot reach.
//!
//! Deriving usernames from a name only works when the two are related. A real
//! paper redacted for "Benyamin Tabarsi" left `btaghiz@ncsu.edu` untouched:
//! every plausible derivation gives `btabarsi` or `benyamin.tabarsi`, and the
//! actual address resembles neither. No amount of variant generation reaches
//! it, because the connection exists only in a directory somewhere.
//!
//! So these are found by *shape* instead. An email address, an ORCID iD, or a
//! phone number in a document being de-identified is worth a look whoever it
//! belongs to. They are offered for review, never applied automatically -
//! plenty of documents cite addresses that identify nobody in the study.

use crate::matching::{Rect, TextItem};

/// What kind of identifier a candidate looks like, for the review list.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum IdKind {
    Email,
    Orcid,
    Phone,
}

impl IdKind {
    pub fn label(self) -> &'static str {
        match self {
            IdKind::Email => "email address",
            IdKind::Orcid => "ORCID iD",
            IdKind::Phone => "phone number",
        }
    }
}

pub struct Candidate {
    pub kind: IdKind,
    pub text: String,
    pub boxes: Vec<Rect>,
    pub item: usize,
    /// Char range within the item, so the caller can build a tight box.
    pub start: usize,
    pub end: usize,
}

/// Match a phone number starting at `i`, returning its end.
///
/// Accepts `(919) 555-0142`, `919-555-0142`, `919 555 0142`, and a contiguous
/// international form like `+441234567890`.
fn phone_at(c: &[char], i: usize) -> Option<usize> {
    let mut j = i;
    let digits_at = |j: usize, n: usize| -> bool {
        j + n <= c.len() && c[j..j + n].iter().all(|d| d.is_ascii_digit())
    };

    // Optional country code.
    if c.get(j) == Some(&'+') {
        j += 1;
        let s = j;
        while j < c.len() && c[j].is_ascii_digit() {
            j += 1;
        }
        // A long contiguous run after '+' is already a complete number.
        if (10..=15).contains(&(j - s)) {
            return Some(j);
        }
        if j == s || j - s > 3 {
            return None;
        }
        if matches!(c.get(j), Some(' ') | Some('-')) {
            j += 1;
        }
    }

    // Area code, bracketed or bare.
    if c.get(j) == Some(&'(') {
        if !digits_at(j + 1, 3) || c.get(j + 4) != Some(&')') {
            return None;
        }
        j += 5;
    } else {
        if !digits_at(j, 3) {
            return None;
        }
        j += 3;
    }
    if matches!(c.get(j), Some(' ') | Some('-')) {
        j += 1;
    }

    // Exchange and line number.
    if !digits_at(j, 3) {
        return None;
    }
    j += 3;
    if matches!(c.get(j), Some(' ') | Some('-')) {
        j += 1;
    }
    if !digits_at(j, 4) {
        return None;
    }
    j += 4;

    // Must not run straight into more digits, which would make it something else.
    if c.get(j).is_some_and(|d| d.is_ascii_digit()) {
        return None;
    }
    Some(j)
}

fn is_email_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-')
}

/// Scan one fragment for identifier-shaped runs.
///
/// Deliberately per-fragment rather than over joined page text: an address is
/// never split across a line, and working on the fragment keeps the character
/// offsets that a box needs.
fn scan(text: &str) -> Vec<(IdKind, usize, usize)> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();

    // Email: a run of address characters either side of an '@', with a dot in
    // the domain.
    for i in 0..c.len() {
        if c[i] != '@' {
            continue;
        }
        let mut s = i;
        while s > 0 && is_email_char(c[s - 1]) {
            s -= 1;
        }
        let mut e = i + 1;
        while e < c.len() && is_email_char(c[e]) {
            e += 1;
        }
        // Trim trailing punctuation that belongs to the sentence, not the address.
        while e > i + 1 && matches!(c[e - 1], '.' | '-' | '_') {
            e -= 1;
        }
        let domain: String = c[i + 1..e].iter().collect();
        if s < i && domain.contains('.') && domain.len() >= 4 {
            out.push((IdKind::Email, s, e));
        }
    }

    // ORCID iD: 0000-0000-0000-000X, the last character may be 'X'.
    let s: String = c.iter().collect();
    let bytes: Vec<char> = s.chars().collect();
    for i in 0..bytes.len() {
        if i + 19 > bytes.len() {
            break;
        }
        let w = &bytes[i..i + 19];
        let shaped = w.iter().enumerate().all(|(j, &ch)| match j {
            4 | 9 | 14 => ch == '-',
            18 => ch.is_ascii_digit() || ch == 'X' || ch == 'x',
            _ => ch.is_ascii_digit(),
        });
        let free_before = i == 0 || !bytes[i - 1].is_alphanumeric();
        let free_after = i + 19 >= bytes.len() || !bytes[i + 19].is_alphanumeric();
        if shaped && free_before && free_after {
            out.push((IdKind::Orcid, i, i + 19));
        }
    }

    // Phone: a recognised *layout*, not merely enough digits.
    //
    // Counting digits looks reasonable and is unusable in practice: a DOI like
    // 10.1145/3510003.3510209 carries fourteen, and an academic paper is full
    // of them. Five false positives and no true ones is worse than not looking,
    // because a review list nobody trusts is a review list nobody reads. So
    // require grouping a phone number actually has, and refuse the '.'
    // separator entirely - DOIs use it constantly and dotted phone numbers are
    // rare enough to lose.
    for i in 0..c.len() {
        if i > 0 && (c[i - 1].is_alphanumeric() || matches!(c[i - 1], '.' | '/' | ':' | '-')) {
            continue;
        }
        if let Some(e) = phone_at(&c, i) {
            out.push((IdKind::Phone, i, e));
        }
    }

    out
}

/// Anti-aliasing bleeds past the reported glyph box, as elsewhere.
const PAD: f32 = 2.0;

/// Find every identifier-shaped run on a page.
pub fn find_identifiers(items: &[TextItem]) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (idx, it) in items.iter().enumerate() {
        let n = it.text.chars().count();
        if n == 0 || it.w <= 0.0 {
            continue;
        }
        let chars: Vec<char> = it.text.chars().collect();
        for (kind, s, e) in scan(&it.text) {
            let x0 = it.x + it.w * crate::metrics::fraction_at(&chars, s);
            let x1 = it.x + it.w * crate::metrics::fraction_at(&chars, e);
            out.push(Candidate {
                kind,
                text: chars[s..e].iter().collect(),
                boxes: vec![Rect {
                    x: x0 - PAD,
                    y: it.y - PAD,
                    w: (x1 - x0) + PAD * 2.0,
                    h: it.h + PAD * 2.0,
                }],
                item: idx,
                start: s,
                end: e,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(t: &str) -> Vec<(IdKind, String)> {
        scan(t).into_iter()
            .map(|(k, s, e)| (k, t.chars().skip(s).take(e - s).collect()))
            .collect()
    }

    #[test]
    fn finds_an_email_unrelated_to_any_name() {
        let k = kinds("Correspondence: btaghiz@ncsu.edu");
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].0, IdKind::Email);
        assert_eq!(k[0].1, "btaghiz@ncsu.edu");
    }

    #[test]
    fn trims_sentence_punctuation_from_an_address() {
        assert_eq!(kinds("write to a.b@x.edu.")[0].1, "a.b@x.edu");
    }

    #[test]
    fn ignores_an_at_sign_that_is_not_an_address() {
        assert!(kinds("priced @ 5 dollars").is_empty());
        assert!(kinds("see @mention here").is_empty());
    }

    #[test]
    fn finds_an_orcid() {
        let k = kinds("ORCID 0000-0002-1825-0097 listed");
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].0, IdKind::Orcid);
    }

    #[test]
    fn finds_an_orcid_ending_in_x() {
        assert_eq!(kinds("0000-0002-1694-233X")[0].0, IdKind::Orcid);
    }

    #[test]
    fn finds_a_phone_number() {
        let k = kinds("call (919) 555-0142 today");
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].0, IdKind::Phone);
    }

    #[test]
    fn a_doi_or_year_is_not_a_phone_number() {
        assert!(kinds("published in 2025").is_empty());
        assert!(kinds("pp 1607-1619").is_empty());
        // The exact false positive seen on a real paper.
        assert!(kinds("https://doi.org/10.1145/3510003.3510209").is_empty(),
            "a DOI was read as a phone number");
        assert!(kinds("arXiv:2311.09835").is_empty());
        assert!(kinds("ISBN 978-3-032-29770-9").is_empty());
    }

    #[test]
    fn accepts_the_usual_phone_layouts() {
        for t in ["919-555-0142", "(919) 555-0142", "919 555 0142", "+441234567890"] {
            assert_eq!(kinds(t).len(), 1, "missed {:?}", t);
            assert_eq!(kinds(t)[0].0, IdKind::Phone, "{:?}", t);
        }
    }

    #[test]
    fn boxes_land_on_the_address() {
        let items = vec![TextItem {
            text: "mail: a@b.edu".into(),
            x: 50.0, y: 100.0, w: 130.0, h: 10.0, eol: true, confidence: None,
        }];
        let c = find_identifiers(&items);
        assert_eq!(c.len(), 1);
        // The box must start after "mail: ", not at the fragment's left edge.
        assert!(c[0].boxes[0].x > 55.0, "box at {:?}", c[0].boxes[0]);
    }
}
