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
