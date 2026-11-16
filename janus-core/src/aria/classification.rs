//! Explainable Multicolored Hat suggestions.
//!
//! This is a deterministic keyword baseline, not an AI classifier. It returns
//! suggestions and matched terms only; a caller or reviewer chooses the final
//! labels. Classification never grants permissions.

use super::Hat;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HatSuggestion {
    pub hat: Hat,
    /// Matched unique terms; deliberately not presented as calibrated confidence.
    pub match_score: usize,
    pub matched_terms: Vec<String>,
}

const RULES: &[(Hat, &[&str])] = &[
    (Hat::Red, &["electronics","circuit","resistor","voltage","current","diode","transistor","solder","hardware"]),
    (Hat::Orange, &["art","music","writing","design","creative","story","poem","paint"]),
    (Hat::Yellow, &["science","math","mathematics","research","physics","chemistry","biology","experiment"]),
    (Hat::Green, &["code","coding","programming","software","rust","python","algorithm","compiler","database","ai"]),
    (Hat::Blue, &["computer","network","linux","operating","server","storage","wifi","bluetooth","infrastructure"]),
    (Hat::Purple, &["security","cybersecurity","exploit","vulnerability","forensics","pentest","malware","encryption"]),
    (Hat::Black, &["failure","risk","debug","debugging","troubleshoot","diagnosis","root-cause","incident"]),
    (Hat::Brown, &["bike","bicycle","mechanical","tool","repair","engine","practical","assembly","maintenance"]),
    (Hat::White, &["documentation","manual","source","standard","reference","citation","evidence","datasheet"]),
];

/// Suggest hats from token matches. White is suggested when evidence/document
/// terms appear. Rainbow is suggested when two or more non-White disciplines
/// have matches. The result is advisory and intentionally easy to audit.
pub fn suggest_hats(text: &str) -> Vec<HatSuggestion> {
    let tokens: HashSet<String> = text.to_lowercase()
        .split(|ch:char| !ch.is_alphanumeric() && ch!='-')
        .filter(|token|!token.is_empty())
        .map(str::to_owned)
        .collect();
    let mut suggestions: Vec<HatSuggestion> = RULES.iter().filter_map(|(hat,terms)| {
        let matched_terms: Vec<String> = terms.iter()
            .filter(|term|tokens.contains(**term))
            .map(|term|(*term).to_owned()).collect();
        if matched_terms.is_empty() { return None; }
        Some(HatSuggestion {hat:*hat,match_score:matched_terms.len(),matched_terms})
    }).collect();
    let discipline_count=suggestions.iter().filter(|s|s.hat!=Hat::White).count();
    if discipline_count>=2 {
        suggestions.push(HatSuggestion {
            hat:Hat::Rainbow,match_score:discipline_count,
            matched_terms:vec!["cross-domain".into()],
        });
    }
    suggestions.sort_by(|a,b|b.match_score.cmp(&a.match_score)
        .then_with(||format!("{:?}",a.hat).cmp(&format!("{:?}",b.hat))));
    suggestions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_multiple_domains_and_rainbow_with_explainable_matches() {
        let suggestions=suggest_hats(
            "Repair a bicycle computer circuit, test the voltage, and debug its Rust software."
        );
        assert!(suggestions.iter().any(|s|s.hat==Hat::Brown && s.matched_terms.contains(&"bicycle".into())));
        assert!(suggestions.iter().any(|s|s.hat==Hat::Red && s.matched_terms.contains(&"voltage".into())));
        assert!(suggestions.iter().any(|s|s.hat==Hat::Green && s.matched_terms.contains(&"rust".into())));
        assert!(suggestions.iter().any(|s|s.hat==Hat::Rainbow));
    }

    #[test]
    fn no_matches_produce_no_suggestion() {
        assert!(suggest_hats("hello there").is_empty());
    }

    #[test]
    fn white_hat_is_suggested_for_documentation_terms() {
        let suggestions=suggest_hats("Check the official datasheet and documentation.");
        assert!(suggestions.iter().any(|s|s.hat==Hat::White));
        assert!(!suggestions.iter().any(|s|s.hat==Hat::Rainbow));
    }
}
