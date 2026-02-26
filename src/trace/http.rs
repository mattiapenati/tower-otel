//! Middleware that adds tracing to a [`Service`] that handles HTTP requests.

#[cfg(feature = "reqwest_013")]
mod reqwest;

use std::{
    fmt::Display,
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use http::{HeaderMap, Request, Response, StatusCode};
use pin_project::pin_project;
use tower_layer::Layer;
use tower_service::Service;
use tracing::{Level, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::{
    trace::{extractor::HeaderExtractor, injector::HeaderInjector},
    util::{self, AnyUrl},
};

/// Describes the relationship between the [`Span`] and the service producing the span.
#[derive(Clone, Copy, Debug)]
enum SpanKind {
    /// The span describes a request sent to some remote service.
    Client,
    /// The span describes the server-side handling of a request.
    Server,
}

impl SpanKind {
    fn as_str(self) -> &'static str {
        match self {
            SpanKind::Client => "client",
            SpanKind::Server => "server",
        }
    }
}

/// Data extracted from an HTTP request used to build a tracing span.
///
/// `'r` is the lifetime of read-only data (URL, extensions).
/// `'h` is the lifetime of the mutable headers borrow (for context injection/extraction).
struct RequestSpanData<'r, 'h> {
    kind: SpanKind,
    /// Pre-computed via [`util::http_method`] — `'static`, no borrow of the request.
    method: &'static str,
    /// Pre-computed via [`util::http_version`] — `'static`, no borrow of the request.
    version: Option<&'static str>,
    /// Provides `path`, `query`, `host`, `port`, `scheme`, and `full_str` for the span.
    url: AnyUrl<'r>,
    /// For context injection (client) or extraction (server).
    headers: &'h mut HeaderMap,
    /// Server-only: `server.address` extracted from `Forwarded`/`Host` headers.
    server_address: Option<String>,
    /// Server-only: `server.port` extracted from `Forwarded`/`Host` headers.
    server_port: Option<u16>,
    /// Server-only: `url.scheme` extracted from `Forwarded`/`X-Forwarded-Proto` headers.
    url_scheme: Option<String>,
    /// Server-only. Borrowed from `parts.extensions` (disjoint from headers).
    http_route: Option<&'r str>,
    /// Server-only. Copied to avoid a lifetime on `SocketAddr`.
    client_address: Option<std::net::SocketAddr>,
}

impl<'r, 'h> RequestSpanData<'r, 'h> {
    /// Build from the parts of an `http::Request` acting as a **client**.
    ///
    /// Uses disjoint field access on [`http::request::Parts`]: `parts.uri` (shared) and
    /// `parts.headers` (exclusive) can coexist because they are separate struct fields.
    fn from_http_client_parts(parts: &'r mut http::request::Parts) -> Self
    where
        'r: 'h,
    {
        let method = util::http_method(&parts.method);
        let version = util::http_version(parts.version);
        // Borrow parts.uri specifically — disjoint from parts.headers.
        let url = AnyUrl::Uri(&parts.uri);
        // All borrows above are from parts.uri or 'static — disjoint from parts.headers.
        let headers = &mut parts.headers;
        Self {
            kind: SpanKind::Client,
            method,
            version,
            url,
            headers,
            server_address: None,
            server_port: None,
            url_scheme: None,
            http_route: None,
            client_address: None,
        }
    }

    /// Build from the parts of an `http::Request` acting as a **server**.
    ///
    /// `server_address` and `url_scheme` are extracted from headers and immediately owned
    /// so that `parts.headers` can then be mutably borrowed for context extraction.
    fn from_http_server_parts(parts: &'r mut http::request::Parts) -> Self
    where
        'r: 'h,
    {
        let method = util::http_method(&parts.method);
        let version = util::http_version(parts.version);
        // server_address and url_scheme come from parts.headers (shared borrow).
        // Convert to owned immediately so we can take &mut parts.headers next.
        let (server_address, url_scheme, server_port) = {
            let attrs = util::HttpRequestAttributes::from_recv_headers(&parts.headers);
            (
                attrs.server_address.map(ToOwned::to_owned),
                attrs.url_scheme.map(ToOwned::to_owned),
                attrs.server_port,
            )
            // attrs (and shared borrow of parts.headers) dropped here
        };
        // http_route and client_address come from parts.extensions — disjoint from headers.
        let http_route = util::http_route_from_extensions(&parts.extensions);
        let client_address = util::client_address_from_extensions(&parts.extensions).copied();
        // parts.uri and parts.headers are separate fields — disjoint borrows OK.
        let url = AnyUrl::Uri(&parts.uri);
        let headers = &mut parts.headers;
        Self {
            kind: SpanKind::Server,
            method,
            version,
            url,
            headers,
            server_address,
            server_port,
            url_scheme,
            http_route,
            client_address,
        }
    }
}

/// [`Layer`] that adds tracing to a [`Service`] that handles HTTP requests.
#[derive(Clone, Debug)]
pub struct HttpLayer {
    level: Level,
    kind: SpanKind,
}

impl HttpLayer {
    /// [`Span`] are constructed at the given level from server side.
    pub fn server(level: Level) -> Self {
        Self {
            level,
            kind: SpanKind::Server,
        }
    }

    /// [`Span`] are constructed at the given level from client side.
    pub fn client(level: Level) -> Self {
        Self {
            level,
            kind: SpanKind::Client,
        }
    }
}

impl<S> Layer<S> for HttpLayer {
    type Service = Http<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Http {
            inner,
            level: self.level,
            kind: self.kind,
        }
    }
}

/// Middleware that adds tracing to a [`Service`] that handles HTTP requests.
#[derive(Clone, Debug)]
pub struct Http<S> {
    inner: S,
    level: Level,
    kind: SpanKind,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for Http<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    S::Error: Display,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let (mut parts, body) = req.into_parts();
        let span = {
            let mut data = match self.kind {
                SpanKind::Client => RequestSpanData::from_http_client_parts(&mut parts),
                SpanKind::Server => RequestSpanData::from_http_server_parts(&mut parts),
            };
            make_request_span(self.level, &mut data)
        };
        let req = Request::from_parts(parts, body);
        let inner = {
            let _enter = span.enter();
            self.inner.call(req)
        };

        ResponseFuture {
            inner,
            span,
            kind: self.kind,
        }
    }
}

/// Response future for [`Http`].
#[pin_project]
pub struct ResponseFuture<F> {
    #[pin]
    inner: F,
    span: Span,
    kind: SpanKind,
}

impl<F, ResBody, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response<ResBody>, E>>,
    E: Display,
{
    type Output = Result<Response<ResBody>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _enter = this.span.enter();

        match ready!(this.inner.poll(cx)) {
            Ok(response) => {
                record_response(this.span, response.status(), response.headers());
                Poll::Ready(Ok(response))
            }
            Err(err) => {
                record_error(this.span, &err);
                Poll::Ready(Err(err))
            }
        }
    }
}

