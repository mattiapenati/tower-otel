//! Middleware that adds tracing to a [`Service`] that handles gRPC requests.

use std::{
    fmt::Display,
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use http::{Request, Response};
use opentelemetry_http::{HeaderExtractor, HeaderInjector};
use pin_project::pin_project;
use tower_layer::Layer;
use tower_service::Service;
use tracing::{Level, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Describes the relationship between the [`Span`] and the service producing the span.
#[derive(Clone, Copy, Debug)]
enum SpanKind {
    /// The span describes a request sent to some remote service.
    Client,
    /// The span describes the server-side handling of a request.
    Server,
}

/// [`Layer`] that adds tracing to a [`Service`] that handles gRRC requests.
#[derive(Clone, Debug)]
pub struct GrpcLayer {
    level: Level,
    kind: SpanKind,
}

impl GrpcLayer {
    /// [`Span`]s are constructed at the given level from server side.
    pub fn server(level: Level) -> Self {
        Self {
            level,
            kind: SpanKind::Server,
        }
    }

    /// [`Span`]s are constructed at the given level from client side.
    pub fn client(level: Level) -> Self {
        Self {
            level,
            kind: SpanKind::Client,
        }
    }
}

impl<S> Layer<S> for GrpcLayer {
    type Service = Grpc<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Grpc {
            inner,
            level: self.level,
            kind: self.kind,
        }
    }
}

/// Middleware that adds tracing to a [`Service`] that handles gRPC requests.
#[derive(Clone, Debug)]
pub struct Grpc<S> {
    inner: S,
    level: Level,
    kind: SpanKind,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for Grpc<S>
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

        ResponseFuture { inner, span }
    }
}

/// Response future for [`Grpc`].
#[pin_project]
pub struct ResponseFuture<F> {
    #[pin]
    inner: F,
    span: Span,
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
                record_response(this.span, &response);
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
                "GRPC",
                "error.message" = Empty,
                "otel.kind" = Empty,
                "otel.name" = Empty,
                "otel.status_code" = Empty,
                "rpc.grpc.status_code" = Empty,
                "rpc.method" = Empty,
                "rpc.service" = Empty,
                "rpc.system" = "grpc",
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
            let attribute_name = format!("rpc.grpc.request.metadata.{}", header_name);
            span.set_attribute(attribute_name, attribute_value.to_owned());
        }
    }

    span.record("otel.kind", span_kind(kind));

    let path = request.uri().path();
    let name = path.trim_start_matches('/');
    span.record("otel.name", name);
    if let Some((service, method)) = name.split_once('/') {
        span.record("rpc.service", service);
        span.record("rpc.method", method);
    }

    match kind {
        SpanKind::Client => {
            let context = span.context();
            opentelemetry::global::get_text_map_propagator(|injector| {
                injector.inject_context(&context, &mut HeaderInjector(request.headers_mut()));
            });
        }
        SpanKind::Server => {
            let context = opentelemetry::global::get_text_map_propagator(|extractor| {
                extractor.extract(&HeaderExtractor(request.headers_mut()))
            });
            span.set_parent(context);
        }
    }

    span
}

/// Records fields associated to the response.
fn record_response<B>(span: &Span, response: &Response<B>) {
    for (header_name, header_value) in response.headers().iter() {
        if let Ok(attribute_value) = header_value.to_str() {
            let attribute_name = format!("rpc.grpc.response.metadata.{}", header_name);
            span.set_attribute(attribute_name, attribute_value.to_owned());
        }
    }

    if let Some(header_value) = response.headers().get("grpc-status") {
        if let Ok(header_value) = header_value.to_str() {
            if let Ok(status_code) = header_value.parse::<i32>() {
                span.record("rpc.grpc.status_code", status_code);
            }
        }
    } else {
        span.record("rpc.grpc.status_code", 0);
    }
}

/// Records the error message.
fn record_error<E: Display>(span: &Span, err: &E) {
    span.record("otel.status_code", "ERROR");
    span.record("error.message", err.to_string());
}
