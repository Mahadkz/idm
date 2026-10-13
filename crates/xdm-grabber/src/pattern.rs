//! URL extension classification (glob matching lives in `crate::glob`).

use url::Url;

/// Percent-decodes a string; invalid escapes pass through untouched.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&value[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Lowercase file extension of the URL path (after percent-decoding), or
/// `None` when the last path segment has no extension.
pub fn file_extension(url: &Url) -> Option<String> {
    let path = percent_decode(url.path());
    let name = path.rsplit('/').find(|segment| !segment.is_empty())?;
    let (_, extension) = name.rsplit_once('.')?;
    if extension.is_empty() {
        return None;
    }
    Some(extension.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_comes_from_the_decoded_path() {
        let file = Url::parse("http://a/d%20ir/arch ive.rar?q=1").unwrap();
        assert_eq!(file_extension(&file).as_deref(), Some("rar"));
        let page = Url::parse("http://a/dir").unwrap();
        assert_eq!(file_extension(&page), None);
        let dot = Url::parse("http://a/dir/.name.").unwrap();
        assert_eq!(file_extension(&dot), None);
        let root = Url::parse("http://a/").unwrap();
        assert_eq!(file_extension(&root), None);
    }
}
