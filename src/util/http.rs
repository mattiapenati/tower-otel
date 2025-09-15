use http::{Method, Request, Version};
use http_body::Body;

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

/// Get the url scheme from the request.
pub fn http_url_scheme<B>(req: &Request<B>) -> Option<&'static str> {
    // Comments in this function are quoted from MDN.
    //
    // See: https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/X-Forwarded-Proto

    // The HTTP X-Forwarded-Proto (XFP) request header is a de-facto standard header
    // for identifying the protocol (HTTP or HTTPS) that a client used to connect to a proxy or
    // load balancer.
    let x_forwarded_proto = req
        .headers()
        .get("x-forwarded-proto")
        .and_then(|v| match v.to_str() {
            Ok(value) if value.eq_ignore_ascii_case("http") => Some("http"),
            Ok(value) if value.eq_ignore_ascii_case("https") => Some("https"),
            _ => None,
        });
    if let Some(x_forwarded_proto) = x_forwarded_proto {
        return Some(x_forwarded_proto);
    }

    // A standardized version of this header is the HTTP Forwarded header, although it's much less
    // frequently used.
    req.headers()
        .get("forwarded")
        .and_then(|v| extract_proto_from_forwarded_header(v.as_bytes()))
}

fn extract_proto_from_forwarded_header(header_value: &[u8]) -> Option<&'static str> {
    for value_per_proxy in header_value.split(|c| *c == b',') {
        for directive in value_per_proxy.split(|c| *c == b';') {
            let directive = directive.trim_ascii().to_ascii_lowercase();

            if let Some(proto) = directive.strip_prefix(b"proto=") {
                return match proto {
                    b"http" => Some("http"),
                    b"https" => Some("https"),
                    _ => None,
                };
            }
        }
    }
    None
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
        assert_eq!(
            extract_proto_from_forwarded_header(b"for=192.0.2.60;proto=http;by=203.0.113.43"),
            Some("http")
        );

        // Case insensitive
        assert_eq!(
            extract_proto_from_forwarded_header(b"Proto=httpS;by=203.0.113.43"),
            Some("https")
        );
    }
}
