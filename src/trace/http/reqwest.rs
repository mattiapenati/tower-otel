use std::{
    fmt::Display,
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use pin_project::pin_project;
use tower_service::Service;
use tracing::{Level, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use super::{record_error, span_kind, Http, SpanKind};
use crate::{trace::injector::HeaderInjector, util};

impl<S> Service<reqwest::Request> for Http<S>
where
    S: Service<reqwest::Request, Response = reqwest::Response>,
    S::Error: Display,
{
    type Response = reqwest::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: reqwest::Request) -> Self::Future {
        debug_assert!(
            matches!(self.kind, SpanKind::Client),
            "Http middleware with reqwest::Request only supports client-side spans"
        );
        let span = make_span(self.level, self.kind, &mut req);
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

/// Response future for [`Http`] tracing middleware when wrapping a reqwest service.
#[pin_project]
pub struct ResponseFuture<F> {
    #[pin]
    inner: F,
    span: Span,
    kind: SpanKind,
}

impl<F, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<reqwest::Response, E>>,
    E: Display,
{
    type Output = Result<reqwest::Response, E>;

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

fn make_span(level: Level, kind: SpanKind, request: &mut reqwest::Request) -> Span {
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
                "server.address" = Empty,
                "server.port" = Empty,
                "url.full" = Empty,
                "url.path" = request.url().path(),
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

    if let Some(query) = request.url().query() {
        span.record("url.query", query);
    }

    span.record("url.full", request.url().as_str());

    if let Some(server_address) = request.url().host_str() {
        span.record("server.address", server_address);
    }
    if let Some(server_port) = request.url().port_or_known_default() {
        span.record("server.port", server_port as i64);
    }
    span.record("url.scheme", request.url().scheme());

    let context = span.context();
    opentelemetry::global::get_text_map_propagator(|injector| {
        injector.inject_context(&context, &mut HeaderInjector(request.headers_mut()));
    });

    span
}

fn record_response(span: &Span, kind: SpanKind, response: &reqwest::Response) {
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
