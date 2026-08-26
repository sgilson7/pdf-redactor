//! Text normalization with an index map back to the source.
//!
//! Matching has to happen on folded text ("JOSÉ" must match "jose") but the
//! redaction box has to be drawn over the *original* glyphs. So every
//! normalized character remembers which source character produced it.

use unicode_normalization::UnicodeNormalization;

pub struct Normalized {
    /// Folded text: lowercase, decomposed, diacritics and invisibles removed,
    /// whitespace collapsed to single spaces.
    pub text: String,
    /// `map[i]` is the source char index that produced `text`'s i-th char.
    /// Same length as `text.chars()`.
    pub map: Vec<usize>,
}

/// True for characters that carry no visible meaning and only serve to break
/// naive string search. Copy-pasted text from web pages is full of these.
fn is_invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}'              // soft hyphen
        | '\u{200B}'..='\u{200F}' // zero-width space/joiners, bidi marks
        | '\u{202A}'..='\u{202E}' // bidi overrides
        | '\u{2060}'..='\u{2064}' // word joiner, invisible operators
        | '\u{FEFF}'            // BOM / zero-width no-break space
    )
}

/// Combining marks left behind by NFKD decomposition, e.g. the acute in "é".
fn is_combining(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}' | '\u{1AB0}'..='\u{1AFF}' | '\u{20D0}'..='\u{20FF}')
}

/// Normalize `src`, producing folded text plus a map back to source indices.
pub fn normalize(src: &str) -> Normalized {
    let chars: Vec<char> = src.chars().collect();

    // Pass 1: drop invisibles and join words hyphenated across a line break.
    // A PDF that wraps "Johnson" as "Jo-\nhnson" must still match "johnson".
    let mut kept: Vec<(char, usize)> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if is_invisible(c) {
            i += 1;
            continue;
        }
        if c == '-' || c == '\u{2010}' || c == '\u{2011}' {
            // Look past the hyphen for a newline with only spaces in between.
            let mut j = i + 1;
            while j < chars.len() && (chars[j] == ' ' || chars[j] == '\t' || chars[j] == '\r') {
                j += 1;
            }
            if j < chars.len() && chars[j] == '\n' {
                // Drop hyphen and the break: the word continues on the next line.
                i = j + 1;
                continue;
            }
        }
        kept.push((c, i));
        i += 1;
    }

    // Pass 2: decompose, strip marks, lowercase, collapse whitespace.
    let mut text = String::with_capacity(kept.len());
    let mut map = Vec::with_capacity(kept.len());
    let mut pending_space = false;
    let mut started = false;

    for (c, idx) in kept {
        if c.is_whitespace() {
            // Defer, so runs collapse and trailing space never lands.
            pending_space = started;
            continue;
        }
        // NFKD also expands ligatures: "ﬁ" -> "fi", "①" -> "1".
        for d in c.nfkd() {
            if is_combining(d) {
                continue;
            }
            if pending_space {
                text.push(' ');
                map.push(idx);
                pending_space = false;
            }
            for lc in d.to_lowercase() {
                text.push(lc);
                map.push(idx);
            }
            started = true;
        }
    }

    Normalized { text, map }
}

/// Normalize a search term. No index map needed, and the result is directly
/// comparable to `normalize().text`.
pub fn normalize_term(s: &str) -> String {
    normalize(s).text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_case_and_diacritics() {
        assert_eq!(normalize_term("JOSÉ"), "jose");
        assert_eq!(normalize_term("Renée"), "renee");
    }

    #[test]
    fn expands_ligatures() {
        assert_eq!(normalize_term("ﬁnn"), "finn");
    }

    #[test]
    fn strips_invisibles() {
        // Zero-width space wedged into a name defeats naive search.
        assert_eq!(normalize_term("Ja\u{200B}ne"), "jane");
        assert_eq!(normalize_term("Jane\u{00AD}Doe"), "janedoe");
    }

    #[test]
    fn joins_line_break_hyphenation() {
        assert_eq!(normalize_term("Jo-\nhnson"), "johnson");
        // A real hyphenated surname must survive intact.
        assert_eq!(normalize_term("Smith-Jones"), "smith-jones");
    }

    #[test]
    fn collapses_whitespace() {
        assert_eq!(normalize_term("Jane   \n  Doe"), "jane doe");
        assert_eq!(normalize_term("  Jane Doe  "), "jane doe");
    }

    #[test]
    fn map_points_back_at_source() {
        // "Jane Doe" -> the 'd' of "doe" is at source index 5.
        let n = normalize("Jane Doe");
        let d = n.text.find('d').unwrap();
        let chars: Vec<char> = n.text.chars().collect();
        assert_eq!(chars[d], 'd');
        assert_eq!(n.map[d], 5);
    }

    #[test]
    fn map_survives_removed_characters() {
        // The zero-width char shifts source indices but not normalized ones.
        let n = normalize("Ja\u{200B}ne Doe");
        let d = n.text.find('d').unwrap();
        assert_eq!(n.text, "jane doe");
        assert_eq!(n.map[d], 6); // 'D' sits at source index 6
    }
}
