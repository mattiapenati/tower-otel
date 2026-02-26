use std::{borrow::Cow, str::FromStr};

use http::{Method, Request, Version};
use http_body::Body;

const HTTP_DEFAULT_PORT: u16 = 80;
const HTTPS_DEFAULT_PORT: u16 = 443;

const X_FORWARDED_PROTO: http::HeaderName = http::HeaderName::from_static("x-forwarded-proto");
const X_FORWARDED_HOST: http::HeaderName = http::HeaderName::from_static("x-forwarded-host");

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
    http_body_size_from_headers(req.headers()).or_else(|| req.body().size_hint().exact())
}

/// Get the size of the HTTP response body from the `Content-Length` header.
pub fn http_response_size<B: Body>(res: &http::Response<B>) -> Option<u64> {
    http_body_size_from_headers(res.headers()).or_else(|| res.body().size_hint().exact())
}

pub fn http_body_size_from_headers(headers: &http::HeaderMap) -> Option<u64> {
    headers
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
}

/// The URL of an HTTP request, either borrowed from an `http::Uri` or cloned from a `url::Url`.
///
/// For `http::Request` the URI is borrowed directly (zero allocation for URL fields).
/// For `reqwest::Request` the URL is cloned once (one allocation total, then all string
/// accessors borrow from the owned value).
pub(crate) enum AnyUrl<'r> {
    Uri(&'r http::Uri),
    #[cfg(feature = "reqwest_013")]
    Url(reqwest::Url),
}

impl<'r> AnyUrl<'r> {
    pub(crate) fn path(&self) -> &str {
        match self {
            AnyUrl::Uri(uri) => uri.path(),
            #[cfg(feature = "reqwest_013")]
            AnyUrl::Url(url) => url.path(),
        }
    }

    pub(crate) fn query(&self) -> Option<&str> {
        match self {
            AnyUrl::Uri(uri) => uri.query(),
            #[cfg(feature = "reqwest_013")]
            AnyUrl::Url(url) => url.query(),
        }
    }

    /// Returns the full URL string. For `http::Uri` this allocates once; for `url::Url`
    /// it borrows from the owned value (zero allocation).
    pub(crate) fn full_str(&self) -> Cow<'_, str> {
        match self {
            AnyUrl::Uri(uri) => Cow::Owned(uri.to_string()),
            #[cfg(feature = "reqwest_013")]
            AnyUrl::Url(url) => Cow::Borrowed(url.as_str()),
        }
    }

    pub(crate) fn host(&self) -> Option<&str> {
        match self {
            AnyUrl::Uri(uri) => uri.host(),
            #[cfg(feature = "reqwest_013")]
            AnyUrl::Url(url) => url.host_str(),
        }
    }

    pub(crate) fn port_or_default(&self) -> Option<u16> {
        match self {
            AnyUrl::Uri(uri) => uri.port_u16().or_else(|| match uri.scheme_str() {
                Some("http") => Some(80),
                Some("https") => Some(443),
                _ => None,
            }),
            #[cfg(feature = "reqwest_013")]
            AnyUrl::Url(url) => url.port_or_known_default(),
        }
    }

    pub(crate) fn scheme(&self) -> Option<&str> {
        match self {
            AnyUrl::Uri(uri) => uri.scheme_str(),
            #[cfg(feature = "reqwest_013")]
            AnyUrl::Url(url) => Some(url.scheme()),
        }
    }
}

/// Parsed `Forwarded` header.
struct Forwarded<'a> {
    host: Option<&'a str>,
    proto: Option<&'a str>,
}

