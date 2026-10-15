//! HTTP authentication for saved logins (contract N11): Basic and Digest
//! (RFC 7616, MD5 and SHA-256 with their `-sess` variants, `qop=auth`).
//!
//! A [`Login`] rides along with a transfer; the engine answers a 401 with the
//! strongest offered scheme, remembers it, and from then on attaches a fresh
//! `Authorization` header to every request, ranged ones included, with its
//! own incremented `nc`. A repeated 401 is final unless the server says the
//! nonce went `stale`, so wrong credentials never loop.

use sha2::{Digest as _, Sha256};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::Mutex;

/// Username and password for the origin of a transfer. `Debug` hides the password.
#[derive(Clone, PartialEq, Eq)]
pub struct Login {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Login")
            .field("username", &self.username)
            .field("password", &"<hidden>")
            .finish()
    }
}

/// One `WWW-Authenticate` challenge: a scheme and its parameters (names lower-cased).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    pub scheme: String,
    pub params: Vec<(String, String)>,
}

impl Challenge {
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Splits header values into challenges, honoring quoted strings, so
/// `Digest realm="a, b", nonce="x", Basic realm="y"` yields two challenges.
pub fn parse_challenges(value: &str) -> Vec<Challenge> {
    let bytes = value.as_bytes();
    let mut at = 0;
    let mut out: Vec<Challenge> = Vec::new();
    let skip = |at: &mut usize| {
        while *at < bytes.len() && (bytes[*at] == b' ' || bytes[*at] == b'\t' || bytes[*at] == b',') {
            *at += 1;
        }
    };
    let token = |at: &mut usize| -> String {
        let start = *at;
        while *at < bytes.len() && !matches!(bytes[*at], b' ' | b'\t' | b',' | b'=') {
            *at += 1;
        }
        value[start..*at].to_owned()
    };
    loop {
        skip(&mut at);
        if at >= bytes.len() {
            break;
        }
        let word = token(&mut at);
        if word.is_empty() {
            at += 1;
            continue;
        }
        // `word` is a parameter name when `=` follows (after optional blanks).
        let mut peek = at;
        while peek < bytes.len() && (bytes[peek] == b' ' || bytes[peek] == b'\t') {
            peek += 1;
        }
        if peek < bytes.len() && bytes[peek] == b'=' && !out.is_empty() {
            // Token68 padding (`Negotiate abc==`) is not a parameter list.
            at = peek + 1;
            while at < bytes.len() && (bytes[at] == b' ' || bytes[at] == b'\t') {
                at += 1;
            }
            let text = if at < bytes.len() && bytes[at] == b'"' {
                at += 1;
                let mut text = String::new();
                while at < bytes.len() && bytes[at] != b'"' {
                    if bytes[at] == b'\\' && at + 1 < bytes.len() {
                        at += 1;
                    }
                    let ch = value[at..].chars().next().unwrap_or(' ');
                    text.push(ch);
                    at += ch.len_utf8();
                }
                at += 1;
                text
            } else {
                let start = at;
                while at < bytes.len() && bytes[at] != b',' {
                    at += 1;
                }
                value[start..at].trim().to_owned()
            };
            if let Some(last) = out.last_mut() {
                last.params.push((word.to_ascii_lowercase(), text));
            }
        } else {
            out.push(Challenge {
                scheme: word,
                params: Vec::new(),
            });
        }
    }
    out
}

/// Hash algorithm named by a Digest challenge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    Md5,
    Md5Sess,
    Sha256,
    Sha256Sess,
}

impl Algorithm {
    fn parse(name: Option<&str>) -> Option<Self> {
        match name.map(str::to_ascii_uppercase).as_deref() {
            None | Some("MD5") => Some(Self::Md5),
            Some("MD5-SESS") => Some(Self::Md5Sess),
            Some("SHA-256") => Some(Self::Sha256),
            Some("SHA-256-SESS") => Some(Self::Sha256Sess),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Md5Sess => "MD5-sess",
            Self::Sha256 => "SHA-256",
            Self::Sha256Sess => "SHA-256-sess",
        }
    }

