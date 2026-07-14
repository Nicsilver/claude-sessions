//! The decision half of Windows-Terminal tab selection, split out from the UIA plumbing in
//! `wt.rs` so the fuzzy-matching policy is unit-testable (and platform-independent).
//!
//! Matching is fuzzy on purpose: the title Claude writes and the one the recorder captured can
//! drift (renames, stale titles), so we score shared word tokens from BOTH the recorded tab
//! title and the derived topic and only switch on a confident winner — never guessing on a tie.

use std::collections::HashSet;

const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "with", "into", "from", "that", "this", "your", "you", "set", "up", "add",
    "fix", "new", "run", "get", "out", "off", "was", "are", "has",
];

/// Lowercased, punctuation/glyph-free, "administrator:" prefix removed.
pub fn norm(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .flat_map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().collect::<Vec<_>>()
            } else {
                vec![' ']
            }
        })
        .collect();
    let flat = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.strip_prefix("administrator ")
        .unwrap_or(&flat)
        .to_string()
}

/// Meaningful word tokens (len >= 3, not a stopword) for overlap scoring.
pub fn tokens(s: &str) -> Vec<String> {
    norm(s)
        .split_whitespace()
        .filter(|w| w.len() >= 3 && !STOP_WORDS.contains(w))
        .map(str::to_string)
        .collect()
}

/// What we're looking for: distinctive tokens pooled from the recorded tab title and topic,
/// plus their normalized forms for an exact-title win.
pub struct Target {
    tokens: HashSet<String>,
    exact_a: String,
    exact_b: String,
}

impl Target {
    pub fn new(tab_title: &str, topic: &str) -> Self {
        let mut tokens: HashSet<String> = tokens(tab_title).into_iter().collect();
        tokens.extend(self::tokens(topic));
        Target {
            tokens,
            exact_a: norm(tab_title),
            exact_b: norm(topic),
        }
    }

    /// No signal at all to match on — the caller should not attempt a switch.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty() && self.exact_a.is_empty() && self.exact_b.is_empty()
    }
}

/// Given the tab names in tab order, pick the index to select — or `None` to stay put.
/// Mirrors WtTabs.cs: an exact normalized-title match wins outright (the first such tab); else a
/// confident token winner (>= 2 shared tokens, or a single distinctive token no other tab
/// shares); else, if exactly one tab is still the generic "Claude Code", select that; else don't
/// guess.
pub fn choose(names: &[String], target: &Target) -> Option<usize> {
    if names.len() <= 1 {
        return None; // single tab (or none) — nothing to switch to
    }
    let mut best: Option<usize> = None;
    let (mut best_score, mut second) = (0usize, 0usize);
    let mut generic: Vec<usize> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let nn = norm(name);
        if !nn.is_empty() && (nn == target.exact_a || nn == target.exact_b) {
            return Some(i); // exact title wins outright
        }
        if nn == "claude code" {
            generic.push(i); // a tab still showing the default title
        }
        let score = tokens(name)
            .iter()
            .filter(|t| target.tokens.contains(*t))
            .count();
        if score > best_score {
            second = best_score;
            best_score = score;
            best = Some(i);
        } else if score > second {
            second = score;
        }
    }
    if let Some(i) = best {
        if best_score >= 2 || (best_score == 1 && second == 0) {
            return Some(i);
        }
    }
    // Nothing matched confidently: a session whose recorded title matches no tab usually sits in a
    // tab still titled "Claude Code". If there's exactly one such tab it's unambiguous.
    if generic.len() == 1 {
        return Some(generic[0]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_lowercases_and_strips_punctuation_and_admin_prefix() {
        assert_eq!(norm("Administrator: Fix OAuth!"), "fix oauth");
        assert_eq!(
            norm("Port  widget — to Tauri v2"),
            "port widget to tauri v2"
        );
        assert_eq!(norm(""), "");
    }

    #[test]
    fn tokens_drop_short_words_and_stopwords() {
        // "fix" and "the" are stopwords; "to"/"v2"? v2 is len 2 -> dropped; "auth" kept.
        assert_eq!(tokens("Fix the auth bug"), vec!["auth", "bug"]);
        assert!(tokens("up to no").is_empty());
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn exact_title_wins_over_token_score() {
        let t = Target::new("Deploy pipeline", "");
        // tab 0 shares tokens, tab 1 is an exact normalized match — exact must win.
        let idx = choose(
            &names(&["Deploy the release pipeline steps", "Deploy pipeline"]),
            &t,
        );
        assert_eq!(idx, Some(1));
    }

    #[test]
    fn two_shared_tokens_selects() {
        let t = Target::new("Refactor auth middleware", "");
        let idx = choose(&names(&["unrelated build logs", "Refactor auth layer"]), &t);
        assert_eq!(idx, Some(1)); // shares "refactor" + "auth"
    }

    #[test]
    fn single_distinctive_token_selects_when_unique() {
        let t = Target::new("Kubernetes rollout", "");
        let idx = choose(&names(&["random shell", "Kubernetes stuff"]), &t);
        assert_eq!(idx, Some(1)); // one shared distinctive token, no other tab shares it
    }

    #[test]
    fn tie_on_one_token_does_not_guess() {
        let t = Target::new("deploy server", "");
        // both tabs share exactly one token ("deploy" / "server") -> ambiguous -> stay put.
        let idx = choose(&names(&["deploy notes", "server notes"]), &t);
        assert_eq!(idx, None);
    }

    #[test]
    fn single_generic_claude_code_tab_is_the_fallback() {
        let t = Target::new("Write integration tests", "");
        let idx = choose(&names(&["some other work", "Claude Code"]), &t);
        assert_eq!(idx, Some(1));
    }

    #[test]
    fn multiple_generic_tabs_are_ambiguous() {
        let t = Target::new("Write integration tests", "");
        let idx = choose(&names(&["Claude Code", "Claude Code"]), &t);
        assert_eq!(idx, None);
    }

    #[test]
    fn single_tab_never_switches() {
        let t = Target::new("anything", "");
        assert_eq!(choose(&names(&["only tab"]), &t), None);
        assert_eq!(choose(&[], &t), None);
    }

    #[test]
    fn empty_target_is_reported_empty() {
        assert!(Target::new("", "").is_empty());
        assert!(!Target::new("real title", "").is_empty());
    }
}
