//! Property/fuzz-style tests for the IPC frame codec.
//!
//! The generator is a tiny deterministic xorshift, so the tests need neither a
//! nightly toolchain nor extra crates. Each test acts as an independent judge
//! over the same public codec: arbitrary bytes must never panic, the declared
//! length must be checked before a body is read, and valid JSON must
//! round-trip.

use std::io::{self, Read};

use serde_json::Value;
use xdm_ipc::{MAX_FRAME, Reply, read_frame, read_native, write_frame, write_native};

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

    fn bytes(&mut self, max_len: usize) -> Vec<u8> {
        let len = self.below(max_len as u64 + 1) as usize;
        (0..len).map(|_| self.next_u64() as u8).collect()
    }
}

/// A reader that yields only its 4-byte prefix and refuses any further read.
/// An implementation that checks the declared size before allocating and
/// reading the body will error without ever calling us again.
struct PrefixOnly {
    prefix: [u8; 4],
    served: bool,
}

impl Read for PrefixOnly {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        assert!(
            !self.served,
            "frame body was read despite an oversized declared length"
        );
        self.served = true;
        let count = buffer.len().min(self.prefix.len());
        buffer[..count].copy_from_slice(&self.prefix[..count]);
        Ok(count)
    }
}

fn random_value(rng: &mut Rng, depth: u32) -> Value {
    let branch = rng.below(if depth == 0 { 5 } else { 7 });
    match branch {
        0 => Value::Null,
        1 => Value::Bool(rng.next_u64() % 2 == 0),
        2 => Value::from(rng.next_u64() as i64),
        // Integer-valued floats survive serde_json's default (non-round-trip)
        // number parser exactly; the frame codec itself only moves bytes.
        3 => Value::from(rng.below(1_000_000) as f64),
        4 => Value::String(String::from_utf8_lossy(&rng.bytes(16)).into_owned()),
        5 => {
            let len = rng.below(4) as usize;
            Value::Array((0..len).map(|_| random_value(rng, depth - 1)).collect())
        }
        _ => {
            let len = rng.below(4) as usize;
            let mut map = serde_json::Map::new();
            for _ in 0..len {
                let key = String::from_utf8_lossy(&rng.bytes(6)).into_owned();
                map.insert(key, random_value(rng, depth - 1));
            }
            Value::Object(map)
        }
    }
}

#[test]
fn read_frame_never_panics_on_arbitrary_bytes() {
    let mut rng = Rng::new(0x5eed_0001);
    for _ in 0..20_000 {
        let bytes = rng.bytes(64);
        let _ = read_frame::<_, Value>(&mut bytes.as_slice());
        let _ = read_native::<_, Value>(&mut bytes.as_slice());
    }
}

#[test]
fn read_frame_rejects_degenerate_declared_lengths() {
    for declared in [0u32, MAX_FRAME as u32 + 1, u32::MAX] {
        let little = declared.to_le_bytes().to_vec();
        let error = read_frame::<_, Reply>(&mut little.as_slice()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "declared {declared}");
        assert!(error.to_string().contains("invalid frame size"));

        let native = declared.to_ne_bytes().to_vec();
        let error = read_native::<_, Reply>(&mut native.as_slice()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "declared {declared}");
    }
}

#[test]
fn read_frame_does_not_read_a_body_for_an_oversized_frame() {
    let mut reader = PrefixOnly {
        prefix: (MAX_FRAME as u32 + 1).to_le_bytes(),
        served: false,
    };
    assert!(read_frame::<_, Reply>(&mut reader).is_err());
    assert!(reader.served, "prefix was not read at all");

    let mut reader = PrefixOnly {
        prefix: (MAX_FRAME as u32 + 1).to_ne_bytes(),
        served: false,
    };
    assert!(read_native::<_, Reply>(&mut reader).is_err());
    assert!(reader.served, "native prefix was not read at all");
}

#[test]
fn read_frame_rejects_truncated_and_garbage_bodies() {
    let mut rng = Rng::new(0x5eed_0002);
    for _ in 0..5_000 {
        let body = rng.bytes(48);
        let declared = body.len() as u32 + 1 + rng.below(16) as u32;
        let mut truncated = declared.to_le_bytes().to_vec();
        truncated.extend_from_slice(&body);
        assert!(
            read_frame::<_, Value>(&mut truncated.as_slice()).is_err(),
            "a short body must be an error"
        );

        let mut exact = (body.len() as u32).to_le_bytes().to_vec();
        exact.extend_from_slice(&body);
        // Random bytes only occasionally decode as JSON; when they do the
        // value must survive a re-encode.
        if let Ok(value) = read_frame::<_, Value>(&mut exact.as_slice()) {
            let mut encoded = Vec::new();
            write_frame(&mut encoded, &value).unwrap();
            assert_eq!(read_frame::<_, Value>(&mut encoded.as_slice()).unwrap(), value);
        }
    }
}

#[test]
fn write_then_read_round_trips_generated_values() {
    let mut rng = Rng::new(0x5eed_0003);
    for _ in 0..20_000 {
        let value = random_value(&mut rng, 2);
        let mut encoded = Vec::new();
        write_frame(&mut encoded, &value).unwrap();
        assert_eq!(read_frame::<_, Value>(&mut encoded.as_slice()).unwrap(), value);

        let mut native = Vec::new();
        write_native(&mut native, &value).unwrap();
        assert_eq!(read_native::<_, Value>(&mut native.as_slice()).unwrap(), value);
    }
}

#[test]
fn read_frame_admits_the_exact_size_limit() {
    // A declared length equal to the limit is in range: the failure must come
    // from the body not being valid JSON, never from the size guard.
    let mut bytes = (MAX_FRAME as u32).to_le_bytes().to_vec();
    bytes.resize(4 + MAX_FRAME, 0);
    let error = read_frame::<_, Reply>(&mut bytes.as_slice()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(!error.to_string().contains("invalid frame size"));
}

#[test]
fn write_frame_rejects_values_larger_than_the_limit() {
    let oversized = Value::String("x".repeat(MAX_FRAME));
    let error = write_frame(&mut Vec::new(), &oversized).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("frame too large"));
    let error = write_native(&mut Vec::new(), &oversized).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}