    fn sess(self) -> bool {
        matches!(self, Self::Md5Sess | Self::Sha256Sess)
    }

    fn hash(self, data: &str) -> String {
        match self {
            Self::Md5 | Self::Md5Sess => hex(&md5(data.as_bytes())),
            Self::Sha256 | Self::Sha256Sess => hex(&Sha256::digest(data.as_bytes())),
        }
    }
}

/// A usable Digest challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestChallenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    pub algorithm: Algorithm,
    /// Whether the server offered `qop=auth`; absent means RFC 2069 compatibility.
    pub qop_auth: bool,
    pub stale: bool,
}

impl DigestChallenge {
    fn from(challenge: &Challenge) -> Option<Self> {
        if !challenge.scheme.eq_ignore_ascii_case("digest") {
            return None;
        }
        let algorithm = Algorithm::parse(challenge.param("algorithm"))?;
        let qop = challenge.param("qop");
        let qop_auth = qop.map(|list| {
            list.split(',')
                .any(|item| item.trim().eq_ignore_ascii_case("auth"))
        });
        // A server that only offers auth-int cannot be answered without hashing the body.
        if qop_auth == Some(false) {
            return None;
        }
        Some(Self {
            realm: challenge.param("realm").unwrap_or("").to_owned(),
            nonce: challenge.param("nonce")?.to_owned(),
            opaque: challenge.param("opaque").map(str::to_owned),
            algorithm,
            qop_auth: qop_auth.unwrap_or(false),
            stale: challenge
                .param("stale")
                .is_some_and(|value| value.eq_ignore_ascii_case("true")),
        })
    }
}

/// Computes the Digest `response` value (RFC 7616 section 3.4.1).
#[allow(clippy::too_many_arguments)]
pub fn digest_response(
    challenge: &DigestChallenge,
    login: &Login,
    method: &str,
    uri: &str,
    nc: u32,
    cnonce: &str,
) -> String {
    let h = |text: &str| challenge.algorithm.hash(text);
    let mut ha1 = h(&format!("{}:{}:{}", login.username, challenge.realm, login.password));
    if challenge.algorithm.sess() {
        ha1 = h(&format!("{ha1}:{}:{cnonce}", challenge.nonce));
    }
    let ha2 = h(&format!("{method}:{uri}"));
    if challenge.qop_auth {
        h(&format!("{ha1}:{}:{nc:08x}:{cnonce}:auth:{ha2}", challenge.nonce))
    } else {
        h(&format!("{ha1}:{}:{ha2}", challenge.nonce))
    }
}

fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Builds the `Authorization` header value for one request.
pub fn digest_header(
    challenge: &DigestChallenge,
    login: &Login,
    method: &str,
    uri: &str,
    nc: u32,
    cnonce: &str,
) -> String {
    let response = digest_response(challenge, login, method, uri, nc, cnonce);
    let mut header = format!(
        "Digest username={}, realm={}, nonce={}, uri={}, algorithm={}",
        quote(&login.username),
        quote(&challenge.realm),
        quote(&challenge.nonce),
        quote(uri),
        challenge.algorithm.name(),
    );
    if challenge.qop_auth {
        header.push_str(&format!(", qop=auth, nc={nc:08x}, cnonce={}", quote(cnonce)));
    }
    header.push_str(&format!(", response={}", quote(&response)));
    if let Some(opaque) = &challenge.opaque {
        header.push_str(&format!(", opaque={}", quote(opaque)));
    }
    header
}

/// Standard base64 of `user:password` for Basic.
fn basic_value(login: &Login) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let data = format!("{}:{}", login.username, login.password).into_bytes();
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    format!("Basic {out}")
}

/// What the transfer learned from the server.
#[derive(Debug, Clone)]
enum Scheme {
    Basic,
    Digest { challenge: DigestChallenge, nc: u32, uri: String },
}

#[derive(Debug, Default)]
struct State {
    scheme: Option<Scheme>,
    /// An authenticated request has succeeded with the current scheme.
    verified: bool,
}

