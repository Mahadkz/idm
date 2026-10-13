//! robots.txt parsing restricted to what a polite crawler needs: the
//! `Disallow` prefixes of the user-agent group selected for our own token.

/// Parsed robots.txt: user-agent groups with their `Disallow` prefixes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Robots {
    groups: Vec<(Vec<String>, Vec<String>)>,
}

impl Robots {
    /// Parses groups of `User-agent:` / `Disallow:` lines. Comments and
    /// unknown directives are ignored, as are rules before any user-agent.
    pub fn parse(text: &str) -> Self {
        let mut groups: Vec<(Vec<String>, Vec<String>)> = Vec::new();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            match key.trim().to_ascii_lowercase().as_str() {
                "user-agent" => {
                    let agent = value.trim().to_ascii_lowercase();
                    // Consecutive user-agent lines extend the same group.
                    if let Some(last) = groups.last_mut().filter(|(_, rules)| rules.is_empty()) {
                        last.0.push(agent);
                        continue;
                    }
                    groups.push((vec![agent], Vec::new()));
                }
                "disallow" => {
                    if let Some(last) = groups.last_mut() {
                        last.1.push(value.trim().to_string());
                    }
                }
                _ => {}
            }
        }
        Self { groups }
    }

    /// `Disallow` prefixes of the group for `token`, honoring the precedence
    /// rule: a group naming `token` exactly wins; otherwise the last generic
    /// `*` group applies. Empty `Disallow` values (no restriction) are dropped.
    fn disallow_for(&self, token: &str) -> Vec<String> {
        let group = self
            .groups
            .iter()
            .rev()
            .find(|(agents, _)| agents.iter().any(|agent| agent == token))
            .or_else(|| {
                self.groups
                    .iter()
                    .rev()
                    .find(|(agents, _)| agents.iter().any(|agent| agent == "*"))
            });
        let mut disallow = group
            .map(|(_, rules)| rules.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|rule| !rule.is_empty())
            .collect::<Vec<String>>();
        disallow.sort();
        disallow.dedup();
        disallow
    }

    /// Whether `path` (the URL path plus any query) avoids every `Disallow`
    /// prefix of the group applying to `token`. Simple prefix matching, as
    /// documented for robots.txt.
    pub fn allowed(&self, token: &str, path: &str) -> bool {
        self.disallow_for(token)
            .iter()
            .all(|prefix| !path.starts_with(prefix.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_the_generic_agent_group() {
        let robots = Robots::parse(
            "User-Agent: Googlebot\nDisallow: /g/\n\
             User-agent: *\nDisallow: /private/\nDisallow: /tmp\n# trailing comment\n",
        );
        assert!(robots.allowed("xdm", "/"));
        assert!(!robots.allowed("xdm", "/private/x.html"));
        assert!(robots.allowed("xdm", "/private2/open.html"));
        assert!(!robots.allowed("xdm", "/tmp"));
        assert!(robots.allowed("xdm", "/other"));
    }

    #[test]
    fn consecutive_agent_lines_extend_one_group() {
        let robots = Robots::parse("User-agent: x\nUser-agent: *\nDisallow:\nDisallow: /secret\n");
        assert!(!robots.allowed("y", "/secret/x"));
        assert!(robots.allowed("y", "/ok"));
    }

    #[test]
    fn malformed_or_absent_robots_allow_everything() {
        for text in ["", "no rules at all", "Disallow: /\n", "Disallow: /x"] {
            assert!(Robots::parse(text).allowed("u", "/x"), "{text}");
        }
    }

    #[test]
    fn named_agent_group_wins_over_star() {
        let robots = Robots::parse(
            "User-agent: vscode\nDisallow: /vscode\nUser-agent: *\nDisallow: /public\n",
        );
        assert!(robots.allowed("vscode", "/public/x"));
        assert!(!robots.allowed("vscode", "/vscode"));
        assert!(!robots.allowed("other", "/public/x"));
        assert!(robots.allowed("other", "/vscode"));
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;

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

        fn string(&mut self, alphabet: &[u8], max_len: usize) -> String {
            let len = self.below(max_len as u64 + 1) as usize;
            let bytes: Vec<u8> = (0..len)
                .map(|_| alphabet[self.below(alphabet.len() as u64) as usize])
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }

        fn token(&mut self, alphabet: &[u8], max_len: usize) -> String {
            let len = 1 + self.below(max_len as u64) as usize;
            let bytes: Vec<u8> = (0..len)
                .map(|_| alphabet[self.below(alphabet.len() as u64) as usize])
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }
    }

    #[test]
    fn robots_parser_never_panics_on_arbitrary_text() {
        let mut rng = Rng::new(0x20b0_0001);
        let alphabet = b"User-agent: */#\nDisallow:Sitemap: abcXYZ019_ \t[]";
        for _ in 0..20_000 {
            let text = rng.string(alphabet, 60);
            let robots = Robots::parse(&text);
            let token = rng.string(b"xdm*", 5);
            let path = rng.string(b"/abc", 8);
            let _ = robots.allowed(&token, &path);
            let _ = robots.disallow_for(&token);
        }
    }

    #[test]
    fn disallow_prefixes_block_their_paths_and_extensions() {
        let mut rng = Rng::new(0x20b0_0002);
        for _ in 0..5_000 {
            let prefix = rng.token(b"abcABC019_", 6);
            let robots = Robots::parse(&format!("User-agent: *\nDisallow: {prefix}\n"));
            assert!(!robots.allowed("xdm", &prefix));
            assert!(!robots.allowed("xdm", &format!("{prefix}/more")));
            // A path that begins with a character absent from the prefix cannot
            // be blocked by it, nor extend the prefix.
            assert!(robots.allowed("xdm", &format!("zzz{prefix}")));
        }
    }

    #[test]
    fn empty_disallow_lines_never_block() {
        let robots = Robots::parse("User-agent: *\nDisallow:\n");
        let mut rng = Rng::new(0x20b0_0003);
        for _ in 0..5_000 {
            let path = format!("/{}", rng.string(b"abc/", 8));
            assert!(robots.allowed("xdm", &path));
        }
    }

    #[test]
    fn a_named_agent_group_wins_over_the_star_group() {
        let mut rng = Rng::new(0x20b0_0004);
        for _ in 0..3_000 {
            let named = format!("/{}", rng.token(b"abc", 5));
            let star = format!("/{}", rng.token(b"xyz", 5));
            let robots = Robots::parse(&format!(
                "User-agent: xdm\nDisallow: {named}\nUser-agent: *\nDisallow: {star}\n"
            ));
            assert!(!robots.allowed("xdm", &named));
            assert!(robots.allowed("xdm", &star));
            assert!(!robots.allowed("other", &star));
            assert!(robots.allowed("other", &named));
        }
    }

    #[test]
    fn disallow_rules_are_sorted_and_deduplicated() {
        let mut rng = Rng::new(0x20b0_0005);
        for _ in 0..3_000 {
            let mut text = String::from("User-agent: *\n");
            let mut expected = Vec::new();
            for _ in 0..rng.below(10) {
                let prefix = format!("/{}", rng.token(b"abc", 4));
                text.push_str(&format!("Disallow: {prefix}\n"));
                expected.push(prefix);
            }
            let rules = Robots::parse(&text).disallow_for("xdm");
            assert!(rules.windows(2).all(|w| w[0] < w[1]));
            expected.sort();
            expected.dedup();
            assert_eq!(rules, expected);
        }
    }
}
