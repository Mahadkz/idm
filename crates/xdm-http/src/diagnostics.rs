//! Failure classification (contract N17): which stage of a transfer failed,
//! kept as a typed value so task details can say more than free text.

use serde::{Deserialize, Serialize};
use std::error::Error;

/// Stage at which a transfer failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The host name did not resolve.
    Dns,
    /// The server or proxy actively refused the connection.
    ConnectRefused,
    /// No answer to the connection attempt within the timeout.
    ConnectTimeout,
    /// TLS handshake or certificate failure.
    Tls,
    /// The connection went quiet while waiting for headers or body.
    ReadTimeout,
    /// The server answered with an unusable HTTP status.
    HttpStatus,
    /// Local storage failure (space, quota, permissions).
    Disk,
    /// 401/407 challenge.
    Auth,
    /// The proxy configuration could not be used (for example PAC scripts).
    ProxyConfig,
    /// Other connection-level failure (unreachable network, reset).
    Network,
    Other,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dns => "dns",
            Self::ConnectRefused => "connect_refused",
            Self::ConnectTimeout => "connect_timeout",
            Self::Tls => "tls",
            Self::ReadTimeout => "read_timeout",
            Self::HttpStatus => "http_status",
            Self::Disk => "disk",
            Self::Auth => "auth",
            Self::ProxyConfig => "proxy_config",
            Self::Network => "network",
            Self::Other => "other",
        }
    }

    /// Short explanation for task details.
    pub fn label(self) -> &'static str {
        match self {
            Self::Dns => "DNS lookup failed",
            Self::ConnectRefused => "connection refused",
            Self::ConnectTimeout => "connection timed out",
            Self::Tls => "TLS/certificate failure",
            Self::ReadTimeout => "read timed out (stalled connection)",
            Self::HttpStatus => "server returned an error status",
            Self::Disk => "disk error",
            Self::Auth => "authentication required",
            Self::ProxyConfig => "unusable proxy configuration",
            Self::Network => "network error",
            Self::Other => "error",
        }
    }
}

/// An error that carries its class through `Box<dyn Error>`.
#[derive(Debug, Clone)]
pub struct Classified {
    pub kind: ErrorKind,
    pub message: String,
}

impl std::fmt::Display for Classified {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for Classified {}

pub(crate) fn classified(kind: ErrorKind, message: impl Into<String>) -> Box<dyn Error + Send + Sync> {
    Box::new(Classified {
        kind,
        message: message.into(),
    })
}

fn chain_text(error: &(dyn Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text = format!("{text}: {cause}");
        source = cause.source();
    }
    text
}

/// Classifies a transport error from the HTTP client.
pub fn classify_reqwest(error: &reqwest::Error) -> ErrorKind {
    let text = chain_text(error).to_ascii_lowercase();
    let mut io_kind = None;
    let mut source = error.source();
    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            io_kind = Some(io.kind());
        }
        source = cause.source();
    }
    classify_text(&text, io_kind, error.is_connect(), error.is_timeout())
}

fn classify_text(
    text: &str,
    io_kind: Option<std::io::ErrorKind>,
    connect: bool,
    timeout: bool,
) -> ErrorKind {
    const DNS: [&str; 6] = [
        "dns error",
        "failed to lookup address",
        "name or service not known",
        "no address associated",
        "temporary failure in name resolution",
        "nodename nor servname",
    ];
    const TLS: [&str; 18] = [
        "certificate",
        "invalidcert",
        "unknownissuer",
        "badcertificate",
        "handshake",
        "tls alert",
        "tls error",
        "tls connection",
        "rustls",
        "ssl routines",
        "ssl error",
        "received fatal alert",
        "peer sent no certificates",
        "peer is incompatible",
        "peer misbehaved",
        "corrupt message",
        "unexpected message",
        "wrong version number",
    ];
    if DNS.iter().any(|needle| text.contains(needle)) {
        return ErrorKind::Dns;
    }
    if TLS.iter().any(|needle| text.contains(needle)) || text.contains("unknown ca") {
        return ErrorKind::Tls;
    }
    if io_kind == Some(std::io::ErrorKind::ConnectionRefused) || text.contains("connection refused") {
        return ErrorKind::ConnectRefused;
    }
    if timeout || io_kind == Some(std::io::ErrorKind::TimedOut) || text.contains("timed out") {
        return if connect || text.contains("connect") {
            ErrorKind::ConnectTimeout
        } else {
            ErrorKind::ReadTimeout
        };
    }
    if connect {
        return ErrorKind::Network;
    }
    ErrorKind::Other
}

