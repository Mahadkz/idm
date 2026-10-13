//! Property/fuzz-style tests for category site-rule and file matching.
//!
//! A deterministic xorshift provides input; independent judges check that
//! matching never panics, that the general glob path agrees with a separate
//! reference glob, and that classification always returns a category the
//! caller supplied or a built-in one.

use xdm_categories::{
    CATEGORIES, CategoryRule, classify_with, host_matches, normalize_extension, valid_filename,
    valid_rule,
};

/// Deterministic xorshift64 PRNG: reproducible property runs without deps.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 { 0 } else { self.next_u64() % bound }
    }

    /// A string of 0..=max_len characters drawn from `alphabet`.
    fn string(&mut self, alphabet: &[u8], max_len: usize) -> String {
        let len = self.below(max_len as u64 + 1) as usize;
        let bytes: Vec<u8> = (0..len)
            .map(|_| alphabet[self.below(alphabet.len() as u64) as usize])
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// A non-empty string of 1..=max_len characters drawn from `alphabet`.
    fn token(&mut self, alphabet: &[u8], max_len: usize) -> String {
        let len = 1 + self.below(max_len as u64) as usize;
        let bytes: Vec<u8> = (0..len)
            .map(|_| alphabet[self.below(alphabet.len() as u64) as usize])
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// Independent, exponential reference for `*` and `?` globbing.
fn reference_glob(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();
    reference(&pattern, &text)
}

fn reference(pattern: &[char], text: &[char]) -> bool {
    match pattern.first() {
        None => text.is_empty(),
        Some('*') => (0..=text.len()).any(|skip| reference(&pattern[1..], &text[skip..])),
        Some('?') => !text.is_empty() && reference(&pattern[1..], &text[1..]),
        Some(character) => {
            text.first() == Some(character) && reference(&pattern[1..], &text[1..])
        }
    }
}

fn random_rules(rng: &mut Rng) -> Vec<CategoryRule> {
    (0..rng.below(4))
        .map(|index| CategoryRule {
            name: format!("R{index}"),
            extensions: (0..rng.below(3))
                .map(|_| rng.token(b"abcxyz.", 4))
                .collect(),
            destination: String::new(),
            sites: (0..rng.below(3))
                .map(|_| rng.token(b"abc.*?", 6))
                .collect(),
        })
        .collect()
}

#[test]
fn host_matches_never_panics_on_arbitrary_patterns() {
    let mut rng = Rng::new(0xcafe_0001);
    let alphabet = b"ab.*?c-_0123456789[]:";
    for _ in 0..30_000 {
        let pattern = rng.string(alphabet, 12);
        let host = rng.string(alphabet, 12);
        let _ = host_matches(&pattern, &host);
    }
}

#[test]
fn host_matches_agrees_with_a_reference_glob() {
    // Patterns starting with "*." take the bare-domain fast path, which is
    // deliberately not pure globbing; exercise the general path here.
    let pattern_alphabet = [b'a', b'b', b'*', b'?'];
    let text_alphabet = [b'a', b'b'];
    let mut rng = Rng::new(0xcafe_0002);
    for _ in 0..20_000 {
        let pattern = rng.string(&pattern_alphabet, 6);
        if pattern.starts_with("*.") {
            continue;
        }
        let host = rng.string(&text_alphabet, 6);
        assert_eq!(
            host_matches(&pattern, &host),
            reference_glob(&pattern, &host),
            "pattern {pattern:?} host {host:?}"
        );
    }
}

#[test]
fn star_dot_patterns_match_bare_domain_and_subdomains() {
    let mut rng = Rng::new(0xcafe_0003);
    for _ in 0..5_000 {
        let label = rng.token(b"abc", 5);
        let domain = format!("{label}.test");
        let pattern = format!("*.{domain}");
        assert!(host_matches(&pattern, &domain));
        assert!(host_matches(&pattern, &format!("www.{domain}")));
        assert!(host_matches(&pattern, &format!("a.b.{domain}")));
        assert!(!host_matches(&pattern, &format!("not{domain}")));
        assert!(!host_matches(&pattern, &format!("{domain}.org")));
        assert!(host_matches(&format!("*.{}", domain.to_ascii_uppercase()), &domain));
    }
}

#[test]
fn classify_with_always_returns_a_supplied_or_builtin_category() {
    let mut rng = Rng::new(0xcafe_0004);
    for _ in 0..10_000 {
        let rules = random_rules(&mut rng);
        let name = rng.string(b"abcXYZ.0123456789_-", 12);
        let host = rng.string(b"abc.example0123", 12);
        let mime = rng.string(b"abcdefghijklmnopqrstuvwxyz/;=-", 12);
        let category = classify_with(&rules, &name, Some(&host), Some(&mime));
        assert!(
            rules.iter().any(|rule| rule.name == category)
                || CATEGORIES.contains(&category.as_str()),
            "unexpected category {category:?}"
        );
    }
}

#[test]
fn a_matching_site_rule_beats_every_extension_rule() {
    let mut rng = Rng::new(0xcafe_0005);
    for _ in 0..5_000 {
        let label = rng.token(b"abc", 5);
        let host = format!("www.{label}.test");
        let rules = vec![
            CategoryRule {
                name: "BySite".into(),
                extensions: Vec::new(),
                destination: String::new(),
                sites: vec![format!("*.{label}.test")],
            },
            CategoryRule {
                name: "ByExtension".into(),
                extensions: vec!["zip".into()],
                destination: String::new(),
                sites: Vec::new(),
            },
        ];
        assert_eq!(classify_with(&rules, "payload.zip", Some(&host), None), "BySite");
        assert_eq!(classify_with(&rules, "payload.zip", Some("other.test"), None), "ByExtension");
    }
}

#[test]
fn validators_never_panic_on_arbitrary_input() {
    let mut rng = Rng::new(0xcafe_0006);
    let alphabet = b"aZ09._ -/\\:<>\"|?*\t\n\x01";
    for _ in 0..20_000 {
        let text = rng.string(alphabet, 40);
        let _ = valid_filename(&text);
        let _ = normalize_extension(&text);
        let rule = CategoryRule {
            name: text.clone(),
            extensions: vec![text.clone()],
            destination: text.clone(),
            sites: vec![text.clone()],
        };
        let _ = valid_rule(&rule);
    }
}

/// The documented contract is that host globs support `*` and `?`. The `*.`
/// fast path skips glob matching of the remaining suffix, so a wildcard after
/// the leading `*.` can never match.
#[test]
fn star_dot_prefix_should_still_glob_the_suffix() {
    assert!(host_matches("*.example.*", "a.example.com"));
    assert!(host_matches("*.a?c.test", "abc.test"));
}
