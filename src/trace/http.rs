//! Middleware that adds tracing to a [`Service`] that handles HTTP requests.

mod trace_body;

#[cfg(feature = "reqwest_013")]
mod reqwest;

use std::{
    fmt::Display,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{ready, Context, Poll},
};

use http::{HeaderMap, StatusCode};
use pin_project::{pin_project, pinned_drop};
use tower_layer::Layer;
use tower_service::Service;
use tracing::{Level, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::trace::{extractor::HeaderExtractor, injector::HeaderInjector};

#[doc(inline)]
pub use self::trace_body::TraceBody;

/// [`Layer`] that adds tracing to a [`Service`] that handles HTTP requests.
#[derive(Clone, Debug)]
pub struct HttpLayer {
    level: Level,
    kind: sealed::SpanKind,
}

impl HttpLayer {
    /// [`Span`] are constructed at the given level from server side.
    pub fn server(level: Level) -> Self {
        Self {
            level,
            kind: sealed::SpanKind::Server,
        }
    }

    /// [`Span`] are constructed at the given level from client side.
    pub fn client(level: Level) -> Self {
        Self {
            level,
            kind: sealed::SpanKind::Client,
        }
    }

    /// Records response body errors and early drops on the request span.
    pub fn trace_body(self) -> TraceBodyLayer {
        TraceBodyLayer { inner: self }
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

/// HTTP tracing [`Layer`] that also records response body errors and early drops.
#[derive(Clone, Debug)]
pub struct TraceBodyLayer {
    inner: HttpLayer,
}

impl<S> Layer<S> for TraceBodyLayer {
    type Service = TraceBodyService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TraceBodyService {
            inner: self.inner.layer(inner),
        }
    }
}

/// Middleware that adds tracing to a [`Service`] that handles HTTP requests.
#[derive(Clone, Debug)]
pub struct Http<S> {
    inner: S,
    level: Level,
    kind: sealed::SpanKind,
}

impl<S, Req, Res> Service<Req> for Http<S>
where
    S: Service<Req, Response = Res>,
    S::Error: Display,
    Req: HttpRequest,
    Res: HttpResponse,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Req) -> Self::Future {
        let span = make_request_span(self.level, self.kind, &mut req);
        let inner = {
            let _enter = span.enter();
            self.inner.call(req)
        };

        ResponseFuture::new(inner, span, self.kind)
    }
}

/// HTTP tracing middleware that also records response body errors and early drops.
#[derive(Clone, Debug)]
pub struct TraceBodyService<S> {
    inner: Http<S>,
}

impl<S, Req, ResBody> Service<Req> for TraceBodyService<S>
where
    S: Service<Req, Response = http::Response<ResBody>>,
    S::Error: Display,
    Req: HttpRequest,
    ResBody: http_body::Body,
{
    type Response = http::Response<TraceBody<ResBody>>;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future, sealed::UseTraceBody>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Req) -> Self::Future {
        let span = make_request_span(self.inner.level, self.inner.kind, &mut req);
        let inner = {
            let _enter = span.enter();
            self.inner.inner.call(req)
        };

        ResponseFuture::new(inner, span, self.inner.kind)
    }
}

/// Response future for [`Http`] and [`TraceBodyService`].
#[pin_project(PinnedDrop)]
pub struct ResponseFuture<F, M = sealed::Identity> {
    #[pin]
    inner: F,
    span: Option<Span>,
    kind: sealed::SpanKind,
    map: PhantomData<fn() -> M>,
}

impl<F, M> ResponseFuture<F, M> {
    /// Associates a response future with its request span.
    fn new(inner: F, span: Span, kind: sealed::SpanKind) -> Self {
        Self {
            inner,
            span: Some(span),
            kind,
            map: PhantomData,
        }
    }
}

impl<F, Res, E, M> Future for ResponseFuture<F, M>
where
    F: Future<Output = Result<Res, E>>,
    M: sealed::ResponseMapper<Res>,
    Res: HttpResponse,
    E: Display,
{
    type Output = Result<M::Output, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let span = this.span.as_ref().expect("future polled after completion");

        let response = {
            let _enter = span.enter();
            match ready!(this.inner.poll(cx)) {
                Ok(response) => {
                    record_response(span, *this.kind, response.status(), response.headers());
                    Ok(response)
                }
                Err(err) => {
                    record_error(span, &err);
                    Err(err)
                }
            }
        };

        let span = this.span.take().unwrap();
        let response = response.map(|response| M::map_response(response, span));

        Poll::Ready(response)
    }
}

