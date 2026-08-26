//! Generating the search terms for a name, sorted into confidence tiers.
//!
//! Tiers drive the review UI: High is pre-checked, Medium and Low are found and
//! listed but left unchecked so the user opts in. Casting wide is safe *only*
//! because nothing below High is applied without a human looking at it.

use crate::normalize::normalize_term;
use std::collections::HashMap;

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Unambiguous. Pre-checked in the UI.
    High,
    /// Plausible but collides with ordinary words often enough to need eyes.
    Medium,
    /// Speculative: initials, typo-distance, common-word name tokens.
    Low,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Must sit on word boundaries, so "doe" does not fire inside "doesn't".
    Name,
    /// A username or email local part. Word-boundary rules apply, but trailing
    /// digits are allowed so "jdoe" also catches "jdoe2".
    Id,
}

#[derive(Clone, Debug)]
pub struct Variant {
    /// Already normalized, directly comparable to normalized page text.
    pub term: String,
    /// Shown in the review list so the user can tell *why* something matched.
    pub label: &'static str,
    pub tier: Tier,
    pub kind: Kind,
}

/// Name tokens that are also ordinary English words. A student named Will, May,
/// or Song would otherwise have their entire homework blacked out by the
/// first-name-only variant. These get forced down to Low so they are never
/// applied without review.
const COMMON_WORDS: &[&str] = &[
    "will", "may", "song", "art", "bill", "mark", "rose", "grace", "hope", "faith", "dawn",
    "june", "april", "summer", "autumn", "sky", "ray", "dale", "glen", "brook", "chase",
    "chance", "drew", "hunter", "page", "reed", "victor", "jack", "rich", "frank", "young",
    "long", "short", "white", "black", "brown", "green", "gray", "grey", "king", "love",
    "joy", "pearl", "ruby", "jade", "amber", "olive", "sunny", "star", "storm", "river",
    "field", "hill", "stone", "wood", "moss", "bond", "case", "price", "cash", "penny",
    "bell", "ford", "west", "north", "east", "south", "day", "night", "noel", "carol",
    "christian", "angel", "hero", "major", "miles", "bright", "swift", "best", "good",
];

fn is_risky_token(t: &str) -> bool {
    // Only 1-2 character tokens are demoted on length alone. Plenty of real
    // surnames are three letters (Doe, Kim, Lee, Ali, Ito) and belong at
    // Medium; the common-word list is what catches the genuinely ambiguous
    // ones. Neither tier is auto-applied, so this only affects emphasis.
    t.chars().count() <= 2 || COMMON_WORDS.contains(&t)
}

/// Split a typed name into first / middles / last.
fn split(name: &str) -> (String, Vec<String>, String) {
    // Accept both "Jane Doe" and "Doe, Jane".
    let cleaned = if let Some((last, first)) = name.split_once(',') {
        format!("{} {}", first.trim(), last.trim())
    } else {
        name.trim().to_string()
    };
    let toks: Vec<String> = cleaned
        .split_whitespace()
        .map(|t| normalize_term(t))
        .filter(|t| !t.is_empty())
        .collect();
    match toks.len() {
        0 => (String::new(), vec![], String::new()),
        // A mononym is both the first and the last name.
        1 => (toks[0].clone(), vec![], toks[0].clone()),
        _ => (
            toks[0].clone(),
            toks[1..toks.len() - 1].to_vec(),
            toks[toks.len() - 1].clone(),
        ),
    }
}

