//! Portable task lists shared by the CLI, desktop and daemon. Headers are never exported.
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use url::Url;
use xdm_ipc::{Command, ImportEntry, Paths, TransferList};

pub const MAX_ENTRIES: usize = 500;
pub const MAX_LIST_BYTES: usize = 512 * 1024;

pub fn validate(entries: &[ImportEntry]) -> Result<(), String> {
    if entries.is_empty() || entries.len() > MAX_ENTRIES {
        return Err(format!("a list must contain 1–{MAX_ENTRIES} tasks"));
    }
    if serde_json::to_vec(entries)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_LIST_BYTES
    {
        return Err("task list exceeds 512 KiB".into());
    }
    let mut names = HashSet::new();
    for entry in entries {
        let url = Url::parse(&entry.url).map_err(|_| "invalid URL")?;
        if !["http", "https"].contains(&url.scheme())
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || entry.url.len() > 8192
        {
            return Err("use HTTP(S) URLs without embedded credentials".into());
        }
        if !xdm_categories::valid_filename(&entry.filename) {
            return Err(format!("unsafe filename: {}", entry.filename));
        }
        if !names.insert(entry.filename.to_lowercase()) {
            return Err(format!(
                "duplicate destination filename: {}",
                entry.filename
            ));
        }
        if !(0..=2).contains(&entry.priority)
            || !(1..=16).contains(&entry.connections)
            || !xdm_categories::CATEGORIES.contains(&entry.category.as_str())
            || entry
                .rate_limit
                .is_some_and(|rate| rate == 0 || rate > 1024 * 1024 * 1024)
        {
            return Err("invalid priority, connections, category or rate limit".into());
        }
        match entry.kind.as_str() {
            "http" if entry.video.is_none() && entry.audio.is_none() => {}
            "media"
                if entry.connections == 1
                    && entry.rate_limit.is_none()
                    && entry.video.is_some_and(|index| index <= 10000)
                    && entry.audio.is_none_or(|index| index <= 10000)
                    && (url.path().to_lowercase().ends_with(".m3u8")
                        || url.path().to_lowercase().ends_with(".mpd"))
                    && Path::new(&entry.filename)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4")) => {}
            _ => return Err("invalid HTTP or clear-media task configuration".into()),
        }
    }
    Ok(())
}

fn unique_name(proposed: &str, names: &mut HashSet<String>) -> String {
    let base = if xdm_categories::valid_filename(proposed) {
        proposed
    } else {
        "download.bin"
    };
    if names.insert(base.to_lowercase()) {
        return base.into();
    }
    let path = Path::new(base);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let extension = path
        .extension()
        .map_or_else(String::new, |ext| format!(".{}", ext.to_string_lossy()));
    for index in 2..=MAX_ENTRIES + 1 {
        let candidate = format!("{stem} ({index}){extension}");
        let candidate = if candidate.len() <= 240 {
            candidate
        } else {
            format!("download-{index}.bin")
        };
        if names.insert(candidate.to_lowercase()) {
            return candidate;
        }
    }
    unreachable!("at most MAX_ENTRIES names are assigned")
}

pub fn parse(input: &str) -> Result<TransferList, String> {
    parse_with_base(input, None)
}

/// True for an HLS playlist: its tags make it a media manifest, not a list of
/// independent downloads.
pub fn is_hls_playlist(input: &str) -> bool {
    input
        .lines()
        .any(|line| line.trim_start_matches('\u{feff}').trim().starts_with("#EXT-X-"))
}

pub const HLS_MESSAGE: &str = "this is an HLS playlist (#EXT-X- tags): its segments are not independent downloads; use the media workflow (xdm-cli daemon media-job MANIFEST_URL OUTPUT video ...)";

