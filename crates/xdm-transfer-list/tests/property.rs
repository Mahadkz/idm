//! Property/fuzz-style tests for task-list parsing and batch expansion.
//!
//! A deterministic xorshift supplies inputs. Independent judges check that
//! parsing never panics, that expansion's count is the product of its ranges,
//! that a failure comes from the range count rather than the allocation it
//! would have made, and that oversized input is refused by the byte limit.

use std::time::{Duration, Instant};

use xdm_transfer_list::{MAX_LIST_BYTES, expand_batch, export_url, parse};

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
}

#[test]
fn expand_batch_never_panics_on_arbitrary_templates() {
    let mut rng = Rng::new(0xba7c_0001);
    let alphabet = b"[]-:.0123456789abcXYZ/\\?*";
    for _ in 0..30_000 {
        let template = rng.string(alphabet, 20);
        let limit = rng.below(200) as usize;
        let _ = expand_batch(&template, limit);
    }
}

#[test]
fn expand_batch_count_is_the_product_of_its_ranges() {
    let mut rng = Rng::new(0xba7c_0002);
    for _ in 0..2_000 {
        let groups = 1 + rng.below(2) as usize;
        let mut template = String::new();
        let mut reference = vec![String::new()];
        let mut total: u128 = 1;
        for _ in 0..groups {
            let literal = rng.string(b"xY_", 3);
            template.push_str(&literal);
            for line in &mut reference {
                line.push_str(&literal);
            }

            let start = rng.below(10);
            let end = rng.below(10);
            let step = 1 + rng.below(8);
            let count = start.abs_diff(end) as u128 / step as u128 + 1;
            total *= count;
            if step == 1 {
                template.push_str(&format!("[{start}-{end}]"));
            } else {
                template.push_str(&format!("[{start}-{end}:{step}]"));
            }

            // Independent enumeration of the same range.
            let values: Vec<String> = (0..count)
                .map(|index| {
                    let offset = (index * step as u128) as u64;
                    if start <= end {
                        (start + offset).to_string()
                    } else {
                        (start - offset).to_string()
                    }
                })
                .collect();
            let mut expanded = Vec::with_capacity(reference.len() * values.len());
            for prefix in &reference {
                for value in &values {
                    expanded.push(format!("{prefix}{value}"));
                }
            }
            reference = expanded;
        }
        let trailing = rng.string(b"xY_", 3);
        template.push_str(&trailing);
        for line in &mut reference {
            line.push_str(&trailing);
        }

        let limit = total as usize;
        assert_eq!(expand_batch(&template, limit).unwrap(), reference, "{template}");
        assert!(expand_batch(&template, limit - 1).is_err(), "{template}");
    }
}

#[test]
fn expand_batch_rejects_oversized_expansions_without_allocating() {
    let templates = [
        // A single unary range larger than the limit.
        "big[0-4000000000].bin",
        // Product fits u128 but exceeds the limit by a wide margin.
        "a[0-1000000000][0-1000000000]",
        // Product overflows u128 and must be rejected by checked arithmetic.
        "x[0-18446744073709551615][0-18446744073709551615]",
    ];
    for template in templates {
        let started = Instant::now();
        assert!(expand_batch(template, 100).is_err(), "{template}");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(500),
            "{template} took {elapsed:?}, so the limit was not checked before allocating"
        );
    }
}

#[test]
fn parse_never_panics_on_arbitrary_input() {
    let mut rng = Rng::new(0xba7c_0003);
    let alphabet = b"{}[]\":,\n#htp:/?.=&%0123456789abcXYZ \t\\";
    for _ in 0..20_000 {
        let input = rng.string(alphabet, 60);
        let _ = parse(&input);
    }
}

#[test]
fn parse_rejects_input_over_the_byte_limit() {
    let oversized = "a".repeat(MAX_LIST_BYTES + 1);
    assert!(parse(&oversized).is_err());
}

#[test]
fn export_url_strips_embedded_credentials() {
    let mut rng = Rng::new(0xba7c_0004);
    for _ in 0..5_000 {
        let user = {
            let candidate = rng.string(b"abcXYZ09", 6);
            if candidate.is_empty() { "u".into() } else { candidate }
        };
        let password = rng.string(b"abcXYZ09", 6);
        let input = format!("https://{user}:{password}@a.test/f.txt");
        assert_eq!(export_url(&input).unwrap(), "https://a.test/f.txt");
    }
}