/// Build the full tiered variant set for a name, plus any extra identifiers the
/// user typed by hand (a Unity ID, an email, a nickname).
pub fn variants(name: &str, extra: &[String]) -> Vec<Variant> {
    let (first, middles, last) = split(name);
    let mut out: Vec<Variant> = Vec::new();
    let mut push = |term: String, label: &'static str, tier: Tier, kind: Kind| {
        if !term.is_empty() {
            out.push(Variant { term, label, tier, kind });
        }
    };

    if first.is_empty() {
        return out;
    }

    let mononym = first == last && middles.is_empty();

    if !mononym {
        // --- High: forms that are unmistakably this person ---
        push(format!("{} {}", first, last), "full name", Tier::High, Kind::Name);
        push(format!("{}, {}", last, first), "last, first", Tier::High, Kind::Name);
        push(format!("{} {}", last, first), "last first", Tier::High, Kind::Name);
        push(format!("{}{}", first, last), "run together", Tier::High, Kind::Name);

        if !middles.is_empty() {
            let mid = middles.join(" ");
            push(format!("{} {} {}", first, mid, last), "full name w/ middle", Tier::High, Kind::Name);
        }

        // --- High: username / email derivations ---
        // NCSU Unity IDs are first-initial + last + optional digits, which the
        // Id kind matches via its trailing-digit rule.
        let fi = first.chars().next().unwrap();
        for (t, l) in [
            (format!("{}{}", fi, last), "unity id form"),
            (format!("{}.{}", first, last), "email form"),
            (format!("{}_{}", first, last), "username form"),
            (format!("{}{}", first, last.chars().next().unwrap()), "username form"),
        ] {
            push(t, l, Tier::High, Kind::Id);
        }

        // --- Medium: a single name token, on its own ---
        // Demoted when the token doubles as an ordinary word.
        let ftier = if is_risky_token(&first) { Tier::Low } else { Tier::Medium };
        let ltier = if is_risky_token(&last) { Tier::Low } else { Tier::Medium };
        push(first.clone(), "first name only", ftier, Kind::Name);
        push(last.clone(), "last name only", ltier, Kind::Name);

        // --- Low: initials ---
        let li = last.chars().next().unwrap();
        push(format!("{}. {}", fi, last), "initial + last", Tier::Low, Kind::Name);
        push(format!("{} {}", fi, last), "initial + last", Tier::Low, Kind::Name);
        push(format!("{} {}.", first, li), "first + initial", Tier::Low, Kind::Name);
        push(format!("{} {}", first, li), "first + initial", Tier::Low, Kind::Name);
        push(format!("{}{}", fi, li), "initials", Tier::Low, Kind::Name);
        push(format!("{}.{}.", fi, li), "initials", Tier::Low, Kind::Name);
    } else {
        let tier = if is_risky_token(&first) { Tier::Low } else { Tier::High };
        push(first.clone(), "name", tier, Kind::Name);
    }

    for e in extra {
        let t = normalize_term(e);
        // An email typed in full: also register its local part on its own.
        if let Some((local, _domain)) = t.split_once('@') {
            push(local.to_string(), "email local part", Tier::High, Kind::Id);
        }
        push(t, "user-supplied", Tier::High, Kind::Id);
    }

    dedup(out)
}

/// Collapse duplicate terms, keeping the strongest tier each one earned.
fn dedup(v: Vec<Variant>) -> Vec<Variant> {
    let mut best: HashMap<String, Variant> = HashMap::new();
    for item in v {
        best.entry(item.term.clone())
            .and_modify(|e| {
                if item.tier < e.tier {
                    *e = item.clone();
                }
            })
            .or_insert(item);
    }
    let mut out: Vec<Variant> = best.into_values().collect();
    // Longest first: matching prefers the most specific hit, so that
    // "jane doe" wins over the bare "doe" that overlaps it.
    out.sort_by(|a, b| {
        a.tier
            .cmp(&b.tier)
            .then(b.term.chars().count().cmp(&a.term.chars().count()))
            .then(a.term.cmp(&b.term))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(name: &str, tier: Tier) -> Vec<String> {
        variants(name, &[])
            .into_iter()
            .filter(|v| v.tier == tier)
            .map(|v| v.term)
            .collect()
    }

    #[test]
    fn high_tier_covers_the_obvious_forms() {
        let h = terms("Jane Doe", Tier::High);
        for want in ["jane doe", "doe, jane", "doe jane", "janedoe", "jdoe", "jane.doe"] {
            assert!(h.contains(&want.to_string()), "missing {:?} in {:?}", want, h);
        }
    }

    #[test]
    fn bare_name_tokens_are_medium_not_high() {
        let m = terms("Jane Doe", Tier::Medium);
        assert!(m.contains(&"jane".to_string()));
        assert!(m.contains(&"doe".to_string()));
    }

    #[test]
    fn common_word_names_are_demoted() {
        // "Will" as a bare token would otherwise redact every "will" in an essay.
        let m = terms("Will Smith", Tier::Medium);
        assert!(!m.contains(&"will".to_string()), "'will' must not be auto-applied");
        let l = terms("Will Smith", Tier::Low);
        assert!(l.contains(&"will".to_string()), "'will' should still be offered");
        // The full name is unaffected.
        assert!(terms("Will Smith", Tier::High).contains(&"will smith".to_string()));
    }

    #[test]
    fn short_tokens_are_demoted() {
        assert!(!terms("Li Wang", Tier::Medium).contains(&"li".to_string()));
    }

    #[test]
    fn accepts_last_comma_first_input() {
        assert!(terms("Doe, Jane", Tier::High).contains(&"jane doe".to_string()));
    }

    #[test]
    fn handles_middle_names() {
        let h = terms("Jane Marie Doe", Tier::High);
        assert!(h.contains(&"jane marie doe".to_string()));
        assert!(h.contains(&"jane doe".to_string()));
    }

    #[test]
    fn extra_identifiers_are_high_confidence() {
        let v = variants("Jane Doe", &["jdoe2@ncsu.edu".into()]);
        let t: Vec<String> = v.iter().filter(|x| x.tier == Tier::High).map(|x| x.term.clone()).collect();
        assert!(t.contains(&"jdoe2".to_string()), "email local part in {:?}", t);
    }

    #[test]
    fn mononym_does_not_panic() {
        assert!(!variants("Prince", &[]).is_empty());
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(variants("   ", &[]).is_empty());
    }
}