/// Like `parse`, resolving relative playlist links against `base` (the
/// playlist's own URL) when given. M3U/M3U8 and PLS playlists are recognized;
/// HLS playlists are rejected with a pointer to the media workflow.
pub fn parse_with_base(input: &str, base: Option<&Url>) -> Result<TransferList, String> {
    if input.len() > MAX_LIST_BYTES {
        return Err("input exceeds 512 KiB".into());
    }
    let input = input.trim_start_matches('\u{feff}').trim();
    if input.starts_with('{')
        || (input.starts_with('[') && !input.to_ascii_lowercase().starts_with("[playlist]"))
    {
        let list: TransferList =
            serde_json::from_str(input).map_err(|e| format!("invalid JSON task list: {e}"))?;
        if list.version != 1 {
            return Err("unsupported task list version".into());
        }
        if list.entries.iter().any(|entry| !entry.headers.is_empty()) {
            return Err("task lists cannot carry request headers".into());
        }
        validate(&list.entries)?;
        return Ok(list);
    }
    if is_hls_playlist(input) {
        return Err(HLS_MESSAGE.into());
    }
    let pls = input
        .lines()
        .find(|line| !line.trim().is_empty())
        .is_some_and(|line| line.trim().eq_ignore_ascii_case("[playlist]"));
    let mut entries = Vec::new();
    let mut urls = HashSet::new();
    let mut names = HashSet::new();
    for (index, line) in input.lines().enumerate() {
        let mut line = line.trim();
        if pls {
            // Only FileN= keys carry locations; titles, lengths and counts do not.
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            if !key.strip_prefix("file").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
                continue;
            }
            line = value.trim();
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let url = Url::parse(line)
            .or_else(|error| match base {
                Some(base) if error == url::ParseError::RelativeUrlWithoutBase => base.join(line),
                _ => Err(error),
            })
            .map_err(|_| format!("invalid or unresolvable URL on line {}", index + 1))?;
        if !urls.insert(url.to_string()) {
            continue;
        }
        let segment = url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .unwrap_or("");
        let proposed = percent_encoding::percent_decode_str(segment).decode_utf8_lossy();
        let filename = unique_name(&proposed, &mut names);
        entries.push(ImportEntry {
            url: url.to_string(),
            category: xdm_categories::classify(&filename, None).into(),
            filename,
            kind: "http".into(),
            priority: 1,
            connections: 8,
            rate_limit: None,
            video: None,
            audio: None,
            headers: Vec::new(),
        });
        if entries.len() > MAX_ENTRIES {
            return Err("too many tasks".into());
        }
    }
    validate(&entries)?;
    Ok(TransferList {
        version: 1,
        entries,
    })
}

pub fn read(path: &Path) -> Result<TransferList, String> {
    let mut input = String::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take((MAX_LIST_BYTES + 1) as u64)
        .read_to_string(&mut input)
        .map_err(|e| e.to_string())?;
    parse(&input)
}

pub fn export_url(input: &str) -> Result<String, String> {
    let mut url = Url::parse(input).map_err(|_| "cannot export an invalid URL")?;
    url.set_username("")
        .map_err(|_| "cannot remove URL credentials")?;
    url.set_password(None)
        .map_err(|_| "cannot remove URL credentials")?;
    Ok(url.to_string())
}

