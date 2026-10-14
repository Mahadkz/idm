//! Runtime gettext-style catalogs.
//!
//! Slint's own bundled translations need the compiler feature
//! `bundle-translations` (not reachable through the `slint!` macro) or the
//! `gettext` runtime, so labels are written as `Tr.t(@tr("..."), Tr.lang)`:
//! `@tr` keeps the strings extractable with `slint-tr-extractor`, and `Tr.t`
//! looks them up in the `.po` catalog compiled into the binary.
use std::cell::RefCell;
use std::collections::HashMap;

const UR_PO: &str = include_str!("assets/ur.po");

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    entries: HashMap<String, String>,
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let inner = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(s);
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl Catalog {
    pub fn parse_po(content: &str) -> Self {
        let mut entries = HashMap::new();
        let mut id = String::new();
        let mut text = String::new();
        let mut in_str = false;
        let mut in_id = false;
        let mut flush = |id: &mut String, text: &mut String| {
            if !id.is_empty() && !text.is_empty() {
                entries.insert(std::mem::take(id), std::mem::take(text));
            }
            id.clear();
            text.clear();
        };
        for line in content.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("msgid ") {
                flush(&mut id, &mut text);
                in_id = true;
                in_str = false;
                id = unquote(rest);
            } else if let Some(rest) = line.strip_prefix("msgstr ") {
                in_id = false;
                in_str = true;
                text = unquote(rest);
            } else if line.starts_with('"') {
                if in_id {
                    id.push_str(&unquote(line));
                } else if in_str {
                    text.push_str(&unquote(line));
                }
            }
        }
        flush(&mut id, &mut text);
        Self { entries }
    }

    pub fn for_language(lang: &str) -> Self {
        match lang {
            "ur" => Self::parse_po(UR_PO),
            _ => Self::default(),
        }
    }

    pub fn translate(&self, text: &str) -> String {
        self.entries.get(text).cloned().unwrap_or_else(|| text.to_string())
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

thread_local! {
    static CURRENT: RefCell<(String, Catalog)> = RefCell::new(("en".into(), Catalog::default()));
}

pub fn set_language(lang: &str) {
    let lang = if lang == "ur" { "ur" } else { "en" };
    CURRENT.with(|c| *c.borrow_mut() = (lang.to_string(), Catalog::for_language(lang)));
}

pub fn current_language() -> String {
    CURRENT.with(|c| c.borrow().0.clone())
}

pub fn translate(text: &str) -> String {
    CURRENT.with(|c| c.borrow().1.translate(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_translates() {
        let cat = Catalog::for_language("ur");
        assert!(cat.len() > 30);
        assert_eq!(cat.translate("Open folder"), "فولڈر کھولیں");
        assert_eq!(cat.translate("No such string"), "No such string");
    }

    #[test]
    fn multiline_and_escapes() {
        let cat = Catalog::parse_po("msgid \"a\\\"b\"\nmsgstr \"x\"\n\"y\\n\"\n");
        assert_eq!(cat.translate("a\"b"), "xy\n");
    }

    #[test]
    fn english_is_identity() {
        set_language("en");
        assert_eq!(translate("Size"), "Size");
        set_language("ur");
        assert_eq!(translate("Size"), "سائز");
        set_language("en");
    }
}