/// Shared by every worker of one transfer.
#[derive(Debug)]
pub struct AuthState {
    login: Login,
    state: Mutex<State>,
}

/// Request-target of a URL (`path?query`) as Digest `uri` expects it.
pub fn request_target(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let target = rest.find('/').map_or("/", |at| &rest[at..]);
    let target = target.split('#').next().unwrap_or("/");
    target.to_owned()
}

fn random_cnonce() -> String {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    hex(&hasher.finish().to_be_bytes())
}

/// Outcome of offering a 401 challenge to the state.
pub enum Answer {
    /// Resend: the state now holds a scheme for the next request.
    Retry,
    /// Credentials were already tried (or the scheme is unsupported).
    GiveUp,
}

impl AuthState {
    pub fn new(login: Login) -> Self {
        Self {
            login,
            state: Mutex::new(State::default()),
        }
    }

    /// `Authorization` value for the next request, with a fresh `nc` and `cnonce`.
    /// The bool reports whether the header is a reuse of a known scheme.
    pub fn header(&self, method: &str) -> Option<String> {
        let mut state = self.state.lock().ok()?;
        match state.scheme.as_mut()? {
            Scheme::Basic => Some(basic_value(&self.login)),
            Scheme::Digest { challenge, nc, uri } => {
                *nc = nc.wrapping_add(1);
                Some(digest_header(challenge, &self.login, method, uri, *nc, &random_cnonce()))
            }
        }
    }

