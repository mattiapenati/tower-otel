use http::{Method, Request, Version};
use http_body::Body;

pub const HTTP_DEFAULT_PORT: u16 = 80;
pub const HTTPS_DEFAULT_PORT: u16 = 443;

pub const X_FORWARDED_PROTO: http::HeaderName = http::HeaderName::from_static("x-forwarded-proto");
pub const X_FORWARDED_HOST: http::HeaderName = http::HeaderName::from_static("x-forwarded-host");

/// String representation of HTTP method
pub fn http_method(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::DELETE => "DELETE",
        Method::HEAD => "HEAD",
        Method::OPTIONS => "OPTIONS",
        Method::CONNECT => "CONNECT",
        Method::PATCH => "PATCH",
        Method::TRACE => "TRACE",
        _ => "_OTHER",
    }
}

/// String representation of network protocol version
pub fn http_version(version: Version) -> Option<&'static str> {
    match version {
        Version::HTTP_09 => Some("0.9"),
        Version::HTTP_10 => Some("1.0"),
        Version::HTTP_11 => Some("1.1"),
        Version::HTTP_2 => Some("2"),
        Version::HTTP_3 => Some("3"),
        _ => None,
    }
}

/// Get the size of the HTTP request body from the `Content-Length` header.
pub fn http_request_size<B: Body>(req: &Request<B>) -> Option<u64> {
    req.headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .or_else(|| req.body().size_hint().exact())
}

/// Get the size of the HTTP response body from the `Content-Length` header.
pub fn http_response_size<B: Body>(res: &http::Response<B>) -> Option<u64> {
    res.headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .or_else(|| res.body().size_hint().exact())
}

/// Parsed `Forwarded` header.
pub struct Forwarded<'a> {
    pub host: Option<&'a str>,
    pub proto: Option<&'a str>,
}

impl<'a> Forwarded<'a> {
    /// Parse the `Forwarded` header value.
    pub fn parse_header_value(header_value: &'a http::HeaderValue) -> Self {
        let header_value = header_value.as_bytes();
        let proxies = header_value.split(|c| *c == b',');
        let Some(proxy) = proxies.last() else {
            return Forwarded {
                host: None,
                proto: None,
            };
        };

        let mut host = None;
        let mut proto = None;

        let directives = proxy.split(|c| *c == b';');
        for directive in directives {
            let directive = directive.trim_ascii();
            if let Some(directive_host) = directive.strip_prefix_ignore_ascii_case(b"host=") {
                host = std::str::from_utf8(directive_host).ok();
            }
            if let Some(directive_proto) = directive.strip_prefix_ignore_ascii_case(b"proto=") {
                proto = std::str::from_utf8(directive_proto).ok();
            }
        }

        Self { host, proto }
    }
}

trait ByteSliceExt {
    fn strip_prefix_ignore_ascii_case(&self, prefix: &[u8]) -> Option<&[u8]>;
}

impl ByteSliceExt for [u8] {
    fn strip_prefix_ignore_ascii_case(&self, prefix: &[u8]) -> Option<&[u8]> {
        if self.len() < prefix.len() {
            return None;
        }

        self.iter()
            .zip(prefix.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
            .then(|| &self[prefix.len()..])
    }
}

cfg_if::cfg_if! {
    if #[cfg(feature = "axum")] {
        pub fn http_route<B>(req: &http::Request<B>) -> Option<&str> {
            use axum::extract::MatchedPath;
            req.extensions().get::<MatchedPath>().map(|matched_path| matched_path.as_str())
        }
    } else {
        pub fn http_route<B>(_req: &http::Request<B>) -> Option<&str> {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_forwarded_parser() {
        let header_value =
            http::HeaderValue::from_static("for=192.0.2.60;proto=http;by=203.0.113.43");
        let forwarded = Forwarded::parse_header_value(&header_value);

        assert_eq!(forwarded.host, None);
        assert_eq!(forwarded.proto, Some("http"));

        let header_value = http::HeaderValue::from_static("Proto=https;by=203.0.113.43");
        let forwarded = Forwarded::parse_header_value(&header_value);

        assert_eq!(forwarded.host, None);
        assert_eq!(forwarded.proto, Some("https"));
    }
}