pub fn export(paths: &Paths, file: &Path, format: &str) -> Result<usize, String> {
    if !["json", "urls"].contains(&format) {
        return Err("export format must be json or urls".into());
    }
    let reply = paths
        .request(Command::ExportList)
        .map_err(|e| e.to_string())?;
    if !reply.ok {
        return Err(reply.error.unwrap_or_else(|| "export rejected".into()));
    }
    let list = reply.transfer_list.ok_or("daemon returned no task list")?;
    let data = if format == "json" {
        serde_json::to_string_pretty(&list).map_err(|e| e.to_string())? + "\n"
    } else {
        list.entries
            .iter()
            .map(|entry| entry.url.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    };
    if data.len() > MAX_LIST_BYTES {
        return Err("export exceeds 512 KiB".into());
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file)
        .map_err(|e| e.to_string())?;
    output
        .write_all(data.as_bytes())
        .and_then(|_| output.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(list.entries.len())
}

pub fn export_names(entries: &mut [ImportEntry]) {
    let mut names = HashSet::new();
    for entry in entries {
        entry.filename = unique_name(&entry.filename, &mut names);
    }
}

/// A piece of a parsed batch template: literal text or a wildcard group.
enum Part {
    Text(String),
    Range(RangeSpec),
}

/// One `[start-end(:step)]` wildcard group. Letter bounds are stored as their
/// code points so both range kinds share the same stepping arithmetic.
struct RangeSpec {
    start: u64,
    end: u64,
    step: u64,
    /// Number of values the range yields, computed without allocating.
    count: u128,
    /// Zero-padding width for padded numeric ranges, `None` otherwise.
    width: Option<usize>,
    /// The range enumerates single characters instead of numbers.
    letters: bool,
}

impl RangeSpec {
    /// Formats the 0-based `index`-th value of the range.
    fn value(&self, index: u128) -> String {
        let offset = (index * self.step as u128) as u64;
        let number = if self.start <= self.end {
            self.start + offset
        } else {
            self.start - offset
        };
        if self.letters {
            // Bounds are ASCII letters, so the code point fits in a byte.
            return (number as u8 as char).to_string();
        }
        match self.width {
            Some(width) => format!("{number:0>width$}"),
            None => number.to_string(),
        }
    }
}

/// Parses one group body (text between the brackets) into a range.
fn parse_range(body: &str) -> Result<RangeSpec, String> {
    let (range, step) = match body.split_once(':') {
        Some((range, step)) => {
            let step: u64 = step
                .parse()
                .map_err(|_| format!("invalid step in [{body}]"))?;
            if step == 0 {
                return Err(format!("step must be greater than zero in [{body}]"));
            }
            (range, step)
        }
        None => (body, 1),
    };
    let (start, end) = range
        .split_once('-')
        .ok_or_else(|| format!("malformed group [{body}]: expected start-end"))?;
    if start.is_empty() || end.is_empty() {
        return Err(format!("malformed group [{body}]: empty range bound"));
    }
    let start_digits = start.bytes().all(|byte| byte.is_ascii_digit());
    let end_digits = end.bytes().all(|byte| byte.is_ascii_digit());
    let start_letters = start.bytes().all(|byte| byte.is_ascii_alphabetic());
    let end_letters = end.bytes().all(|byte| byte.is_ascii_alphabetic());
    if start_digits && end_digits {
        numeric_range(start, end, step, body)
    } else if start_letters && end_letters {
        letter_range(start, end, step, body)
    } else if (start_digits && end_letters) || (start_letters && end_digits) {
        Err(format!("mixed number and letter range [{body}]"))
    } else {
        Err(format!("malformed group [{body}]"))
    }
}

fn numeric_range(start: &str, end: &str, step: u64, body: &str) -> Result<RangeSpec, String> {
    let start_value: u64 = start
        .parse()
        .map_err(|_| format!("range bound too large in [{body}]"))?;
    let end_value: u64 = end
        .parse()
        .map_err(|_| format!("range bound too large in [{body}]"))?;
    // Zero padding kicks in when a bound carries leading zeros.
    let padded =
        (start.len() > 1 && start.starts_with('0')) || (end.len() > 1 && end.starts_with('0'));
    let diff = start_value.abs_diff(end_value);
    Ok(RangeSpec {
        start: start_value,
        end: end_value,
        step,
        count: diff as u128 / step as u128 + 1,
        width: padded.then_some(start.len().max(end.len())),
        letters: false,
    })
}

fn letter_range(start: &str, end: &str, step: u64, body: &str) -> Result<RangeSpec, String> {
    if start.len() != 1 || end.len() != 1 {
        return Err(format!(
            "letter range bounds must be single characters in [{body}]"
        ));
    }
    let start_char = start.as_bytes()[0] as char;
    let end_char = end.as_bytes()[0] as char;
    if start_char.is_lowercase() != end_char.is_lowercase() {
        return Err(format!("letter range must not mix cases in [{body}]"));
    }
    let start_value = start_char as u64;
    let end_value = end_char as u64;
    Ok(RangeSpec {
        start: start_value,
        end: end_value,
        step,
        count: start_value.abs_diff(end_value) as u128 / step as u128 + 1,
        width: None,
        letters: true,
    })
}

/// Splits a template into literal and group parts, honouring the `[[` and
/// `]]` bracket escapes.
fn parse_parts(template: &str) -> Result<Vec<Part>, String> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut chars = template.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '[' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    text.push('[');
                    continue;
                }
                let mut body = String::new();
                let mut closed = false;
                while let Some(next) = chars.next() {
                    if next == ']' {
                        closed = true;
                        break;
                    }
                    body.push(next);
                }
                if !closed {
                    return Err(format!("unterminated group [{body}"));
                }
                if !text.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut text)));
                }
                parts.push(Part::Range(parse_range(&body)?));
            }
            ']' => {
                if chars.peek() == Some(&']') {
                    chars.next();
                    text.push(']');
                } else {
                    return Err("unmatched ']' in batch template".into());
                }
            }
            other => text.push(other),
        }
    }
    if !text.is_empty() {
        parts.push(Part::Text(text));
    }
    Ok(parts)
}

