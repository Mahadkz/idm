//! Tolerant hand-written HTML tokenizer that extracts link-bearing attribute
//! values. Well enough behaved on malformed input to never panic or abort a
//! whole page's extraction.

/// Attributes whose values count as URLs on the given lowercase tag name.
const URL_ATTRIBUTES: &[(&str, &str)] = &[
    ("a", "href"),
    ("img", "src"),
    ("source", "src"),
    ("video", "src"),
    ("audio", "src"),
    ("link", "href"),
    ("iframe", "src"),
    ("frame", "src"),
    ("area", "href"),
];

/// Elements whose raw text content must not be scanned for tags.
const RAW_TEXT_ELEMENTS: &[&str] = &["script", "style", "textarea"];

#[derive(Debug, Default)]
pub struct Extracted {
    /// Attribute values, raw (still entity-encoded), in document order.
    pub urls: Vec<String>,
    /// Value of the first `<base href>` seen in the document, if any.
    pub base: Option<String>,
    /// Byte ranges of the raw attribute values behind `urls`, with the
    /// decoded value, for link rewriting.
    pub refs: Vec<UrlRef>,
}

#[derive(Debug, Clone)]
pub struct UrlRef {
    pub start: usize,
    pub end: usize,
    pub value: String,
}

/// Scans an HTML document for URL-bearing attributes.
pub fn extract(html: &str) -> Extracted {
    let mut extracted = Extracted::default();
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"<!--") {
            i = skip_until(bytes, i + 4, b"-->").max(i + 1);
            continue;
        }
        let Some((tag, after_tag)) = parse_start_tag(bytes, i) else {
            i += 1;
            continue;
        };
        let tag = tag.to_ascii_lowercase();
        for ((name, value), span) in after_tag.attributes.into_iter().zip(after_tag.spans) {
            if tag == "base" && name == "href" && extracted.base.is_none() {
                extracted.base = Some(decode_entities(&value));
            }
            if URL_ATTRIBUTES
                .iter()
                .any(|(element, attribute)| *element == tag && *attribute == name)
            {
                let decoded = decode_entities(&value);
                // Valueless or empty attributes have no real span to rewrite.
                if span.1 > span.0 {
                    extracted.refs.push(UrlRef {
                        start: span.0,
                        end: span.1,
                        value: decoded.clone(),
                    });
                }
                extracted.urls.push(decoded);
            }
        }
        if RAW_TEXT_ELEMENTS.contains(&tag.as_str()) {
            let closer = format!("</{}", tag);
            i = skip_until(bytes, after_tag.end, closer.as_bytes()).max(after_tag.end);
        } else {
            i = after_tag.end.max(i + 1);
        }
    }
    extracted
}

/// A start tag's attributes in order and the byte offset just past the
/// closing `>`.
struct Tag {
    attributes: Vec<(String, String)>,
    /// Byte range of each attribute's raw value, parallel to `attributes`.
    spans: Vec<(usize, usize)>,
    end: usize,
}

/// Parses `<name ...>` at `at`, tolerating missing quotes and unterminated
/// tags. Returns the tag name plus its attributes in order and the byte
/// offset just past the closing `>`. Returns `None` for closing tags,
/// doctypes and other constructs that do not carry attributes.
fn parse_start_tag(bytes: &[u8], at: usize) -> Option<(String, Tag)> {
    let mut i = at + 1;
    if i >= bytes.len() || bytes[i] == b'/' || bytes[i] == b'!' || bytes[i] == b'?' {
        return None;
    }
    let start = i;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' && bytes[i] != b'/'
    {
        i += 1;
    }
    let name = String::from_utf8_lossy(&bytes[start..i]).into_owned();
    let mut attributes = Vec::new();
    let mut spans = Vec::new();
    let mut end = bytes.len();
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'>' {
            i += 1;
            end = i;
            break;
        }
        if bytes[i] == b'/' {
            i += 1;
            continue;
        }
        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'>'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        let attribute = String::from_utf8_lossy(&bytes[name_start..i]).into_owned();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut span = (0, 0);
        let value = if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let value_start = i;
                while i < bytes.len() && bytes[i] != quote && bytes[i] != b'>' {
                    i += 1;
                }
                let value = String::from_utf8_lossy(&bytes[value_start..i]).into_owned();
                span = (value_start, i);
                if i < bytes.len() && bytes[i] == quote {
                    i += 1;
                }
                value
            } else {
                let value_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                span = (value_start, i);
                String::from_utf8_lossy(&bytes[value_start..i]).into_owned()
            }
        } else {
            String::new()
        };
        if !attribute.is_empty() {
            attributes.push((attribute.to_ascii_lowercase(), value));
            spans.push(span);
        }
    }
    Some((name.clone(), Tag { attributes, spans, end }))
}

/// First offset past `needle` at or after `from`, or `bytes.len()`.
fn skip_until(bytes: &[u8], from: usize, needle: &[u8]) -> usize {
    bytes
        .windows(needle.len())
        .skip(from)
        .position(|window| window == needle)
        .map_or(bytes.len(), |index| index + from + needle.len())
}