/// Classifies any engine error, using the typed value when the engine attached one.
pub fn classify(error: &(dyn Error + Send + Sync + 'static)) -> ErrorKind {
    if let Some(classified) = error.downcast_ref::<Classified>() {
        return classified.kind;
    }
    if error.downcast_ref::<crate::AuthRequired>().is_some() {
        return ErrorKind::Auth;
    }
    if let Some(reqwest) = error.downcast_ref::<reqwest::Error>() {
        return classify_reqwest(reqwest);
    }
    let text = chain_text(error);
    let lower = text.to_ascii_lowercase();
    let io_kind = error.downcast_ref::<std::io::Error>().map(std::io::Error::kind);
    if matches!(
        io_kind,
        Some(
            std::io::ErrorKind::StorageFull
                | std::io::ErrorKind::QuotaExceeded
                | std::io::ErrorKind::FileTooLarge
                | std::io::ErrorKind::PermissionDenied
                | std::io::ErrorKind::ReadOnlyFilesystem
        )
    ) || lower.contains("disk space")
        || lower.contains("no space left")
        || lower.contains("partial download was kept")
        || lower.contains("permission denied")
        || lower.contains("read-only file system")
    {
        return ErrorKind::Disk;
    }
    if lower.contains("pac not supported") || lower.contains("pac script") {
        return ErrorKind::ProxyConfig;
    }
    // "HTTP 404", "returned HTTP 503", "HTTP 429 for a range request".
    if let Some(at) = lower.find("http ") {
        let digits: String = lower[at + 5..].chars().take_while(char::is_ascii_digit).collect();
        if digits.len() == 3 && matches!(digits.as_bytes()[0], b'4' | b'5') {
            return ErrorKind::HttpStatus;
        }
    }
    classify_text(&lower, io_kind, false, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(text: &str) -> Box<dyn Error + Send + Sync> {
        text.to_owned().into()
    }

    #[test]
    fn text_fallbacks_cover_each_class() {
        assert_eq!(classify(boxed("probe failed: HTTP 404 Not Found").as_ref()), ErrorKind::HttpStatus);
        assert_eq!(classify(boxed("download returned HTTP 503").as_ref()), ErrorKind::HttpStatus);
        assert_eq!(
            classify(boxed("not enough disk space (or quota) for the download at /x").as_ref()),
            ErrorKind::Disk
        );
        assert_eq!(classify(boxed("operation timed out").as_ref()), ErrorKind::ReadTimeout);
        assert_eq!(
            classify(Box::new(std::io::Error::from(std::io::ErrorKind::StorageFull)).as_ref()),
            ErrorKind::Disk
        );
        assert_eq!(classify(boxed("system proxy uses a PAC script: PAC not supported").as_ref()), ErrorKind::ProxyConfig);
        assert_eq!(classify(boxed("something else").as_ref()), ErrorKind::Other);
        assert_eq!(classified(ErrorKind::Dns, "x").to_string(), "x");
        assert_eq!(classify(classified(ErrorKind::Dns, "x").as_ref()), ErrorKind::Dns);
    }

    #[test]
    fn transport_texts_classify() {
        let t = |text, io, connect, timeout| classify_text(text, io, connect, timeout);
        assert_eq!(t("error trying to connect: dns error: failed to lookup address information", None, true, false), ErrorKind::Dns);
        assert_eq!(t("invalid peer certificate: unknownissuer", None, true, false), ErrorKind::Tls);
        assert_eq!(t("tcp connect error: connection refused (os error 111)", Some(std::io::ErrorKind::ConnectionRefused), true, false), ErrorKind::ConnectRefused);
        assert_eq!(t("operation timed out", None, true, true), ErrorKind::ConnectTimeout);
        assert_eq!(t("operation timed out", None, false, true), ErrorKind::ReadTimeout);
        assert_eq!(t("network is unreachable", None, true, false), ErrorKind::Network);
        for message in [
            "invalid peer certificate: unknownissuer",
            "peer is incompatible: no shared cipher suites",
            "received corrupt message of type invalidcontenttype",
            "peer sent an unexpected message: handshake",
            "error:0A00010B:SSL routines::wrong version number",
            "tls handshake eof",
        ] {
            assert_eq!(t(message, None, true, false), ErrorKind::Tls, "{message}");
        }
        // The bare letters "tls" inside another word are not TLS evidence.
        assert_eq!(t("hostname settlsone failed to connect", None, true, false), ErrorKind::Network);
    }
}