    pub fn has_scheme(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.scheme.is_some())
    }

    /// A request answered with credentials was accepted.
    pub fn confirm(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.verified = true;
        }
    }

    /// Takes the strongest supported challenge from a 401. `sent_auth` tells
    /// whether the rejected request already carried credentials; that is only
    /// retried when the nonce is stale or credentials worked before.
    pub fn answer(&self, headers: &[String], effective_url: &str, sent_auth: bool) -> Answer {
        let challenges: Vec<Challenge> = headers.iter().flat_map(|value| parse_challenges(value)).collect();
        let digests: Vec<DigestChallenge> = challenges.iter().filter_map(DigestChallenge::from).collect();
        let best = digests
            .iter()
            .max_by_key(|digest| matches!(digest.algorithm, Algorithm::Sha256 | Algorithm::Sha256Sess));
        let Ok(mut state) = self.state.lock() else {
            return Answer::GiveUp;
        };
        let basic = challenges.iter().any(|c| c.scheme.eq_ignore_ascii_case("basic"));
        let stale = best.is_some_and(|digest| digest.stale);
        if sent_auth && !stale && !state.verified {
            return Answer::GiveUp;
        }
        if let Some(digest) = best {
            state.scheme = Some(Scheme::Digest {
                challenge: digest.clone(),
                nc: 0,
                uri: request_target(effective_url),
            });
        } else if basic && !sent_auth {
            state.scheme = Some(Scheme::Basic);
        } else if basic && state.verified {
            state.scheme = Some(Scheme::Basic);
        } else {
            return Answer::GiveUp;
        }
        state.verified = false;
        Answer::Retry
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// MD5 (RFC 1321), needed only because Digest still mandates it.
pub fn md5(input: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32)
        .collect();
    let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let mut data = input.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&((input.len() as u64).wrapping_mul(8)).to_le_bytes());
    for chunk in data.chunks(64) {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, word) in [a0, b0, c0, d0].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_rfc_1321_vectors() {
        for (input, want) in [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ] {
            assert_eq!(hex(&md5(input.as_bytes())), want, "{input}");
        }
    }

    fn challenge(text: &str) -> DigestChallenge {
        DigestChallenge::from(&parse_challenges(text)[0]).unwrap()
    }

    #[test]
    fn rfc_2617_example_md5() {
        // RFC 2617 section 3.5.
        let digest = challenge(
            r#"Digest realm="testrealm@host.com", qop="auth,auth-int", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", opaque="5ccc069c403ebaf9f0171e9517f40e41""#,
        );
        let login = Login { username: "Mufasa".into(), password: "Circle Of Life".into() };
        let response =
            digest_response(&digest, &login, "GET", "/dir/index.html", 1, "0a4f113b");
        assert_eq!(response, "6629fae49393a05397450978507c4ef1");
        let header = digest_header(&digest, &login, "GET", "/dir/index.html", 1, "0a4f113b");
        assert!(header.contains("nc=00000001") && header.contains("opaque=\"5ccc069c403ebaf9f0171e9517f40e41\""));
    }

    #[test]
    fn rfc_7616_example_md5_and_sha256() {
        // RFC 7616 section 3.9.1 (SHA-256) and 3.9.2 (MD5 listed in the same example set).
        let login = Login { username: "Mufasa".into(), password: "Circle of Life".into() };
        let sha = challenge(
            r#"Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=SHA-256, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#,
        );
        assert_eq!(
            digest_response(&sha, &login, "GET", "/dir/index.html", 1, "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ"),
            "753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1"
        );
        let md5c = challenge(
            r#"Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=MD5, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#,
        );
        assert_eq!(
            digest_response(&md5c, &login, "GET", "/dir/index.html", 1, "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ"),
            "8ca523f5e9506fed4657c9700eebdbec"
        );
    }

    #[test]
    fn challenge_parser_handles_quotes_lists_and_unsupported_qop() {
        let all = parse_challenges(
            r#"Digest realm="a, b", nonce="x", qop="auth", Basic realm="y", Negotiate"#,
        );
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].param("realm"), Some("a, b"));
        assert_eq!(all[1].scheme, "Basic");
        assert_eq!(all[2].scheme, "Negotiate");
        let only_int = parse_challenges(r#"Digest realm="r", nonce="n", qop="auth-int""#);
        assert!(DigestChallenge::from(&only_int[0]).is_none());
        let stale = challenge(r#"Digest realm="r", nonce="n", stale=TRUE"#);
        assert!(stale.stale && !stale.qop_auth);
    }

    #[test]
    fn nc_increments_and_wrong_credentials_stop_after_one_retry() {
        let state = AuthState::new(Login { username: "u".into(), password: "p".into() });
        let header = r#"Digest realm="r", nonce="n", qop="auth""#.to_owned();
        assert!(matches!(state.answer(&[header.clone()], "http://h/f?x=1", false), Answer::Retry));
        let first = state.header("GET").unwrap();
        let second = state.header("GET").unwrap();
        assert!(first.contains("nc=00000001") && second.contains("nc=00000002"));
        assert!(first.contains("uri=\"/f?x=1\""));
        // Rejected again with the same nonce: final.
        assert!(matches!(state.answer(&[header.clone()], "http://h/f", true), Answer::GiveUp));
        // A stale nonce earns one fresh attempt.
        let stale = r#"Digest realm="r", nonce="n2", qop="auth", stale=true"#.to_owned();
        assert!(matches!(state.answer(&[stale], "http://h/f", true), Answer::Retry));
        assert!(state.header("GET").unwrap().contains("nonce=\"n2\""));
    }

    #[test]
    fn prefers_sha256_and_basic_header_is_standard() {
        let state = AuthState::new(Login { username: "Aladdin".into(), password: "open sesame".into() });
        let both = r#"Digest realm="r", nonce="n", algorithm=MD5, Digest realm="r", nonce="n", algorithm=SHA-256"#.to_owned();
        assert!(matches!(state.answer(&[both], "http://h/", false), Answer::Retry));
        assert!(state.header("GET").unwrap().contains("algorithm=SHA-256"));
        let basic = AuthState::new(Login { username: "Aladdin".into(), password: "open sesame".into() });
        assert!(matches!(basic.answer(&[r#"Basic realm="x""#.into()], "http://h/", false), Answer::Retry));
        assert_eq!(basic.header("GET").unwrap(), "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
        assert!(!format!("{:?}", Login { username: "a".into(), password: "secret".into() }).contains("secret"));
    }
}