/// Decodes the common named entities and numeric references; unknown input
/// passes through unchanged. Avoids pulling a full HTML entity table.
fn decode_entities(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'&' {
            out.push(value[i..].chars().next().unwrap());
            i += value[i..].chars().next().unwrap().len_utf8();
            continue;
        }
        let Some(semi) = value[i..].find(';').map(|n| i + n) else {
            out.push('&');
            i += 1;
            continue;
        };
        let entity = &value[i + 1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => {
                if let Some(hex) = entity.strip_prefix("#x").or(entity.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                } else if let Some(decimal) = entity.strip_prefix('#') {
                    decimal.parse::<u32>().ok().and_then(char::from_u32)
                } else {
                    None
                }
            }
        };
        match decoded {
            Some(character) => {
                out.push(character);
                i = semi + 1;
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_links_in_document_order() {
        let page = html_fixture();
        let extracted = extract(&page);
        assert_eq!(
            extracted.urls,
            [
                "/css/sheet.css",
                "/pages/a.html",
                "/img/zip%20file.zip",
                "mailto:x@y.z",
                "javascript:void(0)",
                "Pages",
                "/files/nested-with-quotes.pdf",
                "/pages/unterminated.html",
                "/amp/x.zip",
            ]
        );
    }

    fn html_fixture() -> String {
        String::from(
            "<!DOCTYPE html>\n<!-- note: <a href=\"/in/comment.html\"> -->\n\
             <html><head><style>a{content:\"<img src=x>\"}</style>\
             <link rel=stylesheet href=\"/css/sheet.css\"></head><body>\n\
             <a href = \"/pages/a.html\">A</a>\n\
             <img src='/img/zip%20file.zip'>\n\
             <a href=\"mailto:x@y.z\">m</a><a href=\"javascript:void(0)\">j</a>\n\
             <a href= Pages toVisit.html>spaces</a>\n\
             <a href=\"/files/nested-with-quotes.pdf\" title=\"un > closed data\">q</a>\n\
             <script>var x = \"<video src=oops.mp3>\";</script>\n\
             <iframe src=\"/pages/unterminated.html\"\n\
             </body></html><a href=\"/amp/x.zip\">&amp;.zip</a>",
        )
    }

    #[test]
    fn base_href_is_taken_from_the_first_base_tag() {
        let page = "<html><head><base href='/b1/'><base href='/b2/'></head></html>";
        let extracted = extract(page);
        assert_eq!(extracted.base.as_deref(), Some("/b1/"));
    }

    #[test]
    fn entities_are_decoded_in_attribute_values() {
        let page = "<a href=\"a?x=1&amp;y=2&#38;char=%20\">x</a>\
                    <a href=\"&#1052;&#1080;&#1088;\">cyrillic</a>\
                    <a href=\"no-semicolon&x\">c</a>";
        let extracted = extract(page);
        assert_eq!(extracted.urls[0], "a?x=1&y=2&char=%20");
        assert_eq!(extracted.urls[1], "\u{41c}\u{438}\u{440}");
        assert_eq!(extracted.urls[2], "no-semicolon&x");
    }

    #[test]
    fn raw_text_elements_are_not_scanned() {
        let page = "<script>fetch('/script/leak.zip')\
                    var html = '<a href=\"/script/in-string.zip\">';</script>\
                    http://plain-text-in-body.zip";
        assert!(extract(page).urls.is_empty());
    }

    #[test]
    fn fragments_of_markup_never_panic() {
        for fragment in [
            "",
            "<",
            "<<<<",
            "<a",
            "<a href",
            "<a href=",
            "<a href='",
            "<a href=\"unterminated",
            "</",
            "<!",
            "<!----",
            "<svg><style><a href='/a.zip'>",
        ] {
            let _ = extract(fragment);
        }
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

        fn bytes(&mut self, max_len: usize) -> Vec<u8> {
            let len = self.below(max_len as u64 + 1) as usize;
            (0..len).map(|_| self.next_u64() as u8).collect()
        }
    }

    #[test]
    fn extract_never_panics_on_markup_fragments_and_bounds_output() {
        let mut rng = Rng::new(0x4a7b_0001);
        let alphabet = b"<>=/\"' \tabcdehgilmnoprstuvxyz&;#0123456789-!?[]%";
        for _ in 0..20_000 {
            let text = rng.string(alphabet, 80);
            let extracted = extract(&text);
            // Every extracted URL consumes at least the bytes of its attribute
            // name, so the output stays bounded by the input size.
            assert!(
                extracted.urls.len() <= text.len(),
                "{} urls from {} bytes",
                extracted.urls.len(),
                text.len()
            );
            assert!(extracted.urls.iter().all(|url| url.len() <= text.len()));
        }
    }

    #[test]
    fn extract_never_panics_on_arbitrary_bytes() {
        let mut rng = Rng::new(0x4a7b_0002);
        for _ in 0..10_000 {
            let bytes = rng.bytes(64);
            let text = String::from_utf8_lossy(&bytes);
            let extracted = extract(&text);
            assert!(extracted.base.is_none() || extracted.base.as_deref().is_some());
        }
    }

    #[test]
    fn decode_entities_never_expands_its_input() {
        let mut rng = Rng::new(0x4a7b_0003);
        let alphabet = b"&;#xX0123456789abcdefABCDEFampletgtquotaposnbspzz";
        for _ in 0..30_000 {
            let text = rng.string(alphabet, 40);
            let decoded = decode_entities(&text);
            assert!(
                decoded.len() <= text.len(),
                "{text:?} expanded to {decoded:?}"
            );
        }
    }
}
