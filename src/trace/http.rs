//! Middleware that adds tracing to a [`Service`] that handles HTTP requests.

#[cfg(feature = "reqwest_013")]
mod reqwest;

use std::{
    fmt::Display,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{ready, Context, Poll},
};

use pin_project::pin_project;
use tower_layer::Layer;
use tower_service::Service;
use tracing::{Level, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use super::{OnError, OnRequest, OnResponse, SpanHandler};

/// [`Layer`] that adds tracing to a [`Service`] that handles HTTP requests.
#[derive(Clone, Debug)]
pub struct HttpLayer<H = DefaultSpanHandler> {
    handler: Arc<H>,
}

impl HttpLayer {
    /// [`Span`] are constructed at the given level from server side.
    pub fn server(level: Level) -> Self {
        Self::new(DefaultSpanHandler::server(level))
    }

    /// [`Span`] are constructed at the given level from client side.
    pub fn client(level: Level) -> Self {
        Self::new(DefaultSpanHandler::client(level))
    }
}

impl<H> HttpLayer<H> {
    /// Customize how to handle [`Span`] from request to response.
    pub fn new(handler: H) -> Self {
        Self {
            handler: Arc::new(handler),
        }
    }
}

impl<S, H> Layer<S> for HttpLayer<H> {
    type Service = Http<S, H>;

    fn layer(&self, inner: S) -> Self::Service {
        Http {
            inner,
            handler: self.handler.clone(),
        }
    }
}

/// Middleware that adds tracing to a [`Service`] that handles HTTP requests.
#[derive(Clone, Debug)]
pub struct Http<S, H = DefaultSpanHandler> {
    inner: S,
    handler: Arc<H>,
}

impl<S, H, Req, Res> Service<Req> for Http<S, H>
where
    S: Service<Req, Response = Res>,
    H: SpanHandler<Req, S::Response, S::Error>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future, H>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Req) -> Self::Future {
        let span = self.handler.make_span(&mut req);
        let inner = {
            let _enter = span.enter();
            self.inner.call(req)
        };

        ResponseFuture {
            inner,
            span,
            handler: self.handler.clone(),
        }
    }
}

/// Response future for [`Http`].
#[pin_project]
pub struct ResponseFuture<F, H> {
    #[pin]
    inner: F,
    span: Span,
    handler: Arc<H>,
}

impl<F, H, Res, E> Future for ResponseFuture<F, H>
where
    F: Future<Output = Result<Res, E>>,
    H: OnResponse<Res> + OnError<E>,
{
    type Output = Result<Res, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _enter = this.span.enter();

        match ready!(this.inner.poll(cx)) {
            Ok(response) => {
                this.handler.record_response(this.span, &response);
                Poll::Ready(Ok(response))
            }
            Err(err) => {
                this.handler.record_error(this.span, &err);
                Poll::Ready(Err(err))
            }
        }
    }
}

/// Abstraction over HTTP requests that can be used by the middleware.
pub trait HttpRequest: sealed::HttpRequest {}

impl<B> HttpRequest for http::Request<B> {}

/// Abstraction over HTTP responses that can be used by the middleware.
pub trait HttpResponse: sealed::HttpResponse {}

impl<B> HttpResponse for http::Response<B> {}

/// Default implementation of [`SpanHandler`] trait for HTTP services.
#[derive(Clone, Debug)]
pub struct DefaultSpanHandler {
    level: Level,
    kind: sealed::SpanKind,
}

impl DefaultSpanHandler {
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
}

impl<Req> OnRequest<Req> for DefaultSpanHandler
where
    Req: HttpRequest,
{
    fn make_span(&self, request: &mut Req) -> Span {
        let Self { kind, level } = *self;

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
                    "http.request.method" = data.method,
                    "http.response.status_code" = Empty,
                    "network.protocol.name" = "http",
                    "network.protocol.version" = data.version,
                    "otel.kind" = self.kind.as_str(),
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

        #[cfg(feature = "propagate")]
        match kind {
            sealed::SpanKind::Client => {
                use crate::trace::propagate::HeaderInjector;

                let context = span.context();
                opentelemetry::global::get_text_map_propagator(|injector| {
                    injector.inject_context(&context, &mut HeaderInjector(request.headers_mut()));
                });
            }
            sealed::SpanKind::Server => {
                use crate::trace::propagate::HeaderExtractor;

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
}

impl<Res> OnResponse<Res> for DefaultSpanHandler
where
    Res: HttpResponse,
{
    fn record_response(&self, span: &Span, response: &Res) {
        let Self { kind, .. } = *self;
        let status = response.status();
        let headers = response.headers();

        span.record("http.response.status_code", status.as_u16() as i64);

        for (header_name, header_value) in headers.iter() {
            if let Ok(attribute_value) = header_value.to_str() {
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
}

impl<E> OnError<E> for DefaultSpanHandler
where
    E: Display,
{
    fn record_error(&self, span: &Span, err: &E) {
        span.record("otel.status_code", "ERROR");
        span.record("error.message", err.to_string());
    }
}

pub(crate) mod sealed {
    use http::{HeaderMap, Response, StatusCode};

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

    /// Data extracted from an HTTP request used to build a tracing span.
    pub struct RequestSpanData<'r> {
        pub(crate) method: &'static str,
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