#[pinned_drop]
impl<F, Map> PinnedDrop for ResponseFuture<F, Map> {
    fn drop(self: Pin<&mut Self>) {
        let this = self.project();

        if let Some(span) = this.span.as_ref() {
            record_cancel(span, *this.kind);
        }
    }
}

/// Abstraction over HTTP requests that can be used by the middleware.
pub trait HttpRequest: sealed::HttpRequest {}

impl<B> HttpRequest for http::Request<B> {}

/// Abstraction over HTTP responses that can be used by the middleware.
pub trait HttpResponse: sealed::HttpResponse {}

impl<B> HttpResponse for http::Response<B> {}

/// Creates a new [`Span`] for the given request.
fn make_request_span(level: Level, kind: sealed::SpanKind, request: &mut impl HttpRequest) -> Span {
    let data = request.extract_span_data(kind);

    macro_rules! make_span {
        ($level:expr) => {{
            use tracing::field::Empty;

            tracing::span!(
                $level,
                "HTTP",
                "client.address" = Empty,
                "client.port" = Empty,
                "error.message" = Empty,
                "error.type" = Empty,
                "http.request.method" = data.method.unwrap_or("_OTHER"),
                "http.response.status_code" = Empty,
                "http.route" = Empty,
                "network.protocol.name" = "http",
                "network.protocol.version" = data.version,
                "otel.kind" = kind.as_str(),
                "otel.status_code" = Empty,
                "otel.status_description" = Empty,
                "otel.name" = Empty,
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

    let otel_name = {
        let method = data.method.unwrap_or("HTTP");
        if let Some(target) = data.http_route {
            &format!("{method} {target}")
        } else {
            method
        }
    };
    span.record("otel.name", otel_name);

    for (header_name, header_value) in data.headers.iter() {
        let attribute_value = if header_value.is_sensitive() {
            Some("Sensitive")
        } else {
            header_value.to_str().ok()
        };
        if let Some(attribute_value) = attribute_value {
            let attribute_name = format!("http.request.header.{}", header_name);
            span.set_attribute(attribute_name, attribute_value.to_owned());
        }
    }

    if let Some(query) = data.url.query() {
        span.record("url.query", query);
    }

    match kind {
        sealed::SpanKind::Client => {
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
        }
        sealed::SpanKind::Server => {
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
        }
    }

    match kind {
        sealed::SpanKind::Client => {
            let context = span.context();
            opentelemetry::global::get_text_map_propagator(|injector| {
                injector.inject_context(&context, &mut HeaderInjector(request.headers_mut()));
            });
        }
        sealed::SpanKind::Server => {
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
fn record_response(span: &Span, kind: sealed::SpanKind, status: StatusCode, headers: &HeaderMap) {
    span.record("http.response.status_code", status.as_u16() as i64);

    for (header_name, header_value) in headers.iter() {
        let attribute_value = if header_value.is_sensitive() {
            Some("Sensitive")
        } else {
            header_value.to_str().ok()
        };
        if let Some(attribute_value) = attribute_value {
            let attribute_name = format!("http.response.header.{}", header_name);
            span.set_attribute(attribute_name, attribute_value.to_owned());
        }
    }

    if let sealed::SpanKind::Client = kind {
        if status.is_client_error() {
            span.record("otel.status_code", "ERROR");
        }
    }
    if status.is_server_error() {
        span.record("otel.status_code", "ERROR");
    }
}

/// Records the error message.
fn record_error<E: Display>(span: &Span, err: &E) {
    span.record("otel.status_code", "ERROR");
    span.record("error.message", err.to_string());
}

/// Records a cancelled server request.
fn record_cancel(span: &Span, kind: sealed::SpanKind) {
    const CANCELLED_ERROR_TYPE: &str = "cancelled";

    if let sealed::SpanKind::Client = kind {
        return;
    }

    span.record("otel.status_code", "ERROR");
    span.record("error.type", CANCELLED_ERROR_TYPE);
}

pub(crate) mod sealed {
    use http::{HeaderMap, Response, StatusCode};
    use tracing::Span;

    use crate::util;

    /// Describes the relationship between the [`Span`] and the service producing the span.
    #[derive(Clone, Copy, Debug)]
    pub enum SpanKind {
        /// The span describes a request sent to some remote service.
        Client,
        /// The span describes the server-side handling of a request.
        Server,
    }

    impl SpanKind {
        pub fn as_str(self) -> &'static str {
            match self {
                SpanKind::Client => "client",
                SpanKind::Server => "server",
            }
        }
    }

    /// Transforms a response using its request span.
    pub trait ResponseMapper<Res> {
        /// Mapped response type.
        type Output;

        /// Maps the response, taking ownership of the request span.
        fn map_response(response: Res, span: Span) -> Self::Output;
    }

    /// Returns the response unchanged.
    #[non_exhaustive]
    pub struct Identity;

    impl<Res> ResponseMapper<Res> for Identity {
        type Output = Res;

        #[inline]
        fn map_response(response: Res, _span: Span) -> Self::Output {
            response
        }
    }

    /// Wraps the response body to record errors and early drops.
    #[non_exhaustive]
    pub struct UseTraceBody;

    impl<B> ResponseMapper<http::Response<B>> for UseTraceBody
    where
        B: http_body::Body,
    {
        type Output = http::Response<super::TraceBody<B>>;

        #[inline]
        fn map_response(response: http::Response<B>, span: Span) -> Self::Output {
            response.map(|body| super::TraceBody::new(body, span))
        }
    }

    /// Data extracted from an HTTP request used to build a tracing span.
    pub struct RequestSpanData<'r> {
        pub(crate) method: Option<&'static str>,
        pub(crate) version: Option<&'static str>,
        pub(crate) url: util::Uri<'r>,
        pub(crate) headers: &'r HeaderMap,
        pub(crate) server_address: Option<String>,
        pub(crate) server_port: Option<u16>,
        pub(crate) url_scheme: Option<String>,
        pub(crate) http_route: Option<&'r str>,
        pub(crate) client_address: Option<std::net::SocketAddr>,
    }

    pub trait HttpRequest {
        /// Extract the request data used to create span
        fn extract_span_data<'r>(&'r mut self, kind: SpanKind) -> RequestSpanData<'r>;

        /// Gets a mutable reference to the request headers, used for context injection.
        fn headers_mut(&mut self) -> &mut HeaderMap;
    }

    impl<B> HttpRequest for http::Request<B> {
        #[inline(always)]
        fn extract_span_data<'r>(&'r mut self, kind: SpanKind) -> RequestSpanData<'r> {
            match kind {
                SpanKind::Client => RequestSpanData {
                    method: util::http_method(self.method()),
                    version: util::http_version(self.version()),
                    url: util::Uri::Http(self.uri()),
                    headers: self.headers(),
                    server_address: None,
                    server_port: None,
                    url_scheme: None,
                    http_route: None,
                    client_address: None,
                },
                SpanKind::Server => {
                    let (server_address, url_scheme, server_port) = {
                        let attrs = util::HttpRequestAttributes::from_recv_headers(self.headers());
                        (
                            attrs.server_address.map(ToOwned::to_owned),
                            attrs.url_scheme.map(ToOwned::to_owned),
                            attrs.server_port,
                        )
                    };
                    RequestSpanData {
                        method: util::http_method(self.method()),
                        version: util::http_version(self.version()),
                        url: util::Uri::Http(self.uri()),
                        headers: self.headers(),
                        server_address,
                        server_port,
                        url_scheme,
                        http_route: util::http_route_from_extensions(self.extensions()),
                        client_address: util::client_address_from_extensions(self.extensions())
                            .copied(),
                    }
                }
            }
        }

        #[inline(always)]
        fn headers_mut(&mut self) -> &mut HeaderMap {
            http::Request::headers_mut(self)
        }
    }

    pub trait HttpResponse {
        /// Returns the HTTP status code of the response.
        fn status(&self) -> StatusCode;

        /// Returns the HTTP headers of the response.
        fn headers(&self) -> &HeaderMap;
    }

    impl<B> HttpResponse for Response<B> {
        #[inline(always)]
        fn status(&self) -> StatusCode {
            Response::status(self)
        }

        #[inline(always)]
        fn headers(&self) -> &HeaderMap {
            Response::headers(self)
        }
    }
}
