//! Minimal glob matching for include/exclude URL patterns.
//!
//! Supports `*` (any run of characters, including `/`) and `?` (a single
//! character). Matching is case-insensitive so patterns behave the same on
//! Linux and Windows.

/// Returns `true` when `text` matches `pattern`.
pub fn matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let (mut p, mut t) = (0, 0);
    let (mut star, mut retry) = (None, 0);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            retry = t;
            p += 1;
        } else if let Some(at) = star {
            p = at + 1;
            retry += 1;
            t = retry;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// Returns `true` when any pattern matches `text`.
pub fn matches_any(patterns: &[String], text: &str) -> bool {
    patterns.iter().any(|pattern| matches(pattern, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_extension_patterns() {
        assert!(matches("*.zip", "http://host/a/b.zip"));
        assert!(!matches("*.zip", "http://host/a/b.zipper"));
        assert!(matches("*.ZIP", "http://host/A.ZIP"));
    }

    #[test]
    fn matches_path_patterns_and_wildcards() {
        assert!(matches("*/private/*", "http://host/files/private/x.zip"));
        assert!(!matches("*/private/*", "http://host/files/public/x.zip"));
        assert!(matches("a?c", "abc"));
        assert!(matches("a*c", "ac"));
        assert!(matches("*", "anything"));
        assert!(matches("", ""));
        assert!(!matches("", "x"));
        assert!(!matches("*.zip", "http://host/a.zi"));
    }

    #[test]
    fn matches_any_is_a_disjunction() {
        let patterns = vec!["*.zip".to_string(), "*.pdf".to_string()];
        assert!(matches_any(&patterns, "http://host/a.pdf"));
        assert!(!matches_any(&patterns, "http://host/a.mp4"));
    }
}