/// Creates a new [`Span`] for the given request.
fn make_request_span(level: Level, data: &mut RequestSpanData<'_, '_>) -> Span {
    macro_rules! make_span {
        ($level:expr) => {{
            use tracing::field::Empty;

            tracing::span!(
                $level,
                "HTTP",
                "client.address" = Empty,
                "client.port" = Empty,
                "error.message" = Empty,
                "http.request.method" = data.method,
                "http.response.status_code" = Empty,
                "network.protocol.name" = "http",
                "network.protocol.version" = data.version,
                "otel.kind" = data.kind.as_str(),
                "otel.status_code" = Empty,
                "server.address" = Empty,
                "server.port" = Empty,
                "url.full" = Empty,
                "url.path" = data.url.path(),
                "url.query" = Empty,
                "url.scheme" = Empty,
            )
        }};
    }

    let span = match level {
        Level::ERROR => make_span!(Level::ERROR),
        Level::WARN => make_span!(Level::WARN),
        Level::INFO => make_span!(Level::INFO),
        Level::DEBUG => make_span!(Level::DEBUG),
        Level::TRACE => make_span!(Level::TRACE),
    };

    for (header_name, header_value) in data.headers.iter() {
        if let Ok(attribute_value) = header_value.to_str() {
            let attribute_name = format!("http.request.header.{}", header_name);
            span.set_attribute(attribute_name, attribute_value.to_owned());
        }
    }

    if let Some(query) = data.url.query() {
        span.record("url.query", query);
    }

    match data.kind {
        SpanKind::Client => {
            span.record("url.full", data.url.full_str().as_ref());
            if let Some(host) = data.url.host() {
                span.record("server.address", host);
            }
            if let Some(port) = data.url.port_or_default() {
                span.record("server.port", port);
            }
            if let Some(scheme) = data.url.scheme() {
                span.record("url.scheme", scheme);
            }

            let context = span.context();
            opentelemetry::global::get_text_map_propagator(|injector| {
                injector.inject_context(&context, &mut HeaderInjector(data.headers));
            });
        }
        SpanKind::Server => {
            if let Some(http_route) = data.http_route {
                span.record("http.route", http_route);
            }
            if let Some(client_address) = data.client_address {
                span.record(
                    "client.address",
                    tracing::field::display(client_address.ip()),
                );
                span.record("client.port", client_address.port());
            }
            if let Some(ref server_address) = data.server_address {
                span.record("server.address", server_address.as_str());
            }
            if let Some(server_port) = data.server_port {
                span.record("server.port", server_port);
            }
            if let Some(ref url_scheme) = data.url_scheme {
                span.record("url.scheme", url_scheme.as_str());
            }

            let context = opentelemetry::global::get_text_map_propagator(|extractor| {
                extractor.extract(&HeaderExtractor(data.headers))
            });
            if let Err(err) = span.set_parent(context) {
                tracing::warn!("Failed to set parent span: {err}");
            }
        }
    }

    span
}

/// Records fields associated to the response.
fn record_response(span: &Span, status: StatusCode, headers: &HeaderMap) {
    span.record("http.response.status_code", status.as_u16() as i64);

    for (header_name, header_value) in headers.iter() {
        if let Ok(attribute_value) = header_value.to_str() {
            let attribute_name = format!("http.response.header.{}", header_name);
            span.set_attribute(attribute_name, attribute_value.to_owned());
        }
    }

    if status.is_server_error() || status.is_client_error() {
        span.record("otel.status_code", "ERROR");
    }
}

/// Records the error message.
fn record_error<E: Display>(span: &Span, err: &E) {
    span.record("otel.status_code", "ERROR");
    span.record("error.message", err.to_string());
}
