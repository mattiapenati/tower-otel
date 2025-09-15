//! Middleware that adds tracing to a [`Service`] that handles HTTP requests.

use std::{
    fmt::Display,
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use http::{Request, Response};
use pin_project::pin_project;
use tower_layer::Layer;
use tower_service::Service;
use tracing::{Level, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::{
    trace::{extractor::HeaderExtractor, injector::HeaderInjector},
    util,
};

/// Describes the relationship between the [`Span`] and the service producing the span.
#[derive(Clone, Copy, Debug)]
enum SpanKind {
    /// The span describes a request sent to some remote service.
    Client,
    /// The span describes the server-side handling of a request.
    Server,
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

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        let span = make_request_span(self.level, self.kind, &mut req);
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
                record_response(this.span, *this.kind, &response);
                Poll::Ready(Ok(response))
            }
            Err(err) => {
                record_error(this.span, &err);
                Poll::Ready(Err(err))
            }
        }
    }
}

/// String representation of span kind
fn span_kind(kind: SpanKind) -> &'static str {
    match kind {
        SpanKind::Client => "client",
        SpanKind::Server => "server",
    }
}

/// Creates a new [`Span`] for the given request.
fn make_request_span<B>(level: Level, kind: SpanKind, request: &mut Request<B>) -> Span {
    macro_rules! make_span {
        ($level:expr) => {{
            use tracing::field::Empty;

            tracing::span!(
                $level,
                "HTTP",
                "error.message" = Empty,
                "http.request.method" = util::http_method(request.method()),
                "http.response.status_code" = Empty,
                "network.protocol.name" = "http",
                "network.protocol.version" = util::http_version(request.version()),
                "otel.kind" = span_kind(kind),
                "otel.status_code" = Empty,
                "url.full" = Empty,
                "url.path" = request.uri().path(),
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

    for (header_name, header_value) in request.headers().iter() {
        if let Ok(attribute_value) = header_value.to_str() {
            let attribute_name = format!("http.request.header.{}", header_name);
            span.set_attribute(attribute_name, attribute_value.to_owned());
        }
    }

    if let Some(query) = request.uri().query() {
        span.record("url.query", query);
    }

    match kind {
        SpanKind::Client => {
            span.record("url.full", tracing::field::display(request.uri()));

            if let Some(url_scheme) = request.uri().scheme_str() {
                span.record("url.scheme", url_scheme);
            }

            let context = span.context();
            opentelemetry::global::get_text_map_propagator(|injector| {
                injector.inject_context(&context, &mut HeaderInjector(request.headers_mut()));
            });
        }
        SpanKind::Server => {
            if let Some(http_route) = util::http_route(request) {
                span.record("http.route", http_route);
            }

            if let Some(url_scheme) = util::http_url_scheme(request) {
                span.record("url.scheme", url_scheme);
            }

            let context = opentelemetry::global::get_text_map_propagator(|extractor| {
                extractor.extract(&HeaderExtractor(request.headers_mut()))
            });
            span.set_parent(context);
        }
    }

    span
}

/// Records fields associated to the response.
fn record_response<B>(span: &Span, kind: SpanKind, response: &Response<B>) {
    span.record(
        "http.response.status_code",
        response.status().as_u16() as i64,
    );

    for (header_name, header_value) in response.headers().iter() {
        if let Ok(attribute_value) = header_value.to_str() {
            let attribute_name = format!("http.response.header.{}", header_name);
            span.set_attribute(attribute_name, attribute_value.to_owned());
        }
    }

    if let SpanKind::Client = kind {
        if response.status().is_client_error() {
            span.record("otel.status_code", "ERROR");
        }
    }
    if response.status().is_server_error() {
        span.record("otel.status_code", "ERROR");
    }
}

/// Records the error message.
fn record_error<E: Display>(span: &Span, err: &E) {
    span.record("otel.status_code", "ERROR");
    span.record("error.message", err.to_string());
}