impl<'a> Forwarded<'a> {
    /// Parse the `Forwarded` header value.
    fn parse_header_value(header_value: &'a http::HeaderValue) -> Self {
        let header_value = header_value.as_bytes();
        let mut proxies = header_value.split(|c| *c == b',');
        let Some(proxy) = proxies.next_back() else {
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

/// Attributes related to HTTP requests.
#[derive(Debug)]
pub struct HttpRequestAttributes<'a> {
    pub url_scheme: Option<&'a str>,
    pub server_address: Option<&'a str>,
    pub server_port: Option<u16>,
}

impl<'a> HttpRequestAttributes<'a> {
    /// Extract HTTP request attributes from a URI (sent request).
    ///
    /// Prefer this over [`from_sent_request`] when you need to borrow only the URI
    /// so that other parts of the request (e.g. headers) can be borrowed independently.
    pub fn from_uri(uri: &'a http::Uri) -> Self {
        let url_scheme = uri.scheme_str();
        let server_address = uri.host();
        let server_port = uri.port_u16().or(match url_scheme {
            Some("http") => Some(HTTP_DEFAULT_PORT),
            Some("https") => Some(HTTPS_DEFAULT_PORT),
            _ => None,
        });
        Self {
            url_scheme,
            server_address,
            server_port,
        }
    }

    /// Extract HTTP request attributes from a sent request.
    pub fn from_sent_request<B>(request: &'a Request<B>) -> Self {
        Self::from_uri(request.uri())
    }

    /// Extract HTTP request attributes from the headers of a received request.
    ///
    /// Prefer this over [`from_recv_request`] when you need to borrow only the headers
    /// so that other parts of the request (e.g. URI) can be borrowed independently.
    pub fn from_recv_headers(headers: &'a http::HeaderMap) -> Self {
        let (host, url_scheme) = headers
            .get(http::header::FORWARDED)
            .map(Forwarded::parse_header_value)
            .map(|Forwarded { host, proto }| (host, proto))
            .unwrap_or_default();

        let host = host
            .or_else(|| headers.get(X_FORWARDED_HOST).and_then(|v| v.to_str().ok()))
            .or_else(|| {
                headers
                    .get(http::header::HOST)
                    .and_then(|v| v.to_str().ok())
            });

        let url_scheme =
            url_scheme.or_else(|| headers.get(X_FORWARDED_PROTO).and_then(|v| v.to_str().ok()));

        let (server_address, server_port) = host
            .and_then(|host| host.split_once(':'))
            .map_or_else(|| (host, None), |(host, port)| (Some(host), Some(port)));

        let server_port = server_port
            .and_then(|server_port| u16::from_str(server_port).ok())
            .or(match url_scheme {
                Some("http") => Some(HTTP_DEFAULT_PORT),
                Some("https") => Some(HTTPS_DEFAULT_PORT),
                _ => None,
            });

        Self {
            url_scheme,
            server_address,
            server_port,
        }
    }

    /// Extract HTTP request attributes from a received request.
    pub fn from_recv_request<B>(request: &'a Request<B>) -> Self {
        Self::from_recv_headers(request.headers())
    }
}

cfg_if::cfg_if! {
    if #[cfg(feature = "axum")] {
        pub fn http_route_from_extensions(extensions: &http::Extensions) -> Option<&str> {
            use axum::extract::MatchedPath;
            extensions
                .get::<MatchedPath>()
                .map(|matched_path| matched_path.as_str())
        }

        pub fn client_address_from_extensions(extensions: &http::Extensions) -> Option<&std::net::SocketAddr> {
            use axum::extract::ConnectInfo;
            extensions
                .get::<ConnectInfo<std::net::SocketAddr>>()
                .map(|ConnectInfo(addr)| addr)
        }

        pub fn http_route<B>(req: &http::Request<B>) -> Option<&str> {
            http_route_from_extensions(req.extensions())
        }

        pub fn client_address<B>(req: &http::Request<B>) -> Option<&'_ std::net::SocketAddr> {
            client_address_from_extensions(req.extensions())
        }
    } else {
        pub fn http_route_from_extensions(_extensions: &http::Extensions) -> Option<&str> {
            None
        }

        pub fn client_address_from_extensions(_extensions: &http::Extensions) -> Option<&std::net::SocketAddr> {
            None
        }

        pub fn http_route<B>(_req: &http::Request<B>) -> Option<&str> {
            None
        }

        pub fn client_address<B>(_req: &http::Request<B>) -> Option<&'_ std::net::SocketAddr> {
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