/// Expands an IDM-style batch template such as `photo[001-100].jpg` into one
/// string per combination of wildcard groups. Groups are numeric ranges
/// (`[1-10]`, zero-padded `[001-100]`, stepped `[0-100:5]`, descending
/// `[10-1]`) or letter ranges (`[a-z]`, `[A-F]`); several groups form a
/// cartesian product enumerated left to right. `[[` and `]]` produce literal
/// brackets. Malformed or mixed ranges and a zero step are rejected, as is an
/// expansion beyond `limit`, which is checked before the output is built.
pub fn expand_batch(template: &str, limit: usize) -> Result<Vec<String>, String> {
    let parts = parse_parts(template)?;
    let mut total: u128 = 1;
    for part in &parts {
        if let Part::Range(range) = part {
            total = total
                .checked_mul(range.count)
                .ok_or_else(|| format!("expansion exceeds the limit of {limit}"))?;
        }
    }
    if total > limit as u128 {
        return Err(format!(
            "expansion of {total} entries exceeds the limit of {limit}"
        ));
    }
    let mut out = vec![String::new()];
    for part in &parts {
        match part {
            Part::Text(text) => {
                for line in &mut out {
                    line.push_str(text);
                }
            }
            Part::Range(range) => {
                let mut expanded = Vec::with_capacity(out.len() * range.count as usize);
                for prefix in &out {
                    for index in 0..range.count {
                        let mut line = prefix.clone();
                        line.push_str(&range.value(index));
                        expanded.push(line);
                    }
                }
                out = expanded;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_names_duplicates_and_traversal() {
        let list = parse("# comment\nhttps://a.test/file.bin\nhttps://a.test/file.bin\nhttps://b.test/file.bin\nhttps://b.test/CON.txt\nhttps://b.test/%2Fetc%2Fpasswd\n").unwrap();
        assert_eq!(list.entries.len(), 4);
        assert_eq!(list.entries[1].filename, "file (2).bin");
        assert_eq!(list.entries[2].filename, "download.bin");
        assert_eq!(list.entries[3].filename, "download (2).bin");
        assert!(parse("file:///etc/passwd").is_err());
        assert!(parse("https://name:password@a.test/file").is_err());
    }
    #[test]
    fn hls_playlists_are_not_expanded_and_audio_playlists_are() {
        let hls = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6,\nhttp://a.test/s0.ts\n#EXTINF:6,\nhttp://a.test/s1.ts\n#EXT-X-ENDLIST\n";
        assert!(parse(hls).unwrap_err().contains("media workflow"));
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nlow.m3u8\n";
        assert!(parse(master).is_err());
        let audio = "#EXTM3U\n#EXTINF:1,A\nhttp://a.test/one.mp3\n#EXTINF:1,B\nhttp://a.test/one.mp3\ntwo.ogg\n";
        assert!(parse(audio).is_err(), "relative link without a base is malformed");
        let base = Url::parse("http://a.test/dir/list.m3u").unwrap();
        let list = parse_with_base(audio, Some(&base)).unwrap();
        let urls: Vec<_> = list.entries.iter().map(|e| e.url.as_str()).collect();
        assert_eq!(urls, ["http://a.test/one.mp3", "http://a.test/dir/two.ogg"]);
        let pls = "[playlist]\nNumberOfEntries=2\nFile1=http://a.test/x.mp3\nTitle1=X\nLength1=5\nFile2=http://a.test/y.aac\nVersion=2\n";
        assert_eq!(parse(pls).unwrap().entries.len(), 2);
        assert!(parse("#EXTM3U\nnot a url\n").unwrap_err().contains("line 2"));
    }
    #[test]
    fn json_round_trip_and_no_header_fields() {
        let list = parse("https://a.test/file.bin").unwrap();
        let encoded = serde_json::to_string(&list).unwrap();
        assert_eq!(parse(&encoded).unwrap().entries[0].url, list.entries[0].url);
        assert!(parse(&encoded.replace("\"kind\":", "\"headers\":[[\"Cookie\",\"a=b\"]],\"kind\":")).is_err());
        assert_eq!(
            export_url("https://name:secret@a.test/file?token=signed").unwrap(),
            "https://a.test/file?token=signed"
        );
    }
    #[test]
    fn batch_expands_numeric_ranges() {
        assert_eq!(
            expand_batch("f[1-3].bin", 10).unwrap(),
            ["f1.bin", "f2.bin", "f3.bin"]
        );
        assert_eq!(
            expand_batch("img[001-003].jpg", 10).unwrap(),
            ["img001.jpg", "img002.jpg", "img003.jpg"]
        );
        assert_eq!(
            expand_batch("[0-10:5]", 10).unwrap(),
            ["0", "5", "10"],
            "stepped range lands on the end bound"
        );
        assert_eq!(expand_batch("[10-1]", 10).unwrap().len(), 10);
        assert_eq!(expand_batch("[10-8]", 10).unwrap(), ["10", "9", "8"]);
        assert_eq!(
            expand_batch("[010-008]", 10).unwrap(),
            ["010", "009", "008"]
        );
        assert_eq!(expand_batch("[10-0:4]", 10).unwrap(), ["10", "6", "2"]);
        assert_eq!(
            expand_batch("[000-010:2]", 10).unwrap(),
            ["000", "002", "004", "006", "008", "010"]
        );
        let padded = expand_batch("[001-100]", 100).unwrap();
        assert_eq!(padded.len(), 100);
        assert_eq!(padded.first().unwrap(), "001");
        assert_eq!(padded.last().unwrap(), "100");
        assert_eq!(expand_batch("[01-12]", 20).unwrap().len(), 12);
    }
    #[test]
    fn batch_expands_letter_ranges() {
        assert_eq!(expand_batch("[a-c]x", 10).unwrap(), ["ax", "bx", "cx"]);
        assert_eq!(
            expand_batch("[A-F]", 10).unwrap(),
            ["A", "B", "C", "D", "E", "F"]
        );
        assert_eq!(expand_batch("[c-a]", 10).unwrap(), ["c", "b", "a"]);
        assert_eq!(expand_batch("[a-e:2]", 10).unwrap(), ["a", "c", "e"]);
        assert_eq!(expand_batch("[F-A:2]", 10).unwrap(), ["F", "D", "B"]);
    }
    #[test]
    fn batch_groups_expand_as_a_left_to_right_product() {
        assert_eq!(
            expand_batch("s[1-2][a-b].txt", 10).unwrap(),
            ["s1a.txt", "s1b.txt", "s2a.txt", "s2b.txt"]
        );
        assert_eq!(
            expand_batch("[1-3]/[a-b]", 10).unwrap(),
            ["1/a", "1/b", "2/a", "2/b", "3/a", "3/b"]
        );
    }
    #[test]
    fn batch_escapes_literal_brackets() {
        assert_eq!(expand_batch("[[1-2]]", 10).unwrap(), ["[1-2]"]);
        assert_eq!(expand_batch("a[[b]]c", 10).unwrap(), ["a[b]c"]);
        assert_eq!(expand_batch("[[n[1-2]]]", 10).unwrap(), ["[n1]", "[n2]"]);
    }
    #[test]
    fn batch_expands_plain_templates_unchanged() {
        assert_eq!(
            expand_batch("https://a/f.zip", 10).unwrap(),
            ["https://a/f.zip"]
        );
        assert_eq!(expand_batch("", 1).unwrap(), [""]);
    }
    #[test]
    fn batch_rejects_malformed_groups() {
        for template in [
            "f[",       // unterminated group
            "f]",       // unmatched bracket
            "f[]",      // no range inside
            "f[5]",     // missing separator
            "f[5-]",    // empty bound
            "f[-5]",    // empty bound
            "f[1-2-3]", // junk in the end bound
            "f[1--2]",  // junk in the end bound
            "f[a b]",   // non-alphanumeric junk
            "f[ab-cd]", // multi-character letter bounds
            "f[a-Z]",   // mixed letter cases
        ] {
            assert!(expand_batch(template, 100).is_err(), "{template}");
        }
    }
    #[test]
    fn batch_rejects_mixed_ranges_and_bad_steps() {
        assert!(expand_batch("f[1-a]", 100).is_err());
        assert!(expand_batch("f[a-1]", 100).is_err());
        assert!(expand_batch("f[1-9:0]", 100).is_err());
        assert!(expand_batch("f[1-9:x]", 100).is_err());
        assert!(expand_batch("f[1-9:2:3]", 100).is_err());
    }
    #[test]
    fn batch_checks_the_limit_before_allocating() {
        assert!(expand_batch("f[1-1000]", 999).is_err());
        assert_eq!(expand_batch("f[1-1000]", 1000).unwrap().len(), 1000);
        assert!(expand_batch("a[1-10][a-z]", 259).is_err());
        assert_eq!(expand_batch("a[1-10][a-z]", 260).unwrap().len(), 260);
        // The check must not materialise the expansion first: allocating four
        // billion strings would exhaust memory instead of failing fast.
        assert!(expand_batch("big[0-4000000000].bin", 100).is_err());
        assert!(expand_batch("x", 0).is_err());
    }
}
